//! Several devices of one shop, at random (M11/M2): each works offline,
//! sends and takes in any order, sometimes sends twice (an answer lost).
//! In the end every copy is the server's, row for row; no operation is lost
//! or counted twice; every entry is balanced; stock is what its movements
//! say.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nb_core::FixedClock;
use nb_server::{
    CURSOR_HEADER, EPOCH_HEADER, Invite, JoinRequest, Joined, Linked, Position, PullResult,
    PushRequest, PushResult, router,
};
use nb_store::usecases::customers::{CustomerInput, save_customer};
use nb_store::usecases::debts::{ReceiptInput, collect};
use nb_store::usecases::expenses::{ExpenseInput, expense_categories, record_expense};
use nb_store::usecases::products::{Opening, ProductInput, create_product};
use nb_store::usecases::returns::{ReturnInput, ReturnLineInput, return_sale};
use nb_store::usecases::sales::{
    PaymentInput, SaleInput, SaleLineInput, complete_sale, open_shift,
};
use nb_store::usecases::setup::{SetupInput, SetupResult, setup_organization};
use nb_store::usecases::stock::{AdjustInput, adjust_stock};
use nb_store::{Error, OpMeta, Store, StoreOptions};
use tower::ServiceExt;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const KEY: &str = "enroll-key-0123456789";

/// How many times a device took the server's copy because its own had
/// come out different.
static HEALED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A small reproducible random source.
struct Rng(u64);

impl Rng {
    fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

async fn request(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Vec<u8>,
) -> Res<axum::response::Response> {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    Ok(app.clone().oneshot(req.body(Body::from(body))?).await?)
}

async fn ok(res: axum::response::Response) -> Res<Vec<u8>> {
    let status = res.status();
    let bytes = res.into_body().collect().await?.to_bytes().to_vec();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    Ok(bytes)
}

struct Device {
    store: Store,
    token: String,
    /// The last batch it sent, to send again (an answer lost).
    last_sent: Vec<nb_store::ops::LoggedOp>,
}

async fn copy(app: &axum::Router, token: &str) -> Res<(Vec<u8>, Position)> {
    let res = request(app, "GET", "/v1/snapshot", token, Vec::new()).await?;
    let num =
        |n: &str| -> Res<i64> { Ok(res.headers().get(n).ok_or("position")?.to_str()?.parse()?) };
    let at = Position {
        cursor: num(CURSOR_HEADER)?,
        epoch: num(EPOCH_HEADER)?,
    };
    Ok((ok(res).await?, at))
}

async fn push(app: &axum::Router, d: &mut Device, rng: &mut Rng) -> Res<String> {
    let ops = d.store.unpushed(1 + rng.next(8) as u32)?;
    d.last_sent = ops.clone();
    let body = serde_json::to_vec(&PushRequest { ops })?;
    let bytes = ok(request(app, "POST", "/v1/push", &d.token, body).await?).await?;
    let r: PushResult = serde_json::from_slice(&bytes)?;
    assert!(r.failed.is_none(), "{:?}", r.failed);
    // Sometimes the answer is lost: the device does not learn it was taken.
    if rng.next(5) != 0 {
        d.store.mark_pushed(&r.accepted)?;
    }
    Ok(r.fingerprint)
}

async fn pull(app: &axum::Router, d: &mut Device) -> Res<PullResult> {
    loop {
        let at = d.store.pull_position()?;
        let path = format!("/v1/pull?after={}&epoch={}", at.cursor, at.epoch);
        let bytes = ok(request(app, "GET", &path, &d.token, Vec::new()).await?).await?;
        let r: PullResult = serde_json::from_slice(&bytes)?;
        if r.epoch != at.epoch {
            let (db, at) = copy(app, &d.token).await?;
            d.store.resync(&db, at)?;
            continue;
        }
        d.store.apply_pulled(r.ops.clone(), r.cursor)?;
        if !r.more {
            // As the app does: a copy that ended up different (the order
            // ops ran in mattered) takes the server's.
            if d.store.unpushed_count()? == 0 && d.store.fingerprint()? != r.fingerprint {
                HEALED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if std::env::var("NB_SIM_STRICT").is_ok() {
                    let mine = d.store.fingerprints()?;
                    let (db, _) = copy(app, &d.token).await?;
                    let path = std::env::temp_dir()
                        .join(format!("nb-sim-{}.sqlite", nb_core::OperationId::new()));
                    std::fs::write(&path, &db)?;
                    let theirs = Store::open(&path, StoreOptions::default())?.fingerprints()?;
                    let differ: Vec<_> = mine
                        .iter()
                        .zip(&theirs)
                        .filter(|(a, b)| a != b)
                        .map(|(a, _)| a.0.clone())
                        .collect();
                    panic!("a copy differs from the server's in {differ:?}");
                }
                let (db, at) = copy(app, &d.token).await?;
                d.store.resync(&db, at)?;
            }
            return Ok(r);
        }
    }
}

/// What a device may refuse knowing what it knows (an empty shelf, a
/// credit limit, a sale already returned): not an error of the test.
fn soft<T>(r: Result<T, Error>) -> Result<bool, Error> {
    match r {
        Ok(_) => Ok(true),
        Err(Error::Rules(_) | Error::Forbidden { .. } | Error::NotFound(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

fn ids(s: &Store, sql: &str) -> Result<Vec<String>, Error> {
    s.read(|r| {
        let mut st = r.conn().prepare(sql)?;
        let ids = st
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(ids)
    })
}

fn register(s: &Store) -> Result<String, Error> {
    Ok(s.read(|r| {
        let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
        nb_store::usecases::org::registers(r.conn(), o.org_id()?)
    })?
    .remove(0)
    .id)
}

fn products(s: &Store) -> Result<Vec<String>, Error> {
    s.read(|r| {
        let mut st = r
            .conn()
            .prepare("SELECT id FROM products WHERE kind = 'goods' ORDER BY id")?;
        let ids = st
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(ids)
    })
}

/// What must hold on any copy.
fn sound(s: &Store) -> Res<()> {
    let q = |sql: &str| -> Result<i64, Error> {
        s.read(|r| Ok(r.conn().query_row(sql, [], |row| row.get(0))?))
    };
    assert_eq!(
        q(
            "SELECT COUNT(*) FROM (SELECT entry_id FROM journal_lines GROUP BY entry_id
           HAVING SUM(debit_minor) <> SUM(credit_minor))"
        )?,
        0,
        "every entry balanced"
    );
    assert_eq!(
        q(
            "SELECT COUNT(*) FROM stock_balances b WHERE on_hand_milli <>
           (SELECT COALESCE(SUM(qty_milli), 0) FROM stock_movements m
            WHERE m.product_id = b.product_id AND m.warehouse_id = b.warehouse_id)"
        )?,
        0,
        "stock is its movements"
    );
    assert_eq!(
        q("SELECT COUNT(*) - COUNT(DISTINCT number) FROM sales")?,
        0,
        "no sale number twice"
    );
    Ok(())
}

async fn scenario(seed: u64, devices: usize, steps: usize) -> Res<()> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let dir = std::env::temp_dir().join(format!("nb-sim-{}", nb_core::OperationId::new()));
    let app = router(nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?);
    let clock = Arc::new(FixedClock::new(1_794_304_800_000));
    let options = || StoreOptions {
        clock: clock.clone(),
        ..Default::default()
    };

    let mut first = Store::open_in_memory(options())?;
    let setup: SetupResult = setup_organization(
        &mut first,
        SetupInput {
            org_name: "Superette".into(),
            activity: "grocery".into(),
            country: "DZ".into(),
            currency: "DZD".into(),
            currency_exp: 2,
            branch_name: "Biskra".into(),
            owner_name: "Ahmed".into(),
            language: "fr".into(),
            fiscal_year_start: 1,
        },
    )?;
    let meta = |kind: &'static str| OpMeta::new(kind, Some(setup.org), Some(setup.owner));
    for (i, cost) in [(0, 30_000), (1, 1_000), (2, 8_000)] {
        create_product(
            &mut first,
            meta("p"),
            ProductInput {
                name: format!("Article {i}"),
                kind: "goods".into(),
                base_unit: "pc".into(),
                price_minor: cost * 3 / 2,
                opening: Some(Opening {
                    warehouse_id: setup.warehouse.to_string(),
                    qty: "20".into(),
                    unit_cost_minor: cost,
                    tracked: Default::default(),
                }),
                ..Default::default()
            },
        )?;
    }
    let reg = register(&first)?;
    open_shift(&mut first, meta("shift"), &reg, 10_000)?;
    let linked: Linked = serde_json::from_slice(
        &ok(request(&app, "POST", "/v1/link", KEY, first.snapshot()?).await?).await?,
    )?;
    first.set_sync_link("http://server", &linked.token)?;
    let mut all = vec![Device {
        store: first,
        token: linked.token.clone(),
        last_sent: Vec::new(),
    }];
    for k in 1..devices {
        let invite: Invite = serde_json::from_slice(
            &ok(request(&app, "POST", "/v1/invite", &all[0].token, Vec::new()).await?).await?,
        )?;
        let body = serde_json::to_vec(&JoinRequest {
            code: invite.code,
            name: format!("Caisse {k}"),
        })?;
        let joined: Joined =
            serde_json::from_slice(&ok(request(&app, "POST", "/v1/join", "", body).await?).await?)?;
        let (db, at) = copy(&app, &joined.token).await?;
        let mut store = Store::open_in_memory(options())?;
        store.join_with(&db, at, &joined, "http://server")?;
        all.push(Device {
            store,
            token: joined.token,
            last_sent: Vec::new(),
        });
    }

    let mut made = 0usize;
    for step in 0..steps {
        let d = rng.next(devices);
        let dev = &mut all[d];
        let wh = setup.warehouse.to_string();
        let done = match rng.next(16) {
            // Selling (most of the day), sometimes more than what is left,
            // sometimes on credit to a customer this device knows.
            0..=5 => {
                let items = products(&dev.store)?;
                let lines = (0..1 + rng.next(2))
                    .map(|_| SaleLineInput {
                        product_id: items[rng.next(items.len())].clone(),
                        qty: format!("{}", 1 + rng.next(4)),
                        ..Default::default()
                    })
                    .collect();
                let people = ids(&dev.store, "SELECT id FROM customers ORDER BY id")?;
                let credit = !people.is_empty() && rng.next(3) == 0;
                soft(complete_sale(
                    &mut dev.store,
                    meta("sale"),
                    SaleInput {
                        register_id: reg.clone(),
                        warehouse_id: wh.clone(),
                        customer_id: credit.then(|| people[rng.next(people.len())].clone()),
                        lines,
                        payments: if credit {
                            Vec::new()
                        } else {
                            vec![PaymentInput {
                                method: "cash".into(),
                                amount_minor: 10_000_000,
                            }]
                        },
                        ..Default::default()
                    },
                ))?
            }
            6 => soft(save_customer(
                &mut dev.store,
                meta("c"),
                CustomerInput {
                    kind: "person".into(),
                    name: format!("Client {step}"),
                    credit_limit_minor: 5_000_000,
                    status: "active".into(),
                    ..Default::default()
                },
            ))?,
            7 => soft(create_product(
                &mut dev.store,
                meta("p"),
                ProductInput {
                    name: format!("Nouveau {step}"),
                    kind: "goods".into(),
                    base_unit: "pc".into(),
                    price_minor: 5_000,
                    opening: Some(Opening {
                        warehouse_id: wh.clone(),
                        qty: "5".into(),
                        unit_cost_minor: 2_000 + step as i64,
                        tracked: Default::default(),
                    }),
                    ..Default::default()
                },
            ))?,
            8 => {
                let people = ids(&dev.store, "SELECT id FROM customers ORDER BY id")?;
                if people.is_empty() {
                    false
                } else {
                    soft(collect(
                        &mut dev.store,
                        meta("receipt"),
                        ReceiptInput {
                            customer_id: people[rng.next(people.len())].clone(),
                            amount_minor: 100 * (1 + rng.next(50)) as i64,
                            method: "cash".into(),
                            register_id: Some(reg.clone()),
                            unallocated: true,
                            ..Default::default()
                        },
                    ))?
                }
            }
            9 => {
                let items = products(&dev.store)?;
                soft(adjust_stock(
                    &mut dev.store,
                    meta("adjust"),
                    AdjustInput {
                        product_id: items[rng.next(items.len())].clone(),
                        warehouse_id: wh.clone(),
                        kind: ["adjust_in", "adjust_out", "waste"][rng.next(3)].into(),
                        qty: (1 + rng.next(4)).to_string(),
                        unit: "pc".into(),
                        unit_cost_minor: None,
                        reason: "sim".into(),
                        may_go_negative: false,
                        approval: None,
                        tracked: Default::default(),
                    },
                ))?
            }
            10 => {
                let sales = ids(&dev.store, "SELECT id FROM sales ORDER BY id")?;
                if sales.is_empty() {
                    false
                } else {
                    soft(return_sale(
                        &mut dev.store,
                        meta("return"),
                        ReturnInput {
                            sale: sales[rng.next(sales.len())].clone(),
                            register_id: reg.clone(),
                            lines: vec![ReturnLineInput {
                                position: 1,
                                qty: "1".into(),
                                restock: rng.next(2) == 0,
                            }],
                            refund: "cash".into(),
                            reason: "sim".into(),
                            approval: None,
                        },
                    ))?
                }
            }
            11 => {
                let cat = expense_categories(&dev.store)?
                    .into_iter()
                    .find(|c| c.kind == "expense")
                    .map(|c| c.id)
                    .unwrap_or_default();
                soft(record_expense(
                    &mut dev.store,
                    meta("expense"),
                    ExpenseInput {
                        category_id: cat,
                        total_minor: 1_000 * (1 + rng.next(20)) as i64,
                        vat_minor: 0,
                        method: "cash".into(),
                        register_id: Some(reg.clone()),
                        payee: String::new(),
                        reference: String::new(),
                        note: String::new(),
                    },
                ))?
            }
            12 | 13 => {
                push(&app, dev, &mut rng).await?;
                false
            }
            _ => {
                if rng.next(3) == 0 && !dev.last_sent.is_empty() {
                    // The same batch again.
                    let body = serde_json::to_vec(&PushRequest {
                        ops: dev.last_sent.clone(),
                    })?;
                    ok(request(&app, "POST", "/v1/push", &dev.token, body).await?).await?;
                }
                pull(&app, dev).await?;
                false
            }
        };
        made += usize::from(done);
        clock.advance(1 + rng.next(5_000) as i64);
    }

    // Everyone online: send everything, then take everything.
    for dev in all.iter_mut() {
        while dev.store.unpushed_count()? > 0 {
            push(&app, dev, &mut rng).await?;
        }
    }
    let mut last = String::new();
    for dev in all.iter_mut() {
        last = pull(&app, dev).await?.fingerprint;
    }
    let (db, _) = copy(&app, &all[0].token).await?;
    let path = dir.join("server-copy.sqlite");
    std::fs::write(&path, db)?;
    let server_copy = Store::open(&path, StoreOptions::default())?;
    let theirs = server_copy.fingerprints()?;
    for (k, dev) in all.iter().enumerate() {
        let mine = dev.store.fingerprints()?;
        let differ: Vec<&str> = mine
            .iter()
            .zip(&theirs)
            .filter(|(a, b)| a != b)
            .map(|(a, _)| a.0.as_str())
            .collect();
        if !differ.is_empty() {
            if std::env::var("NB_SIM_DUMP").is_ok() {
                let out = std::env::temp_dir().join(format!("nb-sim-dev{k}.sqlite"));
                std::fs::write(&out, dev.store.snapshot()?)?;
                std::fs::copy(&path, std::env::temp_dir().join("nb-sim-server.sqlite"))?;
            }
            panic!("seed {seed}: device {k} differs in {differ:?}");
        }
        assert_eq!(
            dev.store.fingerprint()?,
            last,
            "seed {seed}: device {k} differs"
        );
        sound(&dev.store)?;
    }
    let operations = |s: &Store| -> Result<i64, Error> {
        s.read(|r| {
            Ok(r.conn().query_row(
                "SELECT COUNT(*) FROM operations WHERE call IS NOT NULL",
                [],
                |row| row.get(0),
            )?)
        })
    };
    let n = operations(&all[0].store)?;
    for dev in &all {
        assert_eq!(
            operations(&dev.store)?,
            n,
            "seed {seed}: an operation lost or doubled"
        );
    }
    assert!(n as usize >= made, "seed {seed}: {n} logged < {made} made");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[tokio::test]
async fn devices_at_random_meet_in_the_same_state() -> Res<()> {
    let runs: u64 = std::env::var("NB_SIM_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    for seed in 1..=runs {
        scenario(seed, 2 + (seed % 4) as usize, 160).await?;
    }
    eprintln!(
        "{runs} scenarios; copies taken again after a different result: {}",
        HEALED.load(std::sync::atomic::Ordering::Relaxed)
    );
    Ok(())
}

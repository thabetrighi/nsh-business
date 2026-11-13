//! Two devices of one shop (M11/M2): the second joins with a code from the
//! first, both sell the same goods while offline, then each sends what it
//! did and takes what the other did. All three copies end the same; the
//! goods sold twice show as a shortfall to review, not as a refused sale.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nb_core::FixedClock;
use nb_server::{
    CURSOR_HEADER, EPOCH_HEADER, Invite, JoinRequest, Joined, Linked, Position, PullResult,
    PushRequest, PushResult, StateInfo, router,
};
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::customers::{CustomerInput, save_customer};
use nb_store::usecases::products::{Opening, ProductInput, create_product};
use nb_store::usecases::sales::{
    PaymentInput, SaleInput, SaleLineInput, complete_sale, open_shift,
};
use nb_store::usecases::setup::{SetupInput, SetupResult, setup_organization};
use nb_store::{Error, OpMeta, Store, StoreOptions};
use tower::ServiceExt;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const KEY: &str = "enroll-key-0123456789";

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Vec<u8>,
) -> Res<(StatusCode, Vec<u8>)> {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    let res = app.clone().oneshot(req.body(Body::from(body))?).await?;
    let status = res.status();
    let bytes = res.into_body().collect().await?.to_bytes().to_vec();
    assert!(
        status.is_success(),
        "{method} {path}: {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    Ok((status, bytes))
}

async fn push(app: &axum::Router, token: &str, s: &mut Store) -> Res<PushResult> {
    let body = serde_json::to_vec(&PushRequest {
        ops: s.unpushed(500)?,
    })?;
    let (_, bytes) = call(app, "POST", "/v1/push", token, body).await?;
    let r: PushResult = serde_json::from_slice(&bytes)?;
    s.mark_pushed(&r.accepted)?;
    Ok(r)
}

/// The server's database and the position it is at.
async fn copy(app: &axum::Router, token: &str) -> Res<(Vec<u8>, Position)> {
    let req = Request::builder()
        .method("GET")
        .uri("/v1/snapshot")
        .header("authorization", format!("Bearer {token}"));
    let res = app.clone().oneshot(req.body(Body::empty())?).await?;
    assert_eq!(res.status(), StatusCode::OK);
    let header = |name: &str| -> Res<i64> {
        Ok(res
            .headers()
            .get(name)
            .ok_or("no position")?
            .to_str()?
            .parse()?)
    };
    let at = Position {
        cursor: header(CURSOR_HEADER)?,
        epoch: header(EPOCH_HEADER)?,
    };
    Ok((res.into_body().collect().await?.to_bytes().to_vec(), at))
}

async fn pull(app: &axum::Router, token: &str, s: &mut Store) -> Res<PullResult> {
    loop {
        let at = s.pull_position()?;
        let (_, bytes) = call(
            app,
            "GET",
            &format!("/v1/pull?after={}&epoch={}", at.cursor, at.epoch),
            token,
            Vec::new(),
        )
        .await?;
        let r: PullResult = serde_json::from_slice(&bytes)?;
        if r.epoch != at.epoch {
            // Another device sent its whole database: take it again.
            let (db, at) = copy(app, token).await?;
            s.resync(&db, at)?;
            continue;
        }
        s.apply_pulled(r.ops.clone(), r.cursor)?;
        if !r.more {
            return Ok(r);
        }
    }
}

fn sell(s: &mut Store, setup: &SetupResult, reg: &str, product: &str, qty: &str) -> Res<String> {
    let sale = complete_sale(
        s,
        OpMeta::new("sale", Some(setup.org), Some(setup.owner)),
        SaleInput {
            register_id: reg.into(),
            warehouse_id: setup.warehouse.to_string(),
            lines: vec![SaleLineInput {
                product_id: product.into(),
                qty: qty.into(),
                ..Default::default()
            }],
            payments: vec![PaymentInput {
                method: "cash".into(),
                amount_minor: 1_000_000,
            }],
            ..Default::default()
        },
    )?;
    Ok(sale.number)
}

fn register(s: &Store) -> Result<String, Error> {
    Ok(s.read(|r| {
        let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
        nb_store::usecases::org::registers(r.conn(), o.org_id()?)
    })?
    .remove(0)
    .id)
}

fn stock(s: &Store, product: &str) -> Result<i64, Error> {
    s.read(|r| {
        Ok(r.conn().query_row(
            "SELECT COALESCE(SUM(on_hand_milli), 0) FROM stock_balances WHERE product_id = ?1",
            [product],
            |row| row.get(0),
        )?)
    })
}

#[tokio::test]
async fn two_devices_sell_offline_and_meet() -> Res<()> {
    let dir = std::env::temp_dir().join(format!("nb-devices-test-{}", nb_core::OperationId::new()));
    let app = router(nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?);
    let clock = Arc::new(FixedClock::new(1_794_304_800_000));

    // Device A: the shop, 5 bags of coffee, linked.
    let mut a = Store::open_in_memory(StoreOptions {
        clock: clock.clone(),
        ..Default::default()
    })?;
    let setup = setup_organization(
        &mut a,
        SetupInput {
            org_name: "Magasin En-Nour".into(),
            activity: "grocery".into(),
            country: "DZ".into(),
            currency: "DZD".into(),
            currency_exp: 2,
            branch_name: "El Oued".into(),
            owner_name: "Ahmed".into(),
            language: "fr".into(),
            fiscal_year_start: 1,
        },
    )?;
    let coffee = create_product(
        &mut a,
        OpMeta::new("p", Some(setup.org), Some(setup.owner)),
        ProductInput {
            name: "Café 250g".into(),
            kind: "goods".into(),
            base_unit: "pc".into(),
            tax_bps: 1_900,
            price_minor: 45_000,
            opening: Some(Opening {
                warehouse_id: setup.warehouse.to_string(),
                qty: "5".into(),
                unit_cost_minor: 30_000,
                tracked: Default::default(),
            }),
            ..Default::default()
        },
    )?
    .id;
    let reg = register(&a)?;
    open_shift(
        &mut a,
        OpMeta::new("shift", Some(setup.org), Some(setup.owner)),
        &reg,
        10_000,
    )?;
    let (_, bytes) = call(&app, "POST", "/v1/link", KEY, a.snapshot()?).await?;
    let linked: Linked = serde_json::from_slice(&bytes)?;
    a.set_sync_link("http://server", &linked.token)?;

    // Device B joins with a code from A: the server's copy, its own
    // identity and letter.
    let (_, bytes) = call(&app, "POST", "/v1/invite", &linked.token, Vec::new()).await?;
    let invite: Invite = serde_json::from_slice(&bytes)?;
    let (_, bytes) = call(
        &app,
        "POST",
        "/v1/join",
        "",
        serde_json::to_vec(&JoinRequest {
            code: invite.code.to_lowercase(),
            name: "Caisse 2".into(),
        })?,
    )
    .await?;
    let joined: Joined = serde_json::from_slice(&bytes)?;
    assert_eq!(joined.prefix, "B");
    let (db, at) = copy(&app, &joined.token).await?;
    let mut b = Store::open_in_memory(StoreOptions {
        clock: clock.clone(),
        ..Default::default()
    })?;
    b.join_with(&db, at, &joined, "http://server")?;
    assert_eq!(b.device().prefix, "B");
    assert_eq!(b.fingerprint()?, a.fingerprint()?);
    assert_eq!(b.unpushed_count()?, 0);
    // A code works once.
    let (status, _) = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/join")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&JoinRequest {
                    code: invite.code.clone(),
                    name: String::new(),
                })?))?,
        )
        .await
        .map(|r| (r.status(), ()))?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Offline: both sell 4 of the 5 bags, and both add a customer.
    let on_a = sell(&mut a, &setup, &reg, &coffee, "4")?;
    let on_b = sell(&mut b, &setup, &reg, &coffee, "4")?;
    assert_ne!(on_a, on_b, "two devices never give the same number");
    for (s, name) in [(&mut a, "Karim"), (&mut b, "Nadia")] {
        save_customer(
            s,
            OpMeta::new("c", Some(setup.org), Some(setup.owner)),
            CustomerInput {
                kind: "person".into(),
                name: name.into(),
                status: "active".into(),
                ..Default::default()
            },
        )?;
    }

    // Back online: send, then take what the other did.
    let r = push(&app, &linked.token, &mut a).await?;
    assert!(r.failed.is_none());
    let r = push(&app, &joined.token, &mut b).await?;
    assert!(r.failed.is_none(), "a fact is not refused: {:?}", r.failed);
    let server_print = r.fingerprint;
    let pa = pull(&app, &linked.token, &mut a).await?;
    let pb = pull(&app, &joined.token, &mut b).await?;
    assert_eq!(
        a.unpushed_count()?,
        0,
        "what came from the server is not sent back"
    );
    assert_eq!(b.unpushed_count()?, 0);
    assert_eq!(a.fingerprint()?, server_print);
    assert_eq!(b.fingerprint()?, server_print);
    assert_eq!(pa.fingerprint, server_print);
    assert_eq!(pb.fingerprint, server_print);

    // Eight bags sold out of five: the three missing are to be looked at,
    // on every copy that learnt it late.
    assert_eq!(stock(&a, &coffee)?, -3_000);
    assert_eq!(stock(&b, &coffee)?, -3_000);
    for s in [&a, &b] {
        let reviews = s.open_reviews()?;
        assert!(
            reviews
                .iter()
                .any(|r| r.kind == "stock_shortfall" && r.entity_id == coffee),
            "{reviews:?}"
        );
    }

    // Nothing new: an empty pull; a new sale on B reaches A.
    let again = pull(&app, &linked.token, &mut a).await?;
    assert!(again.ops.is_empty());
    save_customer(
        &mut b,
        OpMeta::new("c", Some(setup.org), Some(setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: "Samir".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    push(&app, &joined.token, &mut b).await?;
    let got = pull(&app, &linked.token, &mut a).await?;
    assert_eq!(got.ops.len(), 1);
    assert_eq!(a.fingerprint()?, b.fingerprint()?);

    // Catching up from the server's copy keeps what was not sent yet.
    save_customer(
        &mut a,
        OpMeta::new("c", Some(setup.org), Some(setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: "Pas encore envoyé".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    let (db, at) = copy(&app, &linked.token).await?;
    let me = a.device().clone();
    a.resync(&db, at)?;
    assert_eq!(a.device().id, me.id);
    assert_eq!(a.unpushed_count()?, 1);
    push(&app, &linked.token, &mut a).await?;
    pull(&app, &joined.token, &mut b).await?;
    assert_eq!(a.fingerprint()?, b.fingerprint()?);

    // A PIN on B: its secret does not travel as an operation, so B sends
    // its database (a new epoch) — but only once it has A's operations.
    save_customer(
        &mut a,
        OpMeta::new("c", Some(setup.org), Some(setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: "Avant le PIN".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    push(&app, &linked.token, &mut a).await?;
    create_user(
        &mut b,
        OpMeta::new("user", Some(setup.org), Some(setup.owner)),
        UserInput {
            name: "Yacine".into(),
            phone: None,
            login: None,
            password: None,
            pin: Some("4826".into()),
            grants: vec![GrantRow::org("cashier")],
        },
    )?;
    let r = push(&app, &joined.token, &mut b).await?;
    assert_eq!(r.failed.map(|f| f.code).as_deref(), Some("opaque"));
    let reseed = |at: Position, db: Vec<u8>| {
        let app = app.clone();
        let token = joined.token.clone();
        async move {
            let req = Request::builder()
                .method("POST")
                .uri(format!("/v1/reseed?after={}&epoch={}", at.cursor, at.epoch))
                .header("authorization", format!("Bearer {token}"));
            let res = app.oneshot(req.body(Body::from(db))?).await?;
            let status = res.status();
            let bytes = res.into_body().collect().await?.to_bytes().to_vec();
            Res::Ok((status, bytes))
        }
    };
    let (status, _) = reseed(b.pull_position()?, b.snapshot()?).await?;
    assert_eq!(status, StatusCode::CONFLICT, "B has not A's last operation");
    pull(&app, &joined.token, &mut b).await?;
    let (status, bytes) = reseed(b.pull_position()?, b.snapshot()?).await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let state: StateInfo = serde_json::from_slice(&bytes)?;
    b.mark_all_pushed()?;
    b.set_pull_position(Position {
        cursor: state.cursor,
        epoch: state.epoch,
    })?;
    assert_eq!(state.fingerprint, b.fingerprint()?);

    // A, with something of its own not sent, meets the new epoch: it takes
    // the copy again and keeps its operation.
    save_customer(
        &mut a,
        OpMeta::new("c", Some(setup.org), Some(setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: "Pendant la copie".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    pull(&app, &linked.token, &mut a).await?;
    assert_eq!(a.unpushed_count()?, 1);
    push(&app, &linked.token, &mut a).await?;
    pull(&app, &joined.token, &mut b).await?;
    assert_eq!(a.fingerprint()?, b.fingerprint()?);
    let users = |s: &Store| -> Result<i64, Error> {
        s.read(|r| {
            Ok(r.conn().query_row(
                "SELECT COUNT(*) FROM users WHERE name = 'Yacine'",
                [],
                |row| row.get(0),
            )?)
        })
    };
    assert_eq!(users(&a)?, 1);

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

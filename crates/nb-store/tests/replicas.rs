//! Ready for replicas (M11/M0, ADR-0006): the same operations (same ids, same
//! times) on two databases make exactly the same business state, so a
//! server running a device's operations again reaches the device's state.
//! Counters run per device, so two devices never give the same number.

mod common;

use common::*;
use nb_core::uuid::Uuid;
use nb_core::{OperationId, id::idgen};
use nb_store::usecases::customers::{CustomerInput, save_customer};
use nb_store::usecases::debts::{ReceiptInput, collect};
use nb_store::usecases::distribution::{
    TourInput, VehicleInput, create_tour, save_vehicle, start_tour,
};
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::sales::{
    PaymentInput, SaleInput, SaleLineInput, complete_sale, open_shift,
};
use nb_store::usecases::setup::{SetupResult, setup_organization};
use nb_store::{Error, OpMeta, Store};

type R = Result<(), Error>;

/// Operation ids from a fixed sequence: the "log" both replicas follow.
struct Log {
    next: u128,
}

impl Log {
    fn meta(&mut self, s: &SetupResult, kind: &'static str) -> OpMeta {
        self.next += 1;
        OpMeta {
            id: OperationId::from_uuid(Uuid::from_u128(
                0x0190_0000_0000_7000_8000_0000_0000_0000 + self.next,
            )),
            kind,
            org: Some(s.org),
            actor: Some(s.owner),
        }
    }
}

/// A day of business on a fresh database with device prefix `prefix`.
fn a_day(prefix: &str) -> Result<(Store, Vec<String>), Error> {
    // The same device on two databases (as the server holds a replica of
    // what a device did).
    let mut opts = options(prefix);
    opts.device_id = Some(nb_core::DeviceId::from_uuid(Uuid::from_u128(
        0x0190_0000_0000_7000_8000_0000_0000_0d01 + u128::from(prefix.as_bytes()[0]),
    )));
    let mut s = Store::open_in_memory(opts)?;
    let setup = idgen::deterministic(Uuid::from_u128(42), T0, || {
        setup_organization(&mut s, setup_input())
    })?;
    let mut log = Log { next: 0 };
    let wh = setup.warehouse.to_string();
    let mut products = Vec::new();
    for (name, price) in [("Lait 1L", 12_000), ("Pain", 1_500), ("Café 250g", 45_000)] {
        let mut p = product(name, "", price);
        p.opening = Some(Opening {
            warehouse_id: wh.clone(),
            qty: "50".into(),
            unit_cost_minor: price / 2,
            tracked: Default::default(),
        });
        products.push(create_product(&mut s, log.meta(&setup, "p"), p)?.id);
    }
    let customer = save_customer(
        &mut s,
        log.meta(&setup, "c"),
        CustomerInput {
            kind: "person".into(),
            name: "Karim".into(),
            phone: "0550000000".into(),
            credit_limit_minor: 1_000_000,
            status: "active".into(),
            ..Default::default()
        },
    )?;
    let reg = s
        .read(|r| {
            let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
            nb_store::usecases::org::registers(r.conn(), o.org_id()?)
        })?
        .remove(0)
        .id;
    open_shift(&mut s, log.meta(&setup, "shift"), &reg, 10_000)?;
    let mut numbers = Vec::new();
    for (k, credit) in [(0usize, false), (1, true), (2, false)] {
        let sale = complete_sale(
            &mut s,
            log.meta(&setup, "sale"),
            SaleInput {
                register_id: reg.clone(),
                warehouse_id: wh.clone(),
                customer_id: credit.then(|| customer.id.clone()),
                lines: vec![SaleLineInput {
                    product_id: products[k].clone(),
                    qty: "3".into(),
                    ..Default::default()
                }],
                payments: if credit {
                    Vec::new()
                } else {
                    vec![PaymentInput {
                        method: "cash".into(),
                        amount_minor: 200_000,
                    }]
                },
                ..Default::default()
            },
        )?;
        numbers.push(sale.number);
    }
    collect(
        &mut s,
        log.meta(&setup, "receipt"),
        ReceiptInput {
            customer_id: customer.id.clone(),
            amount_minor: 1_000,
            method: "cash".into(),
            register_id: Some(reg.clone()),
            ..Default::default()
        },
    )?;
    let van = save_vehicle(
        &mut s,
        log.meta(&setup, "vehicle"),
        VehicleInput {
            name: "Camion".into(),
            kind: "truck".into(),
            ..Default::default()
        },
    )?;
    let tour = create_tour(
        &mut s,
        log.meta(&setup, "tour"),
        TourInput {
            vehicle_id: van.id,
            kind: "delivery".into(),
            customers: vec![customer.id.clone()],
            ..Default::default()
        },
    )?;
    start_tour(&mut s, log.meta(&setup, "start"), &tour.tour.id, None)?;
    numbers.push(customer.code);
    numbers.push(tour.tour.number);
    Ok((s, numbers))
}

#[test]
fn the_same_operations_make_the_same_state() -> R {
    let (a, numbers_a) = a_day("A")?;
    let (b, numbers_b) = a_day("A")?;
    assert_eq!(numbers_a, numbers_b);
    let differ: Vec<String> = a
        .fingerprints()?
        .into_iter()
        .zip(b.fingerprints()?)
        .filter(|(x, y)| x != y)
        .map(|(x, _)| x.0)
        .collect();
    assert!(differ.is_empty(), "tables differ: {differ:?}");
    assert_eq!(a.fingerprint()?, b.fingerprint()?);
    assert!(a.health()?.is_sound(), "{:?}", a.health()?);
    // Anything else changes it.
    let mut c = b;
    let setup = c.read(|r| {
        let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
        Ok((o.org_id()?, o))
    })?;
    let owner = c.read(|r| {
        Ok(r.conn()
            .query_row("SELECT id FROM users LIMIT 1", [], |row| {
                row.get::<_, String>(0)
            })?)
    })?;
    save_customer(
        &mut c,
        OpMeta::new("c", Some(setup.0), owner.parse().ok()),
        CustomerInput {
            kind: "person".into(),
            name: "Autre".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    assert_ne!(a.fingerprint()?, c.fingerprint()?);
    Ok(())
}

#[test]
fn devices_never_give_the_same_numbers() -> R {
    let (_, a) = a_day("A")?;
    let (b_store, b) = a_day("B")?;
    for (x, y) in a.iter().zip(&b) {
        assert_ne!(x, y);
    }
    // The first device keeps the plain codes; another puts its letter in.
    assert_eq!(a[3], "C-00001");
    assert_eq!(b[3], "C-B0001");
    assert!(b[0].contains('B'), "{}", b[0]);
    assert_eq!(b_store.verify_audit_chain()?, None);
    Ok(())
}

#[test]
fn an_older_database_keeps_counting_where_it_was() -> R {
    use nb_store::migrate::{latest, migrate_to};
    let mut conn = rusqlite::Connection::open_in_memory()?;
    migrate_to(&mut conn, 31, None)?;
    conn.execute_batch(
        "INSERT INTO device (id, prefix, name, created_at) VALUES ('d1', 'C', 'old', 0);
         INSERT INTO organizations (id, name, country, currency, currency_exp, activity, created_at, updated_at, device_id)
           VALUES ('o1', 'Old', 'DZ', 'DZD', 2, 'grocery', 0, 0, 'd1');
         INSERT INTO doc_counters (kind, branch_id, year, last) VALUES ('FV', 'b1', 2026, 41);
         INSERT INTO customer_counters (org_id, last) VALUES ('o1', 7);
         INSERT INTO audit_log (operation_id, org_id, actor_id, action, entity, entity_id, detail, created_at, prev_hash, hash)
           VALUES ('op', 'o1', NULL, 'x', 'y', 'z', '{}', 0, 'p', 'h');",
    )?;
    migrate_to(&mut conn, latest(), None)?;
    let doc: (String, String, i64) = conn.query_row(
        "SELECT scope, device_prefix, last FROM counters WHERE name = 'doc:FV'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    assert_eq!(doc, ("b1:2026".into(), "C".into(), 41));
    let customers: i64 = conn.query_row(
        "SELECT last FROM counters WHERE name = 'customer' AND scope = 'o1' AND device_prefix = 'C'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(customers, 7);
    let device: String = conn.query_row("SELECT device_id FROM audit_log", [], |r| r.get(0))?;
    // The chain from before stays whole on its own.
    assert_eq!(device, "");
    Ok(())
}

/// A fresh replica of the same device with only the organization set up.
fn empty_replica(prefix: &str) -> Result<Store, Error> {
    let mut opts = options(prefix);
    opts.device_id = Some(nb_core::DeviceId::from_uuid(Uuid::from_u128(
        0x0190_0000_0000_7000_8000_0000_0000_0d01 + u128::from(prefix.as_bytes()[0]),
    )));
    let mut s = Store::open_in_memory(opts)?;
    idgen::deterministic(Uuid::from_u128(42), T0, || {
        setup_organization(&mut s, setup_input())
    })?;
    Ok(s)
}

#[test]
fn a_device_log_run_again_elsewhere_makes_the_same_state() -> R {
    let (device, _) = a_day("A")?;
    let log = device.operation_log()?;
    assert!(log.len() >= 12, "{}", log.len());
    let mut server = empty_replica("A")?;
    server.replay_log(log.clone())?;
    let differ: Vec<String> = device
        .fingerprints()?
        .into_iter()
        .zip(server.fingerprints()?)
        .filter(|(x, y)| x != y)
        .map(|(x, _)| x.0)
        .collect();
    assert!(differ.is_empty(), "tables differ: {differ:?}");
    assert!(server.health()?.is_sound(), "{:?}", server.health()?);
    // Running it again changes nothing (each operation once).
    server.replay_log(log)?;
    assert_eq!(device.fingerprint()?, server.fingerprint()?);
    Ok(())
}

/// A clock the test moves forward (days of business in minutes).
struct Ticking(std::sync::atomic::AtomicI64);

impl nb_core::Clock for Ticking {
    fn now_ms(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// A small fixed pseudo-random sequence (the same every run).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

#[test]
fn hundreds_of_mixed_operations_replay_to_the_same_state() -> R {
    use nb_store::usecases::expenses::{ExpenseInput, expense_categories, record_expense};
    use nb_store::usecases::returns::{ReturnInput, ReturnLineInput, return_sale};
    use nb_store::usecases::stock::{AdjustInput, adjust_stock};
    let clock = std::sync::Arc::new(Ticking(std::sync::atomic::AtomicI64::new(T0)));
    let mut opts = options("A");
    opts.clock = clock.clone();
    let device_id = nb_core::DeviceId::from_uuid(Uuid::from_u128(7));
    opts.device_id = Some(device_id);
    let mut s = Store::open_in_memory(opts)?;
    let setup = idgen::deterministic(Uuid::from_u128(42), T0, || {
        setup_organization(&mut s, setup_input())
    })?;
    let wh = setup.warehouse.to_string();
    let reg = s
        .read(|r| {
            let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
            nb_store::usecases::org::registers(r.conn(), o.org_id()?)
        })?
        .remove(0)
        .id;
    let m = |kind: &'static str| OpMeta::new(kind, Some(setup.org), Some(setup.owner));
    let mut rng = Lcg(2026);
    let (mut products, mut customers, mut sales) = (Vec::new(), Vec::new(), Vec::new());
    let mut done = 0;
    open_shift(&mut s, m("shift"), &reg, 50_000)?;
    for step in 0..500 {
        clock.0.fetch_add(
            60_000 * (1 + rng.next(240) as i64),
            std::sync::atomic::Ordering::SeqCst,
        );
        let ok = match (step < 10, rng.next(8)) {
            (true, _) | (_, 0) => {
                let mut p = product(
                    &format!("Produit {step}"),
                    "",
                    1_000 + 100 * rng.next(500) as i64,
                );
                p.opening = Some(Opening {
                    warehouse_id: wh.clone(),
                    qty: (5 + rng.next(40)).to_string(),
                    unit_cost_minor: 500 + 50 * rng.next(100) as i64,
                    tracked: Default::default(),
                });
                create_product(&mut s, m("p"), p)
                    .map(|p| products.push(p.id))
                    .is_ok()
            }
            (_, 1) => save_customer(
                &mut s,
                m("c"),
                CustomerInput {
                    kind: "person".into(),
                    name: format!("Client {step}"),
                    credit_limit_minor: 5_000_000,
                    status: "active".into(),
                    ..Default::default()
                },
            )
            .map(|c| customers.push(c.id))
            .is_ok(),
            (_, 2 | 3) if !products.is_empty() => {
                let credit = !customers.is_empty() && rng.next(3) == 0;
                let lines = (0..1 + rng.next(3))
                    .map(|_| SaleLineInput {
                        product_id: products[rng.next(products.len())].clone(),
                        qty: (1 + rng.next(3)).to_string(),
                        ..Default::default()
                    })
                    .collect();
                complete_sale(
                    &mut s,
                    m("sale"),
                    SaleInput {
                        register_id: reg.clone(),
                        warehouse_id: wh.clone(),
                        customer_id: credit.then(|| customers[rng.next(customers.len())].clone()),
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
                )
                .map(|r| sales.push(r.id))
                .is_ok()
            }
            (_, 4) if !customers.is_empty() => collect(
                &mut s,
                m("receipt"),
                ReceiptInput {
                    customer_id: customers[rng.next(customers.len())].clone(),
                    amount_minor: 100 * (1 + rng.next(50)) as i64,
                    method: "cash".into(),
                    register_id: Some(reg.clone()),
                    unallocated: true,
                    ..Default::default()
                },
            )
            .is_ok(),
            (_, 5) if !products.is_empty() => adjust_stock(
                &mut s,
                m("adjust"),
                AdjustInput {
                    product_id: products[rng.next(products.len())].clone(),
                    warehouse_id: wh.clone(),
                    kind: ["adjust_in", "adjust_out", "waste"][rng.next(3)].into(),
                    qty: (1 + rng.next(4)).to_string(),
                    unit: "pc".into(),
                    unit_cost_minor: None,
                    reason: "test".into(),
                    may_go_negative: false,
                    approval: None,
                    tracked: Default::default(),
                },
            )
            .is_ok(),
            (_, 6) if !sales.is_empty() => return_sale(
                &mut s,
                m("return"),
                ReturnInput {
                    sale: sales[rng.next(sales.len())].clone(),
                    register_id: reg.clone(),
                    lines: vec![ReturnLineInput {
                        position: 1,
                        qty: "1".into(),
                        restock: rng.next(2) == 0,
                    }],
                    refund: "cash".into(),
                    reason: "test".into(),
                    approval: None,
                },
            )
            .is_ok(),
            _ => {
                let cats = expense_categories(&s)?;
                let cat = cats
                    .iter()
                    .find(|c| c.kind == "expense")
                    .map(|c| c.id.clone())
                    .unwrap_or_default();
                record_expense(
                    &mut s,
                    m("expense"),
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
                )
                .is_ok()
            }
        };
        done += usize::from(ok);
    }
    assert!(done > 300, "only {done} operations went through");
    assert!(s.health()?.is_sound(), "{:?}", s.health()?);
    let log = s.operation_log()?;
    let mut replica_opts = options("A");
    replica_opts.device_id = Some(device_id);
    let mut replica = Store::open_in_memory(replica_opts)?;
    idgen::deterministic(Uuid::from_u128(42), T0, || {
        setup_organization(&mut replica, setup_input())
    })?;
    replica.replay_log(log)?;
    let differ: Vec<String> = s
        .fingerprints()?
        .into_iter()
        .zip(replica.fingerprints()?)
        .filter(|(x, y)| x != y)
        .map(|(x, _)| x.0)
        .collect();
    assert!(differ.is_empty(), "tables differ: {differ:?}");
    assert!(replica.health()?.is_sound(), "{:?}", replica.health()?);
    Ok(())
}

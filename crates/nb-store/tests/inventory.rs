//! Inventory (M6): stock overview, reservations, transfers, counts, and the
//! movement journal — with balances that always rebuild from movements and
//! entries that always balance.

mod common;

use common::*;
use nb_domain::{ProductId, WarehouseId};
use nb_store::Error;
use nb_store::repo::access::GrantRow;
use nb_store::repo::catalog::StockState;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::inventory::*;
use nb_store::usecases::org::create_branch;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::stock::rebuild_balances;
use nb_store::{OpMeta, Store};

type R = Result<(), Error>;

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
    }
}

fn stocked(
    s: &mut Store,
    setup: &nb_store::usecases::setup::SetupResult,
    name: &str,
    qty: &str,
    cost: i64,
) -> Result<ProductId, Error> {
    let mut p = product(name, "", cost * 2);
    p.min_stock = Some("5".into());
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: qty.into(),
        unit_cost_minor: cost,
        tracked: Default::default(),
    });
    Ok(create_product(s, meta(setup, "product.create"), p)?
        .id
        .parse()
        .unwrap_or_default())
}

fn healthy(s: &Store) -> R {
    assert!(
        rebuild_balances(s)?.is_empty(),
        "balances = replayed movements"
    );
    assert!(
        s.read(|r| nb_store::repo::ledger::unbalanced_entries(r.conn()))?
            .is_empty()
    );
    assert_eq!(s.verify_audit_chain()?, None);
    Ok(())
}

#[test]
fn overview_and_reservations() -> R {
    let (mut s, setup) = store();
    let milk = stocked(&mut s, &setup, "Lait", "10", 1_000)?;
    stocked(&mut s, &setup, "Beurre", "3", 2_000)?;
    create_product(
        &mut s,
        meta(&setup, "product.create"),
        product("Sel", "", 500),
    )?;
    let page = stock_overview(
        &s,
        &StockQuery {
            limit: 100,
            ..Default::default()
        },
    )?;
    assert_eq!(page.counts, [3, 1, 1, 1]);
    assert_eq!(page.value_minor, 10 * 1_000 + 3 * 2_000);
    let low = stock_overview(
        &s,
        &StockQuery {
            state: Some(StockState::Low),
            limit: 100,
            ..Default::default()
        },
    )?;
    assert_eq!(
        low.rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        ["Beurre"]
    );

    let wh = setup.warehouse;
    let r1 = reserve(&mut s, meta(&setup, "reserve"), milk, wh, "4", "BC-1")?;
    assert_eq!(available(&s, milk, wh)?.to_string(), "6");
    assert_eq!(
        err(reserve(
            &mut s,
            meta(&setup, "reserve"),
            milk,
            wh,
            "7",
            "BC-2"
        )),
        ["stock.insufficient"]
    );
    let row = stock_overview(
        &s,
        &StockQuery {
            text: "lait".into(),
            limit: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(
        (row.rows[0].reserved_milli, row.rows[0].available_milli),
        (4_000, 6_000)
    );
    close_reservation(&mut s, meta(&setup, "reserve.close"), &r1, false)?;
    assert_eq!(available(&s, milk, wh)?.to_string(), "10");
    assert!(matches!(
        close_reservation(&mut s, meta(&setup, "reserve.close"), &r1, false),
        Err(Error::NotFound(_))
    ));
    healthy(&s)
}

#[test]
fn transfers_move_stock_and_cost_and_book_shortfalls() -> R {
    let (mut s, setup) = store();
    let milk = stocked(&mut s, &setup, "Lait", "10", 1_000)?;
    let other = create_branch(&mut s, meta(&setup, "branch"), "Touggourt")?;
    let to: WarehouseId = other.warehouses[0].id.parse().unwrap_or_default();
    let input = TransferInput {
        from_warehouse: setup.warehouse.to_string(),
        to_warehouse: to.to_string(),
        lines: vec![TransferLineInput {
            product_id: milk.to_string(),
            qty: "6".into(),
            unit: String::new(),
            tracked: Default::default(),
        }],
        note: "réassort".into(),
    };
    let t = send_transfer(&mut s, meta(&setup, "transfer.send"), input.clone())?;
    assert!(t.number.starts_with("TR-01-26-A"));
    assert_eq!(
        (
            t.status.as_str(),
            t.lines[0].sent_milli,
            t.lines[0].unit_cost_minor
        ),
        ("sent", 6_000, 1_000)
    );
    assert_eq!(available(&s, milk, setup.warehouse)?.to_string(), "4");
    assert_eq!(available(&s, milk, to)?.to_string(), "0", "in transit");
    // More than was sent cannot arrive.
    assert_eq!(
        err(receive_transfer(
            &mut s,
            meta(&setup, "transfer.receive"),
            &t.id,
            vec![(0, "7".into())]
        )),
        ["transfer.received_more"]
    );
    let r = receive_transfer(
        &mut s,
        meta(&setup, "transfer.receive"),
        &t.id,
        vec![(0, "5".into())],
    )?;
    assert_eq!(
        (r.status.as_str(), r.lines[0].received_milli),
        ("received", Some(5_000))
    );
    assert_eq!(available(&s, milk, to)?.to_string(), "5");
    // The missing unit is a loss at its cost.
    let loss: i64 = s.connection().query_row(
        "SELECT SUM(l.debit_minor) FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id WHERE e.source = 'stock.transfer' AND l.account_code = '658'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(loss, 1_000);
    assert_eq!(
        err(receive_transfer(
            &mut s,
            meta(&setup, "transfer.receive"),
            &t.id,
            vec![]
        )),
        ["transfer.already_received"]
    );
    // Same warehouse, empty, too much without the right to go negative.
    let same = TransferInput {
        to_warehouse: setup.warehouse.to_string(),
        ..input.clone()
    };
    assert_eq!(
        err(send_transfer(&mut s, meta(&setup, "transfer.send"), same)),
        ["transfer.same_warehouse"]
    );
    let empty = TransferInput {
        lines: vec![],
        ..input.clone()
    };
    assert_eq!(
        err(send_transfer(&mut s, meta(&setup, "transfer.send"), empty)),
        ["transfer.empty"]
    );
    let keeper = create_user(
        &mut s,
        meta(&setup, "user.create"),
        UserInput {
            name: "M".into(),
            phone: None,
            login: None,
            password: None,
            pin: None,
            grants: vec![GrantRow::org("stock_keeper")],
        },
    )?;
    let as_keeper = OpMeta::new(
        "transfer.send",
        Some(setup.org),
        Some(keeper.id.parse().unwrap_or_default()),
    );
    let too_much = TransferInput {
        lines: vec![TransferLineInput {
            product_id: milk.to_string(),
            qty: "50".into(),
            unit: String::new(),
            tracked: Default::default(),
        }],
        ..input
    };
    assert_eq!(
        err(send_transfer(&mut s, as_keeper, too_much)),
        ["stock.insufficient"]
    );
    assert_eq!(list_transfers(&s, 10)?.len(), 1);
    healthy(&s)
}

#[test]
fn counts_bring_the_book_to_what_was_counted() -> R {
    let (mut s, setup) = store();
    let milk = stocked(&mut s, &setup, "Lait", "10", 1_000)?;
    let oil = stocked(&mut s, &setup, "Huile", "10", 3_000)?;
    let keeper = create_user(
        &mut s,
        meta(&setup, "user.create"),
        UserInput {
            name: "M".into(),
            phone: None,
            login: None,
            password: None,
            pin: None,
            grants: vec![GrantRow::org("stock_keeper")],
        },
    )?;
    let as_keeper = |kind| {
        OpMeta::new(
            kind,
            Some(setup.org),
            Some(keeper.id.parse().unwrap_or_default()),
        )
    };
    let c = open_count(&mut s, as_keeper("count.open"), setup.warehouse, None)?;
    assert!(c.number.starts_with("INV-01-26-A"));
    assert_eq!(
        err(open_count(
            &mut s,
            meta(&setup, "count.open"),
            setup.warehouse,
            None
        )),
        ["count.already_open"]
    );
    record_count(&mut s, as_keeper("count.record"), &c.id, milk, "7", "")?;
    record_count(&mut s, as_keeper("count.record"), &c.id, oil, "11", "")?;
    // A correction replaces the first count.
    record_count(&mut s, as_keeper("count.record"), &c.id, milk, "8", "")?;
    let d = count_detail(&s, &c.id)?;
    assert_eq!(d.counted, 2);
    assert_eq!(
        d.lines.iter().filter(|l| l.counted_milli.is_some()).count(),
        2
    );
    // A stock keeper counts; booking the differences needs stock.adjust.
    let e = close_count(&mut s, as_keeper("count.close"), &c.id, None);
    assert!(matches!(
        e,
        Err(Error::Forbidden {
            permission: "stock.adjust",
            needs_approval: true
        })
    ));
    let out = close_count(&mut s, meta(&setup, "count.close"), &c.id, None)?;
    assert_eq!(
        out,
        CountResult {
            gains_minor: 3_000,
            losses_minor: 2_000,
            adjusted: 2
        }
    );
    assert_eq!(available(&s, milk, setup.warehouse)?.to_string(), "8");
    assert_eq!(available(&s, oil, setup.warehouse)?.to_string(), "11");
    assert_eq!(
        err(record_count(
            &mut s,
            meta(&setup, "count.record"),
            &c.id,
            milk,
            "1",
            ""
        )),
        ["count.closed"]
    );
    let journal = movements(
        &s,
        &MovementQuery {
            kinds: vec!["count_gain".into(), "count_loss".into()],
            limit: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(
        journal
            .iter()
            .map(|m| (m.kind.as_str(), m.qty_milli))
            .collect::<Vec<_>>(),
        [("count_gain", 1_000), ("count_loss", -2_000)]
    );
    // A new count may open now.
    let c2 = open_count(&mut s, meta(&setup, "count.open"), setup.warehouse, None)?;
    cancel_count(&mut s, meta(&setup, "count.cancel"), &c2.id)?;
    assert_eq!(
        list_counts(&s, 10)?
            .iter()
            .map(|c| c.status.as_str())
            .collect::<Vec<_>>(),
        ["cancelled", "closed"]
    );
    healthy(&s)
}

// --- Lots, expiry and serial numbers (M6.8, M6.9) ---

use nb_store::usecases::stock::{AdjustInput, adjust_stock};
use nb_store::usecases::tracked::Tracked;

fn tracked_product(
    s: &mut Store,
    setup: &nb_store::usecases::setup::SetupResult,
    name: &str,
    tracking: &str,
    opening: Option<(&str, Tracked)>,
) -> Result<ProductId, Error> {
    let mut p = product(name, "", 2_000);
    p.tracking = Some(tracking.into());
    p.opening = opening.map(|(qty, tracked)| Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: qty.into(),
        unit_cost_minor: 1_000,
        tracked,
    });
    Ok(create_product(s, meta(setup, "product.create"), p)?
        .id
        .parse()
        .unwrap_or_default())
}

fn lot(code: &str, expiry: &str) -> Tracked {
    Tracked {
        lot: Some(code.into()),
        expiry: Some(expiry.into()),
        serials: vec![],
    }
}

fn serials(list: &[&str]) -> Tracked {
    Tracked {
        serials: list.iter().map(|s| (*s).to_owned()).collect(),
        ..Default::default()
    }
}

fn move_stock(
    product: ProductId,
    wh: WarehouseId,
    kind: &str,
    qty: &str,
    tracked: Tracked,
) -> AdjustInput {
    AdjustInput {
        product_id: product.to_string(),
        warehouse_id: wh.to_string(),
        kind: kind.into(),
        qty: qty.into(),
        unit: "pc".into(),
        unit_cost_minor: Some(1_000),
        reason: "test".into(),
        may_go_negative: false,
        approval: None,
        tracked,
    }
}

/// Lot codes with stock in a warehouse, by expiry (none first), in
/// thousandths.
fn lots(s: &Store, product: ProductId, wh: WarehouseId) -> Vec<(String, i64)> {
    let mut q = s
        .connection()
        .prepare(
            "SELECT l.code, b.on_hand_milli FROM lots l JOIN lot_balances b ON b.lot_id = l.id
             WHERE l.product_id = ?1 AND b.warehouse_id = ?2 AND b.on_hand_milli <> 0 ORDER BY l.expiry, l.code",
        )
        .unwrap_or_else(|e| panic!("{e}"));
    q.query_map([product.to_string(), wh.to_string()], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .and_then(|rows| rows.collect())
    .unwrap_or_else(|e| panic!("{e}"))
}

fn in_stock(s: &Store, product: ProductId, wh: WarehouseId) -> Vec<String> {
    let mut q = s
        .connection()
        .prepare("SELECT serial FROM serials WHERE product_id = ?1 AND warehouse_id = ?2 AND status = 'in_stock' ORDER BY serial")
        .unwrap_or_else(|e| panic!("{e}"));
    q.query_map([product.to_string(), wh.to_string()], |r| r.get(0))
        .and_then(|rows| rows.collect())
        .unwrap_or_else(|e| panic!("{e}"))
}

fn tracked_healthy(s: &Store) -> R {
    assert_eq!(
        nb_store::repo::tracking::rebuild_lot_balances(s.connection())?,
        0,
        "lot balances = replayed lots"
    );
    healthy(s)
}

#[test]
fn lots_go_first_expiring_first_and_follow_transfers() -> R {
    let (mut s, setup) = store();
    let wh = setup.warehouse;
    // Opening stock needs its lot.
    assert_eq!(
        err(tracked_product(
            &mut s,
            &setup,
            "Yaourt",
            "lot",
            Some(("10", Tracked::default()))
        )),
        ["lot.required"]
    );
    assert_eq!(
        err(tracked_product(
            &mut s,
            &setup,
            "Yaourt",
            "lot",
            Some(("10", lot("a1", "2031/01/01")))
        )),
        ["lot.expiry_format"]
    );
    let yog = tracked_product(
        &mut s,
        &setup,
        "Yaourt",
        "lot",
        Some(("10", lot("a1", "2031-03-01"))),
    )?;
    adjust_stock(
        &mut s,
        meta(&setup, "stock.adjust"),
        move_stock(yog, wh, "adjust_in", "5", lot("B2", "2030-12-01")),
    )?;
    adjust_stock(
        &mut s,
        meta(&setup, "stock.adjust"),
        move_stock(yog, wh, "adjust_in", "4", lot("OLD", "2020-01-01")),
    )?;
    // A known lot keeps its expiry.
    assert_eq!(
        err(adjust_stock(
            &mut s,
            meta(&setup, "stock.adjust"),
            move_stock(yog, wh, "adjust_in", "1", lot("B2", "2032-01-01"))
        )),
        ["lot.expiry_mismatch"]
    );
    assert_eq!(
        lots(&s, yog, wh),
        [
            ("OLD".into(), 4_000),
            ("B2".into(), 5_000),
            ("A1".into(), 10_000)
        ]
    );

    // The expired lot shows in the alerts; far lots do not.
    let soon = expiring_lots(&s, 30)?;
    assert_eq!(
        soon.iter()
            .map(|l| (l.lot.as_str(), l.qty_milli))
            .collect::<Vec<_>>(),
        [("OLD", 4_000)]
    );
    assert!(soon[0].days_left < 0);
    assert_eq!(expiring_lots(&s, 365 * 10)?.len(), 3);
    // Waste takes the expired lot first.
    adjust_stock(
        &mut s,
        meta(&setup, "stock.adjust"),
        move_stock(yog, wh, "waste", "4", Tracked::default()),
    )?;
    assert_eq!(
        lots(&s, yog, wh),
        [("B2".into(), 5_000), ("A1".into(), 10_000)]
    );
    // A named lot, then not more than it holds.
    adjust_stock(
        &mut s,
        meta(&setup, "stock.adjust"),
        move_stock(
            yog,
            wh,
            "adjust_out",
            "2",
            Tracked {
                lot: Some("a1".into()),
                ..Default::default()
            },
        ),
    )?;
    assert_eq!(
        err(adjust_stock(
            &mut s,
            meta(&setup, "stock.adjust"),
            move_stock(
                yog,
                wh,
                "adjust_out",
                "6",
                Tracked {
                    lot: Some("B2".into()),
                    ..Default::default()
                }
            )
        )),
        ["lot.insufficient"]
    );
    assert_eq!(
        err(adjust_stock(
            &mut s,
            meta(&setup, "stock.adjust"),
            move_stock(
                yog,
                wh,
                "adjust_out",
                "1",
                Tracked {
                    lot: Some("ZZ".into()),
                    ..Default::default()
                }
            )
        )),
        ["lot.unknown"]
    );

    // A transfer sends the first expiring lots; they arrive as they left.
    let other = create_branch(&mut s, meta(&setup, "branch"), "Touggourt")?;
    let to: WarehouseId = other.warehouses[0].id.parse().unwrap_or_default();
    let t = send_transfer(
        &mut s,
        meta(&setup, "transfer.send"),
        TransferInput {
            from_warehouse: wh.to_string(),
            to_warehouse: to.to_string(),
            lines: vec![TransferLineInput {
                product_id: yog.to_string(),
                qty: "7".into(),
                unit: String::new(),
                tracked: Tracked::default(),
            }],
            note: String::new(),
        },
    )?;
    assert_eq!(lots(&s, yog, wh), [("A1".into(), 6_000)]);
    receive_transfer(
        &mut s,
        meta(&setup, "transfer.receive"),
        &t.id,
        vec![(0, "6".into())],
    )?;
    assert_eq!(
        lots(&s, yog, to),
        [("B2".into(), 5_000), ("A1".into(), 1_000)]
    );

    // Units found at a count form a lot named after the count.
    let c = open_count(&mut s, meta(&setup, "count.open"), wh, None)?;
    record_count(&mut s, meta(&setup, "count.record"), &c.id, yog, "8", "")?;
    close_count(&mut s, meta(&setup, "count.close"), &c.id, None)?;
    assert_eq!(
        lots(&s, yog, wh),
        [(c.number.clone(), 2_000), ("A1".into(), 6_000)]
    );
    tracked_healthy(&s)
}

#[test]
fn serial_numbers_enter_and_leave_once() -> R {
    let (mut s, setup) = store();
    let wh = setup.warehouse;
    assert_eq!(
        err(tracked_product(
            &mut s,
            &setup,
            "Téléphone",
            "serial",
            Some(("2", serials(&["S1"])))
        )),
        ["serial.count"]
    );
    assert_eq!(
        err(tracked_product(
            &mut s,
            &setup,
            "Téléphone",
            "serial",
            Some(("2", serials(&["S1", "S1"])))
        )),
        ["serial.duplicate"]
    );
    let phone = tracked_product(
        &mut s,
        &setup,
        "Téléphone",
        "serial",
        Some(("2", serials(&["S1", "S2"]))),
    )?;
    assert_eq!(
        err(adjust_stock(
            &mut s,
            meta(&setup, "stock.adjust"),
            move_stock(phone, wh, "adjust_in", "1", serials(&["S2"]))
        )),
        ["serial.in_stock"]
    );
    adjust_stock(
        &mut s,
        meta(&setup, "stock.adjust"),
        move_stock(phone, wh, "adjust_in", "1", serials(&["S3"])),
    )?;
    adjust_stock(
        &mut s,
        meta(&setup, "stock.adjust"),
        move_stock(phone, wh, "waste", "1", serials(&["S1"])),
    )?;
    assert_eq!(
        err(adjust_stock(
            &mut s,
            meta(&setup, "stock.adjust"),
            move_stock(phone, wh, "waste", "1", serials(&["S1"]))
        )),
        ["serial.not_in_stock"]
    );
    assert_eq!(in_stock(&s, phone, wh), ["S2", "S3"]);

    // Serials travel with a transfer, and arrive whole.
    let other = create_branch(&mut s, meta(&setup, "branch"), "Touggourt")?;
    let to: WarehouseId = other.warehouses[0].id.parse().unwrap_or_default();
    let t = send_transfer(
        &mut s,
        meta(&setup, "transfer.send"),
        TransferInput {
            from_warehouse: wh.to_string(),
            to_warehouse: to.to_string(),
            lines: vec![TransferLineInput {
                product_id: phone.to_string(),
                qty: "2".into(),
                unit: String::new(),
                tracked: serials(&["S2", "S3"]),
            }],
            note: String::new(),
        },
    )?;
    assert!(in_stock(&s, phone, wh).is_empty());
    assert_eq!(
        err(receive_transfer(
            &mut s,
            meta(&setup, "transfer.receive"),
            &t.id,
            vec![(0, "1".into())]
        )),
        ["transfer.serial_partial"]
    );
    receive_transfer(&mut s, meta(&setup, "transfer.receive"), &t.id, vec![])?;
    assert_eq!(in_stock(&s, phone, to), ["S2", "S3"]);

    // Counted unit by unit elsewhere; tracking is fixed once stock moved.
    let c = open_count(&mut s, meta(&setup, "count.open"), to, None)?;
    assert_eq!(
        err(record_count(
            &mut s,
            meta(&setup, "count.record"),
            &c.id,
            phone,
            "2",
            ""
        )),
        ["count.serial_product"]
    );
    // An untracked product refuses lots.
    let salt = create_product(
        &mut s,
        meta(&setup, "product.create"),
        product("Sel", "", 500),
    )?;
    let salt: ProductId = salt.id.parse().unwrap_or_default();
    assert_eq!(
        err(adjust_stock(
            &mut s,
            meta(&setup, "stock.adjust"),
            move_stock(salt, wh, "adjust_in", "1", lot("X", "2030-01-01"))
        )),
        ["tracking.not_tracked"]
    );
    tracked_healthy(&s)
}

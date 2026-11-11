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

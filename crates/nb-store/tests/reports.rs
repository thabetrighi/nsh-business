//! Reports (M13): sales by day, product, cashier and method with returns
//! and profit; stock with its value, what runs out and what does not move;
//! the dashboard's series against the period before.

mod common;

use std::sync::Arc;

use common::*;
use nb_core::FixedClock;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::reports::*;
use nb_store::usecases::returns::{ReturnInput, ReturnLineInput, return_sale};
use nb_store::usecases::sales::*;
use nb_store::usecases::setup::setup_organization;
use nb_store::{Error, Store, StoreOptions};

type R = Result<(), Error>;
const DAY: i64 = 86_400_000;

fn span(from: &str, to: &str) -> Span {
    Span {
        from: from.into(),
        to: to.into(),
    }
}

#[test]
fn sales_profit_and_stock_add_up() -> R {
    // T0 is 2026-11-10 11:00 at the store (UTC+1).
    let clock = Arc::new(FixedClock::new(T0 - 2 * DAY));
    let mut s = Store::open_in_memory(StoreOptions {
        clock: clock.clone(),
        ..options("A")
    })?;
    let setup = setup_organization(&mut s, setup_input())?;
    let stocked =
        |s: &mut Store, name: &str, qty: &str, cost: i64, price: i64, min: Option<&str>| {
            let mut p = product(name, "", price);
            p.min_stock = min.map(Into::into);
            p.opening = Some(Opening {
                warehouse_id: setup.warehouse.to_string(),
                qty: qty.into(),
                unit_cost_minor: cost,
                tracked: Default::default(),
            });
            create_product(s, meta(&setup, "p"), p).map(|p| p.id)
        };
    let milk = stocked(&mut s, "Lait", "10", 800, 1_190, Some("8"))?;
    let oil = stocked(&mut s, "Huile", "5", 2_500, 3_570, None)?;
    let salt = stocked(&mut s, "Sel", "4", 100, 238, None)?;
    let reg = s
        .read(|r| {
            let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
            nb_store::usecases::org::registers(r.conn(), o.org_id()?)
        })?
        .remove(0)
        .id;
    open_shift(&mut s, meta(&setup, "shift"), &reg, 0)?;
    let sell = |s: &mut Store, lines: Vec<(&str, &str)>, method: &str, amount: i64| {
        complete_sale(
            s,
            meta(&setup, "sale"),
            SaleInput {
                register_id: reg.clone(),
                warehouse_id: setup.warehouse.to_string(),
                lines: lines
                    .into_iter()
                    .map(|(p, q)| SaleLineInput {
                        product_id: p.into(),
                        qty: q.into(),
                        ..Default::default()
                    })
                    .collect(),
                payments: vec![PaymentInput {
                    method: method.into(),
                    amount_minor: amount,
                }],
                ..Default::default()
            },
        )
    };
    // 2026-11-08: three milks in cash.
    let first = sell(&mut s, vec![(&milk, "3")], "cash", 3_570)?;
    // 2026-11-10: an oil by card, then a milk and an oil in cash; one milk
    // of the first sale comes back.
    clock.set(T0);
    sell(&mut s, vec![(&oil, "1")], "card", 3_570)?;
    sell(&mut s, vec![(&milk, "1"), (&oil, "1")], "cash", 4_760)?;
    return_sale(
        &mut s,
        meta(&setup, "ret"),
        ReturnInput {
            sale: first.number.clone(),
            register_id: reg.clone(),
            lines: vec![ReturnLineInput {
                position: 0,
                qty: "1".into(),
                restock: true,
            }],
            refund: "cash".into(),
            reason: "Client".into(),
            approval: None,
        },
    )?;

    assert_eq!(
        sales_report(&s, &span("2026-11-10", "2026-11-01"))
            .err()
            .map(|e| e.rule_codes()),
        Some(vec![(Some("to"), "report.span")])
    );
    let r = sales_report(&s, &span("2026-11-01", "2026-11-30"))?;
    // Sold: 3 + 1 milks (10.00 net each), 2 oils (30.00 net each).
    assert_eq!(
        (r.sales, r.total_minor, r.net_minor, r.tax_minor),
        (3, 11_900, 10_000, 1_900)
    );
    // One milk back: 10.00 net, its 8.00 cost back in stock.
    assert_eq!(
        (r.returns, r.returns_total_minor, r.returns_net_minor),
        (1, 1_190, 1_000)
    );
    assert_eq!(r.cost_minor, 4 * 800 + 2 * 2_500 - 800);
    assert_eq!(r.net_after_returns_minor(), 9_000);
    assert_eq!(r.profit_minor(), 9_000 - 7_400);
    assert_eq!(r.margin_bps(), 1_777);
    assert_eq!(r.basket_minor(), 3_966);
    assert_eq!(
        r.by_day
            .iter()
            .map(|d| (
                d.date.as_str(),
                d.sales,
                d.total_minor,
                d.returns_minor,
                d.profit_minor()
            ))
            .collect::<Vec<_>>(),
        [
            ("2026-11-08", 1, 3_570, 0, 3_000 - 2_400),
            ("2026-11-10", 2, 8_330, 1_190, 7_000 - 5_800 - 1_000 + 800),
        ]
    );
    assert_eq!(
        r.by_product
            .iter()
            .map(|p| (p.name.as_str(), p.qty_milli, p.net_minor, p.cost_minor))
            .collect::<Vec<_>>(),
        [
            ("Huile", 2_000, 6_000, 5_000),
            ("Lait", 4_000, 4_000, 3_200)
        ]
    );
    assert_eq!(
        r.by_method
            .iter()
            .map(|m| (m.key.as_str(), m.net_minor))
            .collect::<Vec<_>>(),
        [("cash", 8_330), ("card", 3_570)]
    );
    assert_eq!(r.by_cashier.len(), 1);
    assert_eq!(r.by_cashier[0].count, 3);
    // Only that day.
    let day = sales_report(&s, &span("2026-11-08", "2026-11-08"))?;
    assert_eq!((day.sales, day.returns), (1, 0));

    // Stock: value at average cost; milk under its minimum; salt idle.
    let st = stock_report(&s, 30)?;
    let row = |name: &str| st.rows.iter().find(|r| r.name == name).cloned();
    let m = row("Lait").unwrap_or_default();
    assert_eq!(
        (m.on_hand_milli, m.sold_milli, m.low()),
        (7_000, 4_000, true)
    );
    let sel = row("Sel").unwrap_or_default();
    assert!(sel.idle() && !sel.low());
    assert_eq!(
        st.value_minor,
        7 * 800 + 3 * 2_500 + 4 * 100,
        "the value is what is there at its cost"
    );
    assert_eq!(
        (st.low, st.out, st.idle, st.idle_value_minor),
        (1, 0, 1, 400)
    );
    let _ = salt;

    // The dashboard: the last 7 days, today last.
    let d = dashboard(&s, Period::Week)?;
    assert_eq!(d.labels.len(), 7);
    assert_eq!(d.labels.last().map(String::as_str), Some("2026-11-10"));
    assert_eq!(d.current[6], 8_330 - 1_190);
    assert_eq!(d.current[4], 3_570);
    assert_eq!(d.orders.iter().sum::<i64>(), 3);
    assert_eq!(d.previous.iter().sum::<i64>(), 0);
    assert_eq!(d.recent.len(), 3);
    assert_eq!(
        d.low_stock
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["Lait"]
    );
    // A week later, those sales are the period before.
    clock.set(T0 + 7 * DAY);
    let d = dashboard(&s, Period::Week)?;
    assert_eq!(d.current.iter().sum::<i64>(), 0);
    assert_eq!(d.previous.iter().sum::<i64>(), 3_570 + 8_330 - 1_190);
    assert_eq!(d.orders_previous, 3);
    let y = dashboard(&s, Period::Year)?;
    assert_eq!(
        (y.labels.first().map(String::as_str), y.current[11]),
        (Some("2025-12-01"), 3_570 + 8_330 - 1_190)
    );
    Ok(())
}

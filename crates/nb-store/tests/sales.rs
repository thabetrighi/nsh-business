//! Point of sale (M8): till sessions, the completed sale and everything it
//! writes at once, a double tap that sells once, and what a cashier may do
//! without a supervisor.

mod common;

use common::*;
use nb_domain::{ProductId, WarehouseId};
use nb_store::Error;
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::inventory::available;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::sales::*;
use nb_store::usecases::stock::rebuild_balances;
use nb_store::{OpMeta, Store};

type R = Result<(), Error>;
type Setup = nb_store::usecases::setup::SetupResult;

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
    }
}

fn register(s: &Store) -> String {
    s.read(|r| {
        let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
        nb_store::usecases::org::registers(r.conn(), o.org_id()?)
    })
    .ok()
    .and_then(|v| v.into_iter().next())
    .map(|r| r.id)
    .unwrap_or_default()
}

/// A product with `qty` in stock at `cost`, sold at `price` (VAT 19 % in).
fn stocked(
    s: &mut Store,
    setup: &Setup,
    name: &str,
    qty: &str,
    cost: i64,
    price: i64,
) -> Result<ProductId, Error> {
    let mut p = product(name, "", price);
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

fn sale(
    setup: &Setup,
    register: &str,
    lines: Vec<(ProductId, &str)>,
    payments: Vec<(&str, i64)>,
) -> SaleInput {
    SaleInput {
        register_id: register.into(),
        warehouse_id: setup.warehouse.to_string(),
        lines: lines
            .into_iter()
            .map(|(p, q)| SaleLineInput {
                product_id: p.to_string(),
                qty: q.into(),
                ..Default::default()
            })
            .collect(),
        payments: payments
            .into_iter()
            .map(|(m, a)| PaymentInput {
                method: m.into(),
                amount_minor: a,
            })
            .collect(),
        ..Default::default()
    }
}

fn balance(s: &Store, code: &str) -> i64 {
    s.connection()
        .query_row(
            "SELECT COALESCE(SUM(debit_minor - credit_minor), 0) FROM journal_lines WHERE account_code = ?1",
            [code],
            |r| r.get(0),
        )
        .unwrap_or(i64::MIN)
}

fn healthy(s: &Store) -> R {
    assert!(
        rebuild_balances(s)?.is_empty(),
        "stock = replayed movements"
    );
    assert!(
        s.read(|r| nb_store::repo::ledger::unbalanced_entries(r.conn()))?
            .is_empty()
    );
    assert_eq!(s.verify_audit_chain()?, None);
    Ok(())
}

#[test]
fn a_sale_writes_everything_at_once_and_only_once() -> R {
    let (mut s, setup) = store();
    let reg = register(&s);
    let milk = stocked(&mut s, &setup, "Lait", "10", 800, 1_190)?;
    let oil = stocked(&mut s, &setup, "Huile", "5", 2_500, 3_570)?;
    // No sale without an open till session.
    assert_eq!(
        err(complete_sale(
            &mut s,
            meta(&setup, "sale"),
            sale(&setup, &reg, vec![(milk, "1")], vec![("cash", 1_190)])
        )),
        ["shift.none_open"]
    );
    let shift = open_shift(&mut s, meta(&setup, "shift.open"), &reg, 10_000)?;
    assert_eq!(
        err(open_shift(&mut s, meta(&setup, "shift.open"), &reg, 0)),
        ["shift.already_open"]
    );

    // 3 milk + 1 oil = 35.70 + 35.70 = 71.40, paid with 100.00 cash.
    let tap = meta(&setup, "sale");
    let input = sale(
        &setup,
        &reg,
        vec![(milk, "3"), (oil, "1")],
        vec![("cash", 10_000)],
    );
    let r = complete_sale(&mut s, tap, input.clone())?;
    assert!(r.number.starts_with("FV-01-"));
    assert_eq!(
        (r.total_minor, r.cash_minor, r.change_minor),
        (7_140, 7_140, 2_860)
    );
    assert_eq!(r.net_minor + r.tax_minor, r.total_minor);
    assert_eq!(r.cost_minor, 3 * 800 + 2_500);
    // The same tap again: the same sale, nothing more.
    let again = complete_sale(&mut s, tap, input)?;
    assert_eq!(again, r);
    assert_eq!(count(&s, "SELECT COUNT(*) FROM sales"), 1);
    assert_eq!(available(&s, milk, setup.warehouse)?.to_string(), "7");
    assert_eq!(available(&s, oil, setup.warehouse)?.to_string(), "4");
    // The entry: till, sales, VAT; cost of sales out of stock.
    assert_eq!(balance(&s, "531"), 7_140);
    assert_eq!(balance(&s, "700") + balance(&s, "4457"), -7_140);
    assert_eq!(balance(&s, "600"), 4_900);
    assert_eq!(balance(&s, "30"), 10 * 800 + 5 * 2_500 - 4_900);

    // Card and credit.
    let r = complete_sale(
        &mut s,
        meta(&setup, "sale"),
        sale(&setup, &reg, vec![(oil, "1")], vec![("card", 3_570)]),
    )?;
    assert_eq!((r.card_minor, r.cash_minor), (3_570, 0));
    assert_eq!(
        err(complete_sale(
            &mut s,
            meta(&setup, "sale"),
            sale(&setup, &reg, vec![(oil, "1")], vec![("cash", 1_000)])
        )),
        ["sale.underpaid"]
    );
    let mut on_account = sale(&setup, &reg, vec![(oil, "1")], vec![("cash", 1_000)]);
    on_account.customer_id = Some("client-1".into());
    let r = complete_sale(&mut s, meta(&setup, "sale"), on_account)?;
    assert_eq!((r.cash_minor, r.credit_minor), (1_000, 2_570));
    assert_eq!(balance(&s, "411"), 2_570);
    assert_eq!(balance(&s, "5121"), 3_570);
    // Not more than in stock, for a cashier (the owner may go below zero).
    let cashier = create_user(
        &mut s,
        meta(&setup, "user.create"),
        UserInput {
            name: "K".into(),
            phone: None,
            login: None,
            password: None,
            pin: None,
            grants: vec![GrantRow::org("cashier")],
        },
    )?;
    let as_cashier = OpMeta::new(
        "sale",
        Some(setup.org),
        Some(cashier.id.parse().unwrap_or_default()),
    );
    assert_eq!(
        err(complete_sale(
            &mut s,
            as_cashier,
            sale(&setup, &reg, vec![(oil, "9")], vec![("cash", 50_000)])
        )),
        ["stock.insufficient"]
    );

    // The session: 100.00 + 71.40 + 10.00 cash sales; 50.00 taken to the bank.
    let sum = shift_summary(&s, &shift.id)?;
    assert_eq!(
        (
            sum.sales,
            sum.cash_sales_minor,
            sum.card_minor,
            sum.credit_minor
        ),
        (3, 8_140, 3_570, 2_570)
    );
    assert_eq!(sum.expected_minor, 18_140);
    assert_eq!(
        err(cash_movement(
            &mut s,
            meta(&setup, "cash"),
            &shift.id,
            false,
            99_999,
            "banque"
        )),
        ["shift.not_enough_cash"]
    );
    let sum = cash_movement(
        &mut s,
        meta(&setup, "cash"),
        &shift.id,
        false,
        5_000,
        "versement banque",
    )?;
    assert_eq!(sum.expected_minor, 13_140);
    assert_eq!(balance(&s, "58"), 5_000);
    // Counted 2.00 short: booked as a shortage.
    let closed = close_shift(&mut s, meta(&setup, "shift.close"), &shift.id, 12_940, "")?;
    assert_eq!(
        (closed.status.as_str(), closed.difference_minor),
        ("closed", Some(-200))
    );
    assert_eq!(
        balance(&s, "531"),
        12_940 - 10_000,
        "the till's account = cash in it, less the opening float"
    );
    assert_eq!(
        err(close_shift(
            &mut s,
            meta(&setup, "shift.close"),
            &shift.id,
            0,
            ""
        )),
        ["shift.closed"]
    );
    assert_eq!(
        err(complete_sale(
            &mut s,
            meta(&setup, "sale"),
            sale(&setup, &reg, vec![(milk, "1")], vec![("cash", 1_190)])
        )),
        ["shift.none_open"]
    );
    assert!(
        s.connection()
            .execute("UPDATE sales SET total_minor = 0", [])
            .is_err(),
        "sales are immutable"
    );
    assert_eq!(recent_sales(&s, 10)?.len(), 3);
    healthy(&s)
}

#[test]
fn discounts_and_prices_beyond_a_cashier_need_a_supervisor() -> R {
    let (mut s, setup) = store();
    let reg = register(&s);
    let mut p = product("Café", "", 10_000);
    p.min_price_minor = Some(9_000);
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "20".into(),
        unit_cost_minor: 6_000,
        tracked: Default::default(),
    });
    let coffee: ProductId = create_product(&mut s, meta(&setup, "product.create"), p)?
        .id
        .parse()
        .unwrap_or_default();
    open_shift(&mut s, meta(&setup, "shift.open"), &reg, 0)?;
    let cashier = create_user(
        &mut s,
        meta(&setup, "user.create"),
        UserInput {
            name: "K".into(),
            phone: None,
            login: None,
            password: None,
            pin: None,
            grants: vec![GrantRow::org("cashier")],
        },
    )?;
    let as_cashier = || {
        OpMeta::new(
            "sale",
            Some(setup.org),
            Some(cashier.id.parse().unwrap_or_default()),
        )
    };
    let with = |bps: Option<i64>, price: Option<i64>| {
        let mut i = sale(&setup, &reg, vec![(coffee, "1")], vec![("cash", 10_000)]);
        i.lines[0].discount_bps = bps;
        i.lines[0].unit_price_minor = price;
        i
    };
    // 5 %: an ordinary discount.
    let r = complete_sale(&mut s, as_cashier(), with(Some(500), None))?;
    assert_eq!(r.total_minor, 9_500);
    // 15 %: exceptional, and below the floor price.
    assert!(matches!(
        complete_sale(&mut s, as_cashier(), with(Some(1_500), None)),
        Err(Error::Forbidden {
            permission: "sales.discount_exceptional",
            needs_approval: true
        })
    ));
    assert!(matches!(
        complete_sale(&mut s, as_cashier(), with(None, Some(9_500))),
        Err(Error::Forbidden {
            permission: "sales.price_override",
            needs_approval: true
        })
    ));
    // The owner may.
    let r = complete_sale(&mut s, meta(&setup, "sale"), with(Some(1_500), None))?;
    assert_eq!(r.total_minor, 8_500);
    healthy(&s)
}

#[test]
fn serial_numbers_leave_with_the_sale() -> R {
    let (mut s, setup) = store();
    let reg = register(&s);
    let mut p = product("Téléphone", "", 3_000_000);
    p.tracking = Some("serial".into());
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "2".into(),
        unit_cost_minor: 2_000_000,
        tracked: nb_store::usecases::tracked::Tracked {
            serials: vec!["IMEI-1".into(), "IMEI-2".into()],
            ..Default::default()
        },
    });
    let phone: ProductId = create_product(&mut s, meta(&setup, "product.create"), p)?
        .id
        .parse()
        .unwrap_or_default();
    open_shift(&mut s, meta(&setup, "shift.open"), &reg, 0)?;
    let mut input = sale(&setup, &reg, vec![(phone, "1")], vec![("card", 3_000_000)]);
    assert_eq!(
        err(complete_sale(&mut s, meta(&setup, "sale"), input.clone())),
        ["serial.count"]
    );
    input.lines[0].tracked.serials = vec!["IMEI-2".into()];
    complete_sale(&mut s, meta(&setup, "sale"), input.clone())?;
    // The same number cannot be sold twice.
    assert_eq!(
        err(complete_sale(&mut s, meta(&setup, "sale"), input)),
        ["serial.not_in_stock"]
    );
    let _: WarehouseId = setup.warehouse;
    healthy(&s)
}

#[test]
fn items_not_in_the_catalog_and_renamed_lines() -> R {
    let (mut s, setup) = store();
    let reg = register(&s);
    let milk = stocked(&mut s, &setup, "Lait", "10", 800, 1_190)?;
    open_shift(&mut s, meta(&setup, "shift.open"), &reg, 0)?;
    let mut input = sale(&setup, &reg, vec![(milk, "1")], vec![("cash", 5_000)]);
    input.lines[0].name = Some("Lait demi-écrémé".into());
    input.lines.push(SaleLineInput {
        product_id: String::new(),
        qty: "2".into(),
        name: Some("Sac cabas".into()),
        unit_price_minor: Some(1_000),
        tax_bps: Some(1_900),
        ..Default::default()
    });
    let r = complete_sale(&mut s, meta(&setup, "sale"), input.clone())?;
    assert_eq!(r.total_minor, 1_190 + 2_000);
    assert_eq!(
        r.cost_minor, 800,
        "a typed item has no cost and moves no stock"
    );
    let names: Vec<(String, Option<String>)> = s
        .connection()
        .prepare("SELECT name, product_id FROM sale_lines ORDER BY position")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    assert_eq!(names[0].0, "Lait demi-écrémé");
    assert_eq!(names[1], ("Sac cabas".into(), None));
    // A typed item needs its name and its price.
    let mut nameless = input.clone();
    nameless.lines[1].name = None;
    assert_eq!(
        err(complete_sale(&mut s, meta(&setup, "sale"), nameless)),
        ["sale.free_name"]
    );
    let mut priceless = input;
    priceless.lines[1].unit_price_minor = None;
    assert_eq!(
        err(complete_sale(&mut s, meta(&setup, "sale"), priceless)),
        ["sale.free_price"]
    );
    healthy(&s)
}

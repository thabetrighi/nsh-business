//! Expenses and other income (M9.11), and debts adjusted (M9.9).

mod common;

use common::*;
use nb_store::Error;
use nb_store::Store;
use nb_store::usecases::customers::{CustomerInput, get_customer, save_customer};
use nb_store::usecases::debts::{AdjustInput, adjust_debt, open_invoices};
use nb_store::usecases::expenses::*;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::sales::*;

type R = Result<(), Error>;
type Setup = nb_store::usecases::setup::SetupResult;

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
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

fn category(s: &Store, name: &str) -> String {
    expense_categories(s)
        .unwrap_or_default()
        .into_iter()
        .find(|c| c.name == name)
        .map(|c| c.id)
        .unwrap_or_else(|| panic!("{name}"))
}

#[test]
fn expenses_come_out_of_the_till_or_the_bank() -> R {
    let (mut s, setup) = store();
    let reg = register(&s);
    let cats = expense_categories(&s)?;
    assert!(
        cats.iter()
            .any(|c| c.name == "Loyer" && c.account_code == "613")
    );
    let rent = category(&s, "Loyer");
    let power = category(&s, "Électricité, eau et gaz");
    let input = |cat: &str, total: i64, vat: i64, method: &str| ExpenseInput {
        category_id: cat.into(),
        total_minor: total,
        vat_minor: vat,
        method: method.into(),
        register_id: Some(reg.clone()),
        payee: "Sonelgaz".into(),
        ..Default::default()
    };
    // Cash needs an open till, and no more than it holds.
    assert_eq!(
        err(record_expense(
            &mut s,
            meta(&setup, "e"),
            input(&power, 5_000, 0, "cash")
        )),
        ["shift.none_open"]
    );
    open_shift(&mut s, meta(&setup, "shift"), &reg, 10_000)?;
    assert_eq!(
        err(record_expense(
            &mut s,
            meta(&setup, "e"),
            input(&power, 20_000, 0, "cash")
        )),
        ["shift.not_enough_cash"]
    );
    assert_eq!(
        err(record_expense(
            &mut s,
            meta(&setup, "e"),
            input(&power, 5_000, 5_000, "cash")
        )),
        ["expense.vat"]
    );
    // 59.50 of power, 9.50 of it deductible VAT, from the till.
    let e = record_expense(&mut s, meta(&setup, "e"), input(&power, 5_950, 950, "cash"))?;
    assert!(e.number.starts_with("DP-"));
    assert_eq!((e.net_minor, e.vat_minor), (5_000, 950));
    assert_eq!((balance(&s, "607"), balance(&s, "4456")), (5_000, 950));
    assert_eq!(balance(&s, "531"), -5_950);
    // Rent by the bank: no till needed.
    let mut by_bank = input(&rent, 300_000, 0, "bank");
    by_bank.register_id = None;
    record_expense(&mut s, meta(&setup, "e"), by_bank)?;
    assert_eq!(
        (balance(&s, "613"), balance(&s, "5121")),
        (300_000, -300_000)
    );
    // Income into the till.
    let other = category(&s, "Autres produits");
    record_expense(&mut s, meta(&setup, "e"), input(&other, 2_000, 0, "cash"))?;
    assert_eq!(balance(&s, "758"), -2_000);
    // The till expects what moved.
    let shift = current_shift(&s, &reg)?.unwrap_or_else(|| panic!("open"));
    assert_eq!(
        (
            shift.expenses_minor,
            shift.other_income_minor,
            shift.expected_minor
        ),
        (5_950, 2_000, 10_000 - 5_950 + 2_000)
    );
    // Listed, with totals by category.
    let page = list_expenses(
        &s,
        &ExpenseQuery {
            limit: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(
        (page.total, page.expenses_minor, page.income_minor),
        (3, 305_950, 2_000)
    );
    assert_eq!(
        page.by_category[0],
        ("Loyer".into(), "expense".into(), 300_000)
    );
    // A new category, only on a charge account that takes entries.
    assert_eq!(
        err(create_category(
            &mut s,
            meta(&setup, "c"),
            "expense",
            "Gardiennage",
            "700"
        )),
        ["expense.wrong_account"]
    );
    assert_eq!(
        err(create_category(
            &mut s,
            meta(&setup, "c"),
            "expense",
            "Gardiennage",
            "62"
        )),
        ["expense.wrong_account"]
    );
    let c = create_category(&mut s, meta(&setup, "c"), "expense", "Gardiennage", "615")?;
    assert_eq!(c.account_code, "615");
    assert!(s.health()?.is_sound());
    Ok(())
}

fn credit_sale(s: &mut Store, setup: &Setup, customer: &str) -> Result<SaleResult, Error> {
    let reg = register(s);
    if current_shift(s, &reg)?.is_none() {
        open_shift(s, meta(setup, "shift"), &reg, 0)?;
    }
    let mut p = product("Huile", "", 3_570);
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "10".into(),
        unit_cost_minor: 2_500,
        tracked: Default::default(),
    });
    let oil = create_product(s, meta(setup, "p"), p)?.id;
    complete_sale(
        s,
        meta(setup, "sale"),
        SaleInput {
            register_id: reg,
            warehouse_id: setup.warehouse.to_string(),
            lines: vec![SaleLineInput {
                product_id: oil,
                qty: "2".into(),
                ..Default::default()
            }],
            payments: vec![PaymentInput {
                method: "credit".into(),
                amount_minor: 0,
            }],
            customer_id: Some(customer.into()),
            ..Default::default()
        },
    )
}

#[test]
fn debts_are_written_off_or_eased_with_a_reason() -> R {
    let (mut s, setup) = store();
    let c = save_customer(
        &mut s,
        meta(&setup, "c"),
        CustomerInput {
            kind: "person".into(),
            name: "Samir".into(),
            credit_limit_minor: 100_000,
            status: "active".into(),
            ..Default::default()
        },
    )?;
    // What it owed before the books started: 120.00.
    let adjust = |kind: &str, amount: i64, reason: &str| AdjustInput {
        customer_id: c.id.clone(),
        kind: kind.into(),
        amount_minor: amount,
        reason: reason.into(),
        approval: None,
    };
    assert_eq!(
        err(adjust_debt(
            &mut s,
            meta(&setup, "a"),
            adjust("opening", 12_000, " ")
        )),
        ["required"]
    );
    adjust_debt(
        &mut s,
        meta(&setup, "a"),
        adjust("opening", 12_000, "Cahier de crédit"),
    )?;
    assert_eq!(balance(&s, "411"), 12_000);
    assert_eq!(get_customer(&s, &c.id)?.balance_minor, 12_000);
    // An invoice on account (71.40), then 10.00 off as an allowance.
    let sale = credit_sale(&mut s, &setup, &c.id)?;
    let r = adjust_debt(
        &mut s,
        meta(&setup, "a"),
        adjust("allowance", 1_000, "Geste commercial"),
    )?;
    assert_eq!(r.settled, [(sale.number.clone(), 1_000)]);
    assert_eq!(balance(&s, "709"), 1_000);
    assert_eq!(open_invoices(&s, &c.id)?[0].open_minor, 6_140);
    // Never more than it owes; the rest given up.
    assert_eq!(
        err(adjust_debt(
            &mut s,
            meta(&setup, "a"),
            adjust("write_off", 20_000, "Parti")
        )),
        ["adjust.more_than_owed"]
    );
    let r = adjust_debt(
        &mut s,
        meta(&setup, "a"),
        adjust("write_off", 18_140, "Parti sans laisser d'adresse"),
    )?;
    assert_eq!(r.balance_minor, 0);
    assert_eq!(balance(&s, "654"), 18_140);
    assert!(open_invoices(&s, &c.id)?.is_empty());
    assert!(s.health()?.is_sound());
    Ok(())
}

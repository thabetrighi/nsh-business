//! Collecting debts (M9.4) and seeing them (M9.5): a receipt settles the
//! oldest invoices or the ones chosen, never more than they owe; what is
//! left waits as an advance; the statement and the ageing follow the
//! entries.

mod common;

use std::sync::Arc;

use common::*;
use nb_core::FixedClock;
use nb_store::usecases::customers::{CustomerInput, get_customer, save_customer};
use nb_store::usecases::debts::*;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::sales::*;
use nb_store::usecases::setup::{SetupResult, setup_organization};
use nb_store::{Error, Store, StoreOptions};

type R = Result<(), Error>;

const DAY: i64 = 86_400_000;

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
    }
}

struct Shop {
    s: Store,
    setup: SetupResult,
    clock: Arc<FixedClock>,
    reg: String,
    oil: String,
    customer: String,
}

/// A shop whose clock the test moves; a customer with 1 000.00 of credit
/// and 30 days to pay.
fn shop() -> Result<Shop, Error> {
    let clock = Arc::new(FixedClock::new(T0));
    let mut s = Store::open_in_memory(StoreOptions {
        clock: clock.clone(),
        ..options("A")
    })?;
    let setup = setup_organization(&mut s, setup_input())?;
    let mut p = product("Huile", "", 3_570);
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "100".into(),
        unit_cost_minor: 2_500,
        tracked: Default::default(),
    });
    let oil = create_product(&mut s, meta(&setup, "p"), p)?.id;
    let reg = s
        .read(|r| {
            let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
            nb_store::usecases::org::registers(r.conn(), o.org_id()?)
        })?
        .remove(0)
        .id;
    open_shift(&mut s, meta(&setup, "shift"), &reg, 0)?;
    let customer = save_customer(
        &mut s,
        meta(&setup, "c"),
        CustomerInput {
            kind: "company".into(),
            name: "El Amel".into(),
            credit_limit_minor: 100_000,
            payment_days: 30,
            status: "active".into(),
            ..Default::default()
        },
    )?
    .id;
    Ok(Shop {
        s,
        setup,
        clock,
        reg,
        oil,
        customer,
    })
}

impl Shop {
    /// `qty` oil all on account.
    fn on_account(&mut self, qty: &str) -> Result<SaleResult, Error> {
        let input = SaleInput {
            register_id: self.reg.clone(),
            warehouse_id: self.setup.warehouse.to_string(),
            lines: vec![SaleLineInput {
                product_id: self.oil.clone(),
                qty: qty.into(),
                ..Default::default()
            }],
            payments: vec![PaymentInput {
                method: "credit".into(),
                amount_minor: 0,
            }],
            customer_id: Some(self.customer.clone()),
            ..Default::default()
        };
        complete_sale(&mut self.s, meta(&self.setup, "sale"), input)
    }
    fn pay(&mut self, input: ReceiptInput) -> Result<ReceiptResult, Error> {
        let m = meta(&self.setup, "receipt");
        collect(&mut self.s, m, input)
    }
    fn pay_as(&mut self, m: nb_store::OpMeta, input: ReceiptInput) -> Result<ReceiptResult, Error> {
        collect(&mut self.s, m, input)
    }
    fn receipt(&self, amount: i64, method: &str) -> ReceiptInput {
        ReceiptInput {
            customer_id: self.customer.clone(),
            amount_minor: amount,
            method: method.into(),
            register_id: Some(self.reg.clone()),
            ..Default::default()
        }
    }
}

#[test]
fn a_receipt_settles_the_oldest_invoices_first() -> R {
    let mut shop = shop()?;
    // Three invoices on account: 100 days ago, 45 days ago, today.
    shop.clock.set(T0 - 100 * DAY);
    let a = shop.on_account("2")?; // 71.40
    shop.clock.set(T0 - 45 * DAY);
    let b = shop.on_account("1")?; // 35.70
    shop.clock.set(T0);
    let c = shop.on_account("3")?; // 107.10
    assert_eq!(a.credit_minor, 7_140);
    let open = open_invoices(&shop.s, &shop.customer)?;
    assert_eq!(
        open.iter()
            .map(|o| (o.age_days, o.overdue))
            .collect::<Vec<_>>(),
        [(100, true), (45, true), (0, false)]
    );

    // Ageing: by the age of what is open.
    let ag = ageing(&shop.s)?;
    assert_eq!(ag.rows[0].buckets, [10_710, 3_570, 0, 7_140]);
    assert_eq!(
        (ag.rows[0].overdue_minor, ag.balance_minor),
        (10_710, 21_420)
    );

    // 80.00 in cash: the oldest whole (71.40), then 8.60 of the next.
    let r = shop.pay(shop.receipt(8_000, "cash"))?;
    assert!(r.number.starts_with("RC-"));
    assert_eq!(
        r.settled,
        [(a.number.clone(), 7_140), (b.number.clone(), 860)]
    );
    assert_eq!((r.unallocated_minor, r.balance_minor), (0, 21_420 - 8_000));
    let open = open_invoices(&shop.s, &shop.customer)?;
    assert_eq!(
        open.iter().map(|o| o.open_minor).collect::<Vec<_>>(),
        [3_570 - 860, 10_710]
    );
    // The till holds the cash received.
    let shift = current_shift(&shop.s, &shop.reg)?.unwrap_or_else(|| panic!("open"));
    assert_eq!(
        (shift.collections_minor, shift.expected_minor),
        (8_000, 8_000)
    );

    // Chosen by hand: the newest one, by card; never more than it owes.
    let mut chosen = shop.receipt(20_000, "card");
    chosen.allocations = vec![AllocationInput {
        sale_id: c.id.clone(),
        amount_minor: 20_000,
    }];
    assert_eq!(err(shop.pay(chosen.clone())), ["receipt.more_than_open"]);
    chosen.allocations[0].amount_minor = 10_710;
    let r = shop.pay(chosen)?;
    // The rest waits as an advance.
    assert_eq!(
        (r.unallocated_minor, r.balance_minor),
        (20_000 - 10_710, 13_420 - 20_000)
    );
    assert_eq!(get_customer(&shop.s, &shop.customer)?.balance_minor, -6_580);
    let card_in_bank: i64 = shop.s.connection().query_row(
        "SELECT SUM(debit_minor - credit_minor) FROM journal_lines WHERE account_code = '5121'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(card_in_bank, 20_000);

    // An invoice already settled takes nothing more.
    let mut again = shop.receipt(100, "card");
    again.allocations = vec![AllocationInput {
        sale_id: a.id.clone(),
        amount_minor: 100,
    }];
    assert_eq!(err(shop.pay(again)), ["receipt.not_open"]);
    assert!(shop.s.health()?.is_sound());
    Ok(())
}

#[test]
fn receipts_are_recorded_once_and_need_a_till_for_cash() -> R {
    let mut shop = shop()?;
    shop.on_account("2")?;
    assert_eq!(
        err(shop.pay(shop.receipt(0, "cash"))),
        ["receipt.amount_positive"]
    );
    let mut no_till = shop.receipt(1_000, "cash");
    no_till.register_id = None;
    assert_eq!(err(shop.pay(no_till)), ["shift.none_open"]);
    // A transfer needs no till.
    let mut transfer = shop.receipt(1_000, "transfer");
    transfer.register_id = None;
    shop.pay(transfer)?;
    // A double tap collects once.
    let tap = meta(&shop.setup, "receipt");
    let first = shop.pay_as(tap, shop.receipt(2_000, "cash"))?;
    let second = shop.pay_as(tap, shop.receipt(2_000, "cash"))?;
    assert_eq!(first, second);
    assert_eq!(receipts_of(&shop.s, &shop.customer)?.len(), 2);
    assert_eq!(
        get_customer(&shop.s, &shop.customer)?.balance_minor,
        7_140 - 3_000
    );
    Ok(())
}

#[test]
fn the_statement_runs_from_its_opening_balance() -> R {
    let mut shop = shop()?;
    shop.clock.set(T0 - 40 * DAY);
    shop.on_account("2")?; // +71.40
    shop.clock.set(T0 - 10 * DAY);
    shop.pay(shop.receipt(5_000, "cash"))?; // −50.00
    shop.clock.set(T0);
    shop.on_account("1")?; // +35.70
    let all = customer_statement(&shop.s, &shop.customer, None, None)?;
    assert_eq!(
        all.lines
            .iter()
            .map(|l| (
                l.source.as_str(),
                l.debit_minor,
                l.credit_minor,
                l.balance_minor
            ))
            .collect::<Vec<_>>(),
        [
            ("sale", 7_140, 0, 7_140),
            ("collection", 0, 5_000, 2_140),
            ("sale", 3_570, 0, 5_710)
        ]
    );
    // From 20 days ago: what came before is the opening.
    let from = nb_store::time::iso_date(T0 - 20 * DAY, 60);
    let recent = customer_statement(&shop.s, &shop.customer, Some(&from), None)?;
    assert_eq!(
        (
            recent.opening_minor,
            recent.lines.len(),
            recent.closing_minor
        ),
        (7_140, 2, 5_710)
    );
    assert_eq!(
        recent.closing_minor,
        get_customer(&shop.s, &shop.customer)?.balance_minor
    );
    Ok(())
}

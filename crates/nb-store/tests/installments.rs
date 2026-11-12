//! Instalments (M9.6–M9.8): an invoice on account split into instalments
//! that add up to it, paid oldest first (by the instalment screen or by any
//! receipt given to the invoice), late when past their date, rescheduled
//! with approval as a new version that keeps the old one.

mod common;

use std::sync::Arc;

use common::*;
use nb_core::FixedClock;
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::customers::{CustomerInput, get_customer, save_customer};
use nb_store::usecases::debts::{ReceiptInput, collect};
use nb_store::usecases::installments::*;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::sales::*;
use nb_store::usecases::setup::{SetupResult, setup_organization};
use nb_store::{Error, OpMeta, Store, StoreOptions};

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

fn shop() -> Result<Shop, Error> {
    let clock = Arc::new(FixedClock::new(T0));
    let mut s = Store::open_in_memory(StoreOptions {
        clock: clock.clone(),
        ..options("A")
    })?;
    let setup = setup_organization(&mut s, setup_input())?;
    let mut p = product("Réfrigérateur", "", 3_570);
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
            kind: "person".into(),
            name: "Karim".into(),
            credit_limit_minor: 100_000,
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
    /// `qty` sold, `cash` paid now, the rest on account.
    fn sell(&mut self, qty: &str, cash: i64) -> Result<SaleResult, Error> {
        let input = SaleInput {
            register_id: self.reg.clone(),
            warehouse_id: self.setup.warehouse.to_string(),
            lines: vec![SaleLineInput {
                product_id: self.oil.clone(),
                qty: qty.into(),
                ..Default::default()
            }],
            payments: vec![PaymentInput {
                method: "cash".into(),
                amount_minor: cash,
            }],
            customer_id: Some(self.customer.clone()),
            ..Default::default()
        };
        complete_sale(&mut self.s, meta(&self.setup, "sale"), input)
    }
    fn day(&self, n: i64) -> String {
        nb_store::time::iso_date(T0 + n * DAY, 60)
    }
    fn plan(&mut self, sale: &str, count: u32, frequency: &str, first: i64) -> Result<Plan, Error> {
        let input = PlanInput {
            sale_id: sale.into(),
            count,
            frequency: frequency.into(),
            first_due: self.day(first),
        };
        create_plan(&mut self.s, meta(&self.setup, "plan"), input)
    }
    fn pay(
        &mut self,
        plan: &str,
        amount: i64,
    ) -> Result<nb_store::usecases::debts::ReceiptResult, Error> {
        let reg = Some(self.reg.clone());
        pay_installments(
            &mut self.s,
            meta(&self.setup, "pay"),
            plan,
            amount,
            "cash",
            reg,
        )
    }
}

#[test]
fn a_plan_adds_up_and_is_paid_oldest_first() -> R {
    let mut shop = shop()?;
    // 10 × 35.70 = 357.00: 57.00 down, 300.00 on account.
    let sale = shop.sell("10", 5_700)?;
    assert_eq!(sale.credit_minor, 30_000);
    let paid_sale = shop.sell("1", 3_570)?;
    assert_eq!(
        err(shop.plan(&paid_sale.id, 3, "monthly", 30)),
        ["plan.nothing_to_finance"]
    );
    assert_eq!(err(shop.plan(&sale.id, 0, "monthly", 30)), ["plan.count"]);
    assert_eq!(err(shop.plan(&sale.id, 3, "yearly", 30)), ["invalid_value"]);

    // 300.00 in 7 monthly instalments: 42.86 for the first two, 42.85 after.
    let plan = shop.plan(&sale.id, 7, "monthly", 30)?;
    assert!(plan.number.starts_with("TQ-"));
    assert_eq!((plan.down_minor, plan.financed_minor), (5_700, 30_000));
    let amounts: Vec<i64> = plan.installments.iter().map(|i| i.amount_minor).collect();
    assert_eq!(amounts, [4_286, 4_286, 4_286, 4_286, 4_286, 4_285, 4_285]);
    assert_eq!(amounts.iter().sum::<i64>(), 30_000);
    assert_eq!(plan.installments[0].due_date, shop.day(30));
    assert_eq!(err(shop.plan(&sale.id, 3, "monthly", 30)), ["plan.already"]);

    // 50.00: the first whole, 7.14 of the second.
    shop.pay(&plan.id, 5_000)?;
    let plan = get_plan(&shop.s, &plan.id)?;
    assert_eq!(
        plan.installments
            .iter()
            .take(3)
            .map(|i| (i.paid_minor, i.state.as_str()))
            .collect::<Vec<_>>(),
        [(4_286, "paid"), (714, "partial"), (0, "upcoming")]
    );
    assert_eq!(get_customer(&shop.s, &shop.customer)?.balance_minor, 25_000);
    // Any receipt given to the invoice pays its instalments too.
    let rc = collect(
        &mut shop.s,
        meta(&shop.setup, "receipt"),
        ReceiptInput {
            customer_id: shop.customer.clone(),
            amount_minor: 3_572,
            method: "cash".into(),
            register_id: Some(shop.reg.clone()),
            ..Default::default()
        },
    )?;
    let plan = get_plan(&shop.s, &plan.id)?;
    assert_eq!(
        (
            plan.installments[1].state.as_str(),
            plan.installments[2].paid_minor
        ),
        ("paid", 0)
    );
    // Its voucher: who paid, how, and what it paid (the invoice, the
    // instalments).
    let v = nb_store::usecases::printing::print_voucher(&shop.s, &rc.number)?;
    assert_eq!(
        v,
        nb_store::usecases::printing::print_voucher(&shop.s, &rc.id)?
    );
    assert_eq!((v.amount_minor, v.method.as_str()), (3_572, "cash"));
    assert_eq!(v.invoices, [(sale.number.clone(), 3_572)]);
    assert_eq!(v.installments, [(plan.number.clone(), 2, 3_572)]);
    assert!(!v.customer.0.is_empty());
    assert_eq!(plan.open_minor(), 30_000 - 8_572);

    // Two months on, the third is late and listed.
    shop.clock.set(T0 + 95 * DAY);
    let plan = get_plan(&shop.s, &plan.id)?;
    assert_eq!(plan.installments[2].state, "late");
    assert_eq!(plan.late_minor(), 4_286);
    let due = due_installments(&shop.s, 7)?;
    assert_eq!(
        due.iter()
            .map(|d| (d.seq, d.state.as_str()))
            .collect::<Vec<_>>(),
        [(3, "late")]
    );
    assert_eq!(due[0].days_late, 3);

    // Never more than left; the rest pays it off and closes it.
    assert_eq!(
        err(shop.pay(&plan.id, plan.open_minor() + 1)),
        ["plan.more_than_left"]
    );
    shop.pay(&plan.id, plan.open_minor())?;
    let plan = get_plan(&shop.s, &plan.id)?;
    assert_eq!((plan.status.as_str(), plan.open_minor()), ("done", 0));
    assert_eq!(get_customer(&shop.s, &shop.customer)?.balance_minor, 0);
    assert!(shop.s.health()?.is_sound());
    Ok(())
}

#[test]
fn rescheduling_needs_approval_and_keeps_the_old_plan() -> R {
    let mut shop = shop()?;
    let sale = shop.sell("10", 0)?;
    let plan = shop.plan(&sale.id, 3, "monthly", 30)?;
    shop.pay(&plan.id, 11_900)?;
    let cashier = create_user(
        &mut shop.s,
        meta(&shop.setup, "user"),
        UserInput {
            name: "K".into(),
            phone: None,
            login: None,
            password: None,
            pin: None,
            grants: vec![GrantRow::org("cashier")],
        },
    )?;
    let input = RescheduleInput {
        count: 4,
        frequency: "weekly".into(),
        first_due: shop.day(10),
        reason: "Fin de saison".into(),
        approval: None,
    };
    let as_cashier = OpMeta::new(
        "reschedule",
        Some(shop.setup.org),
        Some(cashier.id.parse().unwrap_or_default()),
    );
    assert_eq!(
        err(reschedule(&mut shop.s, as_cashier, &plan.id, input.clone())),
        ["access.needs_approval"]
    );
    let mut no_reason = input.clone();
    no_reason.reason = " ".into();
    assert_eq!(
        err(reschedule(
            &mut shop.s,
            meta(&shop.setup, "r"),
            &plan.id,
            no_reason
        )),
        ["required"]
    );
    // What is left (238.00) in 4 weekly instalments; version 2.
    let new = reschedule(&mut shop.s, meta(&shop.setup, "r"), &plan.id, input)?;
    assert_eq!(
        (new.version, new.financed_minor, new.number.as_str()),
        (2, 23_800, plan.number.as_str())
    );
    assert_eq!(
        new.installments.iter().map(|i| i.amount_minor).sum::<i64>(),
        23_800
    );
    assert_eq!(new.installments[1].due_date, shop.day(17));
    // The old plan is kept as it was: its paid instalment, the rest moved.
    let old = get_plan(&shop.s, &plan.id)?;
    assert_eq!(old.status, "rescheduled");
    assert_eq!(
        old.installments
            .iter()
            .map(|i| i.state.as_str())
            .collect::<Vec<_>>(),
        ["paid", "rescheduled", "rescheduled"]
    );
    assert!(
        shop.s
            .connection()
            .execute(
                "UPDATE installment_plans SET status = 'active' WHERE id = ?1",
                [&plan.id]
            )
            .is_err()
    );
    let all = plans_of(&shop.s, &shop.customer)?;
    assert_eq!(all.iter().map(|p| p.version).collect::<Vec<_>>(), [2, 1]);
    // Payments now go to the new schedule.
    shop.pay(&new.id, 5_950)?;
    assert_eq!(get_plan(&shop.s, &new.id)?.installments[0].state, "paid");
    Ok(())
}

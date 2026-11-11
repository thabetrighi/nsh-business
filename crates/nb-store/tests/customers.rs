//! Customers (M9.1), their debt as the sum of their receivable lines
//! (M9.2), and credit at the till (M9.3).

mod common;

use common::*;
use nb_store::Error;
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::customers::*;
use nb_store::usecases::ledger::reverse_entry;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::returns::{ReturnInput, ReturnLineInput, return_sale};
use nb_store::usecases::sales::*;
use nb_store::{OpMeta, Store};

type R = Result<(), Error>;
type Setup = nb_store::usecases::setup::SetupResult;

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
    }
}

fn customer(name: &str, limit: i64) -> CustomerInput {
    CustomerInput {
        kind: "company".into(),
        name: name.into(),
        credit_limit_minor: limit,
        status: "active".into(),
        ..Default::default()
    }
}

struct Shop {
    s: Store,
    setup: Setup,
    c: Ctx,
}

struct Ctx {
    warehouse: String,
    reg: String,
    oil: String,
}

fn shop() -> Result<Shop, Error> {
    let (mut s, setup) = store();
    let mut p = product("Huile", "", 3_570);
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "50".into(),
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
    let c = Ctx {
        warehouse: setup.warehouse.to_string(),
        reg,
        oil,
    };
    Ok(Shop { s, setup, c })
}

impl Ctx {
    /// `qty` oil, paying `cash` now and the rest on `customer`'s account.
    fn on_account(&self, customer: &str, qty: &str, cash: i64) -> SaleInput {
        SaleInput {
            register_id: self.reg.clone(),
            warehouse_id: self.warehouse.clone(),
            lines: vec![SaleLineInput {
                product_id: self.oil.clone(),
                qty: qty.into(),
                ..Default::default()
            }],
            payments: vec![PaymentInput {
                method: "cash".into(),
                amount_minor: cash,
            }],
            customer_id: Some(customer.into()),
            ..Default::default()
        }
    }
}

#[test]
fn customers_are_numbered_checked_and_found() -> R {
    let (mut s, setup) = store();
    assert_eq!(
        err(save_customer(&mut s, meta(&setup, "c"), customer(" ", 0))),
        ["required"]
    );
    let mut bad = customer("X", 0);
    bad.email = "pas-un-email".into();
    assert_eq!(
        err(save_customer(&mut s, meta(&setup, "c"), bad)),
        ["invalid_email"]
    );
    let a = save_customer(
        &mut s,
        meta(&setup, "c"),
        customer("Boulangerie El Amel", 0),
    )?;
    let mut input = customer("Karim Bouzid", 0);
    input.kind = "person".into();
    input.phone = "0661 23 45 67".into();
    input.city = "El Oued".into();
    let b = save_customer(&mut s, meta(&setup, "c"), input.clone())?;
    assert_eq!((a.code.as_str(), b.code.as_str()), ("C-00001", "C-00002"));
    // Changed in place.
    input.id = Some(b.id.clone());
    input.name = "Karim Bouzid (Guemar)".into();
    let b = save_customer(&mut s, meta(&setup, "c"), input)?;
    assert_eq!((b.version, b.code.as_str()), (2, "C-00002"));

    let g = create_group(&mut s, meta(&setup, "g"), "Boulangeries")?;
    assert!(matches!(
        create_group(&mut s, meta(&setup, "g"), "boulangeries"),
        Err(Error::Duplicate(_))
    ));
    let mut a_in = customer("Boulangerie El Amel", 0);
    a_in.id = Some(a.id.clone());
    a_in.group_id = Some(g.id.clone());
    save_customer(&mut s, meta(&setup, "c"), a_in)?;
    assert_eq!(customer_groups(&s)?[0].customers, 1);

    let found = list_customers(
        &s,
        &CustomerQuery {
            search: "0661".into(),
            limit: 10,
            ..Default::default()
        },
    )?;
    assert_eq!(found.rows.len(), 1);
    assert_eq!(found.rows[0].name, "Karim Bouzid (Guemar)");
    Ok(())
}

#[test]
fn a_debt_is_the_sum_of_its_entries() -> R {
    let mut shop = shop()?;
    let c = save_customer(
        &mut shop.s,
        meta(&shop.setup, "c"),
        customer("El Amel", 20_000),
    )?;
    // 2 oil = 71.40: 21.40 cash, 50.00 on account.
    let input = shop.c.on_account(&c.id, "2", 2_140);
    let sale = complete_sale(&mut shop.s, meta(&shop.setup, "sale"), input)?;
    assert_eq!(sale.credit_minor, 5_000);
    assert_eq!(get_customer(&shop.s, &c.id)?.balance_minor, 5_000);
    // One back, refunded on the account: 35.70 off the debt.
    return_sale(
        &mut shop.s,
        meta(&shop.setup, "return"),
        ReturnInput {
            sale: sale.number.clone(),
            register_id: shop.c.reg.clone(),
            lines: vec![ReturnLineInput {
                position: 0,
                qty: "1".into(),
                restock: true,
            }],
            refund: "credit".into(),
            reason: "Abîmé".into(),
            approval: None,
        },
    )?;
    assert_eq!(get_customer(&shop.s, &c.id)?.balance_minor, 5_000 - 3_570);
    // A reversed entry moves the debt back too.
    let entry: String = shop.s.connection().query_row(
        "SELECT id FROM journal_entries WHERE source = 'sale.return'",
        [],
        |r| r.get(0),
    )?;
    reverse_entry(&mut shop.s, meta(&shop.setup, "rev"), &entry, "Erreur")?;
    assert_eq!(get_customer(&shop.s, &c.id)?.balance_minor, 5_000);
    // The projection equals the lines, and the lines the account 411.
    assert!(customer_drift(shop.s.connection())?.is_empty());
    let on_411: i64 = shop.s.connection().query_row(
        "SELECT SUM(debit_minor - credit_minor) FROM journal_lines WHERE account_code = '411'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(on_411, 5_000);
    assert!(shop.s.health()?.is_sound());
    // A balance changed by hand is caught.
    shop.s.connection().execute(
        "UPDATE customers SET balance_minor = 1 WHERE id = ?1",
        [&c.id],
    )?;
    assert_eq!(customer_drift(shop.s.connection())?.len(), 1);
    assert!(!shop.s.health()?.is_sound());
    Ok(())
}

#[test]
fn credit_past_the_limit_needs_a_supervisor() -> R {
    let mut shop = shop()?;
    let c = save_customer(
        &mut shop.s,
        meta(&shop.setup, "c"),
        customer("El Amel", 5_000),
    )?;
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
    let (org, cashier_id) = (shop.setup.org, cashier.id.parse().unwrap_or_default());
    let as_cashier = move || OpMeta::new("sale", Some(org), Some(cashier_id));
    // Within the limit: the cashier sells on account.
    complete_sale(&mut shop.s, as_cashier(), shop.c.on_account(&c.id, "1", 0))?;
    // 35.70 + 35.70 > 50.00: past the limit.
    let codes = err(complete_sale(
        &mut shop.s,
        as_cashier(),
        shop.c.on_account(&c.id, "1", 0),
    ));
    assert_eq!(codes, ["access.needs_approval"]);
    // Paid in full, the same customer buys without credit.
    complete_sale(
        &mut shop.s,
        as_cashier(),
        shop.c.on_account(&c.id, "1", 3_570),
    )?;
    // The owner may go past it.
    complete_sale(
        &mut shop.s,
        meta(&shop.setup, "sale"),
        shop.c.on_account(&c.id, "1", 0),
    )?;
    assert_eq!(get_customer(&shop.s, &c.id)?.balance_minor, 7_140);
    // Blocked: no more credit, even for the owner.
    let mut input = customer("El Amel", 5_000);
    input.id = Some(c.id.clone());
    input.status = "blocked".into();
    save_customer(&mut shop.s, meta(&shop.setup, "c"), input)?;
    assert_eq!(
        err(complete_sale(
            &mut shop.s,
            meta(&shop.setup, "sale"),
            shop.c.on_account(&c.id, "1", 0)
        )),
        ["customer.blocked"]
    );
    // An unknown customer is refused.
    assert_eq!(
        err(complete_sale(
            &mut shop.s,
            meta(&shop.setup, "sale"),
            shop.c.on_account("nobody", "1", 3_570)
        )),
        ["customer.unknown"]
    );
    // A cashier keeps no customer file (and sets no credit limit).
    let mut raise = customer("Nouveau", 100_000);
    raise.kind = "person".into();
    assert_eq!(
        err(save_customer(&mut shop.s, as_cashier(), raise)),
        ["access.denied"]
    );
    Ok(())
}

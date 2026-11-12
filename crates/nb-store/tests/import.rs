//! The import wizard (M14.3): a sheet's columns mapped to fields, checked
//! without writing, then written all at once — customers and suppliers
//! with their opening balances, products with their opening stock.

mod common;

use common::*;
use nb_domain::import::{Kind, guess, parse, rows};
use nb_store::Error;
use nb_store::usecases::customers::{CustomerQuery, list_customers};
use nb_store::usecases::import::import_rows;
use nb_store::usecases::purchasing::list_suppliers;

type R = Result<(), Error>;

fn count(s: &nb_store::Store, sql: &str) -> i64 {
    s.connection()
        .query_row(sql, [], |r| r.get(0))
        .unwrap_or(-1)
}

#[test]
fn customers_come_in_with_what_they_owe() -> R {
    let (mut s, setup) = store();
    let sheet = "الاسم;الهاتف;الولاية;البريد;الرصيد;النوع\n\
                 أحمد بن علي;0555 12 34 56;الوادي;;1 200,50;\n\
                 شركة النور;021 00 00 00;الجزائر;contact@nour.dz;-300;شركة\n\
                 كريم;0666;ورقلة;karim-at-mail;0;\n";
    let table = parse(sheet)?;
    let mapping = guess(Kind::Customers, &table.headers);
    let rs = rows(Kind::Customers, &table, &mapping)?;
    // The third line's e-mail is wrong: nothing is written, the line said.
    let r = import_rows(&mut s, None, Kind::Customers, &rs, setup.warehouse)?;
    assert_eq!(
        r.errors
            .iter()
            .map(|e| (e.line, e.field.as_deref(), e.code.as_str()))
            .collect::<Vec<_>>(),
        [(4, Some("email"), "invalid_email")]
    );
    let r = import_rows(
        &mut s,
        Some(meta(&setup, "import")),
        Kind::Customers,
        &rs,
        setup.warehouse,
    )?;
    assert_eq!(r.created, 0);
    assert_eq!(count(&s, "SELECT COUNT(*) FROM customers"), 0);

    // Fixed: checked, then written.
    let fixed = sheet.replace("karim-at-mail", "karim@mail.dz");
    let rs = rows(Kind::Customers, &parse(&fixed)?, &mapping)?;
    let check = import_rows(&mut s, None, Kind::Customers, &rs, setup.warehouse)?;
    assert_eq!((check.created, check.errors.len()), (3, 0));
    assert_eq!(
        count(&s, "SELECT COUNT(*) FROM customers"),
        0,
        "a check writes nothing"
    );
    let done = import_rows(
        &mut s,
        Some(meta(&setup, "import")),
        Kind::Customers,
        &rs,
        setup.warehouse,
    )?;
    assert_eq!(done.created, 3);
    let page = list_customers(
        &s,
        &CustomerQuery {
            limit: 10,
            ..Default::default()
        },
    )?;
    let by = |name: &str| page.rows.iter().find(|c| c.name == name).cloned();
    let ahmed = by("أحمد بن علي").unwrap_or_else(|| panic!("imported"));
    assert_eq!(
        (ahmed.balance_minor, ahmed.city.as_str()),
        (120_050, "الوادي")
    );
    let nour = by("شركة النور").unwrap_or_else(|| panic!("imported"));
    assert_eq!(
        (nour.kind.as_str(), nour.balance_minor),
        ("company", -30_000)
    );
    assert!(s.health()?.is_sound());
    Ok(())
}

#[test]
fn suppliers_and_products_come_in_too() -> R {
    let (mut s, setup) = store();
    let sheet = "Raison sociale,Téléphone,Solde,Délai\nCevital,021,15000,30\nGrossiste,,,\n";
    let t = parse(sheet)?;
    let rs = rows(Kind::Suppliers, &t, &guess(Kind::Suppliers, &t.headers))?;
    let done = import_rows(
        &mut s,
        Some(meta(&setup, "import")),
        Kind::Suppliers,
        &rs,
        setup.warehouse,
    )?;
    assert_eq!(done.created, 2);
    let list = list_suppliers(&s, "", false, 10, 0)?;
    let cevital = list
        .rows
        .iter()
        .find(|r| r.name == "Cevital")
        .cloned()
        .unwrap_or_else(|| panic!("in"));
    assert_eq!(
        (cevital.balance_minor, cevital.payment_days),
        (1_500_000, 30)
    );
    // A supplier never starts owing us.
    let t = parse("Nom,Solde\nX,-5\n")?;
    let rs = rows(Kind::Suppliers, &t, &guess(Kind::Suppliers, &t.headers))?;
    let r = import_rows(&mut s, None, Kind::Suppliers, &rs, setup.warehouse)?;
    assert_eq!(r.errors[0].code, "ledger.negative");

    // Products: the person maps "PV" to the price, the rest is guessed.
    let t = parse(
        "Désignation;PV;Coût;Stock;Famille\nLait;119;80;10;Laitiers\nHuile;357;250;;Épicerie\n",
    )?;
    let mut m = guess(Kind::Products, &t.headers);
    let price = Kind::Products
        .fields()
        .iter()
        .position(|f| f.key == "price")
        .unwrap_or(0);
    assert_eq!(m[price], None);
    m[price] = Some(1);
    let rs = rows(Kind::Products, &t, &m)?;
    let check = import_rows(&mut s, None, Kind::Products, &rs, setup.warehouse)?;
    assert_eq!((check.created, check.errors.len()), (2, 0));
    let before = count(&s, "SELECT COUNT(*) FROM products");
    let done = import_rows(
        &mut s,
        Some(meta(&setup, "import")),
        Kind::Products,
        &rs,
        setup.warehouse,
    )?;
    assert_eq!((done.created, done.categories_created), (2, 2));
    assert_eq!(count(&s, "SELECT COUNT(*) FROM products"), before + 2);
    assert_eq!(
        count(
            &s,
            "SELECT on_hand_milli FROM stock_balances b JOIN products p ON p.id = b.product_id WHERE p.name = 'Lait'"
        ),
        10_000
    );
    assert!(s.health()?.is_sound());
    Ok(())
}

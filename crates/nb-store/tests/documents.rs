//! Sales documents (M8.13): a quote is drafted, approved, becomes an
//! order, and the order is invoiced at the till; what has moved on never
//! changes.

mod common;

use common::*;
use nb_store::Error;
use nb_store::usecases::documents::*;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::sales::*;

type R = Result<(), Error>;

fn err<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
    }
}

fn line(product: &str, qty: &str) -> SaleLineInput {
    SaleLineInput {
        product_id: product.into(),
        qty: qty.into(),
        ..Default::default()
    }
}

#[test]
fn a_quote_becomes_an_order_then_an_invoice() -> R {
    let (mut s, setup) = store();
    let mut p = product("Lait", "", 1_190);
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "50".into(),
        unit_cost_minor: 800,
        tracked: Default::default(),
    });
    let milk = create_product(&mut s, meta(&setup, "product.create"), p)?.id;
    let wh = setup.warehouse.to_string();

    assert_eq!(
        err(save_document(
            &mut s,
            meta(&setup, "doc"),
            DocumentInput {
                kind: "quote".into(),
                warehouse_id: wh.clone(),
                ..Default::default()
            }
        )),
        ["sale.empty"]
    );
    // A quote for 10 milk and a typed item, 10 % off the whole.
    let mut input = DocumentInput {
        kind: "quote".into(),
        warehouse_id: wh.clone(),
        customer_name: "Boulangerie El Amel".into(),
        valid_until: Some("2026-12-31".into()),
        lines: vec![
            line(&milk, "10"),
            SaleLineInput {
                name: Some("Livraison".into()),
                unit_price_minor: Some(500),
                qty: "1".into(),
                ..Default::default()
            },
        ],
        discount_bps: Some(1_000),
        ..Default::default()
    };
    let q = save_document(&mut s, meta(&setup, "doc"), input.clone())?;
    assert!(q.row.number.starts_with("DV-"));
    assert_eq!(
        (q.row.status.as_str(), q.row.total_minor, q.lines.len()),
        ("draft", (11_900 + 500) * 9 / 10, 2)
    );
    // A draft changes freely.
    input.id = Some(q.row.id.clone());
    input.lines[0].qty = "20".into();
    let q = save_document(&mut s, meta(&setup, "doc"), input.clone())?;
    assert_eq!((q.version, q.row.total_minor), (2, (23_800 + 500) * 9 / 10));
    assert_eq!(
        err(convert_to_order(&mut s, meta(&setup, "doc"), &q.row.id)),
        ["doc.wrong_state"]
    );

    // Approved: its contents stay, even for the database.
    let q = approve_document(&mut s, meta(&setup, "doc"), &q.row.id)?;
    assert_eq!(q.row.status, "approved");
    assert_eq!(
        err(save_document(&mut s, meta(&setup, "doc"), input.clone())),
        ["doc.not_draft"]
    );
    assert!(
        s.connection()
            .execute(
                "UPDATE sales_document_lines SET qty_milli = 1 WHERE document_id = ?1",
                [&q.row.id],
            )
            .is_err()
    );

    // It becomes an order; the quote is final.
    let order = convert_to_order(&mut s, meta(&setup, "doc"), &q.row.id)?;
    assert!(order.row.number.starts_with("BC-"));
    assert_eq!(
        (
            order.row.status.as_str(),
            order.row.total_minor,
            order.source_number.as_deref()
        ),
        ("draft", q.row.total_minor, Some(q.row.number.as_str()))
    );
    let q = get_document(&s, &q.row.id)?;
    assert_eq!(
        (q.row.status.as_str(), q.row.next_number.as_deref()),
        ("converted", Some(order.row.number.as_str()))
    );
    assert_eq!(
        err(cancel_document(&mut s, meta(&setup, "doc"), &q.row.id, "x")),
        ["doc.wrong_state"]
    );
    assert!(
        s.connection()
            .execute(
                "UPDATE sales_documents SET note = 'x' WHERE id = ?1",
                [&q.row.id]
            )
            .is_err()
    );

    // Invoiced at the till: the sale remembers it, it is final.
    let reg = s
        .read(|r| {
            let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
            nb_store::usecases::org::registers(r.conn(), o.org_id()?)
        })?
        .remove(0)
        .id;
    open_shift(&mut s, meta(&setup, "shift"), &reg, 0)?;
    let sale_of = |doc: &Document| SaleInput {
        register_id: reg.clone(),
        warehouse_id: wh.clone(),
        lines: vec![line(&milk, "20")],
        payments: vec![PaymentInput {
            method: "cash".into(),
            amount_minor: 30_000,
        }],
        document_id: Some(doc.row.id.clone()),
        ..Default::default()
    };
    assert_eq!(
        err(complete_sale(&mut s, meta(&setup, "sale"), sale_of(&order))),
        ["doc.wrong_state"]
    );
    let order = approve_document(&mut s, meta(&setup, "doc"), &order.row.id)?;
    let sale = complete_sale(&mut s, meta(&setup, "sale"), sale_of(&order))?;
    let order = get_document(&s, &order.row.id)?;
    assert_eq!(
        (order.row.status.as_str(), order.row.next_number.clone()),
        ("invoiced", Some(sale.number))
    );
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM sales WHERE document_id IS NOT NULL"
        ),
        1
    );
    // Once only.
    assert_eq!(
        err(complete_sale(&mut s, meta(&setup, "sale"), sale_of(&order))),
        ["doc.wrong_state"]
    );

    // Lists, by kind and state.
    let all = list_documents(
        &s,
        &DocumentQuery {
            limit: 50,
            ..Default::default()
        },
    )?;
    assert_eq!(all.total, 2);
    let quotes = list_documents(
        &s,
        &DocumentQuery {
            kind: "quote".into(),
            search: "amel".into(),
            limit: 50,
            ..Default::default()
        },
    )?;
    assert_eq!(quotes.rows.len(), 1);
    assert_eq!(s.verify_audit_chain()?, None);
    Ok(())
}

#[test]
fn drafts_are_cancelled_with_a_reason() -> R {
    let (mut s, setup) = store();
    let milk = create_product(&mut s, meta(&setup, "p"), product("Lait", "", 1_190))?.id;
    let d = save_document(
        &mut s,
        meta(&setup, "doc"),
        DocumentInput {
            kind: "order".into(),
            warehouse_id: setup.warehouse.to_string(),
            lines: vec![line(&milk, "2")],
            valid_until: Some("31/12/2026".into()),
            ..Default::default()
        },
    );
    assert_eq!(err(d), ["invalid_date"]);
    let d = save_document(
        &mut s,
        meta(&setup, "doc"),
        DocumentInput {
            kind: "order".into(),
            warehouse_id: setup.warehouse.to_string(),
            lines: vec![line(&milk, "2")],
            ..Default::default()
        },
    )?;
    assert_eq!(
        err(cancel_document(&mut s, meta(&setup, "doc"), &d.row.id, " ")),
        ["required"]
    );
    let d = cancel_document(&mut s, meta(&setup, "doc"), &d.row.id, "Client parti")?;
    assert_eq!(d.row.status, "cancelled");
    assert_eq!(
        err(approve_document(&mut s, meta(&setup, "doc"), &d.row.id)),
        ["doc.wrong_state"]
    );
    Ok(())
}

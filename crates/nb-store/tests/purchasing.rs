//! Suppliers (M9.10) and buying (M10): an invoice brings goods in at their
//! landed cost and moves the average cost; payments settle the oldest
//! invoices; goods go back at their average cost; the books and the
//! supplier's balance agree.

mod common;

use common::*;
use nb_domain::{ProductId, WarehouseId};
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::inventory::available;
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::purchase_orders::*;
use nb_store::usecases::purchasing::*;
use nb_store::usecases::sales::{current_shift, open_shift};
use nb_store::{Error, OpMeta, Store};

type R = Result<(), Error>;

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

fn avg_cost(s: &Store, product: &str) -> i64 {
    s.connection()
        .query_row(
            "SELECT avg_cost_minor FROM stock_balances WHERE product_id = ?1",
            [product],
            |r| r.get(0),
        )
        .unwrap_or(-1)
}

fn line(product: &str, qty: &str, cost: i64) -> PurchaseLineInput {
    PurchaseLineInput {
        product_id: product.into(),
        qty: qty.into(),
        unit: String::new(),
        unit_cost_minor: cost,
        tax_bps: None,
        order_position: None,
    }
}

#[test]
fn an_invoice_brings_goods_in_at_their_landed_cost() -> R {
    let (mut s, setup) = store();
    assert_eq!(
        err(save_supplier(
            &mut s,
            meta(&setup, "s"),
            SupplierInput {
                kind: "company".into(),
                status: "active".into(),
                ..Default::default()
            }
        )),
        ["required"]
    );
    let sup = save_supplier(
        &mut s,
        meta(&setup, "s"),
        SupplierInput {
            kind: "company".into(),
            name: "Cevital".into(),
            payment_days: 30,
            status: "active".into(),
            ..Default::default()
        },
    )?;
    assert_eq!(sup.code, "F-00001");
    // 10 already in stock at 8.00.
    let mut p = product("Huile", "", 3_570);
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "10".into(),
        unit_cost_minor: 800,
        tracked: Default::default(),
    });
    let oil = create_product(&mut s, meta(&setup, "p"), p)?.id;
    let sugar = create_product(&mut s, meta(&setup, "p"), product("Sucre", "", 1_500))?.id;
    let wh = setup.warehouse.to_string();

    // 10 oil at 10.00 and 5 sugar at 20.00 before tax, 10.00 of transport
    // (+1.90 VAT) spread by value; 49.90 by bank, the rest owed.
    let input = PurchaseInput {
        supplier_id: sup.id.clone(),
        warehouse_id: wh.clone(),
        supplier_ref: "FC-2026-118".into(),
        lines: vec![line(&oil, "10", 1_000), line(&sugar, "5", 2_000)],
        charges_minor: 1_000,
        charges_vat_minor: 190,
        bank_minor: 4_990,
        ..Default::default()
    };
    let tap = meta(&setup, "purchase");
    let r = record_purchase(&mut s, tap, input.clone())?;
    assert_eq!(
        record_purchase(&mut s, tap, input)?,
        r,
        "a double tap buys once"
    );
    assert!(r.number.starts_with("FA-"));
    assert_eq!(
        (
            r.goods_minor,
            r.net_minor,
            r.vat_minor,
            r.total_minor,
            r.credit_minor
        ),
        (20_000, 21_000, 3_990, 24_990, 20_000)
    );
    // Landed: (100.00 + 5.00) / 10 and (100.00 + 5.00) / 5.
    assert_eq!(r.landed, [1_050, 2_100]);
    // The average cost moves: (10 × 8.00 + 10 × 10.50) / 20.
    assert_eq!(avg_cost(&s, &oil), 925);
    let (o, w): (ProductId, WarehouseId) = (
        oil.parse().unwrap_or_default(),
        wh.parse().unwrap_or_default(),
    );
    assert_eq!(available(&s, o, w)?.to_string(), "20");
    assert_eq!(
        (balance(&s, "4456"), balance(&s, "5121"), balance(&s, "401")),
        (3_990, -4_990, -20_000)
    );
    assert_eq!(get_supplier(&s, &sup.id)?.balance_minor, 20_000);

    // A second invoice, then 150.00 by bank: the oldest first.
    let second = record_purchase(
        &mut s,
        meta(&setup, "purchase"),
        PurchaseInput {
            supplier_id: sup.id.clone(),
            warehouse_id: wh.clone(),
            lines: vec![line(&sugar, "1", 2_000)],
            ..Default::default()
        },
    )?;
    let paid = pay_supplier(
        &mut s,
        meta(&setup, "pay"),
        SupplierPaymentInput {
            supplier_id: sup.id.clone(),
            amount_minor: 15_000,
            method: "bank".into(),
            ..Default::default()
        },
    )?;
    assert_eq!(paid.settled, [(r.number.clone(), 15_000)]);
    let open = open_purchases(&s, &sup.id)?;
    assert_eq!(
        open.iter().map(|p| p.open_minor).collect::<Vec<_>>(),
        [5_000, second.total_minor]
    );

    // Two oil back, off what is owed: 23.80; they leave at 9.25 each, the
    // 1.50 above that lowers the cost of sales.
    assert_eq!(
        err(return_purchase(
            &mut s,
            meta(&setup, "ret"),
            PurchaseReturnInput {
                purchase_id: r.id.clone(),
                lines: vec![(0, "11".into())],
                refund: "credit".into(),
                reason: "Abîmé".into(),
                ..Default::default()
            }
        )),
        ["return.too_much"]
    );
    let back = return_purchase(
        &mut s,
        meta(&setup, "ret"),
        PurchaseReturnInput {
            purchase_id: r.id.clone(),
            lines: vec![(0, "2".into())],
            refund: "credit".into(),
            reason: "Bidons abîmés".into(),
            ..Default::default()
        },
    )?;
    assert!(back.number.starts_with("RF-"));
    assert_eq!((back.total_minor, back.stock_value_minor), (2_380, 1_850));
    assert_eq!(available(&s, o, w)?.to_string(), "18");
    assert_eq!(balance(&s, "600"), -150);
    assert_eq!(
        get_supplier(&s, &sup.id)?.balance_minor,
        20_000 + second.total_minor - 15_000 - 2_380
    );
    assert_eq!(purchase_lines(&s, &r.id)?[0].returned_milli, 2_000);
    let lines = supplier_statement(&s, &sup.id)?;
    assert_eq!(
        lines.last().map(|l| l.balance_minor),
        Some(get_supplier(&s, &sup.id)?.balance_minor)
    );
    assert!(s.health()?.is_sound());
    Ok(())
}

#[test]
fn cash_goes_out_of_the_open_till() -> R {
    let (mut s, setup) = store();
    let reg = s
        .read(|r| {
            let o = nb_store::repo::org::the_org(r.conn())?.ok_or(Error::NotFound("org"))?;
            nb_store::usecases::org::registers(r.conn(), o.org_id()?)
        })?
        .remove(0)
        .id;
    let sup = save_supplier(
        &mut s,
        meta(&setup, "s"),
        SupplierInput {
            kind: "person".into(),
            name: "Grossiste".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    let oil = create_product(&mut s, meta(&setup, "p"), product("Huile", "", 3_570))?.id;
    let buy = |cash: i64| PurchaseInput {
        supplier_id: sup.id.clone(),
        warehouse_id: setup.warehouse.to_string(),
        lines: vec![line(&oil, "5", 1_000)],
        cash_minor: cash,
        register_id: Some(reg.clone()),
        ..Default::default()
    };
    assert_eq!(
        err(record_purchase(&mut s, meta(&setup, "p"), buy(5_950))),
        ["shift.none_open"]
    );
    open_shift(&mut s, meta(&setup, "shift"), &reg, 2_000)?;
    assert_eq!(
        err(record_purchase(&mut s, meta(&setup, "p"), buy(5_950))),
        ["shift.not_enough_cash"]
    );
    assert_eq!(
        err(record_purchase(&mut s, meta(&setup, "p"), buy(9_000))),
        ["sale.overpaid"]
    );
    // 50.00 + VAT: 15.00 in cash now, 44.50 owed.
    let r = record_purchase(&mut s, meta(&setup, "p"), buy(1_500))?;
    assert_eq!(r.credit_minor, 4_450);
    // What is left in the drawer pays part of the rest.
    assert_eq!(
        err(pay_supplier(
            &mut s,
            meta(&setup, "pay"),
            SupplierPaymentInput {
                supplier_id: sup.id.clone(),
                amount_minor: 600,
                method: "cash".into(),
                register_id: Some(reg.clone()),
                ..Default::default()
            }
        )),
        ["shift.not_enough_cash"]
    );
    pay_supplier(
        &mut s,
        meta(&setup, "pay"),
        SupplierPaymentInput {
            supplier_id: sup.id.clone(),
            amount_minor: 500,
            method: "cash".into(),
            register_id: Some(reg.clone()),
            ..Default::default()
        },
    )?;
    let shift = current_shift(&s, &reg)?.unwrap_or_else(|| panic!("open"));
    assert_eq!((shift.suppliers_minor, shift.expected_minor), (2_000, 0));
    assert_eq!(get_supplier(&s, &sup.id)?.balance_minor, 3_950);
    assert!(s.health()?.is_sound());
    Ok(())
}

#[test]
fn an_order_is_approved_then_received_against_line_by_line() -> R {
    let (mut s, setup) = store();
    let sup = |s: &mut Store, name: &str| {
        save_supplier(
            s,
            meta(&setup, "s"),
            SupplierInput {
                kind: "company".into(),
                name: name.into(),
                status: "active".into(),
                ..Default::default()
            },
        )
    };
    let cevital = sup(&mut s, "Cevital")?;
    let other = sup(&mut s, "Autre")?;
    let oil = create_product(&mut s, meta(&setup, "p"), product("Huile", "", 3_570))?.id;
    let sugar = create_product(&mut s, meta(&setup, "p"), product("Sucre", "", 1_500))?.id;
    let wh = setup.warehouse.to_string();
    let mut input = OrderInput {
        supplier_id: cevital.id.clone(),
        warehouse_id: wh.clone(),
        expected_date: Some("2026-12-01".into()),
        lines: vec![line(&oil, "10", 1_000), line(&sugar, "5", 2_000)],
        ..Default::default()
    };
    let draft = save_order(&mut s, meta(&setup, "o"), input.clone())?;
    assert!(draft.row.number.starts_with("BCF-"));
    assert_eq!(
        (
            draft.row.status.as_str(),
            draft.net_minor,
            draft.row.total_minor
        ),
        ("draft", 20_000, 23_800)
    );
    // A draft changes; nothing is received against it.
    input.id = Some(draft.row.id.clone());
    input.lines[0].qty = "12".into();
    let draft = save_order(&mut s, meta(&setup, "o"), input.clone())?;
    assert_eq!((draft.net_minor, draft.version), (22_000, 2));
    let buy = |lines: Vec<PurchaseLineInput>, supplier: &str| PurchaseInput {
        supplier_id: supplier.into(),
        warehouse_id: wh.clone(),
        lines,
        order_id: Some(draft.row.id.clone()),
        ..Default::default()
    };
    let at = |product: &str, qty: &str, cost: i64, pos: u32| PurchaseLineInput {
        order_position: Some(pos),
        ..line(product, qty, cost)
    };
    assert_eq!(
        err(record_purchase(
            &mut s,
            meta(&setup, "b"),
            buy(vec![at(&oil, "1", 1_000, 0)], &cevital.id)
        )),
        ["doc.wrong_state"]
    );

    // Approving takes a right a buyer only asks for.
    let buyer = create_user(
        &mut s,
        meta(&setup, "user"),
        UserInput {
            name: "Achats".into(),
            phone: None,
            login: None,
            password: None,
            pin: None,
            grants: vec![GrantRow::org("purchasing")],
        },
    )?;
    let as_buyer = |kind: &'static str| {
        OpMeta::new(
            kind,
            Some(setup.org),
            Some(buyer.id.parse().unwrap_or_default()),
        )
    };
    assert_eq!(
        err(approve_order(&mut s, as_buyer("a"), &draft.row.id, None)),
        ["access.needs_approval"]
    );
    let approved = approve_order(&mut s, meta(&setup, "a"), &draft.row.id, None)?;
    assert_eq!(approved.row.status, "approved");
    assert_eq!(
        err(save_order(&mut s, meta(&setup, "o"), input)),
        ["doc.not_draft"]
    );
    assert!(
        s.connection()
            .execute(
                "UPDATE purchase_order_lines SET qty_milli = 1000 WHERE order_id = ?1",
                [&draft.row.id]
            )
            .is_err(),
        "an approved order keeps its contents"
    );

    // Matching: the right supplier, ordered lines, never more than left.
    for (lines, supplier, code) in [
        (
            vec![at(&oil, "4", 1_000, 0)],
            &other.id,
            "purchase.order_mismatch",
        ),
        (
            vec![line(&oil, "4", 1_000)],
            &cevital.id,
            "purchase.not_ordered",
        ),
        (
            vec![at(&sugar, "4", 1_000, 0)],
            &cevital.id,
            "purchase.not_ordered",
        ),
        (
            vec![at(&oil, "13", 1_000, 0)],
            &cevital.id,
            "purchase.over_ordered",
        ),
        (
            vec![at(&oil, "7", 1_000, 0), at(&oil, "6", 1_000, 0)],
            &cevital.id,
            "purchase.over_ordered",
        ),
    ] {
        assert_eq!(
            err(record_purchase(
                &mut s,
                meta(&setup, "b"),
                buy(lines, supplier)
            )),
            [code]
        );
    }
    // A cost above the order needs the right to approve purchases.
    assert_eq!(
        err(record_purchase(
            &mut s,
            as_buyer("b"),
            buy(vec![at(&oil, "4", 1_100, 0)], &cevital.id)
        )),
        ["access.needs_approval"]
    );
    let first = record_purchase(
        &mut s,
        as_buyer("b"),
        buy(
            vec![at(&oil, "4", 1_000, 0), at(&sugar, "5", 1_900, 1)],
            &cevital.id,
        ),
    )?;
    let o = get_order(&s, &draft.row.id)?;
    assert_eq!(o.row.status, "partial");
    assert_eq!(
        o.lines
            .iter()
            .map(|l| l.remaining_milli())
            .collect::<Vec<_>>(),
        [8_000, 0]
    );
    assert_eq!(o.receipts, std::slice::from_ref(&first.number));
    assert_eq!(
        err(cancel_order(&mut s, meta(&setup, "c"), &draft.row.id, "x")),
        ["doc.wrong_state"]
    );
    // The rest: received in full.
    record_purchase(
        &mut s,
        meta(&setup, "b"),
        buy(vec![at(&oil, "8", 1_050, 0)], &cevital.id),
    )?;
    let o = get_order(&s, &draft.row.id)?;
    assert_eq!((o.row.status.as_str(), o.receipts.len()), ("received", 2));
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM purchase_lines WHERE ordered_cost_minor = 1000 AND unit_cost_minor = 1050"
        ),
        1
    );
    assert_eq!(
        list_orders(
            &s,
            &OrderQuery {
                status: "open".into(),
                limit: 10,
                ..Default::default()
            }
        )?
        .total,
        0
    );

    // Another order, partly received then closed; a third cancelled.
    let mut second = OrderInput {
        supplier_id: cevital.id.clone(),
        warehouse_id: wh.clone(),
        lines: vec![line(&oil, "3", 1_000)],
        ..Default::default()
    };
    let o2 = save_order(&mut s, meta(&setup, "o"), second.clone())?;
    assert_eq!(
        err(close_order(
            &mut s,
            meta(&setup, "c"),
            &o2.row.id,
            "Rupture"
        )),
        ["doc.wrong_state"]
    );
    approve_order(&mut s, meta(&setup, "a"), &o2.row.id, None)?;
    record_purchase(
        &mut s,
        meta(&setup, "b"),
        PurchaseInput {
            supplier_id: cevital.id.clone(),
            warehouse_id: wh.clone(),
            lines: vec![at(&oil, "1", 1_000, 0)],
            order_id: Some(o2.row.id.clone()),
            ..Default::default()
        },
    )?;
    assert_eq!(
        err(close_order(&mut s, meta(&setup, "c"), &o2.row.id, " ")),
        ["required"]
    );
    let closed = close_order(
        &mut s,
        meta(&setup, "c"),
        &o2.row.id,
        "Rupture chez le fournisseur",
    )?;
    assert_eq!(
        (closed.row.status.as_str(), closed.reason.as_str()),
        ("closed", "Rupture chez le fournisseur")
    );
    second.lines = vec![line(&sugar, "1", 2_000)];
    let o3 = save_order(&mut s, meta(&setup, "o"), second)?;
    let cancelled = cancel_order(&mut s, meta(&setup, "c"), &o3.row.id, "Doublon")?;
    assert_eq!(cancelled.row.status, "cancelled");
    assert!(
        s.connection()
            .execute(
                "UPDATE purchase_orders SET status = 'draft' WHERE id = ?1",
                [&o3.row.id]
            )
            .is_err()
    );
    assert!(s.health()?.is_sound());
    Ok(())
}

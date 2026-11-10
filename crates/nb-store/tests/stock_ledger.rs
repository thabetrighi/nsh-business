//! Stock adjustments with their accounting, the balance projection, and
//! the journal's guarantees (M6, M7).

mod common;

use common::*;
use nb_store::Error;
use nb_store::repo::{ledger, stock};
use nb_store::usecases::ledger::{EntryLineInput, post_entry, reverse_entry};
use nb_store::usecases::products::{Opening, create_product};
use nb_store::usecases::stock::{AdjustInput, adjust_stock, rebuild_balances};

type R = Result<(), Error>;

fn codes(e: &Error) -> Vec<&'static str> {
    e.rule_codes().into_iter().map(|(_, c)| c).collect()
}

fn setup_item() -> Result<
    (
        nb_store::Store,
        nb_store::usecases::setup::SetupResult,
        String,
    ),
    Error,
> {
    let (mut s, setup) = store();
    let mut p = product("Huile Elio 5L", "", 95_000);
    p.extra_units = vec![("ctn".into(), "4".into())];
    p.opening = Some(Opening {
        warehouse_id: setup.warehouse.to_string(),
        qty: "8".into(),
        unit_cost_minor: 80_000,
    });
    let row = create_product(&mut s, meta(&setup, "product.create"), p)?;
    Ok((s, setup, row.id))
}

fn adjust(product: &str, wh: &str, kind: &str, qty: &str, unit: &str) -> AdjustInput {
    AdjustInput {
        product_id: product.into(),
        warehouse_id: wh.into(),
        kind: kind.into(),
        qty: qty.into(),
        unit: unit.into(),
        unit_cost_minor: None,
        reason: "casse".into(),
        may_go_negative: false,
    }
}

#[test]
fn waste_moves_stock_and_posts_its_entry() -> R {
    let (mut s, setup, id) = setup_item()?;
    let wh = setup.warehouse.to_string();
    let r = adjust_stock(
        &mut s,
        meta(&setup, "stock.adjust"),
        adjust(&id, &wh, "waste", "1", "ctn"),
    )?;
    assert_eq!(r.on_hand_milli, 4_000, "1 carton = 4 pieces");
    assert_eq!(r.value_minor, 320_000);
    assert_eq!(r.number, "AJ-01-26-A00002");
    let tb = s.read(|rd| ledger::trial_balance(rd.conn(), setup.org))?;
    let get = |code: &str| {
        tb.iter()
            .find(|t| t.code == code)
            .map(|t| (t.debit_minor, t.credit_minor))
    };
    assert_eq!(get("658"), Some((320_000, 0)));
    // Opening 8 × 800.00 entered stock (30) against equity (101); waste took 320.00 out.
    assert_eq!(get("30"), Some((640_000, 320_000)));
    assert_eq!(get("101"), Some((0, 640_000)));
    let (d, c): (i64, i64) = tb
        .iter()
        .fold((0, 0), |a, t| (a.0 + t.debit_minor, a.1 + t.credit_minor));
    assert_eq!(d, c, "the trial balance balances");
    assert!(
        s.read(|rd| ledger::unbalanced_entries(rd.conn()))?
            .is_empty()
    );
    Ok(())
}

#[test]
fn negative_stock_needs_permission() -> R {
    let (mut s, setup, id) = setup_item()?;
    let wh = setup.warehouse.to_string();
    let e = adjust_stock(
        &mut s,
        meta(&setup, "stock.adjust"),
        adjust(&id, &wh, "adjust_out", "9", "pc"),
    )
    .err()
    .ok_or(Error::NotFound("error"))?;
    assert_eq!(codes(&e), vec!["stock.insufficient"]);
    let mut allowed = adjust(&id, &wh, "adjust_out", "9", "pc");
    allowed.may_go_negative = true;
    assert_eq!(
        adjust_stock(&mut s, meta(&setup, "stock.adjust"), allowed)?.on_hand_milli,
        -1_000
    );
    Ok(())
}

#[test]
fn adjustments_are_validated() -> R {
    let (mut s, setup, id) = setup_item()?;
    let wh = setup.warehouse.to_string();
    let mut no_reason = adjust(&id, &wh, "waste", "1", "pc");
    no_reason.reason = " ".into();
    let cases = [
        (adjust(&id, &wh, "sale", "1", "pc"), "invalid_value"),
        (no_reason, "required"),
        (adjust(&id, &wh, "waste", "1.5", "pc"), "units.too_precise"),
        (adjust(&id, &wh, "waste", "1", "kg"), "units.unknown"),
        (
            adjust(&id, &wh, "waste", "0", "pc"),
            "stock.quantity_positive",
        ),
    ];
    for (input, code) in cases {
        let e = adjust_stock(&mut s, meta(&setup, "stock.adjust"), input)
            .err()
            .ok_or(Error::NotFound("error"))?;
        assert_eq!(codes(&e), vec![code]);
    }
    Ok(())
}

#[test]
fn movements_are_append_only_and_balances_rebuild() -> R {
    let (mut s, setup, id) = setup_item()?;
    let wh = setup.warehouse.to_string();
    adjust_stock(
        &mut s,
        meta(&setup, "stock.adjust"),
        adjust(&id, &wh, "count_gain", "2", "pc"),
    )?;
    assert!(
        s.connection()
            .execute("UPDATE stock_movements SET qty_milli = 1", [])
            .is_err()
    );
    assert!(
        s.connection()
            .execute("DELETE FROM stock_movements", [])
            .is_err()
    );
    assert!(
        rebuild_balances(&s)?.is_empty(),
        "the projection matches the movements"
    );
    s.connection()
        .execute("UPDATE stock_balances SET on_hand_milli = 999", [])?;
    assert_eq!(
        rebuild_balances(&s)?.len(),
        1,
        "a drifted balance is found and fixed"
    );
    let b = s.read(|rd| {
        stock::balance(
            rd.conn(),
            id.parse().map_err(|_| Error::NotFound("id"))?,
            setup.warehouse,
            nb_core::Currency::DZD,
        )
    })?;
    assert_eq!(b.on_hand, nb_core::Quantity::whole(10));
    Ok(())
}

#[test]
fn journal_entries_are_balanced_immutable_and_reversible() -> R {
    let (mut s, setup) = store();
    let line = |a: &str, d: i64, c: i64| EntryLineInput {
        account: a.into(),
        debit_minor: d,
        credit_minor: c,
    };
    let e = post_entry(
        &mut s,
        meta(&setup, "journal.post"),
        "Apport",
        vec![line("53", 1_000, 0), line("101", 0, 900)],
    )
    .err()
    .ok_or(Error::NotFound("error"))?;
    assert_eq!(codes(&e), vec!["ledger.unbalanced"]);
    let e = post_entry(
        &mut s,
        meta(&setup, "journal.post"),
        "Apport",
        vec![line("53", 1_000, 0), line("999", 0, 1_000)],
    )
    .err()
    .ok_or(Error::NotFound("error"))?;
    assert_eq!(codes(&e), vec!["ledger.unknown_account"]);
    let posted = post_entry(
        &mut s,
        meta(&setup, "journal.post"),
        "Apport",
        vec![line("53", 1_000, 0), line("101", 0, 1_000)],
    )?;
    assert_eq!(posted.number, "OD-01-26-A00001");
    assert!(
        s.connection()
            .execute("UPDATE journal_lines SET debit_minor = 5", [])
            .is_err()
    );
    assert!(
        s.connection()
            .execute("DELETE FROM journal_entries", [])
            .is_err()
    );
    let rev = reverse_entry(
        &mut s,
        meta(&setup, "journal.reverse"),
        &posted.id,
        "erreur de saisie",
    )?;
    assert_eq!(rev.number, "OD-01-26-A00002");
    let e = reverse_entry(
        &mut s,
        meta(&setup, "journal.reverse"),
        &posted.id,
        "encore",
    )
    .err()
    .ok_or(Error::NotFound("error"))?;
    assert_eq!(codes(&e), vec!["ledger.already_reversed"]);
    let tb = s.read(|rd| ledger::trial_balance(rd.conn(), setup.org))?;
    let cash = tb
        .iter()
        .find(|t| t.code == "53")
        .map(|t| t.debit_minor - t.credit_minor);
    assert_eq!(cash, Some(0), "the reversal cancels the entry");
    Ok(())
}

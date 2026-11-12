//! The import wizard's importers (M14.3): rows already mapped to fields
//! (`nb_domain::import`) become products, customers or suppliers, with
//! their opening stock or balances. Every line is checked first; when one
//! is wrong nothing is written and each error names its line. A good file
//! is written in one transaction: all of it or none.

use nb_core::Currency;
use nb_domain::RuleError;
use nb_domain::access::{Permission, Target};
use nb_domain::import::{Kind, Row, kind_of, to_csv};

use crate::uow::OpMeta;
use crate::usecases::catalog::{
    ImportError, ImportReport, check_products_import, import_products, money_text,
};
use crate::usecases::customers::{CustomerInput, check, save_customer_in};
use crate::usecases::debts::{AdjustInput, adjust_in};
use crate::usecases::purchasing::{
    SupplierInput, check_supplier, save_supplier_in, supplier_opening_in,
};
use crate::usecases::sales::the_org;
use crate::{Result, Store};

/// What the opening balances say they come from.
const REASON: &str = "Import";

/// A person or a company read from a row: its fields, its opening balance.
struct Party {
    customer: CustomerInput,
    balance: i64,
}

fn read_party(
    kind: Kind,
    row: &Row,
    currency: Currency,
    errors: &mut Vec<ImportError>,
) -> Option<Party> {
    let mut err = |field: &str, code: &str| {
        errors.push(ImportError {
            line: row.line,
            field: Some(field.into()),
            code: code.into(),
        });
    };
    let Some(person_or_company) = kind_of(row.get("kind")) else {
        err("kind", "invalid_value");
        return None;
    };
    let money = |field: &str| -> std::result::Result<i64, ()> {
        match row.get(field) {
            "" => Ok(0),
            v => money_text(v, currency).map_err(|_| ()),
        }
    };
    let Ok(balance) = money("balance") else {
        err("balance", "price.invalid");
        return None;
    };
    let Ok(credit_limit) = money("credit_limit") else {
        err("credit_limit", "price.invalid");
        return None;
    };
    let days = match row.get("payment_days") {
        "" => 0,
        d => match d.parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                err("payment_days", "invalid_value");
                return None;
            }
        },
    };
    if kind == Kind::Suppliers && balance < 0 {
        err("balance", "ledger.negative");
        return None;
    }
    let s = |k: &str| row.get(k).to_owned();
    let customer = CustomerInput {
        id: None,
        kind: person_or_company.into(),
        name: s("name"),
        group_id: None,
        phone: s("phone"),
        email: s("email"),
        address: s("address"),
        city: s("city"),
        nif: s("nif"),
        nis: s("nis"),
        rc: s("rc"),
        ai: s("ai"),
        credit_limit_minor: credit_limit,
        payment_days: days,
        status: "active".into(),
        note: String::new(),
    };
    let checked = match kind {
        Kind::Suppliers => check_supplier(&supplier_of(&customer)),
        _ => check(&customer),
    };
    if let Err(e) = checked {
        for (f, c) in e.rule_codes() {
            errors.push(ImportError {
                line: row.line,
                field: f.map(Into::into),
                code: c.into(),
            });
        }
        return None;
    }
    Some(Party { customer, balance })
}

fn supplier_of(c: &CustomerInput) -> SupplierInput {
    SupplierInput {
        id: None,
        kind: c.kind.clone(),
        name: c.name.clone(),
        phone: c.phone.clone(),
        email: c.email.clone(),
        address: c.address.clone(),
        city: c.city.clone(),
        nif: c.nif.clone(),
        nis: c.nis.clone(),
        rc: c.rc.clone(),
        ai: c.ai.clone(),
        payment_days: c.payment_days,
        status: c.status.clone(),
        note: c.note.clone(),
    }
}

/// Imports `rows` of `kind` (`meta` None: only checks them). Products'
/// opening stock goes to `warehouse`.
pub fn import_rows(
    store: &mut Store,
    meta: Option<OpMeta>,
    kind: Kind,
    rows: &[Row],
    warehouse: nb_domain::WarehouseId,
) -> Result<ImportReport> {
    if rows.is_empty() {
        return Err(RuleError::on("file", "import.no_rows").into());
    }
    if kind == Kind::Products {
        let text = to_csv(kind, rows);
        return match meta {
            Some(m) => import_products(store, m, &text, warehouse),
            None => check_products_import(store, &text, warehouse),
        };
    }
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.utc_offset_min;
    let mut report = ImportReport::default();
    let parties: Vec<Party> = rows
        .iter()
        .filter_map(|r| read_party(kind, r, currency, &mut report.errors))
        .collect();
    if !report.errors.is_empty() {
        return Ok(report);
    }
    let Some(meta) = meta else {
        report.created = u32::try_from(parties.len()).unwrap_or(u32::MAX);
        return Ok(report);
    };
    store.write(meta, |w| {
        if parties.iter().any(|p| p.balance != 0) {
            w.require(Permission::DebtsAdjust, &Target::org())?;
        }
        for p in &parties {
            match kind {
                Kind::Suppliers => {
                    let id = save_supplier_in(w, &supplier_of(&p.customer))?;
                    if p.balance > 0 {
                        supplier_opening_in(w, &id, p.balance, REASON, currency, offset)?;
                    }
                }
                _ => {
                    let id = save_customer_in(w, &p.customer)?;
                    if p.balance != 0 {
                        adjust_in(
                            w,
                            &AdjustInput {
                                customer_id: id,
                                kind: "opening".into(),
                                amount_minor: p.balance,
                                reason: REASON.into(),
                                approval: None,
                            },
                            currency,
                            offset,
                        )?;
                    }
                }
            }
        }
        w.event(
            "import.done",
            "organization",
            &w.org()?.to_string(),
            &serde_json::json!({ "kind": kind.code(), "rows": parties.len() }),
        )?;
        Ok(())
    })?;
    report.created = u32::try_from(parties.len()).unwrap_or(u32::MAX);
    Ok(report)
}

//! What printed documents show (M12): a sale as recorded — its lines,
//! VAT by rate, payments, who sold it at which till, for which customer —
//! and the shop's identity. Read only; the layout is `nb-docs`'.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::usecases::org::{BusinessProfile, business_profile, settings};
use crate::usecases::sales::the_org;
use crate::{Error, Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintLine {
    pub name: String,
    pub unit: String,
    pub qty_milli: i64,
    pub unit_price_minor: i64,
    pub discount_minor: i64,
    /// What the line came to, as charged.
    pub amount_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintSale {
    pub number: String,
    pub created_at: i64,
    pub cashier: String,
    pub register: String,
    /// The customer's name and tax numbers (NIF, RC), if any.
    pub customer: Option<(String, String)>,
    pub lines: Vec<PrintLine>,
    pub discount_minor: i64,
    pub net_minor: i64,
    /// (rate in basis points, base, tax), by rate.
    pub taxes: Vec<(i64, i64, i64)>,
    pub stamp_minor: i64,
    pub total_minor: i64,
    /// (method, amount): cash as handed over (with the change).
    pub payments: Vec<(String, i64)>,
    pub change_minor: i64,
    /// Left on the customer's account.
    pub owed_minor: i64,
}

/// The shop's name and legal identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintShop {
    pub name: String,
    pub profile: BusinessProfile,
    /// The language documents are printed in (`ar`, `fr`, `en`).
    pub language: String,
}

pub fn print_shop(store: &Store) -> Result<PrintShop> {
    let profile = business_profile(store)?;
    let org = settings(store)?;
    let name = if profile.legal_name.trim().is_empty() {
        org.name
    } else {
        profile.legal_name.clone()
    };
    Ok(PrintShop {
        name,
        profile,
        language: org.language,
    })
}

/// A sale by its number or id, as its documents show it.
pub fn print_sale(store: &Store, number: &str) -> Result<PrintSale> {
    store.read(|r| {
        let c = r.conn();
        let (org, _) = the_org(c)?;
        type Head = (
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        type Money6 = (i64, i64, i64, i64, i64, i64);
        let (id, num, created_at, register, cashier, customer): Head = c
            .query_row(
                "SELECT s.id, s.number, s.created_at, cr.name, u.name, s.customer_id
                 FROM sales s
                 LEFT JOIN cash_registers cr ON cr.id = s.register_id
                 LEFT JOIN users u ON u.id = s.created_by
                 WHERE s.org_id = ?1 AND (s.number = ?2 OR s.id = ?2)",
                params![org.to_string(), number.trim()],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::NotFound("sale"))?;
        let (discount, net, stamp, total, change, owed): Money6 = c.query_row(
            "SELECT discount_minor, net_minor, stamp_minor, total_minor, change_minor, credit_minor
             FROM sales WHERE id = ?1",
            [&id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?;
        let mut s = c.prepare(
            "SELECT name, unit, qty_milli, unit_price_minor, line_discount_minor, gross_minor
             FROM sale_lines WHERE sale_id = ?1 ORDER BY position",
        )?;
        let lines = s
            .query_map([&id], |r| {
                Ok(PrintLine {
                    name: r.get(0)?,
                    unit: r.get(1)?,
                    qty_milli: r.get(2)?,
                    unit_price_minor: r.get(3)?,
                    discount_minor: r.get(4)?,
                    amount_minor: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        let mut s = c.prepare(
            "SELECT tax_bps, SUM(net_minor), SUM(tax_minor) FROM sale_lines
             WHERE sale_id = ?1 GROUP BY tax_bps ORDER BY tax_bps DESC",
        )?;
        let taxes = s
            .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut s = c.prepare(
            "SELECT method, SUM(amount_minor) FROM payments
             WHERE sale_id = ?1 AND method <> 'credit'
             GROUP BY method ORDER BY MIN(rowid)",
        )?;
        let mut payments: Vec<(String, i64)> = s
            .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        // Cash is recorded as kept; the receipt shows what was handed over.
        if change > 0
            && let Some(cash) = payments.iter_mut().find(|p| p.0 == "cash")
        {
            cash.1 += change;
        }
        let customer = match customer {
            Some(cid) => c
                .query_row(
                    "SELECT name, nif, rc FROM customers WHERE id = ?1",
                    [&cid],
                    |r| {
                        let (name, nif, rc): (String, String, String) =
                            (r.get(0)?, r.get(1)?, r.get(2)?);
                        let ids = [("NIF", nif), ("RC", rc)]
                            .into_iter()
                            .filter(|(_, v)| !v.is_empty())
                            .map(|(k, v)| format!("{k}: {v}"))
                            .collect::<Vec<_>>()
                            .join(" · ");
                        Ok((name, ids))
                    },
                )
                .optional()?,
            None => None,
        };
        Ok(PrintSale {
            number: num,
            created_at,
            cashier: cashier.unwrap_or_default(),
            register: register.unwrap_or_default(),
            customer,
            lines,
            discount_minor: discount,
            net_minor: net,
            taxes,
            stamp_minor: stamp,
            total_minor: total,
            payments,
            change_minor: change,
            owed_minor: owed,
        })
    })
}

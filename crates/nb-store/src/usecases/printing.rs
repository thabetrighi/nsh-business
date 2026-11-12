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
    /// The customer's balance before and after the sale.
    pub balance: Option<(i64, i64)>,
    /// Paid on older debt with the sale (counted in `payments`).
    pub settled_minor: i64,
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
        // Older debt paid with it was handed over too.
        let balance: Option<(i64, i64, i64, Option<String>)> = c
            .query_row(
                "SELECT b.before_minor, b.after_minor, b.settled_minor, r.method
                 FROM sale_balances b LEFT JOIN receipts r ON r.id = b.receipt_id
                 WHERE b.sale_id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let settled = balance.as_ref().map_or(0, |b| b.2);
        if let Some((_, _, settled, Some(method))) = &balance
            && *settled > 0
        {
            match payments.iter_mut().find(|p| &p.0 == method) {
                Some(p) => p.1 += settled,
                None => payments.push((method.clone(), *settled)),
            }
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
            balance: balance.map(|b| (b.0, b.1)),
            settled_minor: settled,
        })
    })
}

/// What the shop's documents show (settings › printing), and its logo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocSettings {
    pub tagline: String,
    /// The line at the bottom of receipts (empty: a thank-you).
    pub footer: String,
    /// A notice under it (returns policy…).
    pub notice: String,
    pub legal_ids: bool,
    pub vat: bool,
    pub cashier: bool,
    pub table: bool,
    pub count: bool,
    pub balance: bool,
    pub barcode: bool,
    /// A grey PNG, at most 576 dots wide (made by the app from the image
    /// chosen).
    #[serde(skip)]
    pub logo: Option<Vec<u8>>,
}

impl Default for DocSettings {
    fn default() -> Self {
        Self {
            tagline: String::new(),
            footer: String::new(),
            notice: String::new(),
            legal_ids: true,
            vat: true,
            cashier: true,
            table: true,
            count: true,
            balance: true,
            barcode: true,
            logo: None,
        }
    }
}

/// The largest logo kept.
pub const LOGO_MAX_BYTES: usize = 512 * 1024;

pub fn document_settings(store: &Store) -> Result<DocSettings> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        Ok(r.conn()
            .query_row(
                "SELECT tagline, footer, notice, legal_ids, vat, cashier, table_lines, item_count, balance, barcode, logo
                 FROM document_settings WHERE org_id = ?1",
                [org.to_string()],
                |r| {
                    Ok(DocSettings {
                        tagline: r.get(0)?,
                        footer: r.get(1)?,
                        notice: r.get(2)?,
                        legal_ids: r.get(3)?,
                        vat: r.get(4)?,
                        cashier: r.get(5)?,
                        table: r.get(6)?,
                        count: r.get(7)?,
                        balance: r.get(8)?,
                        barcode: r.get(9)?,
                        logo: r.get(10)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    })
}

/// Saves what documents show; `logo` replaces the logo (`Some(None)`
/// removes it, `None` keeps it).
pub fn save_document_settings(
    store: &mut Store,
    meta: crate::OpMeta,
    s: DocSettings,
    logo: Option<Option<Vec<u8>>>,
) -> Result<DocSettings> {
    let mut errors = Vec::new();
    for (field, text, max) in [
        ("tagline", &s.tagline, 120),
        ("footer", &s.footer, 200),
        ("notice", &s.notice, 300),
    ] {
        if text.chars().count() > max {
            errors.push(nb_domain::RuleError::on(field, "documents.text_long"));
        }
    }
    if let Some(Some(bytes)) = &logo
        && (bytes.len() > LOGO_MAX_BYTES || !bytes.starts_with(&[0x89, b'P', b'N', b'G']))
    {
        errors.push(nb_domain::RuleError::on("logo", "documents.logo"));
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }
    store.write(meta, |w| {
        let id = w.org()?;
        w.require(
            nb_domain::access::Permission::SettingsEdit,
            &nb_domain::access::Target::org(),
        )?;
        let trimmed = DocSettings {
            tagline: s.tagline.trim().into(),
            footer: s.footer.trim().into(),
            notice: s.notice.trim().into(),
            ..s.clone()
        };
        w.tx().execute(
            "INSERT INTO document_settings (org_id, tagline, footer, notice, legal_ids, vat, cashier, table_lines,
                    item_count, balance, barcode, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (org_id) DO UPDATE SET tagline = ?2, footer = ?3, notice = ?4, legal_ids = ?5, vat = ?6,
                cashier = ?7, table_lines = ?8, item_count = ?9, balance = ?10, barcode = ?11, updated_at = ?12,
                version = version + 1",
            params![
                id.to_string(),
                trimmed.tagline,
                trimmed.footer,
                trimmed.notice,
                trimmed.legal_ids,
                trimmed.vat,
                trimmed.cashier,
                trimmed.table,
                trimmed.count,
                trimmed.balance,
                trimmed.barcode,
                w.now()
            ],
        )?;
        if let Some(logo) = &logo {
            w.tx().execute(
                "UPDATE document_settings SET logo = ?2 WHERE org_id = ?1",
                params![id.to_string(), logo],
            )?;
        }
        let logo_now: Option<Vec<u8>> = w.tx().query_row(
            "SELECT logo FROM document_settings WHERE org_id = ?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        w.event("org.documents_updated", "organization", &id.to_string(), &trimmed)?;
        w.audit("update_documents", "organization", &id.to_string(), &trimmed)?;
        Ok(DocSettings {
            logo: logo_now,
            ..trimmed
        })
    })
}

/// A customer receipt as its voucher shows it (سند القبض).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintVoucher {
    pub number: String,
    pub created_at: i64,
    pub cashier: String,
    /// The customer's name and tax numbers.
    pub customer: (String, String),
    pub method: String,
    pub amount_minor: i64,
    pub reference: String,
    pub note: String,
    /// What it paid: (sale number, amount).
    pub invoices: Vec<(String, i64)>,
    /// Instalments it paid: (plan number, instalment, amount).
    pub installments: Vec<(String, u32, i64)>,
    /// Left as an advance.
    pub unallocated_minor: i64,
}

fn customer_ids(nif: String, rc: String) -> String {
    [("NIF", nif), ("RC", rc)]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A receipt by its number or id.
pub fn print_voucher(store: &Store, number: &str) -> Result<PrintVoucher> {
    store.read(|r| {
        let c = r.conn();
        let (org, _) = the_org(c)?;
        type Head = (String, String, i64, Option<String>, String, String, String, i64, String, String, i64);
        let h: Head = c
            .query_row(
                "SELECT r.id, r.number, r.created_at, u.name, cu.name, cu.nif, cu.rc, r.amount_minor,
                        r.method, r.reference || CASE WHEN r.note = '' THEN '' ELSE char(10) || r.note END,
                        r.unallocated_minor
                 FROM receipts r JOIN customers cu ON cu.id = r.customer_id
                 LEFT JOIN users u ON u.id = r.created_by
                 WHERE r.org_id = ?1 AND (r.number = ?2 OR r.id = ?2)",
                params![org.to_string(), number.trim()],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                        r.get(10)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::NotFound("receipt"))?;
        let (reference, note) = match h.9.split_once('\n') {
            Some((a, b)) => (a.to_owned(), b.to_owned()),
            None => (h.9.clone(), String::new()),
        };
        let mut s = c.prepare(
            "SELECT s.number, a.amount_minor FROM payment_allocations a JOIN sales s ON s.id = a.sale_id
             WHERE a.receipt_id = ?1 ORDER BY s.created_at",
        )?;
        let invoices = s
            .query_map([&h.0], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut s = c.prepare(
            "SELECT p.number, ip.seq, ip.amount_minor FROM installment_payments ip
             JOIN installment_plans p ON p.id = ip.plan_id
             WHERE ip.receipt_id = ?1 ORDER BY p.number, ip.seq",
        )?;
        let installments = s
            .query_map([&h.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(PrintVoucher {
            number: h.1,
            created_at: h.2,
            cashier: h.3.unwrap_or_default(),
            customer: (h.4, customer_ids(h.5, h.6)),
            amount_minor: h.7,
            method: h.8,
            reference,
            note,
            invoices,
            installments,
            unallocated_minor: h.10,
        })
    })
}

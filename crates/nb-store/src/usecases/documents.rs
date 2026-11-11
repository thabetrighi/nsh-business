//! Sales documents before the invoice (M8.13): quotes and orders.
//!
//! draft → approved → (quote) converted to an order / (either) invoiced
//! at the till; a draft or an approved document may be cancelled. A draft
//! is priced like a sale (catalog prices, or another price with its
//! right); an approved document keeps its contents; one that has moved on
//! or was cancelled never changes (the database refuses it too). The
//! invoice is the sale completed at the till from the document
//! (`SaleInput::document_id`), which marks it invoiced in the same
//! transaction.

use nb_domain::access::{Permission, Target};
use nb_domain::numbering::DocKind;
use nb_domain::sale::Cart;
use nb_domain::tax::TaxMode;
use nb_domain::{RuleError, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::repo::{org, parse_id, to_milli};
use crate::uow::{OpMeta, Writer};
use crate::usecases::sales::{SaleLineInput, discount, parse_lines, price_lines, the_org};
use crate::{Error, Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DocumentInput {
    /// A draft to change; empty for a new document.
    #[serde(default)]
    pub id: Option<String>,
    /// `quote` or `order`.
    pub kind: String,
    pub warehouse_id: String,
    #[serde(default)]
    pub customer_id: Option<String>,
    #[serde(default)]
    pub customer_name: String,
    /// `YYYY-MM-DD` (quotes).
    #[serde(default)]
    pub valid_until: Option<String>,
    pub lines: Vec<SaleLineInput>,
    #[serde(default)]
    pub discount_bps: Option<i64>,
    #[serde(default)]
    pub discount_minor: Option<i64>,
    #[serde(default)]
    pub note: String,
}

/// A document in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRow {
    pub id: String,
    pub kind: String,
    pub number: String,
    pub status: String,
    pub customer_name: String,
    pub valid_until: Option<String>,
    pub total_minor: i64,
    pub lines: u32,
    pub created_at: i64,
    /// The order a quote became, or the sale an order became.
    pub next_number: Option<String>,
}

/// A line as the document keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentLine {
    pub position: u32,
    pub product_id: Option<String>,
    pub name: String,
    pub unit: String,
    pub qty_milli: i64,
    pub unit_price_minor: i64,
    pub discount_bps: i64,
    pub tax_bps: i64,
    pub gross_minor: i64,
}

/// A document with its lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub row: DocumentRow,
    pub warehouse_id: String,
    pub customer_id: Option<String>,
    pub discount_bps: i64,
    pub discount_minor: i64,
    pub net_minor: i64,
    pub tax_minor: i64,
    pub note: String,
    pub version: i64,
    pub source_number: Option<String>,
    pub lines: Vec<DocumentLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DocumentQuery {
    /// `quote`, `order` or empty for both.
    #[serde(default)]
    pub kind: String,
    /// A state, or empty for all.
    #[serde(default)]
    pub status: String,
    /// In the number or the customer's name.
    #[serde(default)]
    pub search: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DocumentPage {
    pub rows: Vec<DocumentRow>,
    pub total: u32,
}

fn kind_of(code: &str) -> Result<DocKind> {
    match code {
        "quote" => Ok(DocKind::Quote),
        "order" => Ok(DocKind::Order),
        _ => Err(RuleError::on("kind", "invalid_value").into()),
    }
}

pub(crate) fn valid_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && d.chars()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && d[5..7].parse::<u32>().is_ok_and(|m| (1..=12).contains(&m))
        && d[8..10].parse::<u32>().is_ok_and(|x| (1..=31).contains(&x))
}

const ROW_SELECT: &str =
    "SELECT d.id, d.kind, d.number, d.status, d.customer_name, d.valid_until, d.total_minor,
        (SELECT COUNT(*) FROM sales_document_lines l WHERE l.document_id = d.id), d.created_at,
        COALESCE((SELECT n.number FROM sales_documents n WHERE n.id = d.next_id),
                 (SELECT s.number FROM sales s WHERE s.id = d.sale_id))
     FROM sales_documents d";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentRow> {
    Ok(DocumentRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        number: r.get(2)?,
        status: r.get(3)?,
        customer_name: r.get(4)?,
        valid_until: r.get(5)?,
        total_minor: r.get(6)?,
        lines: r.get(7)?,
        created_at: r.get(8)?,
        next_number: r.get(9)?,
    })
}

pub fn list_documents(store: &Store, q: &DocumentQuery) -> Result<DocumentPage> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let filter = " WHERE d.org_id = ?1 AND (?2 = '' OR d.kind = ?2) AND (?3 = '' OR d.status = ?3)
             AND (?4 = '' OR d.number LIKE '%' || ?4 || '%' OR d.customer_name LIKE '%' || ?4 || '%')";
        let search = q.search.trim();
        let total: u32 = r.conn().query_row(
            &format!("SELECT COUNT(*) FROM sales_documents d{filter}"),
            params![org.to_string(), q.kind, q.status, search],
            |r| r.get(0),
        )?;
        let mut s = r.conn().prepare(&format!(
            "{ROW_SELECT}{filter} ORDER BY d.created_at DESC, d.number DESC LIMIT ?5 OFFSET ?6"
        ))?;
        let rows = s
            .query_map(
                params![
                    org.to_string(),
                    q.kind,
                    q.status,
                    search,
                    q.limit.clamp(1, 500),
                    q.offset
                ],
                row_of,
            )?
            .collect::<std::result::Result<_, _>>()?;
        Ok(DocumentPage { rows, total })
    })
}

fn load(conn: &Connection, id: &str) -> Result<Document> {
    let row = conn
        .query_row(&format!("{ROW_SELECT} WHERE d.id = ?1"), [id], row_of)
        .optional()?
        .ok_or(Error::NotFound("document"))?;
    let (warehouse_id, customer_id, discount_bps, discount_minor, net, tax, note, version, source): (
        String,
        Option<String>,
        i64,
        i64,
        i64,
        i64,
        String,
        i64,
        Option<String>,
    ) = conn.query_row(
        "SELECT d.warehouse_id, d.customer_id, d.discount_bps, d.discount_minor, d.net_minor, d.tax_minor, d.note,
                d.version, (SELECT s.number FROM sales_documents s WHERE s.id = d.source_id)
         FROM sales_documents d WHERE d.id = ?1",
        [id],
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
            ))
        },
    )?;
    let mut s = conn.prepare(
        "SELECT position, product_id, name, unit, qty_milli, unit_price_minor, discount_bps, tax_bps, gross_minor
         FROM sales_document_lines WHERE document_id = ?1 ORDER BY position",
    )?;
    let lines = s
        .query_map([id], |r| {
            Ok(DocumentLine {
                position: r.get(0)?,
                product_id: r.get(1)?,
                name: r.get(2)?,
                unit: r.get(3)?,
                qty_milli: r.get(4)?,
                unit_price_minor: r.get(5)?,
                discount_bps: r.get(6)?,
                tax_bps: r.get(7)?,
                gross_minor: r.get(8)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(Document {
        row,
        warehouse_id,
        customer_id,
        discount_bps,
        discount_minor,
        net_minor: net,
        tax_minor: tax,
        note,
        version,
        source_number: source,
        lines,
    })
}

pub fn get_document(store: &Store, id: &str) -> Result<Document> {
    store.read(|r| load(r.conn(), id))
}

/// The document's branch, as a target for its rights.
fn target_of(w: &Writer<'_>, warehouse_id: &str) -> Result<(org::Branch, Target)> {
    let wh: WarehouseId = parse_id(warehouse_id)?;
    let warehouse = org::warehouse(w.tx(), wh)?;
    let branch = org::branch(w.tx(), parse_id(&warehouse.branch_id)?)?;
    let target = Target::warehouse(parse_id(&branch.id)?, wh);
    Ok((branch, target))
}

/// Writes the lines and figures of a draft (priced as a sale would be).
fn write_contents(
    w: &mut Writer<'_>,
    id: &str,
    input: &DocumentInput,
    at: &Target,
) -> Result<(i64, i64, i64)> {
    let (_, currency) = the_org(w.tx())?;
    let parsed = parse_lines(&input.lines)?;
    let (priced, lines) = price_lines(w, &parsed, &input.lines, currency, at, None)?;
    let cart = Cart {
        mode: TaxMode::Inclusive,
        lines,
        discount: discount(input.discount_bps, input.discount_minor, currency),
    };
    let totals = cart.totals()?;
    if totals.discounts.minor() > 0 {
        w.require(Permission::SalesDiscountLine, at)?;
    }
    w.tx().execute(
        "DELETE FROM sales_document_lines WHERE document_id = ?1",
        [id],
    )?;
    for (i, (item, t)) in priced.iter().zip(&totals.lines).enumerate() {
        w.tx().execute(
            "INSERT INTO sales_document_lines (document_id, position, product_id, name, unit, qty_milli,
                    unit_price_minor, discount_bps, tax_bps, net_minor, tax_minor, gross_minor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                i64::try_from(i).unwrap_or(0),
                item.product.as_ref().map(|p| p.id.clone()),
                item.name,
                item.unit,
                to_milli(cart.lines[i].qty)?,
                cart.lines[i].unit_price.minor(),
                input.lines[i].discount_bps.unwrap_or(0),
                item.tax_bps,
                t.taxed.net.minor(),
                t.taxed.tax.minor(),
                t.taxed.gross.minor()
            ],
        )?;
    }
    Ok((totals.net.minor(), totals.tax.minor(), totals.total.minor()))
}

fn check(input: &DocumentInput) -> Result<()> {
    if input.lines.is_empty() {
        return Err(RuleError::on("lines", "sale.empty").into());
    }
    if let Some(d) = input.valid_until.as_deref().filter(|d| !d.is_empty())
        && !valid_date(d)
    {
        return Err(RuleError::on("valid_until", "invalid_date").into());
    }
    Ok(())
}

/// Creates a draft quote or order, or changes a draft.
pub fn save_document(store: &mut Store, meta: OpMeta, input: DocumentInput) -> Result<Document> {
    let kind = kind_of(&input.kind)?;
    check(&input)?;
    let id = store.write(meta, |w| {
        let (branch, at) = target_of(w, &input.warehouse_id)?;
        w.require(Permission::SalesSell, &at)?;
        let valid_until = input.valid_until.clone().filter(|d| !d.is_empty());
        let name = input.customer_name.trim().to_owned();
        let id = match input.id.as_deref().filter(|i| !i.is_empty()) {
            Some(id) => {
                let (status, k): (String, String) = w
                    .tx()
                    .query_row(
                        "SELECT status, kind FROM sales_documents WHERE id = ?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?
                    .ok_or(Error::NotFound("document"))?;
                if status != "draft" {
                    return Err(RuleError::on("status", "doc.not_draft").into());
                }
                if k != input.kind {
                    return Err(RuleError::on("kind", "invalid_value").into());
                }
                let (net, tax, total) = write_contents(w, id, &input, &at)?;
                w.tx().execute(
                    "UPDATE sales_documents SET warehouse_id = ?2, customer_id = ?3, customer_name = ?4,
                            valid_until = ?5, discount_bps = ?6, discount_minor = ?7, net_minor = ?8,
                            tax_minor = ?9, total_minor = ?10, note = ?11, version = version + 1, updated_at = ?12
                     WHERE id = ?1",
                    params![
                        id,
                        input.warehouse_id,
                        input.customer_id,
                        name,
                        valid_until,
                        input.discount_bps.unwrap_or(0),
                        input.discount_minor.unwrap_or(0),
                        net,
                        tax,
                        total,
                        input.note.trim(),
                        w.now()
                    ],
                )?;
                w.audit("update", "sales_document", id, &input)?;
                id.to_owned()
            }
            None => {
                let id = nb_core::uuid::Uuid::now_v7().to_string();
                let number = w.next_number(kind, &branch.id, &branch.code)?;
                // The row first (its lines refer to it), its figures next.
                w.tx().execute(
                    "INSERT INTO sales_documents (id, org_id, branch_id, warehouse_id, kind, number, status,
                            customer_id, customer_name, valid_until, discount_bps, discount_minor,
                            net_minor, tax_minor, total_minor, note, created_at, created_by, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'draft', ?7, ?8, ?9, ?10, ?11, 0, 0, 0, ?12, ?13, ?14, ?13)",
                    params![
                        id,
                        w.org()?.to_string(),
                        branch.id,
                        input.warehouse_id,
                        input.kind,
                        number,
                        input.customer_id,
                        name,
                        valid_until,
                        input.discount_bps.unwrap_or(0),
                        input.discount_minor.unwrap_or(0),
                        input.note.trim(),
                        w.now(),
                        w.actor_text()
                    ],
                )?;
                let (net, tax, total) = write_contents(w, &id, &input, &at)?;
                w.tx().execute(
                    "UPDATE sales_documents SET net_minor = ?2, tax_minor = ?3, total_minor = ?4 WHERE id = ?1",
                    params![id, net, tax, total],
                )?;
                w.audit("create", "sales_document", &id, &input)?;
                id
            }
        };
        let doc = load(w.tx(), &id)?;
        w.event("sales_document.saved", "sales_document", &id, &doc.row)?;
        Ok(id)
    })?;
    get_document(store, &id)
}

/// Moves a document from `from` to `to`, with its right.
fn transition(
    store: &mut Store,
    meta: OpMeta,
    id: &str,
    from: &[&str],
    to: &str,
    reason: &str,
) -> Result<Document> {
    store.write(meta, |w| {
        let doc = load(w.tx(), id)?;
        if !from.contains(&doc.row.status.as_str()) {
            return Err(RuleError::on("status", "doc.wrong_state").into());
        }
        let (_, at) = target_of(w, &doc.warehouse_id)?;
        w.require(Permission::SalesSell, &at)?;
        if to == "approved" {
            w.tx().execute(
                "UPDATE sales_documents SET status = 'approved', approved_at = ?2, approved_by = ?3, updated_at = ?2
                 WHERE id = ?1",
                params![id, w.now(), w.actor_text()],
            )?;
        } else {
            w.tx().execute(
                "UPDATE sales_documents SET status = ?2, cancel_reason = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, to, reason.trim(), w.now()],
            )?;
        }
        let after = load(w.tx(), id)?;
        w.event(
            &format!("sales_document.{to}"),
            "sales_document",
            id,
            &after.row,
        )?;
        w.audit(to, "sales_document", id, &serde_json::json!({ "reason": reason }))?;
        Ok(after)
    })
}

pub fn approve_document(store: &mut Store, meta: OpMeta, id: &str) -> Result<Document> {
    transition(store, meta, id, &["draft"], "approved", "")
}

/// Cancels a draft or an approved document (why, for the record).
pub fn cancel_document(
    store: &mut Store,
    meta: OpMeta,
    id: &str,
    reason: &str,
) -> Result<Document> {
    if reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    transition(store, meta, id, &["draft", "approved"], "cancelled", reason)
}

/// An approved quote becomes an order (a draft, to adjust if needed); the
/// quote is then converted and stays as it was.
pub fn convert_to_order(store: &mut Store, meta: OpMeta, quote_id: &str) -> Result<Document> {
    let id = store.write(meta, |w| {
        let quote = load(w.tx(), quote_id)?;
        if quote.row.kind != "quote" || quote.row.status != "approved" {
            return Err(RuleError::on("status", "doc.wrong_state").into());
        }
        let (branch, at) = target_of(w, &quote.warehouse_id)?;
        w.require(Permission::SalesSell, &at)?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        let number = w.next_number(DocKind::Order, &branch.id, &branch.code)?;
        w.tx().execute(
            "INSERT INTO sales_documents (id, org_id, branch_id, warehouse_id, kind, number, status, customer_id,
                    customer_name, discount_bps, discount_minor, net_minor, tax_minor, total_minor, note, source_id,
                    created_at, created_by, updated_at)
             SELECT ?2, org_id, branch_id, warehouse_id, 'order', ?3, 'draft', customer_id, customer_name,
                    discount_bps, discount_minor, net_minor, tax_minor, total_minor, note, id, ?4, ?5, ?4
             FROM sales_documents WHERE id = ?1",
            params![quote_id, id, number, w.now(), w.actor_text()],
        )?;
        w.tx().execute(
            "INSERT INTO sales_document_lines (document_id, position, product_id, name, unit, qty_milli,
                    unit_price_minor, discount_bps, tax_bps, net_minor, tax_minor, gross_minor)
             SELECT ?2, position, product_id, name, unit, qty_milli, unit_price_minor, discount_bps, tax_bps,
                    net_minor, tax_minor, gross_minor
             FROM sales_document_lines WHERE document_id = ?1",
            params![quote_id, id],
        )?;
        w.tx().execute(
            "UPDATE sales_documents SET status = 'converted', next_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![quote_id, id, w.now()],
        )?;
        let order = load(w.tx(), &id)?;
        w.event("sales_document.converted", "sales_document", quote_id, &order.row)?;
        w.audit("convert", "sales_document", quote_id, &serde_json::json!({ "order": order.row.number }))?;
        Ok(id)
    })?;
    get_document(store, &id)
}

/// The sale completed from `document_id`: the document must be approved;
/// it is invoiced in the sale's own transaction.
pub(crate) fn invoice_from(w: &mut Writer<'_>, document_id: &str, sale_id: &str) -> Result<()> {
    let status: String = w
        .tx()
        .query_row(
            "SELECT status FROM sales_documents WHERE id = ?1",
            [document_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound("document"))?;
    if status != "approved" {
        return Err(RuleError::on("document", "doc.wrong_state").into());
    }
    w.tx().execute(
        "UPDATE sales_documents SET status = 'invoiced', sale_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![document_id, sale_id, w.now()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::valid_date;

    #[test]
    fn dates_typed_as_iso() {
        assert!(valid_date("2026-11-30"));
        assert!(!valid_date("2026-13-01"));
        assert!(!valid_date("30/11/2026"));
        assert!(!valid_date("2026-1-301"));
    }
}

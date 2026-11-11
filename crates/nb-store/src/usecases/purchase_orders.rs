//! Purchase orders (M10.1) and receiving against them (M10.2, M10.3).
//!
//! An order is drafted with the right to order, approved with the right to
//! approve purchases (a supervisor may approve it for the one who asks),
//! then received by purchase invoices that name it
//! (`PurchaseInput::order_id`): each invoice line is matched to an order
//! line — same product and unit, never more than what is left, a cost
//! above the ordered one only with the right to approve purchases — and the
//! order follows (partial, received). The rules are in
//! [`nb_domain::purchase`]; the database keeps an approved order's
//! contents as they were.

use nb_domain::access::{Permission, Target};
use nb_domain::numbering::DocKind;
use nb_domain::purchase::{self, OrderStatus, OrderedLine};
use nb_domain::sale::Cart;
use nb_domain::tax::TaxMode;
use nb_domain::{RuleError, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::repo::{org, parse_id, to_milli};
use crate::uow::{OpMeta, Writer};
use crate::usecases::documents::valid_date;
use crate::usecases::purchasing::{BuyLine, PurchaseLineInput, load_supplier, parse_buy_lines};
use crate::usecases::sales::the_org;
use crate::{Error, Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OrderInput {
    /// A draft to change; empty for a new order.
    #[serde(default)]
    pub id: Option<String>,
    pub supplier_id: String,
    pub warehouse_id: String,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub expected_date: Option<String>,
    /// Costs before tax.
    pub lines: Vec<PurchaseLineInput>,
    #[serde(default)]
    pub note: String,
}

/// An order in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRow {
    pub id: String,
    pub number: String,
    pub status: String,
    pub supplier_id: String,
    pub supplier: String,
    pub expected_date: Option<String>,
    pub total_minor: i64,
    pub lines: u32,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderLine {
    pub position: u32,
    pub product_id: String,
    pub name: String,
    pub unit: String,
    pub qty_milli: i64,
    pub received_milli: i64,
    pub unit_cost_minor: i64,
    pub tax_bps: i64,
    pub net_minor: i64,
}

impl OrderLine {
    pub fn remaining_milli(&self) -> i64 {
        (self.qty_milli - self.received_milli).max(0)
    }
}

/// An order with its lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub row: OrderRow,
    pub branch_id: String,
    pub warehouse_id: String,
    pub net_minor: i64,
    pub vat_minor: i64,
    pub note: String,
    pub reason: String,
    pub version: i64,
    /// The purchase invoices that received it.
    pub receipts: Vec<String>,
    pub lines: Vec<OrderLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OrderQuery {
    /// A state, `open` (approved or partial), or empty for all.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub supplier_id: String,
    /// In the number or the supplier's name.
    #[serde(default)]
    pub search: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OrderPage {
    pub rows: Vec<OrderRow>,
    pub total: u32,
}

const ROW_SELECT: &str =
    "SELECT o.id, o.number, o.status, o.supplier_id, s.name, o.expected_date, o.total_minor,
        (SELECT COUNT(*) FROM purchase_order_lines l WHERE l.order_id = o.id), o.created_at
     FROM purchase_orders o JOIN suppliers s ON s.id = o.supplier_id";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<OrderRow> {
    Ok(OrderRow {
        id: r.get(0)?,
        number: r.get(1)?,
        status: r.get(2)?,
        supplier_id: r.get(3)?,
        supplier: r.get(4)?,
        expected_date: r.get(5)?,
        total_minor: r.get(6)?,
        lines: r.get(7)?,
        created_at: r.get(8)?,
    })
}

pub fn list_orders(store: &Store, q: &OrderQuery) -> Result<OrderPage> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let filter = " WHERE o.org_id = ?1
             AND (?2 = '' OR o.status = ?2 OR (?2 = 'open' AND o.status IN ('approved', 'partial')))
             AND (?3 = '' OR o.supplier_id = ?3)
             AND (?4 = '' OR o.number LIKE '%' || ?4 || '%' OR s.name LIKE '%' || ?4 || '%')";
        let search = q.search.trim();
        let total: u32 = r.conn().query_row(
            &format!(
                "SELECT COUNT(*) FROM purchase_orders o JOIN suppliers s ON s.id = o.supplier_id{filter}"
            ),
            params![org.to_string(), q.status, q.supplier_id, search],
            |r| r.get(0),
        )?;
        let mut s = r.conn().prepare(&format!(
            "{ROW_SELECT}{filter} ORDER BY o.created_at DESC, o.number DESC LIMIT ?5 OFFSET ?6"
        ))?;
        let rows = s
            .query_map(
                params![
                    org.to_string(),
                    q.status,
                    q.supplier_id,
                    search,
                    q.limit.clamp(1, 500),
                    q.offset
                ],
                row_of,
            )?
            .collect::<std::result::Result<_, _>>()?;
        Ok(OrderPage { rows, total })
    })
}

fn load(conn: &Connection, id: &str) -> Result<Order> {
    let row = conn
        .query_row(&format!("{ROW_SELECT} WHERE o.id = ?1"), [id], row_of)
        .optional()?
        .ok_or(Error::NotFound("purchase order"))?;
    type Head = (String, String, i64, i64, String, String, i64);
    let (branch_id, warehouse_id, net, vat, note, reason, version): Head = conn.query_row(
        "SELECT branch_id, warehouse_id, net_minor, vat_minor, note, reason, version
         FROM purchase_orders WHERE id = ?1",
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
            ))
        },
    )?;
    let mut s = conn.prepare(
        "SELECT position, product_id, name, unit, qty_milli, received_milli, unit_cost_minor, tax_bps, net_minor
         FROM purchase_order_lines WHERE order_id = ?1 ORDER BY position",
    )?;
    let lines = s
        .query_map([id], |r| {
            Ok(OrderLine {
                position: r.get(0)?,
                product_id: r.get(1)?,
                name: r.get(2)?,
                unit: r.get(3)?,
                qty_milli: r.get(4)?,
                received_milli: r.get(5)?,
                unit_cost_minor: r.get(6)?,
                tax_bps: r.get(7)?,
                net_minor: r.get(8)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut s = conn
        .prepare("SELECT number FROM purchases WHERE order_id = ?1 ORDER BY created_at, number")?;
    let receipts = s
        .query_map([id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(Order {
        row,
        branch_id,
        warehouse_id,
        net_minor: net,
        vat_minor: vat,
        note,
        reason,
        version,
        receipts,
        lines,
    })
}

pub fn get_order(store: &Store, id: &str) -> Result<Order> {
    store.read(|r| load(r.conn(), id))
}

/// The order's branch, as a target for its rights.
fn target_of(w: &Writer<'_>, warehouse_id: &str) -> Result<(org::Branch, Target)> {
    let wh: WarehouseId = parse_id(warehouse_id)?;
    let warehouse = org::warehouse(w.tx(), wh)?;
    let branch = org::branch(w.tx(), parse_id(&warehouse.branch_id)?)?;
    let target = Target::warehouse(parse_id(&branch.id)?, wh);
    Ok((branch, target))
}

/// Writes a draft's lines; its figures before and after tax.
fn write_lines(w: &mut Writer<'_>, id: &str, input: &OrderInput) -> Result<(i64, i64)> {
    let (_, currency) = the_org(w.tx())?;
    let (products, cart_lines): (Vec<BuyLine>, _) =
        parse_buy_lines(w.tx(), &input.lines, currency)?;
    let totals = Cart {
        mode: TaxMode::Exclusive,
        lines: cart_lines.clone(),
        discount: nb_domain::sale::Discount::None,
    }
    .totals()?;
    w.tx()
        .execute("DELETE FROM purchase_order_lines WHERE order_id = ?1", [id])?;
    for (i, ((p, unit, qty, _, tax_bps), t)) in products.iter().zip(&totals.lines).enumerate() {
        w.tx().execute(
            "INSERT INTO purchase_order_lines (order_id, position, product_id, name, unit, qty_milli,
                    unit_cost_minor, tax_bps, net_minor, tax_minor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                i64::try_from(i).unwrap_or(0),
                p.id,
                p.name,
                unit,
                to_milli(*qty)?,
                cart_lines[i].unit_price.minor(),
                tax_bps,
                t.taxed.net.minor(),
                t.taxed.tax.minor()
            ],
        )?;
    }
    Ok((totals.net.minor(), totals.tax.minor()))
}

/// Creates a draft order, or changes a draft.
pub fn save_order(store: &mut Store, meta: OpMeta, input: OrderInput) -> Result<Order> {
    if input.lines.is_empty() {
        return Err(RuleError::on("lines", "sale.empty").into());
    }
    let expected = input.expected_date.clone().filter(|d| !d.trim().is_empty());
    if let Some(d) = &expected
        && !valid_date(d)
    {
        return Err(RuleError::on("expected_date", "invalid_date").into());
    }
    let id = store.write(meta, |w| {
        let (branch, at) = target_of(w, &input.warehouse_id)?;
        w.require(Permission::PurchasesOrder, &at)?;
        let supplier = load_supplier(w.tx(), &input.supplier_id)?;
        if supplier.status == "blocked" {
            return Err(RuleError::on("supplier", "supplier.blocked").into());
        }
        let id = match input.id.as_deref().filter(|i| !i.is_empty()) {
            Some(id) => {
                let status: String = w
                    .tx()
                    .query_row("SELECT status FROM purchase_orders WHERE id = ?1", [id], |r| r.get(0))
                    .optional()?
                    .ok_or(Error::NotFound("purchase order"))?;
                if status != OrderStatus::Draft.code() {
                    return Err(RuleError::on("status", "doc.not_draft").into());
                }
                let (net, vat) = write_lines(w, id, &input)?;
                w.tx().execute(
                    "UPDATE purchase_orders SET warehouse_id = ?2, supplier_id = ?3, expected_date = ?4,
                            net_minor = ?5, vat_minor = ?6, total_minor = ?7, note = ?8, version = version + 1,
                            updated_at = ?9
                     WHERE id = ?1",
                    params![
                        id,
                        input.warehouse_id,
                        supplier.id,
                        expected,
                        net,
                        vat,
                        net + vat,
                        input.note.trim(),
                        w.now()
                    ],
                )?;
                w.audit("update", "purchase_order", id, &input)?;
                id.to_owned()
            }
            None => {
                let id = nb_core::uuid::Uuid::now_v7().to_string();
                let number = w.next_number(DocKind::PurchaseOrder, &branch.id, &branch.code)?;
                w.tx().execute(
                    "INSERT INTO purchase_orders (id, org_id, branch_id, warehouse_id, supplier_id, number, status,
                            expected_date, net_minor, vat_minor, total_minor, note, created_at, created_by, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'draft', ?7, 0, 0, 0, ?8, ?9, ?10, ?9)",
                    params![
                        id,
                        w.org()?.to_string(),
                        branch.id,
                        input.warehouse_id,
                        supplier.id,
                        number,
                        expected,
                        input.note.trim(),
                        w.now(),
                        w.actor_text()
                    ],
                )?;
                let (net, vat) = write_lines(w, &id, &input)?;
                w.tx().execute(
                    "UPDATE purchase_orders SET net_minor = ?2, vat_minor = ?3, total_minor = ?4 WHERE id = ?1",
                    params![id, net, vat, net + vat],
                )?;
                w.audit("create", "purchase_order", &id, &input)?;
                id
            }
        };
        let order = load(w.tx(), &id)?;
        w.event("purchase_order.saved", "purchase_order", &id, &order.row)?;
        Ok(id)
    })?;
    get_order(store, &id)
}

/// Moves an order by a person's action (approve, cancel, close).
fn transition(
    store: &mut Store,
    meta: OpMeta,
    id: &str,
    to: OrderStatus,
    reason: &str,
    approval: Option<&str>,
) -> Result<Order> {
    let approval = approval
        .filter(|a| !a.is_empty())
        .map(parse_id)
        .transpose()?;
    store.write(meta, |w| {
        let order = load(w.tx(), id)?;
        let from = OrderStatus::parse(&order.row.status).ok_or(RuleError::new("internal"))?;
        if !from.may_become(to) {
            return Err(RuleError::on("status", "doc.wrong_state").into());
        }
        let (_, at) = target_of(w, &order.warehouse_id)?;
        if to == OrderStatus::Approved {
            w.require_approved(Permission::PurchasesApprove, &at, approval)?;
            w.tx().execute(
                "UPDATE purchase_orders SET status = 'approved', approved_at = ?2, approved_by = ?3, updated_at = ?2
                 WHERE id = ?1",
                params![id, w.now(), w.actor_text()],
            )?;
        } else {
            w.require(Permission::PurchasesOrder, &at)?;
            w.tx().execute(
                "UPDATE purchase_orders SET status = ?2, reason = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, to.code(), reason.trim(), w.now()],
            )?;
        }
        let after = load(w.tx(), id)?;
        w.event(
            &format!("purchase_order.{}", to.code()),
            "purchase_order",
            id,
            &after.row,
        )?;
        w.audit(
            to.code(),
            "purchase_order",
            id,
            &serde_json::json!({ "reason": reason }),
        )?;
        Ok(after)
    })
}

/// Approves a draft order (with a supervisor's approval for one who may
/// only ask).
pub fn approve_order(
    store: &mut Store,
    meta: OpMeta,
    id: &str,
    approval: Option<&str>,
) -> Result<Order> {
    transition(store, meta, id, OrderStatus::Approved, "", approval)
}

/// Cancels an order nothing was received against (why, for the record).
pub fn cancel_order(store: &mut Store, meta: OpMeta, id: &str, reason: &str) -> Result<Order> {
    if reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    transition(store, meta, id, OrderStatus::Cancelled, reason, None)
}

/// Closes a partly received order: the rest will not come.
pub fn close_order(store: &mut Store, meta: OpMeta, id: &str, reason: &str) -> Result<Order> {
    if reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    transition(store, meta, id, OrderStatus::Closed, reason, None)
}

/// Receives an invoice's lines against an approved order, in the invoice's
/// own transaction: each line matched to an order line, the order's lines
/// and state moved on. Returns the cost ordered for each line.
pub(crate) fn receive(
    w: &mut Writer<'_>,
    order_id: &str,
    supplier_id: &str,
    warehouse_id: &str,
    lines: &[PurchaseLineInput],
    resolved: &[BuyLine],
    approval: Option<&str>,
) -> Result<Vec<i64>> {
    let order = load(w.tx(), order_id)?;
    let status = OrderStatus::parse(&order.row.status).ok_or(RuleError::new("internal"))?;
    if !status.receivable() {
        return Err(RuleError::on("order", "doc.wrong_state").into());
    }
    if order.row.supplier_id != supplier_id || order.warehouse_id != warehouse_id {
        return Err(RuleError::on("order", "purchase.order_mismatch").into());
    }
    let mut state: Vec<OrderedLine> = order
        .lines
        .iter()
        .map(|l| OrderedLine {
            ordered_milli: l.qty_milli,
            received_milli: l.received_milli,
        })
        .collect();
    let mut costs = Vec::with_capacity(lines.len());
    let mut above = false;
    for (input, (p, unit, qty, _, _)) in lines.iter().zip(resolved) {
        let Some(pos) = input.order_position else {
            return Err(RuleError::on("lines", "purchase.not_ordered").into());
        };
        let Some(i) = order.lines.iter().position(|l| l.position == pos) else {
            return Err(RuleError::on("lines", "purchase.not_ordered").into());
        };
        let ol = &order.lines[i];
        if ol.product_id != p.id || &ol.unit != unit {
            return Err(RuleError::on("lines", "purchase.not_ordered").into());
        }
        let q = to_milli(*qty)?;
        purchase::check_receipt(state[i], q)?;
        state[i].received_milli += q;
        above |= purchase::cost_variance_bps(ol.unit_cost_minor, input.unit_cost_minor) > 0;
        costs.push(ol.unit_cost_minor);
    }
    if above {
        let (_, at) = target_of(w, warehouse_id)?;
        let approval = approval
            .filter(|a| !a.is_empty())
            .map(parse_id)
            .transpose()?;
        w.require_approved(Permission::PurchasesApprove, &at, approval)?;
    }
    for (l, s) in order.lines.iter().zip(&state) {
        if s.received_milli != l.received_milli {
            w.tx().execute(
                "UPDATE purchase_order_lines SET received_milli = ?3 WHERE order_id = ?1 AND position = ?2",
                params![order_id, l.position, s.received_milli],
            )?;
        }
    }
    let next = purchase::after_receipt(&state);
    if next != status {
        w.tx().execute(
            "UPDATE purchase_orders SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![order_id, next.code(), w.now()],
        )?;
    }
    let after = load(w.tx(), order_id)?;
    w.event(
        "purchase_order.received",
        "purchase_order",
        order_id,
        &after.row,
    )?;
    Ok(costs)
}

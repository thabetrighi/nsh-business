//! Returns tied to their sale (M8.8), and carts put aside at the till
//! (M8.7).
//!
//! A return gives back, line by line, never more than what remains of the
//! sale; it refunds in cash (out of the open till), by card or transfer,
//! or on the customer's account; the goods go back to stock at their sale
//! cost, or stay out (damaged). Its entry reverses the sale's for that
//! share, in the same transaction as the stock, the event and the audit.

use nb_core::{Money, Quantity};
use nb_domain::access::{Permission, Target};
use nb_domain::numbering::DocKind;
use nb_domain::posting::{Operation, Settlement};
use nb_domain::sale::{Refund, Sold, refund};
use nb_domain::stock::MovementKind;
use nb_domain::{ProductId, RegisterId, RuleError, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::repo::{org, parse_id, stock};
use crate::uow::OpMeta;
use crate::usecases::accounting::post_operations_at;
use crate::usecases::sales::{the_org, till};
use crate::{Error, Result, Store};

/// A line of a sale, with what is still returnable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoldLine {
    pub position: u32,
    pub product_id: Option<String>,
    pub name: String,
    pub unit: String,
    pub qty_milli: i64,
    pub returned_milli: i64,
    pub unit_price_minor: i64,
    pub gross_minor: i64,
}

impl SoldLine {
    pub fn returnable_milli(&self) -> i64 {
        self.qty_milli - self.returned_milli
    }
}

/// A sale as a return sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaleDetail {
    pub id: String,
    pub number: String,
    pub created_at: i64,
    pub total_minor: i64,
    pub returned_minor: i64,
    pub customer_id: Option<String>,
    pub lines: Vec<SoldLine>,
}

/// The sale with this number (or id), with what is left to return.
pub fn sale_for_return(store: &Store, number: &str) -> Result<SaleDetail> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let key = number.trim().to_uppercase();
        let row: Option<(String, String, i64, i64, Option<String>)> = r
            .conn()
            .query_row(
                "SELECT id, number, created_at, total_minor, customer_id FROM sales
                 WHERE org_id = ?1 AND (upper(number) = ?2 OR id = ?3)",
                params![org.to_string(), key, number.trim()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (id, number, created_at, total_minor, customer_id) =
            row.ok_or(Error::from(RuleError::on("sale", "return.sale_unknown")))?;
        let returned_minor: i64 = r.conn().query_row(
            "SELECT COALESCE(SUM(total_minor), 0) FROM sale_returns WHERE sale_id = ?1",
            [&id],
            |r| r.get(0),
        )?;
        let mut s = r.conn().prepare(
            "SELECT l.position, l.product_id, l.name, l.unit, l.qty_milli,
                    COALESCE((SELECT SUM(x.qty_milli) FROM sale_return_lines x
                              WHERE x.sale_id = l.sale_id AND x.position = l.position), 0),
                    l.unit_price_minor, l.gross_minor
             FROM sale_lines l WHERE l.sale_id = ?1 ORDER BY l.position",
        )?;
        let lines = s
            .query_map([&id], |r| {
                Ok(SoldLine {
                    position: r.get(0)?,
                    product_id: r.get(1)?,
                    name: r.get(2)?,
                    unit: r.get(3)?,
                    qty_milli: r.get(4)?,
                    returned_milli: r.get(5)?,
                    unit_price_minor: r.get(6)?,
                    gross_minor: r.get(7)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(SaleDetail {
            id,
            number,
            created_at,
            total_minor,
            returned_minor,
            customer_id,
            lines,
        })
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnLineInput {
    /// The line of the sale.
    pub position: u32,
    /// In the line's unit (decimal text).
    pub qty: String,
    /// Back to stock (`false`: damaged, kept out).
    #[serde(default = "yes")]
    pub restock: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReturnInput {
    /// The sale's id or number.
    pub sale: String,
    pub register_id: String,
    pub lines: Vec<ReturnLineInput>,
    /// `cash`, `card`, `transfer` or `credit` (the customer's account).
    pub refund: String,
    pub reason: String,
    #[serde(default)]
    pub approval: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnResult {
    pub id: String,
    pub number: String,
    pub net_minor: i64,
    pub tax_minor: i64,
    pub total_minor: i64,
    pub cost_minor: i64,
}

/// A sold line as a return needs it: what was sold, what already came
/// back, its product, the movement that took it out, and the warehouse.
type LineFacts = (Sold, Refund, Option<String>, Option<String>, String);

fn line_facts(conn: &Connection, sale: &str, position: u32) -> Result<LineFacts> {
    let (qty, net, tax, cost, product, movement, wh): (i64, i64, i64, i64, Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT l.base_qty_milli, l.net_minor, l.tax_minor, l.unit_cost_minor, l.product_id, l.movement_id, s.warehouse_id
             FROM sale_lines l JOIN sales s ON s.id = l.sale_id WHERE l.sale_id = ?1 AND l.position = ?2",
            params![sale, position],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()?
        .ok_or(Error::from(RuleError::on("lines", "return.line_unknown")))?;
    let before: (i64, i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(qty_milli), 0), COALESCE(SUM(net_minor), 0), COALESCE(SUM(tax_minor), 0)
         FROM sale_return_lines WHERE sale_id = ?1 AND position = ?2",
        params![sale, position],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok((
        Sold {
            qty_milli: qty,
            net,
            tax,
            unit_cost: cost,
        },
        Refund {
            qty_milli: before.0,
            net: before.1,
            tax: before.2,
            cost: 0,
        },
        product,
        movement,
        wh,
    ))
}

/// Returns goods of a sale: the refund, the stock, the entry, the event and
/// the audit, at once and once per operation.
pub fn return_sale(store: &mut Store, meta: OpMeta, input: ReturnInput) -> Result<ReturnResult> {
    let register: RegisterId = parse_id(&input.register_id)?;
    if input.reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    if input.lines.is_empty() {
        return Err(RuleError::on("lines", "return.empty").into());
    }
    let method = match input.refund.as_str() {
        m @ ("cash" | "card" | "transfer" | "credit") => m.to_owned(),
        _ => return Err(RuleError::on("refund", "invalid_value").into()),
    };
    let sale = sale_for_return(store, &input.sale)?;
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    store.write(meta, |w| {
        let (branch, _, till_account) = till(w, register)?;
        let branch_id = parse_id(&branch.id)?;
        let at_till = Target::register(branch_id, register);
        let approval = input.approval.as_deref().map(parse_id).transpose()?;
        w.require_approved(Permission::SalesReturn, &at_till, approval)?;
        if method == "credit" && sale.customer_id.is_none() {
            return Err(RuleError::on("refund", "sale.credit_needs_customer").into());
        }
        let shift: Option<String> = w
            .tx()
            .query_row(
                "SELECT id FROM shifts WHERE register_id = ?1 AND status = 'open'",
                [register.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if method == "cash" && shift.is_none() {
            return Err(RuleError::on("register", "shift.none_open").into());
        }
        let number = w.next_number(DocKind::SaleReturn, &branch.id, &branch.code)?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();

        let mut lines = Vec::with_capacity(input.lines.len());
        let (mut net, mut tax, mut cost_back) = (0i64, 0i64, 0i64);
        for l in &input.lines {
            if input.lines.iter().filter(|x| x.position == l.position).count() > 1 {
                return Err(RuleError::on("lines", "return.line_twice").into());
            }
            let (sold, before, product, movement, wh) = line_facts(w.tx(), &sale.id, l.position)?;
            let q: Quantity = l
                .qty
                .trim()
                .replace(',', ".")
                .parse()
                .map_err(|_| RuleError::on("qty", "invalid_quantity"))?;
            let qty_milli = crate::repo::to_milli(q)?;
            // Sold quantities are in base units; a return is typed in the
            // line's unit: convert by the sale's own ratio.
            let line_qty: i64 = w.tx().query_row(
                "SELECT qty_milli FROM sale_lines WHERE sale_id = ?1 AND position = ?2",
                params![sale.id, l.position],
                |r| r.get(0),
            )?;
            let base_milli = if line_qty == 0 {
                0
            } else {
                i64::try_from(i128::from(qty_milli) * i128::from(sold.qty_milli) / i128::from(line_qty))
                    .unwrap_or(i64::MAX)
            };
            let back = refund(sold, before, base_milli)?;
            net += back.net;
            tax += back.tax;
            let mut movement_id = None;
            if let (true, Some(product)) = (l.restock, product.as_deref()) {
                let product_id: ProductId = parse_id(product)?;
                let wh_id: WarehouseId = parse_id(&wh)?;
                let warehouse = org::warehouse(w.tx(), wh_id)?;
                let qty = Quantity::from_milli(back.qty_milli);
                let unit_cost = Money::from_minor(sold.unit_cost, currency);
                let before = stock::balance(w.tx(), product_id, wh_id, currency)?;
                let after = before.apply(MovementKind::SaleReturn, qty, Some(unit_cost), warehouse.policy(), true)?;
                let m = stock::append(w, product_id, wh_id, MovementKind::SaleReturn, qty, Some(unit_cost), Some(&number), None, after)?;
                // The lots or serial numbers that left with the sale come back.
                if let Some(out) = movement.as_deref() {
                    crate::usecases::tracked::mirror(w, out, m, product_id, wh_id, qty)?;
                }
                cost_back += back.cost;
                movement_id = Some(m.to_string());
            }
            lines.push((l.position, back, l.restock && product.is_some(), movement_id));
        }
        let total = net + tax;
        if method == "cash"
            && let Some(shift) = shift.as_deref()
            && total > crate::usecases::sales::shift_of(w.tx(), shift)?.expected_minor
        {
            return Err(RuleError::on("refund", "shift.not_enough_cash").into());
        }
        w.tx().execute(
            "INSERT INTO sale_returns (id, org_id, branch_id, sale_id, register_id, shift_id, number, refund_method, reason,
                    net_minor, tax_minor, total_minor, cost_minor, operation_id, created_at, created_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                id, w.org()?.to_string(), branch.id, sale.id, register.to_string(), shift, number, method,
                input.reason.trim(), net, tax, total, cost_back, w.meta().id.to_string(), w.now(), w.actor_text(),
                w.device().id.to_string()
            ],
        )?;
        for (position, back, restocked, movement) in &lines {
            w.tx().execute(
                "INSERT INTO sale_return_lines (return_id, sale_id, position, qty_milli, net_minor, tax_minor, cost_minor, restocked, movement_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, sale.id, position, back.qty_milli, back.net, back.tax, if *restocked { back.cost } else { 0 }, restocked, movement],
            )?;
        }
        let m = |v: i64| Money::from_minor(v, currency);
        let zero = m(0);
        let refund = match method.as_str() {
            "cash" => Settlement { cash: m(total), bank: zero, credit: zero },
            "credit" => Settlement { cash: zero, bank: zero, credit: m(total) },
            _ => Settlement { cash: zero, bank: m(total), credit: zero },
        };
        post_operations_at(
            w,
            &branch,
            offset,
            &[
                Operation::SaleReturn { net: m(net), vat: m(tax), refund },
                Operation::ReturnToStock { cost: m(cost_back) },
            ],
            &format!("{number} — {}", sale.number),
            "sale.return",
            Some(&till_account),
        )?;
        let out = ReturnResult {
            id: id.clone(),
            number,
            net_minor: net,
            tax_minor: tax,
            total_minor: total,
            cost_minor: cost_back,
        };
        w.event("sale.returned", "sale_return", &id, &out)?;
        w.audit("return", "sale", &sale.id, &input)?;
        Ok(out)
    })
}

// ---------------------------------------------------------------------------
// Carts put aside (M8.7)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldCart {
    pub id: String,
    pub label: String,
    pub lines: u32,
    pub total_minor: i64,
    pub created_at: i64,
    /// What the till saved, as it saved it.
    pub payload: String,
}

/// Puts a cart aside at a till, under a label.
pub fn hold_cart(
    store: &mut Store,
    meta: OpMeta,
    register_id: &str,
    label: &str,
    lines: u32,
    total_minor: i64,
    payload: &str,
) -> Result<HeldCart> {
    let register: RegisterId = parse_id(register_id)?;
    if lines == 0 {
        return Err(RuleError::on("lines", "sale.empty").into());
    }
    store.write(meta, |w| {
        let (branch, _, _) = till(w, register)?;
        w.require(Permission::SalesSell, &Target::register(parse_id(&branch.id)?, register))?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        let n: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM held_carts WHERE register_id = ?1",
            [register_id],
            |r| r.get(0),
        )?;
        let label = if label.trim().is_empty() {
            format!("#{}", n + 1)
        } else {
            label.trim().to_owned()
        };
        w.tx().execute(
            "INSERT INTO held_carts (id, org_id, register_id, label, lines, total_minor, payload, created_at, created_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, w.org()?.to_string(), register_id, label, lines, total_minor, payload, w.now(), w.actor_text()],
        )?;
        let held = HeldCart {
            id: id.clone(),
            label,
            lines,
            total_minor,
            created_at: w.now(),
            payload: payload.to_owned(),
        };
        w.event("cart.held", "held_cart", &id, &held)?;
        w.audit("hold", "held_cart", &id, &serde_json::json!({ "label": held.label, "lines": lines }))?;
        Ok(held)
    })
}

pub fn held_carts(store: &Store, register_id: &str) -> Result<Vec<HeldCart>> {
    store.read(|r| {
        let mut s = r.conn().prepare(
            "SELECT id, label, lines, total_minor, created_at, payload FROM held_carts
             WHERE register_id = ?1 ORDER BY created_at",
        )?;
        let rows = s.query_map([register_id], |r| {
            Ok(HeldCart {
                id: r.get(0)?,
                label: r.get(1)?,
                lines: r.get(2)?,
                total_minor: r.get(3)?,
                created_at: r.get(4)?,
                payload: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

/// Takes a cart back (it leaves the list).
pub fn take_held(store: &mut Store, meta: OpMeta, id: &str) -> Result<HeldCart> {
    store.write(meta, |w| {
        let held = w
            .tx()
            .query_row(
                "SELECT id, label, lines, total_minor, created_at, payload FROM held_carts WHERE id = ?1",
                [id],
                |r| {
                    Ok(HeldCart {
                        id: r.get(0)?,
                        label: r.get(1)?,
                        lines: r.get(2)?,
                        total_minor: r.get(3)?,
                        created_at: r.get(4)?,
                        payload: r.get(5)?,
                    })
                },
            )
            .optional()?
            .ok_or(Error::NotFound("held cart"))?;
        w.tx().execute("DELETE FROM held_carts WHERE id = ?1", [id])?;
        w.event("cart.taken", "held_cart", id, &held)?;
        w.audit("take", "held_cart", id, &serde_json::json!({ "label": held.label }))?;
        Ok(held)
    })
}

//! Inventory (M6): stock by product and warehouse, the movement journal,
//! reservations, transfers between warehouses and stock counts. Every
//! change is a movement (append-only) with its balance, document number,
//! accounting entry when value changes hands, event and audit — in one
//! operation.

use nb_core::{Currency, Money, OrganizationId, Quantity};
use nb_domain::access::{Permission, Target};
use nb_domain::numbering::DocKind;
use nb_domain::posting::Operation;
use nb_domain::stock::{self as rules, MovementKind};
use nb_domain::{CategoryId, ProductId, RuleError, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::repo::catalog::{self, StockState};
use crate::repo::org::{self, Branch, Warehouse};
use crate::repo::{from_milli, parse_id, stock, to_milli};
use crate::time::iso_date;
use crate::uow::OpMeta;
use crate::{Error, Result, Store};

fn the_org(conn: &Connection) -> Result<(OrganizationId, Currency)> {
    let o = org::the_org(conn)?.ok_or(Error::NotFound("organization"))?;
    Ok((o.org_id()?, o.currency()?))
}

/// The warehouse, its branch, and the target of a request about it.
fn place(conn: &Connection, wh: WarehouseId) -> Result<(Warehouse, Branch, Target)> {
    let w = org::warehouse(conn, wh)?;
    let b = org::branch(conn, parse_id(&w.branch_id)?)?;
    let t = Target::warehouse(parse_id(&b.id)?, wh);
    Ok((w, b, t))
}

fn qty(field: &'static str, s: &str) -> std::result::Result<Quantity, RuleError> {
    s.trim()
        .replace(',', ".")
        .parse::<Quantity>()
        .map_err(|_| RuleError::on(field, "invalid_quantity"))
}

// ---------------------------------------------------------------------------
// Stock overview (M6.12)

/// Stock of one product over the chosen warehouses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockRow {
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub unit: String,
    pub category: Option<String>,
    pub on_hand_milli: i64,
    pub reserved_milli: i64,
    pub available_milli: i64,
    /// Value at average cost, in minor units.
    pub value_minor: i64,
    pub min_stock_milli: Option<i64>,
    pub state: StockState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockQuery {
    pub warehouse_id: Option<String>,
    pub text: String,
    pub state: Option<StockState>,
    pub limit: u32,
    /// Rows of the matching ones to skip (paging).
    #[serde(default)]
    pub offset: u32,
    /// One category's products.
    #[serde(default)]
    pub category_id: Option<String>,
    /// Only what is held for orders.
    #[serde(default)]
    pub reserved: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockPage {
    pub rows: Vec<StockRow>,
    pub total: u32,
    /// Value of all rows matching the text and warehouse (any state).
    pub value_minor: i64,
    /// Counts: all, ok, low, out.
    pub counts: [u32; 4],
}

pub fn stock_overview(store: &Store, q: &StockQuery) -> Result<StockPage> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut args: Vec<String> = vec![org.to_string()];
        let mut wh_filter = String::new();
        if let Some(w) = q.warehouse_id.as_ref().filter(|w| !w.is_empty()) {
            wh_filter = " AND b.warehouse_id = ?2".into();
            args.push(w.clone());
        }
        let mut text_filter = String::new();
        if let Some(fts) = catalog::fts_query(&q.text) {
            text_filter = format!(
                " AND p.rowid IN (SELECT rowid FROM products_fts WHERE products_fts MATCH ?{})",
                args.len() + 1
            );
            args.push(fts);
        }
        if let Some(c) = q.category_id.as_ref().filter(|c| !c.is_empty()) {
            text_filter.push_str(&format!(" AND p.category_id = ?{}", args.len() + 1));
            args.push(c.clone());
        }
        let sql = format!(
            "SELECT p.id, p.sku, p.name, (SELECT code FROM product_units u WHERE u.product_id = p.id AND u.position = 0),
                    c.name, p.min_stock_milli,
                    COALESCE((SELECT SUM(b.on_hand_milli) FROM stock_balances b WHERE b.product_id = p.id{wh}), 0),
                    COALESCE((SELECT SUM(b.on_hand_milli * b.avg_cost_minor) FROM stock_balances b WHERE b.product_id = p.id{wh}), 0),
                    COALESCE((SELECT SUM(x.qty_milli) FROM stock_reservations x JOIN stock_balances b ON b.product_id = x.product_id AND b.warehouse_id = x.warehouse_id
                              WHERE x.product_id = p.id AND x.status = 'active'{wh}), 0)
             FROM products p LEFT JOIN categories c ON c.id = p.category_id
             WHERE p.org_id = ?1 AND p.deleted_at IS NULL AND p.kind = 'goods'{text_filter}
             ORDER BY p.name COLLATE NOCASE",
            wh = wh_filter
        );
        let mut s = r.conn().prepare(&sql)?;
        let all = s.query_map(params_from_iter(args.iter()), |row| {
            let on_hand: i64 = row.get(6)?;
            let value_milli: i64 = row.get(7)?;
            let reserved: i64 = row.get(8)?;
            let min: Option<i64> = row.get(5)?;
            let state = if on_hand <= 0 {
                StockState::Out
            } else if min.is_some_and(|m| on_hand < m) {
                StockState::Low
            } else {
                StockState::Ok
            };
            Ok(StockRow {
                product_id: row.get(0)?,
                sku: row.get(1)?,
                name: row.get(2)?,
                unit: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                category: row.get(4)?,
                on_hand_milli: on_hand,
                reserved_milli: reserved,
                available_milli: on_hand - reserved,
                // Thousandths × minor units: back to minor, half away from zero.
                value_minor: (value_milli + value_milli.signum() * 500) / 1000,
                min_stock_milli: min,
                state,
            })
        })?;
        let mut page = StockPage::default();
        for row in all {
            let row = row?;
            if q.reserved && row.reserved_milli <= 0 {
                continue;
            }
            page.counts[0] += 1;
            page.counts[match row.state {
                StockState::Ok => 1,
                StockState::Low => 2,
                StockState::Out => 3,
            }] += 1;
            page.value_minor += row.value_minor.max(0);
            if q.state.is_none_or(|s| s == row.state) {
                page.total += 1;
                if page.total > q.offset && page.rows.len() < q.limit.clamp(1, 100_000) as usize {
                    page.rows.push(row);
                }
            }
        }
        Ok(page)
    })
}

// ---------------------------------------------------------------------------
// Movement journal

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovementView {
    pub seq: i64,
    pub created_at: i64,
    pub kind: String,
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub unit: String,
    pub warehouse: String,
    /// Signed, in thousandths of the base unit.
    pub qty_milli: i64,
    pub unit_cost_minor: Option<i64>,
    pub doc_number: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovementQuery {
    pub warehouse_id: Option<String>,
    pub product_id: Option<String>,
    /// Movement kind codes; empty: all.
    pub kinds: Vec<String>,
    pub limit: u32,
}

pub fn movements(store: &Store, q: &MovementQuery) -> Result<Vec<MovementView>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut sql = String::from(
            "SELECT m.seq, m.created_at, m.kind, m.product_id, p.sku, p.name,
                    (SELECT code FROM product_units u WHERE u.product_id = p.id AND u.position = 0),
                    w.name, m.qty_milli, m.unit_cost_minor, m.doc_number, m.reason
             FROM stock_movements m JOIN products p ON p.id = m.product_id JOIN warehouses w ON w.id = m.warehouse_id
             WHERE m.org_id = ?1",
        );
        let mut args = vec![org.to_string()];
        if let Some(w) = q.warehouse_id.as_ref().filter(|w| !w.is_empty()) {
            args.push(w.clone());
            sql.push_str(&format!(" AND m.warehouse_id = ?{}", args.len()));
        }
        if let Some(p) = q.product_id.as_ref().filter(|p| !p.is_empty()) {
            args.push(p.clone());
            sql.push_str(&format!(" AND m.product_id = ?{}", args.len()));
        }
        if !q.kinds.is_empty() {
            let marks: Vec<String> = q
                .kinds
                .iter()
                .map(|k| {
                    args.push(k.clone());
                    format!("?{}", args.len())
                })
                .collect();
            sql.push_str(&format!(" AND m.kind IN ({})", marks.join(", ")));
        }
        sql.push_str(&format!(" ORDER BY m.seq DESC LIMIT {}", q.limit.clamp(1, 5_000)));
        let mut s = r.conn().prepare(&sql)?;
        let rows = s.query_map(params_from_iter(args.iter()), |row| {
            Ok(MovementView {
                seq: row.get(0)?,
                created_at: row.get(1)?,
                kind: row.get(2)?,
                product_id: row.get(3)?,
                sku: row.get(4)?,
                name: row.get(5)?,
                unit: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                warehouse: row.get(7)?,
                qty_milli: row.get(8)?,
                unit_cost_minor: row.get(9)?,
                doc_number: row.get(10)?,
                reason: row.get(11)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

// ---------------------------------------------------------------------------
// Reservations (M6.4)

fn reserved(conn: &Connection, product: ProductId, wh: WarehouseId) -> Result<Quantity> {
    let m: i64 = conn.query_row(
        "SELECT COALESCE(SUM(qty_milli), 0) FROM stock_reservations WHERE product_id = ?1 AND warehouse_id = ?2 AND status = 'active'",
        params![product.to_string(), wh.to_string()],
        |r| r.get(0),
    )?;
    Ok(from_milli(m))
}

/// What can still be sold or sent from a warehouse.
pub fn available(store: &Store, product: ProductId, wh: WarehouseId) -> Result<Quantity> {
    store.read(|r| {
        let (_, currency) = the_org(r.conn())?;
        let b = stock::balance(r.conn(), product, wh, currency)?;
        Ok(rules::available(
            b.on_hand,
            reserved(r.conn(), product, wh)?,
        ))
    })
}

/// Holds a quantity (base units) for an order; refused beyond what is available.
pub fn reserve(
    store: &mut Store,
    meta: OpMeta,
    product: ProductId,
    wh: WarehouseId,
    quantity: &str,
    source: &str,
) -> Result<String> {
    let q = qty("quantity", quantity)?;
    if q.is_negative() || q.is_zero() {
        return Err(RuleError::on("quantity", "stock.quantity_positive").into());
    }
    store.write(meta, |w| {
        let (_, _, target) = place(w.tx(), wh)?;
        w.require(Permission::SalesSell, &target)?;
        let (_, currency) = the_org(w.tx())?;
        let b = stock::balance(w.tx(), product, wh, currency)?;
        let free = rules::available(b.on_hand, reserved(w.tx(), product, wh)?);
        if free.decimal() < q.decimal() {
            return Err(RuleError::on("quantity", "stock.insufficient").into());
        }
        let id = nb_core::OperationId::new().to_string();
        w.tx().execute(
            "INSERT INTO stock_reservations (id, org_id, product_id, warehouse_id, qty_milli, source, status, created_at, created_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8)",
            params![id, w.org()?.to_string(), product.to_string(), wh.to_string(), to_milli(q)?, source, w.now(), w.actor_text()],
        )?;
        w.event("stock.reserved", "product", &product.to_string(), &(id.as_str(), q.to_string(), source))?;
        w.audit("reserve", "product", &product.to_string(), &(id.as_str(), q.to_string(), source))?;
        Ok(id)
    })
}

/// Ends a reservation: `consumed` when the goods left, else `released`.
pub fn close_reservation(store: &mut Store, meta: OpMeta, id: &str, consumed: bool) -> Result<()> {
    store.write(meta, |w| {
        let n = w.tx().execute(
            "UPDATE stock_reservations SET status = ?2, closed_at = ?3 WHERE id = ?1 AND status = 'active'",
            params![id, if consumed { "consumed" } else { "released" }, w.now()],
        )?;
        if n == 0 {
            return Err(Error::NotFound("reservation"));
        }
        w.event("stock.reservation_closed", "reservation", id, &consumed)?;
        w.audit("reservation_close", "reservation", id, &consumed)?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Transfers (M6.5)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferLineInput {
    pub product_id: String,
    /// In `unit`, as typed.
    pub qty: String,
    pub unit: String,
    /// A chosen lot, or the serial numbers sent.
    #[serde(default)]
    pub tracked: crate::usecases::tracked::Tracked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferInput {
    pub from_warehouse: String,
    pub to_warehouse: String,
    pub lines: Vec<TransferLineInput>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferLine {
    pub position: i64,
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub unit: String,
    pub sent_milli: i64,
    pub received_milli: Option<i64>,
    pub unit_cost_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferRow {
    pub id: String,
    pub number: String,
    pub from_warehouse: String,
    pub from_name: String,
    pub to_warehouse: String,
    pub to_name: String,
    /// `sent` (in transit) or `received`.
    pub status: String,
    pub note: String,
    pub sent_at: i64,
    pub received_at: Option<i64>,
    pub lines: Vec<TransferLine>,
}

fn transfer_of(conn: &Connection, id: &str) -> Result<TransferRow> {
    let mut t = conn
        .query_row(
            "SELECT t.id, t.number, t.from_warehouse, a.name, t.to_warehouse, b.name, t.status, t.note, t.sent_at, t.received_at
             FROM transfers t JOIN warehouses a ON a.id = t.from_warehouse JOIN warehouses b ON b.id = t.to_warehouse
             WHERE t.id = ?1",
            [id],
            |r| {
                Ok(TransferRow {
                    id: r.get(0)?,
                    number: r.get(1)?,
                    from_warehouse: r.get(2)?,
                    from_name: r.get(3)?,
                    to_warehouse: r.get(4)?,
                    to_name: r.get(5)?,
                    status: r.get(6)?,
                    note: r.get(7)?,
                    sent_at: r.get(8)?,
                    received_at: r.get(9)?,
                    lines: vec![],
                })
            },
        )
        .optional()?
        .ok_or(Error::NotFound("transfer"))?;
    let mut s = conn.prepare(
        "SELECT l.position, l.product_id, p.sku, p.name,
                (SELECT code FROM product_units u WHERE u.product_id = p.id AND u.position = 0),
                l.qty_sent_milli, l.qty_received_milli, l.unit_cost_minor
         FROM transfer_lines l JOIN products p ON p.id = l.product_id WHERE l.transfer_id = ?1 ORDER BY l.position",
    )?;
    t.lines = s
        .query_map([id], |r| {
            Ok(TransferLine {
                position: r.get(0)?,
                product_id: r.get(1)?,
                sku: r.get(2)?,
                name: r.get(3)?,
                unit: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                sent_milli: r.get(5)?,
                received_milli: r.get(6)?,
                unit_cost_minor: r.get(7)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(t)
}

/// Sends goods from one warehouse to another: they leave the source now
/// (in transit) at its average cost.
pub fn send_transfer(store: &mut Store, meta: OpMeta, input: TransferInput) -> Result<TransferRow> {
    let from: WarehouseId = parse_id(&input.from_warehouse)?;
    let to: WarehouseId = parse_id(&input.to_warehouse)?;
    if from == to {
        return Err(RuleError::on("to_warehouse", "transfer.same_warehouse").into());
    }
    if input.lines.is_empty() {
        return Err(RuleError::on("lines", "transfer.empty").into());
    }
    let mut parsed = Vec::new();
    for l in &input.lines {
        parsed.push((
            parse_id::<ProductId>(&l.product_id)?,
            qty("qty", &l.qty)?,
            l.unit.clone(),
            l.tracked.clone(),
        ));
    }
    store.write(meta, |w| {
        let (src, branch, target) = place(w.tx(), from)?;
        let (dst, _, _) = place(w.tx(), to)?;
        w.require(Permission::StockTransfer, &target)?;
        let may_negative = w.allowed(Permission::StockNegative, &target)?;
        let (org, currency) = the_org(w.tx())?;
        let number = w.next_number(DocKind::Transfer, &branch.id, &branch.code)?;
        let id = nb_core::OperationId::new().to_string();
        w.tx().execute(
            "INSERT INTO transfers (id, org_id, number, from_warehouse, to_warehouse, status, note, sent_at, sent_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, 'sent', ?6, ?7, ?8, ?9)",
            params![id, org.to_string(), number, src.id, dst.id, input.note.trim(), w.now(), w.actor_text(), w.device().id.to_string()],
        )?;
        let mut seen = std::collections::BTreeSet::new();
        for (i, (product, q, unit, tracked)) in parsed.iter().enumerate() {
            if !seen.insert(*product) {
                return Err(RuleError::on("lines", "transfer.duplicate_line").into());
            }
            let p = catalog::get(w.tx(), *product)?;
            if catalog::kind(&p) == nb_domain::product::ProductKind::Service {
                return Err(RuleError::on("lines", "product.service_stock").into());
            }
            let base = p.unit_set()?.to_base(*q, if unit.is_empty() { &p.units[0].code } else { unit })?;
            let before = stock::balance(w.tx(), *product, from, currency)?;
            let after = before.apply(MovementKind::TransferOut, base, None, src.policy(), may_negative)?;
            let (unit_cost, _) = stock::issue_cost(w.tx(), *product, from, base, before.avg_cost)?;
            let movement = stock::append(w, *product, from, MovementKind::TransferOut, base, Some(unit_cost), Some(&number), None, after)?;
            crate::usecases::tracked::record(w, movement, *product, from, MovementKind::TransferOut, base, tracked, false)?;
            w.tx().execute(
                "INSERT INTO transfer_lines (transfer_id, position, product_id, qty_sent_milli, unit_cost_minor, out_movement_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, i64::try_from(i).unwrap_or(0), product.to_string(), to_milli(base)?, unit_cost.minor(), movement.to_string()],
            )?;
        }
        let row = transfer_of(w.tx(), &id)?;
        w.event("transfer.sent", "transfer", &id, &row)?;
        w.audit("send", "transfer", &id, &row)?;
        Ok(row)
    })
}

/// Receives a transfer: what arrived enters the destination at the cost it
/// left with; what is missing is booked as a loss (658 / 30, provisional).
/// `received`: (line position, quantity in base units); lines not given
/// arrived whole.
pub fn receive_transfer(
    store: &mut Store,
    meta: OpMeta,
    id: &str,
    received: Vec<(i64, String)>,
) -> Result<TransferRow> {
    let mut given = std::collections::BTreeMap::new();
    for (pos, q) in &received {
        given.insert(*pos, qty("received", q)?);
    }
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    store.write(meta, |w| {
        let t = transfer_of(w.tx(), id)?;
        if t.status != "sent" {
            return Err(RuleError::on("transfer", "transfer.already_received").into());
        }
        let to: WarehouseId = parse_id(&t.to_warehouse)?;
        let (dst, branch, target) = place(w.tx(), to)?;
        w.require(Permission::StockTransfer, &target)?;
        let (_, currency) = the_org(w.tx())?;
        let mut loss = Money::zero(currency);
        for l in &t.lines {
            let sent = from_milli(l.sent_milli);
            let got = given.get(&l.position).copied().unwrap_or(sent);
            let short = rules::transfer_shortfall(sent, got)?;
            let product: ProductId = parse_id(&l.product_id)?;
            let cost = Money::from_minor(l.unit_cost_minor, currency);
            if !got.is_zero() {
                let before = stock::balance(w.tx(), product, to, currency)?;
                let after = before.apply(MovementKind::TransferIn, got, Some(cost), dst.policy(), true)?;
                let movement = stock::append(w, product, to, MovementKind::TransferIn, got, Some(cost), Some(&t.number), None, after)?;
                let sent_by: Option<String> = w.tx().query_row(
                    "SELECT out_movement_id FROM transfer_lines WHERE transfer_id = ?1 AND position = ?2",
                    params![t.id, l.position],
                    |r| r.get(0),
                )?;
                if let Some(out) = sent_by {
                    crate::usecases::tracked::mirror(w, &out, movement, product, to, got)?;
                }
            }
            loss = loss.checked_add(short.times_price(cost)?)?;
            w.tx().execute(
                "UPDATE transfer_lines SET qty_received_milli = ?3 WHERE transfer_id = ?1 AND position = ?2",
                params![id, l.position, to_milli(got)?],
            )?;
        }
        w.tx().execute(
            "UPDATE transfers SET status = 'received', received_at = ?2, received_by = ?3, version = version + 1 WHERE id = ?1",
            params![id, w.now(), w.actor_text()],
        )?;
        let memo = format!("{} — {}", t.number, "transfer shortfall");
        crate::usecases::accounting::post_operations(w, &branch, offset, &[Operation::StockLoss { value: loss }], &memo, "stock.transfer")?;
        let row = transfer_of(w.tx(), id)?;
        w.event("transfer.received", "transfer", id, &row)?;
        w.audit("receive", "transfer", id, &serde_json::json!({ "transfer": row, "loss": loss.minor() }))?;
        Ok(row)
    })
}

pub fn list_transfers(store: &Store, limit: u32) -> Result<Vec<TransferRow>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare("SELECT id FROM transfers WHERE org_id = ?1 ORDER BY sent_at DESC, number DESC LIMIT ?2")?;
        let ids: Vec<String> = s.query_map(params![org.to_string(), limit], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
        ids.iter().map(|id| transfer_of(r.conn(), id)).collect()
    })
}

// ---------------------------------------------------------------------------
// Stock counts (M6.7)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountLine {
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub unit: String,
    /// The book now (or at closing, once closed).
    pub book_milli: i64,
    pub counted_milli: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountRow {
    pub id: String,
    pub number: String,
    pub warehouse_id: String,
    pub warehouse: String,
    pub category_id: Option<String>,
    pub category: Option<String>,
    /// `open`, `closed` or `cancelled`.
    pub status: String,
    pub opened_at: i64,
    pub closed_at: Option<i64>,
    pub counted: u32,
    pub lines: Vec<CountLine>,
}

fn count_of(conn: &Connection, id: &str, with_lines: bool) -> Result<CountRow> {
    let mut c = conn
        .query_row(
            "SELECT s.id, s.number, s.warehouse_id, w.name, s.category_id, c.name, s.status, s.opened_at, s.closed_at,
                    (SELECT COUNT(*) FROM stock_count_lines l WHERE l.count_id = s.id)
             FROM stock_counts s JOIN warehouses w ON w.id = s.warehouse_id LEFT JOIN categories c ON c.id = s.category_id
             WHERE s.id = ?1",
            [id],
            |r| {
                Ok(CountRow {
                    id: r.get(0)?,
                    number: r.get(1)?,
                    warehouse_id: r.get(2)?,
                    warehouse: r.get(3)?,
                    category_id: r.get(4)?,
                    category: r.get(5)?,
                    status: r.get(6)?,
                    opened_at: r.get(7)?,
                    closed_at: r.get(8)?,
                    counted: r.get(9)?,
                    lines: vec![],
                })
            },
        )
        .optional()?
        .ok_or(Error::NotFound("count"))?;
    if with_lines {
        // Every product in scope; counted ones carry their count.
        let mut s = conn.prepare(
            "SELECT p.id, p.sku, p.name, (SELECT code FROM product_units u WHERE u.product_id = p.id AND u.position = 0),
                    COALESCE(l.book_milli, (SELECT on_hand_milli FROM stock_balances b WHERE b.product_id = p.id AND b.warehouse_id = ?2), 0),
                    l.counted_milli
             FROM products p LEFT JOIN stock_count_lines l ON l.product_id = p.id AND l.count_id = ?1
             WHERE p.org_id = (SELECT org_id FROM stock_counts WHERE id = ?1) AND p.kind = 'goods'
               AND (p.deleted_at IS NULL OR l.counted_milli IS NOT NULL)
               AND (?3 IS NULL OR p.category_id = ?3 OR l.counted_milli IS NOT NULL)
             ORDER BY l.counted_milli IS NULL, p.name COLLATE NOCASE",
        )?;
        c.lines = s
            .query_map(params![id, c.warehouse_id, c.category_id], |r| {
                Ok(CountLine {
                    product_id: r.get(0)?,
                    sku: r.get(1)?,
                    name: r.get(2)?,
                    unit: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    book_milli: r.get(4)?,
                    counted_milli: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
    }
    Ok(c)
}

/// Opens a count of a warehouse (one open count per warehouse), for all
/// products or one category.
pub fn open_count(
    store: &mut Store,
    meta: OpMeta,
    warehouse: WarehouseId,
    category: Option<CategoryId>,
) -> Result<CountRow> {
    store.write(meta, |w| {
        let (wh, branch, target) = place(w.tx(), warehouse)?;
        w.require(Permission::StockCount, &target)?;
        let open: bool = w.tx().query_row(
            "SELECT EXISTS (SELECT 1 FROM stock_counts WHERE warehouse_id = ?1 AND status = 'open')",
            [&wh.id],
            |r| r.get(0),
        )?;
        if open {
            return Err(RuleError::on("warehouse", "count.already_open").into());
        }
        let number = w.next_number(DocKind::StockCount, &branch.id, &branch.code)?;
        let id = nb_core::OperationId::new().to_string();
        w.tx().execute(
            "INSERT INTO stock_counts (id, org_id, number, warehouse_id, category_id, status, opened_at, opened_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, 'open', ?6, ?7, ?8)",
            params![id, w.org()?.to_string(), number, wh.id, category.map(|c| c.to_string()), w.now(), w.actor_text(), w.device().id.to_string()],
        )?;
        let row = count_of(w.tx(), &id, false)?;
        w.event("count.opened", "count", &id, &row)?;
        w.audit("open", "count", &id, &row)?;
        Ok(row)
    })
}

/// Records (or corrects) the counted quantity of a product, in `unit`.
pub fn record_count(
    store: &mut Store,
    meta: OpMeta,
    count: &str,
    product: ProductId,
    quantity: &str,
    unit: &str,
) -> Result<CountLine> {
    let q = qty("counted", quantity)?;
    store.write(meta, |w| {
        let c = count_of(w.tx(), count, false)?;
        if c.status != "open" {
            return Err(RuleError::on("count", "count.closed").into());
        }
        let (_, _, target) = place(w.tx(), parse_id(&c.warehouse_id)?)?;
        w.require(Permission::StockCount, &target)?;
        let p = catalog::get(w.tx(), product)?;
        if p.tracking == "serial" {
            return Err(RuleError::on("product", "count.serial_product").into());
        }
        let base = p.unit_set()?.to_base(q, if unit.is_empty() { &p.units[0].code } else { unit })?;
        if base.is_negative() {
            return Err(RuleError::on("counted", "stock.quantity_positive").into());
        }
        w.tx().execute(
            "INSERT INTO stock_count_lines (count_id, product_id, counted_milli, counted_at, counted_by) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (count_id, product_id) DO UPDATE SET counted_milli = ?3, counted_at = ?4, counted_by = ?5",
            params![count, product.to_string(), to_milli(base)?, w.now(), w.actor_text()],
        )?;
        let line = CountLine {
            product_id: p.id.clone(),
            sku: p.sku.clone(),
            name: p.name.clone(),
            unit: p.units.first().map(|u| u.code.clone()).unwrap_or_default(),
            book_milli: 0,
            counted_milli: Some(to_milli(base)?),
        };
        w.event("count.recorded", "count", count, &(p.id.as_str(), base.to_string()))?;
        w.audit("count", "count", count, &(p.id.as_str(), base.to_string()))?;
        Ok(line)
    })
}

/// What closing would book: (gains value, losses value, lines that differ).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountResult {
    pub gains_minor: i64,
    pub losses_minor: i64,
    pub adjusted: u32,
}

/// Closes a count: each counted product's book is brought to its count
/// (count gain / loss at average cost), in one entry (30 / 758 for gains,
/// 658 / 30 for losses, provisional). Needs `stock.adjust` (or a
/// supervisor's approval).
pub fn close_count(
    store: &mut Store,
    meta: OpMeta,
    count: &str,
    approval: Option<String>,
) -> Result<CountResult> {
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    store.write(meta, |w| {
        let c = count_of(w.tx(), count, false)?;
        if c.status != "open" {
            return Err(RuleError::on("count", "count.closed").into());
        }
        let wh: WarehouseId = parse_id(&c.warehouse_id)?;
        let (warehouse, branch, target) = place(w.tx(), wh)?;
        let approval = approval.as_deref().map(parse_id).transpose()?;
        w.require_approved(Permission::StockAdjust, &target, approval)?;
        let (_, currency) = the_org(w.tx())?;
        let lines: Vec<(String, i64)> = {
            let mut s = w.tx().prepare("SELECT product_id, counted_milli FROM stock_count_lines WHERE count_id = ?1")?;
            let rows = s.query_map([count], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        let mut out = CountResult::default();
        let (mut gains, mut losses) = (Money::zero(currency), Money::zero(currency));
        for (pid, counted) in lines {
            let product: ProductId = parse_id(&pid)?;
            let before = stock::balance(w.tx(), product, wh, currency)?;
            w.tx().execute(
                "UPDATE stock_count_lines SET book_milli = ?3 WHERE count_id = ?1 AND product_id = ?2",
                params![count, pid, to_milli(before.on_hand)?],
            )?;
            let Some((kind, q)) = rules::count_difference(before.on_hand, from_milli(counted))? else {
                continue;
            };
            let after = before.apply(kind, q, None, warehouse.policy(), true)?;
            // A loss is costed as any issue; a gain at the average.
            let (unit_cost, value) = if kind == MovementKind::CountGain {
                (before.avg_cost, q.times_price(before.avg_cost)?)
            } else {
                stock::issue_cost(w.tx(), product, wh, q, before.avg_cost)?
            };
            let movement = stock::append(w, product, wh, kind, q, Some(unit_cost), Some(&c.number), None, after)?;
            // Found units go to a lot named after the count; missing ones
            // leave the expired lots first.
            if crate::repo::tracking::tracking(w.tx(), product)? == crate::repo::tracking::Tracking::Lot {
                let tracked = crate::usecases::tracked::Tracked { lot: (kind == MovementKind::CountGain).then(|| c.number.clone()), ..Default::default() };
                crate::usecases::tracked::record(w, movement, product, wh, kind, q, &tracked, true)?;
            }
            if kind == MovementKind::CountGain {
                gains = gains.checked_add(value)?;
            } else {
                losses = losses.checked_add(value)?;
            }
            out.adjusted += 1;
        }
        w.tx().execute(
            "UPDATE stock_counts SET status = 'closed', closed_at = ?2, closed_by = ?3 WHERE id = ?1",
            params![count, w.now(), w.actor_text()],
        )?;
        crate::usecases::accounting::post_operations(w, &branch, offset, &[Operation::StockGain { value: gains }, Operation::StockLoss { value: losses }], &c.number, "stock.count")?;
        out.gains_minor = gains.minor();
        out.losses_minor = losses.minor();
        w.event("count.closed", "count", count, &out)?;
        w.audit("close", "count", count, &out)?;
        Ok(out)
    })
}

pub fn cancel_count(store: &mut Store, meta: OpMeta, count: &str) -> Result<()> {
    store.write(meta, |w| {
        let c = count_of(w.tx(), count, false)?;
        if c.status != "open" {
            return Err(RuleError::on("count", "count.closed").into());
        }
        let (_, _, target) = place(w.tx(), parse_id(&c.warehouse_id)?)?;
        w.require(Permission::StockCount, &target)?;
        w.tx().execute("UPDATE stock_counts SET status = 'cancelled', closed_at = ?2, closed_by = ?3 WHERE id = ?1", params![count, w.now(), w.actor_text()])?;
        w.event("count.cancelled", "count", count, &())?;
        w.audit("cancel", "count", count, &())?;
        Ok(())
    })
}

pub fn count_detail(store: &Store, id: &str) -> Result<CountRow> {
    store.read(|r| count_of(r.conn(), id, true))
}

pub fn list_counts(store: &Store, limit: u32) -> Result<Vec<CountRow>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare("SELECT id FROM stock_counts WHERE org_id = ?1 ORDER BY opened_at DESC, number DESC LIMIT ?2")?;
        let ids: Vec<String> = s.query_map(params![org.to_string(), limit], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
        ids.iter().map(|id| count_of(r.conn(), id, false)).collect()
    })
}

/// Stock alerts for the dashboard and the menu (M6.11): low, out.
pub fn alerts(store: &Store) -> Result<(u32, u32)> {
    let page = stock_overview(
        store,
        &StockQuery {
            limit: 1,
            ..Default::default()
        },
    )?;
    Ok((page.counts[2], page.counts[3]))
}

/// A lot in stock that expires soon, or has expired (M6.11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiringLot {
    pub product_id: String,
    pub product: String,
    pub lot: String,
    pub expiry: String,
    pub warehouse: String,
    pub qty_milli: i64,
    /// Negative once expired.
    pub days_left: i64,
}

/// Lots in stock expiring within `days` (expired ones included), soonest
/// first.
pub fn expiring_lots(store: &Store, days: i64) -> Result<Vec<ExpiringLot>> {
    let now = store.now_ms();
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let today = iso_date(now, r.utc_offset_min);
        let mut s = r.conn().prepare(
            "SELECT p.id, p.name, l.code, l.expiry, w.name, b.on_hand_milli,
                    CAST(julianday(l.expiry) - julianday(?2) AS INTEGER)
             FROM lots l
             JOIN lot_balances b ON b.lot_id = l.id
             JOIN products p ON p.id = l.product_id
             JOIN warehouses w ON w.id = b.warehouse_id
             WHERE l.org_id = ?1 AND b.on_hand_milli > 0 AND l.expiry IS NOT NULL
               AND julianday(l.expiry) - julianday(?2) <= ?3
             ORDER BY l.expiry, p.name",
        )?;
        let rows = s.query_map(params![org.to_string(), today, days], |r| {
            Ok(ExpiringLot {
                product_id: r.get(0)?,
                product: r.get(1)?,
                lot: r.get(2)?,
                expiry: r.get(3)?,
                warehouse: r.get(4)?,
                qty_milli: r.get(5)?,
                days_left: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

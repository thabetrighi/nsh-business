//! Stock movements (append-only) and their balance projection.

use nb_core::{Currency, Money, Quantity};
use nb_domain::stock::{Balance, MovementKind};
use nb_domain::{MovementId, ProductId, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{from_milli, to_milli};
use crate::uow::Writer;
use crate::{Error, Result};

/// A movement as listed in an item's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovementRow {
    pub seq: i64,
    pub id: String,
    pub warehouse_id: String,
    pub product_id: String,
    pub kind: String,
    /// Signed, in thousandths of the base unit.
    pub qty_milli: i64,
    pub unit_cost_minor: Option<i64>,
    pub doc_number: Option<String>,
    pub reason: Option<String>,
    pub created_at: i64,
}

/// The balance of a product in a warehouse (zero when it never moved).
pub fn balance(
    conn: &Connection,
    product: ProductId,
    warehouse: WarehouseId,
    currency: Currency,
) -> Result<Balance> {
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT on_hand_milli, avg_cost_minor FROM stock_balances WHERE product_id = ?1 AND warehouse_id = ?2",
            params![product.to_string(), warehouse.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        Some((q, c)) => Balance {
            on_hand: from_milli(q),
            avg_cost: Money::from_minor(c, currency),
        },
        None => Balance::empty(Money::zero(currency)),
    })
}

/// Appends a movement and stores the new balance, in the operation's transaction.
#[allow(clippy::too_many_arguments)]
pub fn append(
    w: &Writer<'_>,
    product: ProductId,
    warehouse: WarehouseId,
    kind: MovementKind,
    qty: Quantity,
    unit_cost: Option<Money>,
    doc_number: Option<&str>,
    reason: Option<&str>,
    new_balance: Balance,
) -> Result<MovementId> {
    let id = MovementId::new();
    let signed = to_milli(qty)? * kind.sign();
    w.tx().execute(
        "INSERT INTO stock_movements (id, org_id, warehouse_id, product_id, kind, qty_milli, unit_cost_minor, doc_number, reason,
                                      operation_id, created_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            id.to_string(),
            w.org()?.to_string(),
            warehouse.to_string(),
            product.to_string(),
            kind.code(),
            signed,
            unit_cost.map(|c| c.minor()),
            doc_number,
            reason,
            w.meta().id.to_string(),
            w.now(),
            w.actor_text(),
            w.device().id.to_string()
        ],
    )?;
    let seq = w.tx().last_insert_rowid();
    // FIFO layers follow every movement (whatever the method: switching
    // to FIFO then starts from the right layers).
    let before: i64 = w
        .tx()
        .query_row(
            "SELECT on_hand_milli FROM stock_balances WHERE product_id = ?1 AND warehouse_id = ?2",
            params![product.to_string(), warehouse.to_string()],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let mut l = layers(w.tx(), product, warehouse)?;
    if signed > 0 {
        let cost = unit_cost.map_or(new_balance.avg_cost.minor(), |c| c.minor());
        nb_domain::valuation::receive(&mut l, before, seq, signed, cost);
    } else {
        nb_domain::valuation::issue(&mut l, -signed);
    }
    store_layers(w.tx(), product, warehouse, &l)?;
    w.tx().execute(
        "INSERT INTO stock_balances (product_id, warehouse_id, on_hand_milli, avg_cost_minor, last_seq) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (product_id, warehouse_id) DO UPDATE SET on_hand_milli = ?3, avg_cost_minor = ?4, last_seq = ?5",
        params![
            product.to_string(),
            warehouse.to_string(),
            to_milli(new_balance.on_hand)?,
            new_balance.avg_cost.minor(),
            seq
        ],
    )?;
    Ok(id)
}

/// What is left of each receipt of a product in a warehouse, oldest first.
pub fn layers(
    conn: &Connection,
    product: ProductId,
    warehouse: WarehouseId,
) -> Result<Vec<nb_domain::valuation::Layer>> {
    let mut s = conn.prepare_cached(
        "SELECT seq, qty_milli, unit_cost_minor FROM stock_layers
         WHERE product_id = ?1 AND warehouse_id = ?2 ORDER BY seq",
    )?;
    let rows = s.query_map(params![product.to_string(), warehouse.to_string()], |r| {
        Ok(nb_domain::valuation::Layer {
            seq: r.get(0)?,
            qty_milli: r.get(1)?,
            unit_cost: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn store_layers(
    conn: &Connection,
    product: ProductId,
    warehouse: WarehouseId,
    l: &[nb_domain::valuation::Layer],
) -> Result<()> {
    store_layers_text(conn, &product.to_string(), &warehouse.to_string(), l)
}

fn store_layers_text(
    conn: &Connection,
    product: &str,
    warehouse: &str,
    l: &[nb_domain::valuation::Layer],
) -> Result<()> {
    conn.execute(
        "DELETE FROM stock_layers WHERE product_id = ?1 AND warehouse_id = ?2",
        params![product, warehouse],
    )?;
    let mut s = conn.prepare_cached(
        "INSERT INTO stock_layers (product_id, warehouse_id, seq, qty_milli, unit_cost_minor)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for x in l {
        s.execute(params![product, warehouse, x.seq, x.qty_milli, x.unit_cost])?;
    }
    Ok(())
}

/// The organization's valuation method.
pub fn method(conn: &Connection) -> Result<nb_domain::valuation::Method> {
    let code: Option<String> = conn
        .query_row("SELECT valuation FROM organizations LIMIT 1", [], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(code
        .and_then(|c| nb_domain::valuation::Method::from_code(&c).ok())
        .unwrap_or_default())
}

/// What taking `qty` of a product out of a warehouse costs, under the
/// organization's method: (unit cost, total). The average as it stands,
/// or the oldest layers' cost; the total is exact (what the books carry).
pub fn issue_cost(
    conn: &Connection,
    product: ProductId,
    warehouse: WarehouseId,
    qty: Quantity,
    average: Money,
) -> Result<(Money, Money)> {
    let m = method(conn)?;
    let currency = average.currency();
    if m == nb_domain::valuation::Method::Average {
        return Ok((average, qty.times_price(average)?));
    }
    let l = layers(conn, product, warehouse)?;
    let (unit, total) = nb_domain::valuation::issue_cost(m, &l, to_milli(qty)?, average.minor());
    Ok((
        Money::from_minor(unit, currency),
        Money::from_minor(total, currency),
    ))
}

/// Recomputes every cost layer from the movements.
fn rebuild_layers(conn: &Connection, currency: Currency) -> Result<()> {
    let mut s = conn.prepare(
        "SELECT seq, product_id, warehouse_id, kind, qty_milli, unit_cost_minor FROM stock_movements
         ORDER BY product_id, warehouse_id, seq",
    )?;
    let mut rows = s.query([])?;
    let mut key: Option<(String, String)> = None;
    let mut l: Vec<nb_domain::valuation::Layer> = Vec::new();
    let mut bal = Balance::empty(Money::zero(currency));
    conn.execute("DELETE FROM stock_layers", [])?;
    while let Some(r) = rows.next()? {
        let (seq, p, wh): (i64, String, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
        let kind: String = r.get(3)?;
        let qty: i64 = r.get(4)?;
        let cost: Option<i64> = r.get(5)?;
        if key.as_ref() != Some(&(p.clone(), wh.clone())) {
            if let Some((kp, kw)) = key.take() {
                store_layers_text(conn, &kp, &kw, &l)?;
            }
            key = Some((p.clone(), wh.clone()));
            l.clear();
            bal = Balance::empty(Money::zero(currency));
        }
        let kind = MovementKind::from_code(&kind)
            .ok_or_else(|| Error::Corrupt(format!("movement kind {kind}")))?;
        let before = to_milli(bal.on_hand)?;
        bal = bal.apply(
            kind,
            from_milli(qty.abs()),
            cost.map(|c| Money::from_minor(c, currency)),
            nb_domain::stock::NegativeStock::Allow,
            true,
        )?;
        if qty > 0 {
            nb_domain::valuation::receive(
                &mut l,
                before,
                seq,
                qty,
                cost.unwrap_or(bal.avg_cost.minor()),
            );
        } else {
            nb_domain::valuation::issue(&mut l, -qty);
        }
    }
    if let Some((kp, kw)) = key {
        store_layers_text(conn, &kp, &kw, &l)?;
    }
    Ok(())
}

/// The movements of a product, newest first.
pub fn history(conn: &Connection, product: ProductId, limit: u32) -> Result<Vec<MovementRow>> {
    let mut s = conn.prepare(
        "SELECT seq, id, warehouse_id, product_id, kind, qty_milli, unit_cost_minor, doc_number, reason, created_at
         FROM stock_movements WHERE product_id = ?1 ORDER BY seq DESC LIMIT ?2",
    )?;
    let rows = s.query_map(params![product.to_string(), limit], |r| {
        Ok(MovementRow {
            seq: r.get(0)?,
            id: r.get(1)?,
            warehouse_id: r.get(2)?,
            product_id: r.get(3)?,
            kind: r.get(4)?,
            qty_milli: r.get(5)?,
            unit_cost_minor: r.get(6)?,
            doc_number: r.get(7)?,
            reason: r.get(8)?,
            created_at: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Recomputes every balance from the movements (M6.2) and returns the
/// pairs whose stored balance differed.
pub fn rebuild_balances(conn: &Connection, currency: Currency) -> Result<Vec<(String, String)>> {
    let mut s = conn.prepare(
        "SELECT product_id, warehouse_id, kind, qty_milli, unit_cost_minor FROM stock_movements ORDER BY product_id, warehouse_id, seq",
    )?;
    let mut rows = s.query([])?;
    let mut computed: Vec<(String, String, Balance)> = Vec::new();
    while let Some(r) = rows.next()? {
        let p: String = r.get(0)?;
        let wh: String = r.get(1)?;
        let kind: String = r.get(2)?;
        let qty: i64 = r.get(3)?;
        let cost: Option<i64> = r.get(4)?;
        let kind = MovementKind::from_code(&kind)
            .ok_or_else(|| Error::Corrupt(format!("movement kind {kind}")))?;
        let current = match computed.last() {
            Some((lp, lw, b)) if *lp == p && *lw == wh => *b,
            _ => {
                computed.push((p.clone(), wh.clone(), Balance::empty(Money::zero(currency))));
                Balance::empty(Money::zero(currency))
            }
        };
        let next = current.apply(
            kind,
            from_milli(qty.abs()),
            cost.map(|c| Money::from_minor(c, currency)),
            nb_domain::stock::NegativeStock::Allow,
            true,
        )?;
        if let Some(last) = computed.last_mut() {
            last.2 = next;
        }
    }
    rebuild_layers(conn, currency)?;
    let mut differing = Vec::new();
    for (p, wh, b) in computed {
        let stored: Option<(i64, i64)> = conn
            .query_row(
                "SELECT on_hand_milli, avg_cost_minor FROM stock_balances WHERE product_id = ?1 AND warehouse_id = ?2",
                params![p, wh],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let expected = (to_milli(b.on_hand)?, b.avg_cost.minor());
        if stored != Some(expected) {
            conn.execute(
                "INSERT INTO stock_balances (product_id, warehouse_id, on_hand_milli, avg_cost_minor, last_seq)
                 VALUES (?1, ?2, ?3, ?4, (SELECT MAX(seq) FROM stock_movements WHERE product_id = ?1 AND warehouse_id = ?2))
                 ON CONFLICT (product_id, warehouse_id) DO UPDATE SET on_hand_milli = ?3, avg_cost_minor = ?4",
                params![p, wh, expected.0, expected.1],
            )?;
            differing.push((p, wh));
        }
    }
    Ok(differing)
}

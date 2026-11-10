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

//! Lots and serial numbers of movements (M6.8, M6.9).

use nb_core::{OrganizationId, Quantity};
use nb_domain::stock::LotStock;
use nb_domain::{MovementId, ProductId, RuleError, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params};

use super::{from_milli, to_milli};
use crate::uow::Writer;
use crate::{Error, Result};

/// How a product's units are followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tracking {
    None,
    Lot,
    Serial,
}

impl Tracking {
    pub fn from_code(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Tracking::None),
            "lot" => Some(Tracking::Lot),
            "serial" => Some(Tracking::Serial),
            _ => None,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Tracking::None => "none",
            Tracking::Lot => "lot",
            Tracking::Serial => "serial",
        }
    }
}

pub fn tracking(conn: &Connection, product: ProductId) -> Result<Tracking> {
    let code: String = conn
        .query_row(
            "SELECT tracking FROM products WHERE id = ?1",
            [product.to_string()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound("product"))?;
    Tracking::from_code(&code).ok_or_else(|| Error::Corrupt(format!("tracking {code}")))
}

/// Lots of a product with stock in a warehouse.
pub fn lot_stock(conn: &Connection, product: ProductId, wh: WarehouseId) -> Result<Vec<LotStock>> {
    let mut s = conn.prepare_cached(
        "SELECT l.id, l.expiry, b.on_hand_milli FROM lots l JOIN lot_balances b ON b.lot_id = l.id
         WHERE l.product_id = ?1 AND b.warehouse_id = ?2 AND b.on_hand_milli > 0",
    )?;
    let rows = s.query_map(params![product.to_string(), wh.to_string()], |r| {
        Ok(LotStock {
            lot: r.get(0)?,
            expiry: r.get(1)?,
            qty: from_milli(r.get(2)?),
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// The lot `code` of a product, made if new. An existing lot keeps its
/// expiry: another one given is an error.
pub fn ensure_lot(
    w: &Writer<'_>,
    org: OrganizationId,
    product: ProductId,
    code: &str,
    expiry: Option<&str>,
) -> Result<String> {
    let found: Option<(String, Option<String>)> = w
        .tx()
        .query_row(
            "SELECT id, expiry FROM lots WHERE product_id = ?1 AND code = ?2",
            params![product.to_string(), code],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match found {
        Some((id, stored)) => {
            if expiry.is_some() && stored.as_deref() != expiry {
                return Err(RuleError::on("expiry", "lot.expiry_mismatch").into());
            }
            Ok(id)
        }
        None => {
            let id = nb_core::OperationId::new().to_string();
            w.tx().execute(
                "INSERT INTO lots (id, org_id, product_id, code, expiry, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, org.to_string(), product.to_string(), code, expiry, w.now()],
            )?;
            Ok(id)
        }
    }
}

/// The lot id of `code`, if it exists.
pub fn lot_by_code(conn: &Connection, product: ProductId, code: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM lots WHERE product_id = ?1 AND code = ?2",
            params![product.to_string(), code],
            |r| r.get(0),
        )
        .optional()?)
}

/// Records that a movement moved `signed` of a lot, and updates its balance.
pub fn move_lot(
    w: &Writer<'_>,
    movement: MovementId,
    lot: &str,
    wh: WarehouseId,
    signed: Quantity,
) -> Result<()> {
    let m = to_milli(signed)?;
    w.tx().execute(
        "INSERT INTO movement_lots (movement_id, lot_id, qty_milli) VALUES (?1, ?2, ?3)",
        params![movement.to_string(), lot, m],
    )?;
    w.tx().execute(
        "INSERT INTO lot_balances (lot_id, warehouse_id, on_hand_milli) VALUES (?1, ?2, ?3)
         ON CONFLICT (lot_id, warehouse_id) DO UPDATE SET on_hand_milli = on_hand_milli + ?3",
        params![lot, wh.to_string(), m],
    )?;
    Ok(())
}

/// Lots a movement moved, in the order recorded: (lot, signed quantity).
pub fn lots_of_movement(conn: &Connection, movement: &str) -> Result<Vec<(String, Quantity)>> {
    let mut s = conn.prepare(
        "SELECT lot_id, qty_milli FROM movement_lots WHERE movement_id = ?1 ORDER BY rowid",
    )?;
    let rows = s.query_map([movement], |r| {
        Ok((r.get::<_, String>(0)?, from_milli(r.get(1)?)))
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// A serial number comes into a warehouse (new, or back from outside).
pub fn serial_in(
    w: &Writer<'_>,
    org: OrganizationId,
    product: ProductId,
    wh: WarehouseId,
    movement: MovementId,
    serial: &str,
) -> Result<()> {
    let found: Option<(String, String)> = w
        .tx()
        .query_row(
            "SELECT id, status FROM serials WHERE product_id = ?1 AND serial = ?2",
            params![product.to_string(), serial],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let id = match found {
        Some((_, status)) if status == "in_stock" => {
            return Err(RuleError::on("serials", "serial.in_stock").into());
        }
        Some((id, _)) => {
            w.tx().execute(
                "UPDATE serials SET status = 'in_stock', warehouse_id = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, wh.to_string(), w.now()],
            )?;
            id
        }
        None => {
            let id = nb_core::OperationId::new().to_string();
            w.tx().execute(
                "INSERT INTO serials (id, org_id, product_id, serial, warehouse_id, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'in_stock', ?6, ?6)",
                params![id, org.to_string(), product.to_string(), serial, wh.to_string(), w.now()],
            )?;
            id
        }
    };
    w.tx().execute(
        "INSERT INTO movement_serials (movement_id, serial_id) VALUES (?1, ?2)",
        params![movement.to_string(), id],
    )?;
    Ok(())
}

/// A serial number leaves a warehouse; it must be there (a number is never
/// sold twice).
pub fn serial_out(
    w: &Writer<'_>,
    product: ProductId,
    wh: WarehouseId,
    movement: MovementId,
    serial: &str,
) -> Result<()> {
    let id: Option<String> = w
        .tx()
        .query_row(
            "SELECT id FROM serials WHERE product_id = ?1 AND serial = ?2 AND status = 'in_stock' AND warehouse_id = ?3",
            params![product.to_string(), serial, wh.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let id = id.ok_or(Error::from(RuleError::on("serials", "serial.not_in_stock")))?;
    w.tx().execute(
        "UPDATE serials SET status = 'out', warehouse_id = NULL, updated_at = ?2 WHERE id = ?1",
        params![id, w.now()],
    )?;
    w.tx().execute(
        "INSERT INTO movement_serials (movement_id, serial_id) VALUES (?1, ?2)",
        params![movement.to_string(), id],
    )?;
    Ok(())
}

/// Serial numbers a movement moved.
pub fn serials_of_movement(conn: &Connection, movement: &str) -> Result<Vec<String>> {
    let mut s = conn.prepare(
        "SELECT s.serial FROM movement_serials m JOIN serials s ON s.id = m.serial_id WHERE m.movement_id = ?1 ORDER BY m.rowid",
    )?;
    let rows = s.query_map([movement], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Rebuilds lot balances from the movements' lots; returns how many differed.
pub fn rebuild_lot_balances(conn: &Connection) -> Result<u32> {
    let mut s = conn.prepare(
        "SELECT ml.lot_id, m.warehouse_id, SUM(ml.qty_milli) FROM movement_lots ml JOIN stock_movements m ON m.id = ml.movement_id
         GROUP BY ml.lot_id, m.warehouse_id",
    )?;
    let sums: Vec<(String, String, i64)> = s
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut differing = 0;
    for (lot, wh, sum) in sums {
        let stored: Option<i64> = conn
            .query_row(
                "SELECT on_hand_milli FROM lot_balances WHERE lot_id = ?1 AND warehouse_id = ?2",
                params![lot, wh],
                |r| r.get(0),
            )
            .optional()?;
        if stored != Some(sum) {
            conn.execute(
                "INSERT INTO lot_balances (lot_id, warehouse_id, on_hand_milli) VALUES (?1, ?2, ?3)
                 ON CONFLICT (lot_id, warehouse_id) DO UPDATE SET on_hand_milli = ?3",
                params![lot, wh, sum],
            )?;
            differing += 1;
        }
    }
    Ok(differing)
}

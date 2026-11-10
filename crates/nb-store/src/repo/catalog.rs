//! Products: rows, units, barcodes, prices, the search index, and the list
//! query the products screen pages through.

use nb_core::{Currency, Decimal, Money, OrganizationId, Rate};
use nb_domain::ProductId;
use nb_domain::product::{ProductKind, ValidProduct};
use nb_domain::units::{ProductUnit, UnitSet};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::{from_milli, parse_id, to_milli};
use crate::uow::Writer;
use crate::{Error, Result};

/// A category of products.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
}

/// One unit of a product, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitRow {
    pub code: String,
    pub factor: String,
    pub precision: u32,
}

/// A product as screens see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductRow {
    pub id: String,
    pub sku: String,
    pub name: String,
    pub kind: String,
    pub category_id: Option<String>,
    pub category: Option<String>,
    pub tax_bps: i64,
    pub price_minor: i64,
    pub min_price_minor: Option<i64>,
    pub min_stock_milli: Option<i64>,
    /// On hand in all warehouses, in thousandths of the base unit.
    pub on_hand_milli: i64,
    pub active: bool,
    pub version: i64,
    pub units: Vec<UnitRow>,
    pub barcodes: Vec<String>,
}

/// Stock state for lists and filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StockState {
    Ok,
    Low,
    Out,
}

impl ProductRow {
    pub fn stock_state(&self) -> StockState {
        if self.on_hand_milli <= 0 {
            StockState::Out
        } else if self.min_stock_milli.is_some_and(|m| self.on_hand_milli < m) {
            StockState::Low
        } else {
            StockState::Ok
        }
    }

    pub fn unit_set(&self) -> Result<UnitSet> {
        let units = self
            .units
            .iter()
            .map(|u| {
                Ok(ProductUnit {
                    code: u.code.clone(),
                    factor: u
                        .factor
                        .parse::<Decimal>()
                        .map_err(|_| Error::Corrupt(format!("unit factor {}", u.factor)))?,
                    precision: u.precision,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(UnitSet::new(units)?)
    }

    pub fn price(&self, currency: Currency) -> Money {
        Money::from_minor(self.price_minor, currency)
    }

    pub fn product_id(&self) -> Result<ProductId> {
        parse_id(&self.id)
    }
}

/// Sort keys of the product list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProductSort {
    Name,
    Sku,
    Category,
    Price,
    Stock,
    Status,
    Recent,
}

/// What the product list shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductQuery {
    pub text: String,
    pub category_id: Option<String>,
    pub state: Option<StockState>,
    pub sort: ProductSort,
    pub desc: bool,
    pub limit: u32,
    pub offset: u32,
}

impl Default for ProductQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            category_id: None,
            state: None,
            sort: ProductSort::Recent,
            desc: false,
            limit: 200,
            offset: 0,
        }
    }
}

/// A page of the list, and the counts per stock state for the filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductPage {
    pub rows: Vec<ProductRow>,
    /// Rows matching the query (all pages).
    pub total: u32,
    /// Counts for all, ok, low, out, under the text and category filters.
    pub counts: [u32; 4],
}

/// Turns typed text into an FTS5 query: every word must match as a prefix.
/// Quotes keep user input from being read as FTS syntax.
pub fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "")))
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" "))
    }
}

const STOCK_EXPR: &str =
    "COALESCE((SELECT SUM(b.on_hand_milli) FROM stock_balances b WHERE b.product_id = p.id), 0)";

fn state_expr() -> String {
    format!(
        "CASE WHEN {STOCK_EXPR} <= 0 THEN 'out' WHEN p.min_stock_milli IS NOT NULL AND {STOCK_EXPR} < p.min_stock_milli THEN 'low' ELSE 'ok' END"
    )
}

/// The products screen query: text (name, code, barcode), category, stock
/// state, sort and page.
pub fn query(conn: &Connection, org: OrganizationId, q: &ProductQuery) -> Result<ProductPage> {
    let mut filters = vec!["p.org_id = ?".to_owned(), "p.deleted_at IS NULL".to_owned()];
    let mut args: Vec<String> = vec![org.to_string()];
    if let Some(fts) = fts_query(&q.text) {
        filters
            .push("p.rowid IN (SELECT rowid FROM products_fts WHERE products_fts MATCH ?)".into());
        args.push(fts);
    }
    if let Some(c) = &q.category_id {
        filters.push("p.category_id = ?".into());
        args.push(c.clone());
    }
    let base_where = filters.join(" AND ");
    let state = state_expr();

    // Counts per state under the text/category filters.
    let mut counts = [0u32; 4];
    {
        let sql = format!(
            "SELECT {state} AS st, COUNT(*) FROM products p WHERE {base_where} GROUP BY st"
        );
        let mut s = conn.prepare(&sql)?;
        let mut rows = s.query(params_from_iter(args.iter()))?;
        while let Some(r) = rows.next()? {
            let st: String = r.get(0)?;
            let n: u32 = r.get(1)?;
            counts[0] += n;
            let i = match st.as_str() {
                "ok" => 1,
                "low" => 2,
                _ => 3,
            };
            counts[i] += n;
        }
    }

    let mut where_all = base_where.clone();
    if let Some(st) = q.state {
        where_all.push_str(&format!(
            " AND {state} = '{}'",
            match st {
                StockState::Ok => "ok",
                StockState::Low => "low",
                StockState::Out => "out",
            }
        ));
    }
    let total: u32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM products p WHERE {where_all}"),
        params_from_iter(args.iter()),
        |r| r.get(0),
    )?;
    let dir = if q.desc { "DESC" } else { "ASC" };
    let order = match q.sort {
        ProductSort::Name => format!("p.name COLLATE NOCASE {dir}"),
        ProductSort::Sku => format!("p.sku {dir}"),
        ProductSort::Category => format!("c.name COLLATE NOCASE {dir}, p.name COLLATE NOCASE"),
        ProductSort::Price => format!("p.price_minor {dir}, p.name COLLATE NOCASE"),
        ProductSort::Stock => format!("on_hand {dir}, p.name COLLATE NOCASE"),
        ProductSort::Status => format!(
            "CASE st WHEN 'out' THEN 0 WHEN 'low' THEN 1 ELSE 2 END {dir}, p.name COLLATE NOCASE"
        ),
        ProductSort::Recent => format!(
            "p.created_at {}, p.rowid {}",
            if q.desc { "ASC" } else { "DESC" },
            if q.desc { "ASC" } else { "DESC" }
        ),
    };
    let sql = format!(
        "SELECT p.id, p.sku, p.name, p.kind, p.category_id, c.name, p.tax_bps, p.price_minor, p.min_price_minor,
                p.min_stock_milli, {STOCK_EXPR} AS on_hand, p.active, p.version, {state} AS st
         FROM products p LEFT JOIN categories c ON c.id = p.category_id
         WHERE {where_all}
         ORDER BY {order}
         LIMIT {} OFFSET {}",
        q.limit.min(5_000),
        q.offset
    );
    let mut s = conn.prepare(&sql)?;
    let mut rows = s
        .query_map(params_from_iter(args.iter()), row_basic)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for r in &mut rows {
        load_children(conn, r)?;
    }
    Ok(ProductPage {
        rows,
        total,
        counts,
    })
}

fn row_basic(r: &rusqlite::Row<'_>) -> rusqlite::Result<ProductRow> {
    Ok(ProductRow {
        id: r.get(0)?,
        sku: r.get(1)?,
        name: r.get(2)?,
        kind: r.get(3)?,
        category_id: r.get(4)?,
        category: r.get(5)?,
        tax_bps: r.get(6)?,
        price_minor: r.get(7)?,
        min_price_minor: r.get(8)?,
        min_stock_milli: r.get(9)?,
        on_hand_milli: r.get(10)?,
        active: r.get(11)?,
        version: r.get(12)?,
        units: Vec::new(),
        barcodes: Vec::new(),
    })
}

fn load_children(conn: &Connection, p: &mut ProductRow) -> Result<()> {
    let mut s = conn.prepare_cached(
        "SELECT code, factor, precision FROM product_units WHERE product_id = ?1 ORDER BY position",
    )?;
    p.units = s
        .query_map([&p.id], |r| {
            Ok(UnitRow {
                code: r.get(0)?,
                factor: r.get(1)?,
                precision: r.get(2)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut s = conn
        .prepare_cached("SELECT code FROM product_barcodes WHERE product_id = ?1 ORDER BY code")?;
    p.barcodes = s
        .query_map([&p.id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(())
}

/// One product by id.
pub fn get(conn: &Connection, id: ProductId) -> Result<ProductRow> {
    let sql = format!(
        "SELECT p.id, p.sku, p.name, p.kind, p.category_id, c.name, p.tax_bps, p.price_minor, p.min_price_minor,
                p.min_stock_milli, {STOCK_EXPR}, p.active, p.version
         FROM products p LEFT JOIN categories c ON c.id = p.category_id
         WHERE p.id = ?1 AND p.deleted_at IS NULL"
    );
    let mut row = conn
        .query_row(&sql, [id.to_string()], row_basic)
        .optional()?
        .ok_or(Error::NotFound("product"))?;
    load_children(conn, &mut row)?;
    Ok(row)
}

/// The product a scanned or typed barcode belongs to.
pub fn by_barcode(
    conn: &Connection,
    org: OrganizationId,
    code: &str,
) -> Result<Option<(ProductRow, String)>> {
    let hit: Option<(String, String)> = conn
        .query_row(
            "SELECT product_id, unit_code FROM product_barcodes WHERE org_id = ?1 AND code = ?2",
            params![org.to_string(), code.trim()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match hit {
        Some((pid, unit)) => Ok(Some((get(conn, parse_id(&pid)?)?, unit))),
        None => Ok(None),
    }
}

pub fn sku_taken(
    conn: &Connection,
    org: OrganizationId,
    sku: &str,
    except: Option<ProductId>,
) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM products WHERE org_id = ?1 AND sku = ?2 AND deleted_at IS NULL AND id != ?3",
        params![org.to_string(), sku, except.map(|e| e.to_string()).unwrap_or_default()],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Barcodes among `codes` already used by another product.
pub fn barcodes_taken(
    conn: &Connection,
    org: OrganizationId,
    codes: &[String],
    except: Option<ProductId>,
) -> Result<Vec<String>> {
    let mut taken = Vec::new();
    let mut s = conn.prepare_cached(
        "SELECT product_id FROM product_barcodes WHERE org_id = ?1 AND code = ?2",
    )?;
    for c in codes {
        let owner: Option<String> = s
            .query_row(params![org.to_string(), c], |r| r.get(0))
            .optional()?;
        if let Some(o) = owner
            && Some(o.as_str()) != except.map(|e| e.to_string()).as_deref()
        {
            taken.push(c.clone());
        }
    }
    Ok(taken)
}

/// Writes a new product with its units and barcodes, and indexes it.
pub fn insert(
    w: &Writer<'_>,
    org: OrganizationId,
    id: ProductId,
    p: &ValidProduct,
    category_id: Option<&str>,
) -> Result<()> {
    let d = &p.0;
    w.tx().execute(
        "INSERT INTO products (id, org_id, sku, name, kind, category_id, tax_bps, price_minor, min_price_minor, min_stock_milli,
                               created_at, updated_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12, ?13)",
        params![
            id.to_string(),
            org.to_string(),
            d.sku,
            d.name,
            d.kind.code(),
            category_id,
            d.tax_rate.bps(),
            d.price.minor(),
            d.min_price.map(|m| m.minor()),
            d.min_stock.map(to_milli).transpose()?,
            w.now(),
            w.actor_text(),
            w.device().id.to_string()
        ],
    )?;
    write_children(w, org, id, p)?;
    Ok(())
}

/// Replaces a product's fields, units and barcodes (optimistic: `version`
/// must be the one read, or someone else changed it meanwhile).
pub fn update(
    w: &Writer<'_>,
    org: OrganizationId,
    id: ProductId,
    version: i64,
    p: &ValidProduct,
    category_id: Option<&str>,
) -> Result<()> {
    let d = &p.0;
    let n = w.tx().execute(
        "UPDATE products SET sku = ?1, name = ?2, kind = ?3, category_id = ?4, tax_bps = ?5, price_minor = ?6,
                min_price_minor = ?7, min_stock_milli = ?8, updated_at = ?9, version = version + 1
         WHERE id = ?10 AND version = ?11 AND deleted_at IS NULL",
        params![
            d.sku,
            d.name,
            d.kind.code(),
            category_id,
            d.tax_rate.bps(),
            d.price.minor(),
            d.min_price.map(|m| m.minor()),
            d.min_stock.map(to_milli).transpose()?,
            w.now(),
            id.to_string(),
            version
        ],
    )?;
    if n == 0 {
        return Err(Error::Rules(vec![nb_domain::RuleError::new(
            "stale_version",
        )]));
    }
    w.tx().execute(
        "DELETE FROM product_units WHERE product_id = ?1",
        [id.to_string()],
    )?;
    w.tx().execute(
        "DELETE FROM product_barcodes WHERE product_id = ?1",
        [id.to_string()],
    )?;
    write_children(w, org, id, p)?;
    Ok(())
}

fn write_children(
    w: &Writer<'_>,
    org: OrganizationId,
    id: ProductId,
    p: &ValidProduct,
) -> Result<()> {
    let d = &p.0;
    for (i, u) in d.units.units().iter().enumerate() {
        w.tx().execute(
            "INSERT INTO product_units (product_id, position, code, factor, precision) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), i64::try_from(i).unwrap_or(i64::MAX), u.code, u.factor.normalize().to_string(), u.precision],
        )?;
    }
    let base = d.units.base_unit().code.clone();
    for b in &d.barcodes {
        w.tx().execute(
            "INSERT INTO product_barcodes (org_id, code, product_id, unit_code) VALUES (?1, ?2, ?3, ?4)",
            params![org.to_string(), b, id.to_string(), base],
        )?;
    }
    reindex(w.tx(), id)?;
    Ok(())
}

/// Rebuilds the search entry of a product (name, code, barcodes).
pub fn reindex(conn: &Connection, id: ProductId) -> Result<()> {
    let row: Option<(i64, String, String, Option<i64>)> = conn
        .query_row(
            "SELECT rowid, name, sku, deleted_at FROM products WHERE id = ?1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((rowid, name, sku, deleted)) = row else {
        return Err(Error::NotFound("product"));
    };
    conn.execute("DELETE FROM products_fts WHERE rowid = ?1", [rowid])?;
    if deleted.is_none() {
        let codes: Vec<String> = conn
            .prepare_cached("SELECT code FROM product_barcodes WHERE product_id = ?1")?
            .query_map([id.to_string()], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        conn.execute(
            "INSERT INTO products_fts (rowid, name, sku, barcodes) VALUES (?1, ?2, ?3, ?4)",
            params![rowid, name, sku, codes.join(" ")],
        )?;
    }
    Ok(())
}

/// Archives a product (soft delete: history keeps referring to it).
pub fn archive(w: &Writer<'_>, id: ProductId) -> Result<()> {
    let n = w.tx().execute(
        "UPDATE products SET deleted_at = ?1, updated_at = ?1, version = version + 1 WHERE id = ?2 AND deleted_at IS NULL",
        params![w.now(), id.to_string()],
    )?;
    if n == 0 {
        return Err(Error::NotFound("product"));
    }
    w.tx().execute(
        "DELETE FROM product_barcodes WHERE product_id = ?1",
        [id.to_string()],
    )?;
    reindex(w.tx(), id)?;
    Ok(())
}

pub fn record_price_change(
    w: &Writer<'_>,
    id: ProductId,
    list: &str,
    old: Option<i64>,
    new: i64,
) -> Result<()> {
    w.tx().execute(
        "INSERT INTO price_history (product_id, list, old_minor, new_minor, changed_at, changed_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id.to_string(), list, old, new, w.now(), w.actor_text()],
    )?;
    Ok(())
}

pub fn categories(conn: &Connection, org: OrganizationId) -> Result<Vec<Category>> {
    let mut s = conn.prepare(
        "SELECT id, name FROM categories WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY name COLLATE NOCASE",
    )?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok(Category {
            id: r.get(0)?,
            name: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn insert_category(w: &Writer<'_>, org: OrganizationId, c: &Category) -> Result<()> {
    w.tx().execute(
        "INSERT INTO categories (id, org_id, name, created_at, updated_at, created_by, device_id) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6)",
        params![c.id, org.to_string(), c.name, w.now(), w.actor_text(), w.device().id.to_string()],
    )?;
    Ok(())
}

/// Rate of a stored product.
pub fn tax_rate(p: &ProductRow) -> Rate {
    Rate::from_bps(p.tax_bps)
}

/// Kind of a stored product.
pub fn kind(p: &ProductRow) -> ProductKind {
    ProductKind::from_code(&p.kind).unwrap_or(ProductKind::Goods)
}

/// Quantity helper for read models.
pub fn on_hand(p: &ProductRow) -> nb_core::Quantity {
    from_milli(p.on_hand_milli)
}

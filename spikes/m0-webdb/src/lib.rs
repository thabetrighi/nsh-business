//! M0.4 spike: durable SQLite in the browser for NSH Business.
//!
//! NSH's web runtime runs the app on the page's main thread, while the OPFS
//! "sync access handle pool" VFS (full durability) needs a dedicated worker.
//! So SQLite runs in a Web Worker (this module), and the app talks to it with
//! messages. The same `rusqlite` code compiles natively and for the web.

use std::cell::RefCell;

use rusqlite::{Connection, OpenFlags, params};
use sqlite_wasm_rs::WasmOsCallback;
use sqlite_wasm_vfs::sahpool::{OpfsSAHPoolCfg, install};
use wasm_bindgen::prelude::*;

thread_local! {
    static DB: RefCell<Option<Connection>> = const { RefCell::new(None) };
}

fn err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// Installs the OPFS VFS and opens (or creates) the database. Returns the
/// number of products already stored, which proves persistence across reloads.
#[wasm_bindgen]
pub async fn open() -> Result<u32, JsValue> {
    install::<WasmOsCallback>(&OpfsSAHPoolCfg::default(), true)
        .await
        .map_err(err)?;
    let conn = Connection::open_with_flags_and_vfs(
        "nsh-business.db",
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        c"opfs-sahpool",
    )
    .map_err(err)?;
    conn.execute_batch(
        "PRAGMA synchronous=FULL;
         CREATE TABLE IF NOT EXISTS products(id INTEGER PRIMARY KEY, sku TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL, price_minor INTEGER NOT NULL);
         CREATE VIRTUAL TABLE IF NOT EXISTS products_fts USING fts5(name, sku,
            content='products', content_rowid='id',
            tokenize='unicode61 remove_diacritics 2', prefix='2 3');
         CREATE TABLE IF NOT EXISTS invoices(id INTEGER PRIMARY KEY, operation_id TEXT NOT NULL UNIQUE,
            total_minor INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS stock_movements(id INTEGER PRIMARY KEY, product_id INTEGER NOT NULL,
            qty_milli INTEGER NOT NULL, invoice_id INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS outbox(id INTEGER PRIMARY KEY, operation_id TEXT NOT NULL);",
    )
    .map_err(err)?;
    let n: u32 = conn
        .query_row("SELECT COUNT(*) FROM products", [], |r| r.get(0))
        .map_err(err)?;
    DB.with(|d| *d.borrow_mut() = Some(conn));
    Ok(n)
}

fn with_db<T>(f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, JsValue> {
    DB.with(|d| {
        let mut d = d.borrow_mut();
        let conn = d.as_mut().ok_or_else(|| JsValue::from_str("database not open"))?;
        f(conn).map_err(err)
    })
}

/// Seeds `n` products in one transaction; returns elapsed ms.
#[wasm_bindgen]
pub fn seed(n: u32) -> Result<f64, JsValue> {
    const WORDS: &[&str] = &["حليب", "خبز", "زيت", "سكر", "أرز", "جبن", "عصير", "قهوة", "شاي", "صابون"];
    const BRANDS: &[&str] = &["Ifri", "Cevital", "Soummam", "Danone", "Elio"];
    let t = now_ms();
    with_db(|c| {
        let tx = c.transaction()?;
        {
            let mut ins = tx.prepare("INSERT INTO products(id, sku, name, price_minor) VALUES(?1,?2,?3,?4)")?;
            for i in 0..n as usize {
                let name = format!("{} {} {}", WORDS[i % WORDS.len()], BRANDS[(i / 7) % BRANDS.len()], i);
                ins.execute(params![i as i64 + 1, format!("SKU-{i:06}"), name, 5_000 + (i % 900) as i64 * 10])?;
            }
        }
        tx.execute_batch("INSERT INTO products_fts(rowid,name,sku) SELECT id,name,sku FROM products;")?;
        tx.commit()
    })?;
    Ok(now_ms() - t)
}

/// One FTS5 prefix search; returns the number of rows.
#[wasm_bindgen]
pub fn search(q: &str) -> Result<u32, JsValue> {
    let fts: String = q.split_whitespace().map(|t| format!("\"{}\"*", t.replace('"', ""))).collect::<Vec<_>>().join(" ");
    with_db(|c| {
        let mut st = c.prepare_cached(
            "SELECT p.id, p.name, p.price_minor FROM products_fts f JOIN products p ON p.id=f.rowid
             WHERE products_fts MATCH ?1 ORDER BY rank LIMIT 200",
        )?;
        let rows = st.query_map([fts], |_| Ok(()))?.count();
        Ok(rows as u32)
    })
}

/// Atomic sale: invoice + 5 stock movements + outbox, idempotent by operation id.
/// Returns true when written, false when the operation id was already applied.
#[wasm_bindgen]
pub fn complete_sale(operation_id: &str, seed: u32) -> Result<bool, JsValue> {
    with_db(|c| {
        let tx = c.transaction()?;
        let dup: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM invoices WHERE operation_id=?1)",
            [operation_id],
            |r| r.get(0),
        )?;
        if dup {
            return Ok(false);
        }
        tx.execute("INSERT INTO invoices(operation_id,total_minor) VALUES(?1,0)", [operation_id])?;
        let inv = tx.last_insert_rowid();
        let mut total = 0i64;
        for k in 0..5i64 {
            let pid = ((seed as i64 * 31 + k * 977) % 10_000) + 1;
            total += 1_500;
            tx.execute(
                "INSERT INTO stock_movements(product_id,qty_milli,invoice_id) VALUES(?1,-1000,?2)",
                params![pid, inv],
            )?;
        }
        tx.execute("UPDATE invoices SET total_minor=?1 WHERE id=?2", params![total, inv])?;
        tx.execute("INSERT INTO outbox(operation_id) VALUES(?1)", [operation_id])?;
        tx.commit()?;
        Ok(true)
    })
}

/// Row counts as JSON, for verification after a reload.
#[wasm_bindgen]
pub fn counts() -> Result<String, JsValue> {
    with_db(|c| {
        let q = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
        Ok(format!(
            "{{\"products\":{},\"invoices\":{},\"movements\":{},\"outbox\":{}}}",
            q("SELECT COUNT(*) FROM products")?,
            q("SELECT COUNT(*) FROM invoices")?,
            q("SELECT COUNT(*) FROM stock_movements")?,
            q("SELECT COUNT(*) FROM outbox")?
        ))
    })
}

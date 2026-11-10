//! M0.3 spike: SQLite performance for NSH Business on reference data.
//!
//! Measures: bulk insert of 100k products (+ barcodes), FTS5 search by
//! name (Arabic and Latin) and prefix, exact barcode lookup, and an atomic
//! "complete sale" transaction (invoice + lines + stock movements + outbox).
//! Run: `cargo run --release` (prints a markdown table).

use rusqlite::{Connection, params};
use std::time::{Duration, Instant};

const PRODUCTS: usize = 100_000;

const AR_WORDS: &[&str] = &[
    "حليب", "خبز", "زيت", "سكر", "أرز", "معكرونة", "جبن", "عصير", "ماء", "قهوة", "شاي",
    "صابون", "شامبو", "منظف", "طماطم", "بطاطا", "دجاج", "لحم", "سمك", "بسكويت", "شوكولاتة",
];
const AR_ADJ: &[&str] = &[
    "كامل الدسم", "خفيف", "طبيعي", "مركز", "معلب", "طازج", "مجمد", "عضوي", "ممتاز", "اقتصادي",
];
const EN_BRANDS: &[&str] = &[
    "Ifri", "Cevital", "Soummam", "Danone", "Nestle", "Elio", "Hamoud", "Bimo", "Amor", "Safia",
];
const SIZES: &[&str] = &["250g", "500g", "1kg", "1L", "2L", "330ml", "5L", "100g", "12x", "6x"];

fn schema(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE products(
            id TEXT PRIMARY KEY, sku TEXT NOT NULL UNIQUE, name TEXT NOT NULL,
            price_minor INTEGER NOT NULL, cost_minor INTEGER NOT NULL);
         CREATE TABLE barcodes(code TEXT PRIMARY KEY, product_id TEXT NOT NULL REFERENCES products(id));
         CREATE VIRTUAL TABLE products_fts USING fts5(
            name, sku, content='products', content_rowid='rowid',
            tokenize='unicode61 remove_diacritics 2', prefix='2 3');
         CREATE TABLE stock_movements(
            id TEXT PRIMARY KEY, product_id TEXT NOT NULL REFERENCES products(id),
            warehouse_id TEXT NOT NULL, qty_milli INTEGER NOT NULL, source TEXT NOT NULL);
         CREATE INDEX sm_product ON stock_movements(product_id, warehouse_id);
         CREATE TABLE stock_balances(product_id TEXT, warehouse_id TEXT, qty_milli INTEGER NOT NULL,
            PRIMARY KEY(product_id, warehouse_id));
         CREATE TABLE invoices(id TEXT PRIMARY KEY, number TEXT NOT NULL UNIQUE, total_minor INTEGER NOT NULL,
            operation_id TEXT NOT NULL UNIQUE);
         CREATE TABLE invoice_lines(id TEXT PRIMARY KEY, invoice_id TEXT NOT NULL REFERENCES invoices(id),
            product_id TEXT NOT NULL, qty_milli INTEGER NOT NULL, amount_minor INTEGER NOT NULL);
         CREATE TABLE outbox(id INTEGER PRIMARY KEY, operation_id TEXT NOT NULL, payload TEXT NOT NULL);",
    )
}

fn seed(c: &mut Connection) -> rusqlite::Result<Vec<String>> {
    let tx = c.transaction()?;
    let mut ids = Vec::with_capacity(PRODUCTS);
    {
        let mut p = tx.prepare(
            "INSERT INTO products(id,sku,name,price_minor,cost_minor) VALUES(?1,?2,?3,?4,?5)",
        )?;
        let mut b = tx.prepare("INSERT INTO barcodes(code,product_id) VALUES(?1,?2)")?;
        let mut s = tx.prepare(
            "INSERT INTO stock_balances(product_id,warehouse_id,qty_milli) VALUES(?1,'W1',1000000)",
        )?;
        for i in 0..PRODUCTS {
            let id = uuid::Uuid::now_v7().to_string();
            let name = format!(
                "{} {} {} {}",
                AR_WORDS[i % AR_WORDS.len()],
                AR_ADJ[(i / 7) % AR_ADJ.len()],
                EN_BRANDS[(i / 13) % EN_BRANDS.len()],
                SIZES[(i / 3) % SIZES.len()]
            );
            let sku = format!("SKU-{i:06}");
            p.execute(params![id, sku, name, 1000 + (i % 5000) as i64 * 10, 800])?;
            b.execute(params![format!("613{:010}", i), id])?;
            s.execute(params![id])?;
            ids.push(id);
        }
    }
    tx.execute_batch("INSERT INTO products_fts(rowid,name,sku) SELECT rowid,name,sku FROM products;")?;
    tx.commit()?;
    Ok(ids)
}

/// Runs `f` `n` times; returns (median, p95).
fn bench(n: usize, mut f: impl FnMut(usize)) -> (Duration, Duration) {
    let mut v: Vec<Duration> = (0..n)
        .map(|i| {
            let t = Instant::now();
            f(i);
            t.elapsed()
        })
        .collect();
    v.sort();
    (v[n / 2], v[(n * 95) / 100])
}

fn search(c: &Connection, q: &str) -> usize {
    let mut st = c
        .prepare_cached(
            "SELECT p.id, p.name, p.price_minor FROM products_fts f
             JOIN products p ON p.rowid = f.rowid
             WHERE products_fts MATCH ?1 ORDER BY rank LIMIT 50",
        )
        .expect("prepare search");
    st.query_map([q], |_| Ok(()))
        .expect("query")
        .count()
}

fn complete_sale(c: &mut Connection, ids: &[String], i: usize) -> rusqlite::Result<()> {
    let tx = c.transaction()?;
    let inv = uuid::Uuid::now_v7().to_string();
    let op = uuid::Uuid::now_v7().to_string();
    tx.execute(
        "INSERT INTO invoices(id,number,total_minor,operation_id) VALUES(?1,?2,?3,?4)",
        params![inv, format!("D1-2026-{i:07}"), 0, op],
    )?;
    let mut total = 0i64;
    for k in 0..5 {
        let pid = &ids[(i * 31 + k * 977) % ids.len()];
        let amount = 1500i64;
        total += amount;
        tx.execute(
            "INSERT INTO invoice_lines(id,invoice_id,product_id,qty_milli,amount_minor) VALUES(?1,?2,?3,1000,?4)",
            params![uuid::Uuid::now_v7().to_string(), inv, pid, amount],
        )?;
        tx.execute(
            "INSERT INTO stock_movements(id,product_id,warehouse_id,qty_milli,source) VALUES(?1,?2,'W1',-1000,?3)",
            params![uuid::Uuid::now_v7().to_string(), pid, inv],
        )?;
        tx.execute(
            "UPDATE stock_balances SET qty_milli = qty_milli - 1000 WHERE product_id=?1 AND warehouse_id='W1'",
            params![pid],
        )?;
    }
    tx.execute("UPDATE invoices SET total_minor=?1 WHERE id=?2", params![total, inv])?;
    tx.execute(
        "INSERT INTO outbox(operation_id,payload) VALUES(?1,?2)",
        params![op, format!("{{\"type\":\"SaleCompleted\",\"invoice\":\"{inv}\"}}")],
    )?;
    tx.commit()
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

fn main() -> rusqlite::Result<()> {
    let dir = std::env::temp_dir().join("nsh-m0-sqlite");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join("bench.db");
    let mut c = Connection::open(&path)?;
    schema(&c)?;

    let t = Instant::now();
    let ids = seed(&mut c)?;
    let seed_time = t.elapsed();

    println!("| القياس | الوسيط | p95 | النتائج |");
    println!("|---|---|---|---|");
    println!("| إدخال {PRODUCTS} منتج + باركود + رصيد + فهرسة FTS (معاملة واحدة) | {} | — | — |", ms(seed_time));

    for (label, q) in [
        ("بحث عربي كلمة كاملة `حليب`", "حليب"),
        ("بحث عربي بادئة `حلي*`", "حلي*"),
        ("بحث مركب `حليب Ifri`", "حليب Ifri"),
        ("بحث لاتيني بادئة `Cev*`", "Cev*"),
        ("بحث بالـ SKU `\"SKU-04217*\"`", "\"SKU-04217\"*"),
    ] {
        let n = search(&c, q);
        let (med, p95) = bench(200, |_| {
            search(&c, q);
        });
        println!("| {label} | {} | {} | {n} |", ms(med), ms(p95));
    }

    let (med, p95) = bench(1000, |i| {
        let code = format!("613{:010}", (i * 97) % PRODUCTS);
        let _: String = c
            .query_row(
                "SELECT p.name FROM barcodes b JOIN products p ON p.id=b.product_id WHERE b.code=?1",
                [code],
                |r| r.get(0),
            )
            .expect("barcode");
    });
    println!("| بحث باركود دقيق | {} | {} | 1 |", ms(med), ms(p95));

    let (med, p95) = bench(500, |i| complete_sale(&mut c, &ids, i).expect("sale"));
    println!("| `CompleteSale` ذرية (فاتورة + 5 أسطر + 5 حركات + أرصدة + outbox) | {} | {} | — |", ms(med), ms(p95));

    // Idempotency: same operation_id must be rejected by the UNIQUE constraint.
    let dup = c.execute(
        "INSERT INTO invoices(id,number,total_minor,operation_id)
         SELECT 'x','x',0,operation_id FROM invoices LIMIT 1",
        [],
    );
    println!("| رفض operation_id مكرر | {} | — | — |", if dup.is_err() { "✅" } else { "❌" });

    // Projection check: balance = initial + sum(movements).
    let mismatch: i64 = c.query_row(
        "SELECT COUNT(*) FROM stock_balances b WHERE b.qty_milli <> 1000000 +
           COALESCE((SELECT SUM(qty_milli) FROM stock_movements m WHERE m.product_id=b.product_id),0)",
        [],
        |r| r.get(0),
    )?;
    println!("| الرصيد = الافتتاحي + مجموع الحركات | {} | — | {mismatch} اختلاف |", if mismatch == 0 { "✅" } else { "❌" });

    drop(c);
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("| حجم قاعدة البيانات | {:.1} MB | — | — |", size as f64 / 1e6);
    Ok(())
}

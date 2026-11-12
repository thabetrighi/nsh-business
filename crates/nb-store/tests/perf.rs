//! Performance on reference data (M14.5, PRD §17.1). Ignored by default:
//!
//! ```text
//! cargo test --release -p nb-store --test perf -- --ignored --nocapture
//! ```
//!
//! Sizes: `NB_PERF_PRODUCTS` (100 000), `NB_PERF_SALES` (100 000, three
//! lines each on average, over a year). `NB_PERF_DB`: a file to keep the
//! database in, and to measure again from without seeding. Prints every
//! timing; fails when a PRD target is missed (search 150 ms, saving a sale
//! 200 ms, a page 100 ms, the start 3 s).

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use nb_core::FixedClock;
use nb_domain::ProductId;
use nb_store::demo::{self, DemoLang};
use nb_store::repo::catalog::ProductQuery;
use nb_store::usecases::products::list_products;
use nb_store::usecases::reports::{
    Period, Span, dashboard, income_statement, sales_report, stock_report,
};
use nb_store::usecases::sales::*;
use nb_store::usecases::setup::{SetupResult, setup_organization};
use nb_store::{Error, Store, StoreOptions};

const DAY: i64 = 86_400_000;

fn size(var: &str, default: usize) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The median of `runs` timings of `f`.
fn median<T>(runs: usize, mut f: impl FnMut(usize) -> Result<T, Error>) -> Result<Duration, Error> {
    let mut t = Vec::with_capacity(runs);
    for i in 0..runs {
        let start = Instant::now();
        f(i)?;
        t.push(start.elapsed());
    }
    t.sort();
    Ok(t[runs / 2])
}

fn show(what: &str, d: Duration, target_ms: Option<u128>) {
    let mark = match target_ms {
        Some(t) if d.as_millis() > t => format!("  ✗ target {t} ms"),
        Some(t) => format!("  ✓ < {t} ms"),
        None => String::new(),
    };
    let us = d.as_micros();
    println!("{what:<40} {:>8}.{} ms{mark}", us / 1000, us % 1000 / 100);
}

/// What selling needs, read from the database.
struct Shop {
    setup: SetupResult,
    register: String,
    products: Vec<ProductId>,
}

impl Shop {
    fn read(s: &Store) -> Result<Self, Error> {
        s.read(|r| {
            let c = r.conn();
            let o = nb_store::repo::org::the_org(c)?.ok_or(Error::NotFound("org"))?;
            let org = o.org_id()?;
            let branch = nb_store::repo::org::branches(c, org)?.remove(0);
            let warehouse = nb_store::repo::org::warehouses(c, org)?.remove(0);
            let owner: String = c.query_row(
                "SELECT id FROM users WHERE role = 'owner' LIMIT 1",
                [],
                |r| r.get(0),
            )?;
            let register = nb_store::usecases::org::registers(c, org)?.remove(0).id;
            let mut q = c.prepare("SELECT id FROM products ORDER BY rowid")?;
            let products = q
                .query_map([], |r| r.get::<_, String>(0))?
                .filter_map(|x| x.ok()?.parse().ok())
                .collect();
            Ok(Shop {
                setup: SetupResult {
                    org,
                    branch: nb_store::repo::parse_id(&branch.id)?,
                    warehouse: nb_store::repo::parse_id(&warehouse.id)?,
                    owner: nb_store::repo::parse_id(&owner)?,
                },
                register,
                products,
            })
        })
    }

    /// The `i`-th sale: one to five lines, paid in cash.
    fn sale(&self, i: usize) -> SaleInput {
        SaleInput {
            register_id: self.register.clone(),
            warehouse_id: self.setup.warehouse.to_string(),
            lines: (0..1 + i % 5)
                .map(|k| SaleLineInput {
                    product_id: self.products[(i * 7 + k * 131) % self.products.len()].to_string(),
                    qty: "1".into(),
                    ..Default::default()
                })
                .collect(),
            payments: vec![PaymentInput {
                method: "cash".into(),
                amount_minor: 100_000_000,
            }],
            ..Default::default()
        }
    }
}

#[test]
#[ignore = "long: run with --release -- --ignored"]
fn reference_data_meets_the_targets() -> Result<(), Error> {
    let products = size("NB_PERF_PRODUCTS", 100_000);
    let sales = size("NB_PERF_SALES", 100_000);
    let kept = std::env::var("NB_PERF_DB").ok().map(PathBuf::from);
    let clock = Arc::new(FixedClock::new(T0));
    if let Some(p) = kept.as_ref().filter(|p| p.exists()) {
        return measure(p, &clock);
    }
    let dir = std::env::temp_dir().join(format!(
        "nb-perf-{}",
        nb_core::uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| Error::Backup(e.to_string()))?;
    let path = kept.clone().unwrap_or_else(|| dir.join("store.db"));
    // A year back, then forward a sale at a time.
    let start = T0 - 365 * DAY;
    clock.set(start);
    let mut s = Store::open(
        &path,
        StoreOptions {
            clock: clock.clone(),
            ..options("A")
        },
    )?;
    let setup = setup_organization(&mut s, setup_input())?;
    let t = Instant::now();
    demo::seed(&mut s, &setup, DemoLang::Fr, products)?;
    show(&format!("seed {products} products"), t.elapsed(), None);
    // The demo catalog has no barcodes; a real one does (13 digits each).
    s.connection().execute_batch(
        "INSERT INTO product_barcodes (org_id, code, product_id, unit_code)
           SELECT p.org_id, printf('613%010d', p.rowid), p.id,
                  (SELECT code FROM product_units u WHERE u.product_id = p.id ORDER BY position LIMIT 1)
           FROM products p;
         UPDATE products_fts SET barcodes = printf('613%010d', rowid);",
    )?;
    let shop = Shop::read(&s)?;
    open_shift(&mut s, meta(&setup, "shift"), &shop.register, 0)?;
    // Seeding only: no wait on the disk per sale (measured after, reopened
    // with the real settings).
    s.connection().pragma_update(None, "synchronous", "OFF")?;
    let t = Instant::now();
    let mut sold = 0usize;
    for i in 0..sales {
        clock.set(start + (i as i64) * (365 * DAY) / (sales.max(1) as i64));
        if complete_sale(&mut s, meta(&setup, "sale"), shop.sale(i)).is_ok() {
            sold += 1;
        }
    }
    show(&format!("record {sold} sales"), t.elapsed(), None);
    drop(s);
    measure(&path, &clock)?;
    if kept.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}

/// Times what the PRD sets targets for, on the database at `path`.
fn measure(path: &Path, clock: &Arc<FixedClock>) -> Result<(), Error> {
    clock.set(T0);
    // The start: open (and migrate) the file; the full check runs after,
    // in the background.
    let t = Instant::now();
    let mut s = Store::open(
        path,
        StoreOptions {
            clock: clock.clone(),
            ..options("A")
        },
    )?;
    let opened = t.elapsed();
    show("open the store", opened, Some(3_000));
    let count =
        |sql: &str| -> Result<i64, Error> { Ok(s.connection().query_row(sql, [], |r| r.get(0))?) };
    println!(
        "{:<40} {} products, {} sales, {} movements, {} MB",
        "data",
        count("SELECT COUNT(*) FROM products")?,
        count("SELECT COUNT(*) FROM sales")?,
        count("SELECT COUNT(*) FROM stock_movements")?,
        std::fs::metadata(path)
            .map(|m| m.len() / 1_000_000)
            .unwrap_or(0)
    );
    let base = count("SELECT COUNT(*) FROM sales")? as usize;
    let t = Instant::now();
    let health = s.health()?;
    show("full check (background)", t.elapsed(), None);
    assert!(health.is_sound(), "{health:?}");
    let shop = Shop::read(&s)?;

    let search = median(21, |i| {
        let words = [
            "bisc", "lait dem", "huile", "café", "eau min", "riz", "sucre",
        ];
        list_products(
            &s,
            &ProductQuery {
                text: words[i % words.len()].into(),
                limit: 25,
                with_counts: false,
                ..Default::default()
            },
        )
    })?;
    show("search a product (median)", search, Some(150));
    let code: String = s.connection().query_row(
        "SELECT code FROM product_barcodes LIMIT 1 OFFSET (SELECT COUNT(*) / 2 FROM product_barcodes)",
        [],
        |r| r.get(0),
    )?;
    let by_code = median(21, |_| {
        list_products(
            &s,
            &ProductQuery {
                text: code.clone(),
                limit: 25,
                with_counts: false,
                ..Default::default()
            },
        )
    })?;
    show("find by barcode (median)", by_code, Some(150));
    let page = median(11, |i| {
        list_products(
            &s,
            &ProductQuery {
                limit: 25,
                offset: (i as u32) * 25,
                with_counts: false,
                ..Default::default()
            },
        )
    })?;
    show("products page (median)", page, Some(100));
    let counted = median(5, |_| {
        list_products(
            &s,
            &ProductQuery {
                limit: 0,
                ..Default::default()
            },
        )
    })?;
    show("counts per stock state (after the page)", counted, None);

    let save = median(101, |i| {
        clock.set(T0 + i as i64 * 1_000);
        complete_sale(&mut s, meta(&shop.setup, "sale"), shop.sale(base + i))
    })?;
    show("save a sale (median)", save, Some(200));

    let month = Span {
        from: "2026-10-01".into(),
        to: "2026-10-31".into(),
    };
    let year = Span {
        from: "2025-11-10".into(),
        to: "2026-11-10".into(),
    };
    show(
        "sales report, a month",
        median(3, |_| sales_report(&s, &month))?,
        None,
    );
    show(
        "sales report, a year",
        median(3, |_| sales_report(&s, &year))?,
        None,
    );
    show("stock report", median(3, |_| stock_report(&s, 30))?, None);
    show(
        "dashboard, 30 days",
        median(3, |_| dashboard(&s, Period::Month))?,
        None,
    );
    show(
        "dashboard, 12 months",
        median(3, |_| dashboard(&s, Period::Year))?,
        None,
    );
    show(
        "income statement, a year",
        median(3, |_| income_statement(&s, &year))?,
        None,
    );
    let recent = median(11, |_| {
        list_sales(
            &s,
            &SalesQuery {
                limit: 25,
                ..Default::default()
            },
        )
    })?;
    show("recent sales", recent, Some(100));
    let t = Instant::now();
    let snap = s.snapshot()?;
    show(
        &format!("snapshot ({} MB)", snap.len() / 1_000_000),
        t.elapsed(),
        None,
    );

    assert!(search.as_millis() <= 150, "search {search:?}");
    assert!(by_code.as_millis() <= 150, "barcode {by_code:?}");
    assert!(page.as_millis() <= 100, "page {page:?}");
    assert!(save.as_millis() <= 200, "save {save:?}");
    assert!(opened.as_millis() <= 3_000, "open {opened:?}");
    Ok(())
}

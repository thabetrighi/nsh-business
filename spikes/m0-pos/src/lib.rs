//! M0.6 spike: POS screen on NSH with a real SQLite catalog.
//!
//! - 100k products in SQLite (FTS5), searched on every keystroke.
//! - Product grid: `virtual_list(..).columns(n)` over up to 10k results.
//! - Cart with integer money (minor units), RTL layout.
//!
//! Spike code: the catalog lives on the UI thread; production code moves
//! queries to `background` tasks (see docs/spikes/M0-report.md).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use nsh_base::theme;
use nsh_core::element::input;
use nsh_core::prelude::*;
use rusqlite::{Connection, params};

#[native_module("m0-pos/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// Products seeded into the catalog.
pub const PRODUCTS: usize = 100_000;
/// Rows shown when the search box is empty.
const BROWSE_LIMIT: usize = 10_000;
/// Rows returned by a search.
const SEARCH_LIMIT: usize = 200;

#[derive(Clone, PartialEq, Debug)]
pub struct Product {
    pub id: i64,
    pub name: String,
    pub sku: String,
    pub price_minor: i64,
}

#[derive(Clone, PartialEq, Debug)]
struct CartLine {
    product: Product,
    qty: i64,
}

const AR_WORDS: &[&str] = &[
    "حليب", "خبز", "زيت", "سكر", "أرز", "معكرونة", "جبن", "عصير", "ماء", "قهوة", "شاي",
    "صابون", "شامبو", "منظف", "طماطم", "بطاطا", "دجاج", "لحم", "سمك", "بسكويت", "شوكولاتة",
];
const AR_ADJ: &[&str] = &[
    "كامل الدسم", "خفيف", "طبيعي", "مركز", "معلب", "طازج", "مجمد", "عضوي", "ممتاز", "اقتصادي",
];
const BRANDS: &[&str] = &[
    "Ifri", "Cevital", "Soummam", "Danone", "Nestle", "Elio", "Hamoud", "Bimo", "Amor", "Safia",
];
const SIZES: &[&str] = &["250g", "500g", "1kg", "1L", "2L", "330ml", "5L", "100g", "12x", "6x"];

/// SQLite catalog (spike: in memory, rebuilt at start).
pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    pub fn seeded(n: usize) -> rusqlite::Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE products(id INTEGER PRIMARY KEY, sku TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL, price_minor INTEGER NOT NULL);
             CREATE VIRTUAL TABLE products_fts USING fts5(name, sku,
                content='products', content_rowid='id',
                tokenize='unicode61 remove_diacritics 2', prefix='2 3');",
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins = tx.prepare(
                "INSERT INTO products(id, sku, name, price_minor) VALUES(?1, ?2, ?3, ?4)",
            )?;
            for i in 0..n {
                let name = format!(
                    "{} {} {} {}",
                    AR_WORDS[i % AR_WORDS.len()],
                    AR_ADJ[(i / 7) % AR_ADJ.len()],
                    BRANDS[(i / 13) % BRANDS.len()],
                    SIZES[(i / 3) % SIZES.len()]
                );
                let price = 5_000 + ((i * 37) % 5_000) as i64 * 10;
                ins.execute(params![i as i64 + 1, format!("SKU-{i:06}"), name, price])?;
            }
        }
        tx.execute_batch("INSERT INTO products_fts(rowid, name, sku) SELECT id, name, sku FROM products;")?;
        tx.commit()?;
        Ok(Self { conn })
    }

    /// Empty query: first `BROWSE_LIMIT` products. Otherwise FTS5 prefix search.
    pub fn search(&self, query: &str) -> rusqlite::Result<Vec<Product>> {
        let map = |r: &rusqlite::Row<'_>| {
            Ok(Product { id: r.get(0)?, name: r.get(1)?, sku: r.get(2)?, price_minor: r.get(3)? })
        };
        let tokens: Vec<String> = query
            .split_whitespace()
            .map(|t| format!("\"{}\"*", t.replace('"', "")))
            .collect();
        if tokens.is_empty() {
            let mut st = self
                .conn
                .prepare_cached("SELECT id, name, sku, price_minor FROM products ORDER BY id LIMIT ?1")?;
            return st.query_map([BROWSE_LIMIT as i64], map)?.collect();
        }
        let mut st = self.conn.prepare_cached(
            "SELECT p.id, p.name, p.sku, p.price_minor FROM products_fts f
             JOIN products p ON p.id = f.rowid
             WHERE products_fts MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        st.query_map(params![tokens.join(" "), SEARCH_LIMIT as i64], map)?.collect()
    }
}

/// Formats minor units (centimes) as `1 234.50 دج`.
pub fn format_dzd(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    let units = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in units.chars().enumerate() {
        if i > 0 && (units.len() - i).is_multiple_of(3) {
            grouped.push('\u{202F}');
        }
        grouped.push(c);
    }
    // FSI/PDI isolate the number so it keeps its order inside Arabic text on the web.
    format!("\u{2068}{sign}{grouped}.{:02}\u{2069} دج", abs % 100)
}

fn product_card(results: Signal<Vec<Product>>, at: Position, on_add: Rc<dyn Fn(Product)>) -> impl IntoElement {
    let product = computed(move || results.with(|r| r.get(at.index()).cloned()));
    view()
        .test_id("product")
        .p_3()
        .gap_2()
        .h(px(96.0))
        .justify_between()
        .rounded(px(10.0))
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .active(|s| s.opacity(0.6))
        .on_press(move || {
            if let Some(p) = product.get_untracked() {
                on_add(p);
            }
        })
        .child(
            text(move || product.with(|p| p.as_ref().map(|p| p.name.clone()).unwrap_or_default()))
                .number_of_lines(2)
                .text_size(px(14.0)),
        )
        .child(
            text(move || product.with(|p| p.as_ref().map(|p| format_dzd(p.price_minor)).unwrap_or_default()))
                .text_size(px(15.0))
                .font_weight(FontWeight::BOLD),
        )
}

#[component]
pub fn App() -> impl IntoElement {
    let started = Instant::now();
    let catalog = Rc::new(RefCell::new(None::<Catalog>));
    let seed_ms = signal(0.0_f64);
    let query = signal(String::new());
    let results = signal(Vec::<Product>::new());
    let search_ms = signal(0.0_f64);
    let cart = signal(Vec::<CartLine>::new());

    // Spike: seed synchronously at mount and record the time it took.
    match Catalog::seeded(PRODUCTS) {
        Ok(c) => {
            seed_ms.set(started.elapsed().as_secs_f64() * 1000.0);
            *catalog.borrow_mut() = Some(c);
        }
        Err(e) => log::error!("catalog seed failed: {e}"),
    }

    let run_search = {
        let catalog = catalog.clone();
        move |q: String| {
            let t = Instant::now();
            let found = catalog
                .borrow()
                .as_ref()
                .map(|c| c.search(&q).unwrap_or_default())
                .unwrap_or_default();
            search_ms.set(t.elapsed().as_secs_f64() * 1000.0);
            results.set(found);
        }
    };
    run_search(String::new());

    let add_to_cart: Rc<dyn Fn(Product)> = Rc::new(move |p: Product| {
        cart.update(|lines| match lines.iter_mut().find(|l| l.product.id == p.id) {
            Some(line) => line.qty += 1,
            None => lines.push(CartLine { product: p, qty: 1 }),
        });
    });
    let total = computed(move || cart.with(|l| l.iter().map(|l| l.product.price_minor * l.qty).sum::<i64>()));
    let items = computed(move || cart.with(|l| l.iter().map(|l| l.qty).sum::<i64>()));

    let grid = virtual_list(
        move || results.with(Vec::len),
        move |at| product_card(results, at, add_to_cart.clone()),
    )
    .test_id("grid")
    .columns(4)
    .estimated_item_size(px(96.0))
    .content_padding(px(12.0))
    .row_gap(px(10.0))
    .column_gap(px(10.0))
    .flex_1();

    let cart_list = virtual_list(
        move || cart.with(Vec::len),
        move |at| {
            let line = computed(move || cart.with(|l| l.get(at.index()).cloned()));
            view()
                .flex_row()
                .justify_between()
                .gap_2()
                .py_2()
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    text(move || {
                        line.with(|l| l.as_ref().map(|l| format!("{} × {}", l.qty, l.product.name)).unwrap_or_default())
                    })
                    .flex_1()
                    .number_of_lines(2)
                    .text_size(px(14.0)),
                )
                .child(
                    text(move || {
                        line.with(|l| l.as_ref().map(|l| format_dzd(l.product.price_minor * l.qty)).unwrap_or_default())
                    })
                    .text_size(px(14.0)),
                )
        },
    )
    .estimated_item_size(px(44.0))
    .flex_1();

    view()
        .direction(LayoutDirection::Rtl)
        .flex_1()
        .flex_row()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        // Products side.
        .child(
            view()
                .flex_1()
                .child(
                    view()
                        .flex_row()
                        .gap_3()
                        .p_3()
                        .items_center()
                        .child(
                            input()
                                .test_id("search")
                                .placeholder("ابحث بالاسم أو SKU أو الباركود…")
                                .value(move || query.get())
                                .on_change(move |t: &str| {
                                    query.set(t.to_owned());
                                    run_search(t.to_owned());
                                })
                                .flex_1()
                                .px_3()
                                .py_2()
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(move || theme().input),
                        )
                        .child(
                            text(move || {
                                format!(
                                    "{} نتيجة · بحث {:.1} ms · تحميل {:.0} ms",
                                    results.with(Vec::len),
                                    search_ms.get(),
                                    seed_ms.get()
                                )
                            })
                            .test_id("stats")
                            .text_size(px(13.0))
                            .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(grid),
        )
        // Cart side.
        .child(
            view()
                .w(px(360.0))
                .p_4()
                .gap_3()
                .bg(move || theme().muted)
                .child(text("السلة").text_size(px(20.0)).font_weight(FontWeight::BOLD))
                .child(cart_list)
                .child(
                    view()
                        .flex_row()
                        .justify_between()
                        .child(text(move || format!("{} قطعة", items.get())).test_id("items"))
                        .child(
                            text(move || format_dzd(total.get()))
                                .test_id("total")
                                .text_size(px(22.0))
                                .font_weight(FontWeight::BOLD),
                        ),
                )
                .child(
                    view()
                        .py_4()
                        .items_center()
                        .rounded(px(10.0))
                        .bg(move || theme().primary)
                        .active(|s| s.opacity(0.7))
                        .on_press(move || cart.set(Vec::new()))
                        .child(
                            text("دفع نقدي")
                                .text_size(px(18.0))
                                .text_color(move || theme().primary_foreground),
                        ),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_money_without_floats() {
        assert_eq!(format_dzd(123_450), "\u{2068}1\u{202F}234.50\u{2069} دج");
        assert_eq!(format_dzd(5), "\u{2068}0.05\u{2069} دج");
        assert_eq!(format_dzd(-1_000_000), "\u{2068}-10\u{202F}000.00\u{2069} دج");
    }

    #[test]
    fn searches_arabic_prefix() {
        let c = Catalog::seeded(2_000).expect("seed");
        assert_eq!(c.search("").expect("browse").len(), 2_000);
        let r = c.search("حلي").expect("search");
        assert!(!r.is_empty() && r.iter().all(|p| p.name.contains("حليب")));
        let r = c.search("حليب Ifri").expect("search");
        assert!(r.iter().all(|p| p.name.contains("Ifri")));
    }
}

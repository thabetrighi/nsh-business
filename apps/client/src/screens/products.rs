//! Products: searchable, sortable catalog table.
//!
//! SAMPLE DATA (2,000 generated products) until the catalog module (M5)
//! serves real rows from SQLite. Prices are integer centimes.

use nb_core::{Currency, Money, Quantity};
use nb_i18n::Lang;
use nsh_base::{
    DataTableAlign, DataTableCell, DataTableClasses, DataTableColumn, DataTableRows, DataTableSort,
    DataTableSortDirection, data_table,
};
use nsh_core::element::input;
use nsh_core::prelude::*;

use super::product_form::product_form;
use crate::state::AppState;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{label, primary_button, size};
use crate::ui::overlay::{modal, show_toast};
use crate::ui::palette::Tone;

#[derive(Clone, PartialEq)]
pub struct SampleProduct {
    pub sku: String,
    pub name_ar: String,
    pub name_fr: String,
    pub category: usize,
    pub unit: usize,
    pub price_minor: i64,
    pub stock: i64,
    pub min_stock: i64,
}

pub const CATEGORIES: [(&str, &str); 6] = [
    ("ألبان", "Produits laitiers"),
    ("مخبوزات", "Boulangerie"),
    ("زيوت وسكر", "Huiles et sucre"),
    ("مشروبات", "Boissons"),
    ("منظفات", "Entretien"),
    ("خضر وفواكه", "Fruits et légumes"),
];
pub const UNITS: [(&str, &str); 4] = [
    ("قطعة", "pièce"),
    ("علبة", "boîte"),
    ("كغ", "kg"),
    ("لتر", "litre"),
];
const NAMES: [(&str, &str, usize, usize); 12] = [
    ("حليب كامل الدسم", "Lait entier", 0, 3),
    ("ياغورت بالفراولة", "Yaourt fraise", 0, 1),
    ("جبن مثلثات", "Fromage portions", 0, 1),
    ("خبز تقليدي", "Pain traditionnel", 1, 0),
    ("بسكويت بالشوكولاتة", "Biscuits chocolat", 1, 1),
    ("زيت مائدة", "Huile de table", 2, 3),
    ("سكر أبيض", "Sucre blanc", 2, 2),
    ("ماء معدني", "Eau minérale", 3, 3),
    ("مشروب غازي", "Boisson gazeuse", 3, 3),
    ("مسحوق غسيل", "Lessive en poudre", 4, 2),
    ("طماطم طازجة", "Tomates fraîches", 5, 2),
    ("بطاطا", "Pommes de terre", 5, 2),
];
const BRANDS: [&str; 6] = ["Soummam", "Cevital", "Ifri", "Elio", "Hamoud", "Bimo"];

/// Deterministic sample catalog.
pub fn sample_catalog(n: usize) -> Vec<SampleProduct> {
    (0..n)
        .map(|i| {
            let (ar, fr, cat, unit) = NAMES[i % NAMES.len()];
            let brand = BRANDS[(i / NAMES.len()) % BRANDS.len()];
            let variant = i / (NAMES.len() * BRANDS.len());
            let stock = ((i * 37) % 140) as i64 - 4;
            SampleProduct {
                sku: format!("P-{:05}", 1000 + i),
                name_ar: format!(
                    "{ar} {brand}{}",
                    if variant > 0 {
                        format!(" #{variant}")
                    } else {
                        String::new()
                    }
                ),
                name_fr: format!(
                    "{fr} {brand}{}",
                    if variant > 0 {
                        format!(" #{variant}")
                    } else {
                        String::new()
                    }
                ),
                category: cat,
                unit,
                price_minor: 2_500 + ((i * 7_919) % 180) as i64 * 250,
                stock,
                min_stock: 10,
            }
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Col {
    Sku,
    Name,
    Category,
    Unit,
    Price,
    Stock,
}

impl Col {
    const ALL: [Col; 6] = [
        Col::Sku,
        Col::Name,
        Col::Category,
        Col::Unit,
        Col::Price,
        Col::Stock,
    ];

    fn key(self) -> &'static str {
        match self {
            Col::Sku => "sku",
            Col::Name => "name",
            Col::Category => "category",
            Col::Unit => "unit",
            Col::Price => "price",
            Col::Stock => "stock",
        }
    }

    fn title_key(self) -> &'static str {
        match self {
            Col::Sku => "products.col.sku",
            Col::Name => "products.col.name",
            Col::Category => "products.col.category",
            Col::Unit => "products.col.unit",
            Col::Price => "products.col.price",
            Col::Stock => "products.col.stock",
        }
    }

    fn of(key: &str) -> Option<Col> {
        Col::ALL.into_iter().find(|c| c.key() == key)
    }
}

fn pick(lang: Lang, pair: (&'static str, &'static str)) -> &'static str {
    if lang == Lang::Ar { pair.0 } else { pair.1 }
}

pub fn products(st: AppState, compact: bool) -> impl IntoElement {
    let all = signal(sample_catalog(2_000));
    // NB_OPEN=add_product (development): open the form at start, for screenshots.
    let adding = signal(std::env::var("NB_OPEN").as_deref() == Ok("add_product"));
    let query = signal(String::new());
    let sort = signal(None::<DataTableSort>);

    // Filter + sort the sample rows (the catalog module will do this in SQL).
    let rows = {
        computed(move || {
            let lang = st.lang();
            let q = query.get().trim().to_lowercase();
            let mut v: Vec<SampleProduct> = all.with(|all| {
                all.iter()
                    .filter(|p| {
                        q.is_empty()
                            || p.sku.to_lowercase().contains(&q)
                            || p.name_ar.contains(&q)
                            || p.name_fr.to_lowercase().contains(&q)
                    })
                    .cloned()
                    .collect()
            });
            if let Some(s) = sort.get() {
                let name = |p: &SampleProduct| {
                    if lang == Lang::Ar {
                        p.name_ar.clone()
                    } else {
                        p.name_fr.clone()
                    }
                };
                match Col::of(&s.key) {
                    Some(Col::Sku) => v.sort_by(|a, b| a.sku.cmp(&b.sku)),
                    Some(Col::Name) => v.sort_by_key(name),
                    Some(Col::Category) => v.sort_by_key(|p| p.category),
                    Some(Col::Unit) => v.sort_by_key(|p| p.unit),
                    Some(Col::Price) => v.sort_by_key(|p| p.price_minor),
                    Some(Col::Stock) => v.sort_by_key(|p| p.stock),
                    None => {}
                }
                if s.direction == DataTableSortDirection::Descending {
                    v.reverse();
                }
            }
            v
        })
    };

    // NSH's DataTable lays out left to right only (its fixed column is always on
    // the left). For Arabic we keep the table LTR and mirror it ourselves:
    // columns in reverse order, text aligned right, numbers to the reading end.
    let widths = |c: Col| match c {
        Col::Sku => 104.0,
        Col::Name => 0.0, // fills the remaining width
        Col::Category => 170.0,
        Col::Unit => 96.0,
        Col::Price => 140.0,
        Col::Stock => 110.0,
    };
    let ordered = move || {
        let mut v = Col::ALL.to_vec();
        if st.rtl() {
            v.reverse();
        }
        v
    };
    let columns = move || {
        let rtl = st.rtl();
        // Width left for the name column: window minus the shell and paddings.
        let shell = if compact { 32.0 } else { 248.0 + 64.0 };
        let fixed: f32 = Col::ALL.iter().map(|&c| widths(c)).sum();
        let name_w = (window().width.0 - shell - fixed - 4.0).max(220.0);
        ordered()
            .into_iter()
            .map(|c| {
                let mut col = DataTableColumn::default();
                col.key = c.key().into();
                col.title = Some(nb_i18n::tr(st.lang(), c.title_key()).into_owned().into());
                col.sortable = true;
                let numeric = matches!(c, Col::Price | Col::Stock);
                col.align = match (numeric, rtl) {
                    (false, false) | (true, true) => DataTableAlign::Start,
                    (false, true) | (true, false) => DataTableAlign::End,
                };
                col.fixed = c == Col::Sku && !rtl;
                col.width = Some(px(if c == Col::Name { name_w } else { widths(c) }));
                col
            })
            .collect::<Vec<_>>()
    };

    let data = move || {
        let lang = st.lang();
        let fmt = st.fmt();
        rows.with(|v| {
            DataTableRows::new(v.len())
                .texts("sku", v.iter().map(|p| p.sku.clone()))
                .texts(
                    "name",
                    v.iter().map(|p| {
                        if lang == Lang::Ar {
                            p.name_ar.clone()
                        } else {
                            p.name_fr.clone()
                        }
                    }),
                )
                .texts(
                    "category",
                    v.iter().map(|p| pick(lang, CATEGORIES[p.category])),
                )
                .texts("unit", v.iter().map(|p| pick(lang, UNITS[p.unit])))
                .texts(
                    "price",
                    v.iter()
                        .map(|p| fmt.money(Money::from_minor(p.price_minor, Currency::DZD))),
                )
                .texts(
                    "stock",
                    v.iter().map(|p| fmt.quantity(Quantity::whole(p.stock), 0)),
                )
        })
    };

    // Cells: tabular figures for numbers; stock under its minimum in the
    // warning tone. The table passes plain text (bidi controls stripped), so
    // numbers are isolated again here to keep signs in place.
    let render_cell = move |cell: &DataTableCell| {
        let row = cell.row as usize;
        let col = Col::of(&cell.key);
        let rtl = st.rtl();
        let numeric = matches!(col, Some(Col::Price | Col::Stock));
        let raw = nb_i18n::bidi::strip_controls(&cell.text);
        let content = if numeric {
            nb_i18n::bidi::isolate_ltr(&raw)
        } else {
            nb_i18n::bidi::isolate_runs(&raw, st.lang().direction())
        };
        let tone = match col {
            Some(Col::Stock) => rows.with(|v| match v.get(row) {
                Some(p) if p.stock <= 0 => Tone::Owed,
                Some(p) if p.stock < p.min_stock => Tone::Warn,
                _ => Tone::Ink,
            }),
            Some(Col::Sku | Col::Category | Col::Unit) => Tone::Muted,
            _ => Tone::Ink,
        };
        let weight = if numeric {
            FontWeight::MEDIUM
        } else {
            FontWeight::NORMAL
        };
        let mut t = text(content)
            .text_size(px(size::BODY))
            .font_weight(weight)
            .number_of_lines(1)
            .text_color(tone.color());
        if numeric {
            t = t.tabular_nums();
        }
        // Text at the reading start, numbers at the reading end (logical
        // alignment, so both follow the cell's direction).
        let cell_view = view().flex_1().justify_center().px_3().direction(if rtl {
            LayoutDirection::Rtl
        } else {
            LayoutDirection::Ltr
        });
        let cell_view = if numeric {
            cell_view.items_end()
        } else {
            cell_view.items_start()
        };
        cell_view.child(t)
    };

    let mut classes = DataTableClasses::default();
    classes.root = Some("bg-card border border-border rounded-xl overflow-hidden".into());
    classes.header = Some("bg-muted border-b border-border".into());
    classes.header_cell = Some("px-3 text-xs font-semibold text-muted-foreground".into());
    classes.row = Some("border-b border-border".into());
    classes.row_hover = Some("bg-accent/60".into());
    classes.row_selected = Some("bg-accent".into());
    classes.fixed_edge = Some("border-r border-border".into());

    let count = move || {
        st.t_with("products.count", move |s| {
            vec![("count", s.fmt().integer(rows.with(Vec::len) as i64))]
        })()
    };

    view()
        .flex_1()
        .w_full()
        .p(px(if compact { 16.0 } else { 32.0 }))
        .gap(px(16.0))
        // Title row.
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(
                    view()
                        .flex_1()
                        .child(label(
                            st.t("nav.products"),
                            size::HEADLINE,
                            FontWeight::BOLD,
                            Tone::Ink,
                        ))
                        .child(label(count, size::BODY, FontWeight::NORMAL, Tone::Muted)),
                )
                .child(primary_button(
                    st.t("products.add"),
                    Some(Icon::Plus),
                    move || adding.set(true),
                )),
        )
        // Search within the catalog.
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .h(px(40.0))
                .px_3()
                .rounded(px(8.0))
                .bg(Tone::Surface.color())
                .border_1()
                .border_color(Tone::Line.color())
                .child(icon(Icon::Search, 18.0, || Tone::Muted))
                .child(
                    input()
                        .test_id("product-search")
                        .flex_1()
                        .placeholder(st.t("products.search"))
                        .value(move || query.get())
                        .on_change(move |t: &str| query.set(t.to_owned()))
                        .text_size(px(size::BODY))
                        .text_color(Tone::Ink.color()),
                ),
        )
        .child(
            data_table()
                .direction(LayoutDirection::Ltr)
                .columns(columns)
                .data(data)
                .sort(move || sort.get())
                .on_sort_change(move |s: &Option<DataTableSort>| sort.set(s.clone()))
                .row_height(px(44.0))
                .header_height(px(40.0))
                .render_cell(render_cell)
                .classes(classes)
                .flex_1()
                .min_h(px(320.0)),
        )
        .child(modal(adding, compact, move || {
            let next_sku = format!("P-{:05}", 1000 + all.with(Vec::len));
            product_form(
                st,
                next_sku,
                move || adding.set(false),
                move |p: SampleProduct| {
                    let name = if st.lang() == Lang::Ar {
                        p.name_ar.clone()
                    } else {
                        p.name_fr.clone()
                    };
                    all.update(|v| v.insert(0, p));
                    adding.set(false);
                    show_toast(
                        st,
                        nb_i18n::tr_args(st.lang(), "products.saved", &[("name", &name)]),
                    );
                },
            )
            .into_element()
        }))
}

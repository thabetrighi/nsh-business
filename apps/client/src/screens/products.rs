//! Products: searchable, sortable catalog table.
//!
//! SAMPLE DATA (2,000 generated products) until the catalog module (M5)
//! serves real rows from SQLite. Prices are integer centimes.

use nb_core::{Currency, Money, Quantity};
use nb_i18n::Lang;
use nsh_core::element::View;
use nsh_core::prelude::*;

use super::product_form::product_form;
use crate::state::{AppState, Intent};
use crate::ui::form::search_field;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, figure, label_line, spacer};
use crate::ui::overlay::{modal, show_toast};
use crate::ui::popup::{Action, dropdown, row_menu};
use crate::ui::surface::{card, empty_state, page_header, page_pad};
use crate::ui::table::{Column, Selection, Sort, Width, table, table_footer};
use crate::ui::tokens::{Status, Tone, radius, sp, ty};

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

fn pick(lang: Lang, pair: (&'static str, &'static str)) -> &'static str {
    if lang == Lang::Ar { pair.0 } else { pair.1 }
}

/// Stock state of a product, shown as a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StockState {
    Ok,
    Low,
    Out,
}

impl StockState {
    fn of(p: &SampleProduct) -> Self {
        if p.stock <= 0 {
            StockState::Out
        } else if p.stock < p.min_stock {
            StockState::Low
        } else {
            StockState::Ok
        }
    }
    fn key(self) -> &'static str {
        match self {
            StockState::Ok => "stock.ok",
            StockState::Low => "stock.low",
            StockState::Out => "stock.out",
        }
    }
    fn status(self) -> Status {
        match self {
            StockState::Ok => Status::Success,
            StockState::Low => Status::Warning,
            StockState::Out => Status::Danger,
        }
    }
}

/// The status tabs over the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Only(StockState),
}

fn name_of(lang: Lang, p: &SampleProduct) -> String {
    if lang == Lang::Ar {
        p.name_ar.clone()
    } else {
        p.name_fr.clone()
    }
}

/// Identity of a sample row (its code's number).
fn id_of(p: &SampleProduct) -> u64 {
    p.sku.trim_start_matches("P-").parse().unwrap_or(0)
}

pub fn products(st: AppState, compact: bool) -> View {
    let all = signal(sample_catalog(2_000));
    // NB_OPEN=add_product (development): open the form at start, for screenshots.
    let adding = signal(std::env::var("NB_OPEN").as_deref() == Ok("add_product"));
    // Opened from the command palette's "Add a product".
    effect(move || {
        if st.intent.get() == Some(Intent::AddProduct) {
            st.intent.set(None);
            adding.set(true);
        }
    });
    let query = signal(String::new());
    let filter = signal(Filter::All);
    // usize::MAX: every category.
    let category = signal(usize::MAX);
    let sort = signal(None::<Sort>);
    let selection = Selection::new();

    // Filter + sort the sample rows (the catalog module will do this in SQL).
    let rows = computed(move || {
        let lang = st.lang();
        let q = query.get().trim().to_lowercase();
        let f = filter.get();
        let cat = category.get();
        let mut v: Vec<SampleProduct> = all.with(|all| {
            all.iter()
                .filter(|p| {
                    (q.is_empty()
                        || p.sku.to_lowercase().contains(&q)
                        || p.name_ar.contains(&q)
                        || p.name_fr.to_lowercase().contains(&q))
                        && (cat == usize::MAX || p.category == cat)
                        && match f {
                            Filter::All => true,
                            Filter::Only(s) => StockState::of(p) == s,
                        }
                })
                .cloned()
                .collect()
        });
        if let Some(s) = sort.get() {
            match s.key {
                "name" => v.sort_by_key(|p| name_of(lang, p)),
                "category" => v.sort_by_key(|p| pick(lang, CATEGORIES[p.category])),
                "status" => v.sort_by_key(|p| p.stock.min(p.min_stock)),
                "stock" => v.sort_by_key(|p| p.stock),
                "price" => v.sort_by_key(|p| p.price_minor),
                _ => {}
            }
            if s.desc {
                v.reverse();
            }
        }
        v
    });
    let counts = computed(move || {
        all.with(|v| {
            let mut c = [0i64; 4];
            for p in v {
                c[0] += 1;
                c[match StockState::of(p) {
                    StockState::Ok => 1,
                    StockState::Low => 2,
                    StockState::Out => 3,
                }] += 1;
            }
            c
        })
    });

    // ---------------------------------------------------------- toolbar
    let tab = move |f: Filter, key: &'static str, idx: usize| {
        let active = computed(move || filter.get() == f);
        view()
            .test_id(&format!("tab-{idx}"))
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(px(sp(40.0)))
            .px(px(4.0))
            .border_b(px(2.0))
            .cursor_pointer()
            .border_color(move || {
                if active.get() {
                    Tone::Primary.color()
                } else {
                    rgb(0x000000).opacity(0.0).into()
                }
            })
            .on_press(move || {
                filter.set(f);
                selection.clear();
            })
            .child(
                label_line(st.t(key), ty::SM, FontWeight::SEMIBOLD, Tone::Muted).text_color(
                    move || {
                        if active.get() {
                            Tone::Primary.color()
                        } else {
                            Tone::Muted.color()
                        }
                    },
                ),
            )
            .child(
                view()
                    .px(px(6.0))
                    .h(px(18.0))
                    .justify_center()
                    .rounded_full()
                    .bg(move || {
                        if active.get() {
                            Tone::PrimarySoft.color()
                        } else {
                            Tone::Subtle.color()
                        }
                    })
                    .child(figure(
                        move || st.fmt().integer(counts.get()[idx]),
                        11.0,
                        FontWeight::SEMIBOLD,
                        move || {
                            if active.get() {
                                Tone::Primary
                            } else {
                                Tone::Muted
                            }
                        },
                    )),
            )
    };
    let tabs = view()
        .flex_row()
        .items_end()
        .gap(px(18.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(Tone::Border.color())
        .child(tab(Filter::All, "products.filter.all", 0))
        .child(tab(Filter::Only(StockState::Ok), "stock.ok", 1))
        .child(tab(Filter::Only(StockState::Low), "stock.low", 2))
        .child(tab(Filter::Only(StockState::Out), "stock.out", 3));

    let categories = move || {
        let lang = st.lang();
        let mut v = vec![(usize::MAX, st.tn("products.category.all"))];
        v.extend(
            CATEGORIES
                .iter()
                .enumerate()
                .map(|(i, &c)| (i, pick(lang, c).to_owned())),
        );
        v
    };
    let tools = view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(12.0))
        .child(
            view()
                .w(px(if compact { 180.0 } else { 320.0 }))
                .flex_shrink(1.0)
                .child(search_field(
                    query,
                    st.t("products.search"),
                    "product-search",
                )),
        )
        .child(
            view()
                .w(px(if compact { 150.0 } else { 200.0 }))
                .child(dropdown(
                    categories,
                    category,
                    Some(Icon::Tag),
                    st.t("products.col.category"),
                    "category-filter",
                )),
        )
        .child(spacer())
        .child(when(
            move || selection.count() > 0,
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(label_line(
                        st.t_with("table.selected", move |s| {
                            vec![("count", s.fmt().integer(selection.count() as i64))]
                        }),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    ))
                    .child(button(
                        Variant::Ghost,
                        Size::Sm,
                        None,
                        st.t("action.clear"),
                        move || selection.clear(),
                    ))
                    .child(button(
                        Variant::Danger,
                        Size::Sm,
                        Some(Icon::Trash),
                        st.t("action.delete"),
                        move || {
                            let gone = selection.ids.get_untracked();
                            let n = gone.len();
                            all.update(|v| v.retain(|p| !gone.contains(&id_of(p))));
                            selection.clear();
                            show_toast(
                                st,
                                nb_i18n::tr_args(
                                    st.lang(),
                                    "products.deleted",
                                    &[("count", &st.fmt().integer(n as i64))],
                                ),
                            );
                        },
                    ))
                    .into_element()
            },
        ));

    // ---------------------------------------------------------- table
    let build_columns = move || {
        let mut columns = vec![
            Column::new("name", st.t("products.col.name"), Width::Flex(3.0, 220.0)).sortable(),
        ];
        if !compact {
            columns.push(
                Column::new(
                    "category",
                    st.t("products.col.category"),
                    Width::Flex(1.4, 140.0),
                )
                .sortable(),
            );
            columns.push(Column::new("status", st.t("col.status"), Width::Fixed(140.0)).sortable());
            columns.push(
                Column::new("stock", st.t("products.col.stock"), Width::Fixed(110.0))
                    .end()
                    .sortable(),
            );
        }
        columns.push(
            Column::new(
                "price",
                st.t("products.col.price"),
                Width::Fixed(if compact { 120.0 } else { 150.0 }),
            )
            .end()
            .sortable(),
        );
        if !compact {
            columns.push(Column::new("actions", String::new, Width::Fixed(52.0)));
        }
        columns
    };

    let row_of = move |at: Position| computed(move || rows.with(|r| r.get(at.index()).cloned()));
    let cell = move |at: Position, key: &'static str| -> AnyElement {
        let p = row_of(at);
        match key {
            "name" => view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .w_full()
                .child(
                    view()
                        .w(px(32.0))
                        .h(px(32.0))
                        .rounded(px(radius::md()))
                        .items_center()
                        .justify_center()
                        .bg(Tone::Subtle.color())
                        .border_1()
                        .border_color(Tone::Border.color())
                        .child(icon(Icon::Package, 16.0, || Tone::Muted)),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label_line(
                            move || {
                                p.get()
                                    .map(|p| {
                                        nb_i18n::bidi::isolate_runs(
                                            &name_of(st.lang(), &p),
                                            st.lang().direction(),
                                        )
                                    })
                                    .unwrap_or_default()
                            },
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            move || {
                                p.get()
                                    .map(|p| {
                                        if compact {
                                            format!("{} · {}", p.sku, st.fmt().integer(p.stock))
                                        } else {
                                            p.sku
                                        }
                                    })
                                    .unwrap_or_default()
                            },
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .into_element(),
            "category" => label_line(
                move || {
                    p.get()
                        .map(|p| pick(st.lang(), CATEGORIES[p.category]).to_owned())
                        .unwrap_or_default()
                },
                ty::SM,
                FontWeight::NORMAL,
                Tone::Body,
            )
            .into_element(),
            "status" => dynamic(move || {
                let s = p.get().map_or(StockState::Ok, |p| StockState::of(&p));
                let ic = match s {
                    StockState::Ok => Icon::Check,
                    StockState::Low => Icon::Alert,
                    StockState::Out => Icon::XCircle,
                };
                badge(s.status(), Some(ic), st.t(s.key())).into_element()
            })
            .into_element(),
            "stock" => figure(
                move || {
                    p.get()
                        .map(|p| st.fmt().quantity(Quantity::whole(p.stock), 0))
                        .unwrap_or_default()
                },
                ty::SM,
                FontWeight::MEDIUM,
                move || match p.get().map(|p| StockState::of(&p)) {
                    Some(StockState::Out) => Tone::Danger,
                    Some(StockState::Low) => Tone::Warning,
                    _ => Tone::Body,
                },
            )
            .into_element(),
            "price" => figure(
                move || {
                    p.get()
                        .map(|p| {
                            st.fmt()
                                .money(Money::from_minor(p.price_minor, Currency::DZD))
                        })
                        .unwrap_or_default()
                },
                ty::SM,
                FontWeight::SEMIBOLD,
                || Tone::Ink,
            )
            .into_element(),
            _ => row_menu(
                move || {
                    vec![
                        Action::new(Icon::Pencil, st.tn("action.edit"), || {}),
                        Action::new(Icon::Copy, st.tn("action.duplicate"), || {}),
                        Action::new(Icon::Printer, st.tn("products.print_label"), || {}),
                        Action::new(Icon::Trash, st.tn("action.delete"), move || {
                            if let Some(gone) = p.get_untracked() {
                                all.update(|v| v.retain(|q| q.sku != gone.sku));
                            }
                        })
                        .danger(),
                    ]
                },
                st.t("action.row_actions"),
            )
            .into_element(),
        }
    };

    let grid = move || {
        table(
            build_columns(),
            move || rows.with(Vec::len),
            sort,
            (!compact).then_some(selection),
            move |i| rows.with(|r| r.get(i).map(id_of)),
            cell,
            "products-table",
        )
        .into_element()
    };

    let body = when(
        move || rows.with(Vec::is_empty),
        move || {
            view()
                .flex_1()
                .child(empty_state(
                    Icon::Search,
                    st.t("table.empty.title"),
                    st.t("table.empty.body"),
                    Some(button(
                        Variant::Secondary,
                        Size::Md,
                        None,
                        st.t("action.clear"),
                        move || {
                            query.set(String::new());
                            filter.set(Filter::All);
                            category.set(usize::MAX);
                        },
                    )),
                ))
                .into_element()
        },
    )
    .otherwise(grid);

    let actions = if compact {
        view().child(button(
            Variant::Primary,
            Size::Md,
            Some(Icon::Plus),
            st.t("products.add"),
            move || adding.set(true),
        ))
    } else {
        view()
            .child(button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Upload),
                st.t("action.import"),
                || {},
            ))
            .child(button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Download),
                st.t("action.export"),
                || {},
            ))
            .child(button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("products.add"),
                move || adding.set(true),
            ))
    };

    let count = st.t_with("table.results", move |s| {
        vec![("count", s.fmt().integer(rows.with(Vec::len) as i64))]
    });

    view()
        .flex_1()
        .w_full()
        .p(px(page_pad(compact)))
        .gap(px(if compact { 16.0 } else { 24.0 }))
        .child(page_header(
            st.t("nav.products"),
            if compact {
                None
            } else {
                Some(Box::new(st.t("products.subtitle")))
            },
            actions,
            compact,
        ))
        .child(
            card()
                .flex_1()
                .min_h(px(360.0))
                .child(scroll().horizontal().flex_shrink_0().child(tabs))
                .child(tools)
                .child(
                    view()
                        .flex_1()
                        .border_t_1()
                        .border_color(Tone::Border.color())
                        .child(body),
                )
                .child(table_footer(count, view())),
        )
        .child(modal(adding, compact, move || {
            let next_sku = format!("P-{:05}", 1000 + all.with(Vec::len));
            product_form(
                st,
                next_sku,
                move || adding.set(false),
                move |p: SampleProduct| {
                    let name = name_of(st.lang(), &p);
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

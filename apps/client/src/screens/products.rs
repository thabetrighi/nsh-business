//! Products: the catalog from the local database (M5). Search, filters and
//! sort run in SQL (`list_products`); adding and archiving go through their
//! use cases, then the list loads again.

use std::hash::{Hash, Hasher};

use nb_core::{Currency, Money, Quantity};
use nb_domain::ProductId;
use nb_store::repo::catalog::{self, Category, ProductPage, ProductQuery, ProductRow, ProductSort};
use nb_store::usecases::products::{archive_products, list_products};
use nsh_core::element::View;
use nsh_core::prelude::*;

use super::product_editor::{Target, product_editor};
use crate::nav::Route;
use crate::screens::common::{page_head, section_tabs};
use crate::state::{AppState, Intent};
use crate::ui::form::search_field;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, figure, icon_button, label, label_line, menu_button, spacer,
};
use crate::ui::overlay::{modal, show_toast, wide_modal};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, empty_state, page_pad};
use crate::ui::table::{Column, Paging, Selection, Sort, Width, pager, table};
use crate::ui::tokens::{Status, Tone, control, radius, ty};
use nsh_base::{menu, menu_item};
use nsh_core::element::{Align, Placement};

/// Stock state of a product, shown as a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StockState {
    Ok,
    Low,
    Out,
}

impl StockState {
    fn of(p: &ProductRow) -> Self {
        match p.stock_state() {
            catalog::StockState::Ok => StockState::Ok,
            catalog::StockState::Low => StockState::Low,
            catalog::StockState::Out => StockState::Out,
        }
    }
    fn stored(self) -> catalog::StockState {
        match self {
            StockState::Ok => catalog::StockState::Ok,
            StockState::Low => catalog::StockState::Low,
            StockState::Out => catalog::StockState::Out,
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

/// Identity of a row for the selection (stable across reloads).
fn id_of(p: &ProductRow) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.id.hash(&mut h);
    h.finish()
}

fn sort_of(s: Option<Sort>) -> (ProductSort, bool) {
    match s {
        Some(s) => (
            match s.key {
                "name" => ProductSort::Name,
                "category" => ProductSort::Category,
                "status" => ProductSort::Status,
                "stock" => ProductSort::Stock,
                "price" => ProductSort::Price,
                _ => ProductSort::Recent,
            },
            s.desc,
        ),
        None => (ProductSort::Recent, false),
    }
}

/// A product as a row of text (to copy or export).
fn product_cells(st: AppState, p: &ProductRow) -> Vec<String> {
    vec![
        p.sku.clone(),
        p.name.clone(),
        p.category.clone().unwrap_or_default(),
        p.barcodes.first().cloned().unwrap_or_default(),
        st.fmt().quantity(Quantity::from_milli(p.on_hand_milli), 3),
        Money::from_minor(
            p.price_minor,
            st.session
                .get_untracked()
                .map_or(Currency::DZD, |s| s.currency),
        )
        .to_plain_string(),
    ]
}

/// The columns of an exported or copied list of products.
fn product_head(st: AppState) -> Vec<String> {
    vec![
        st.tn("products.col.sku"),
        st.tn("products.col.name"),
        st.tn("products.col.category"),
        st.tn("products.col.barcode"),
        st.tn("products.col.stock"),
        st.tn("products.col.price"),
    ]
}

/// Undoes one of the [`Extra`] filters.
type Clear = Box<dyn Fn(&mut Extra)>;

/// Filters beyond the stock state and the category (owner notes).
#[derive(Debug, Clone, Default, PartialEq)]
struct Extra {
    kind: Option<&'static str>,
    active: Option<bool>,
    barcode: Option<bool>,
    below_cost: bool,
    /// One of [`PRICES`].
    price: Option<usize>,
    /// (id, name).
    brand: Option<(String, String)>,
}

/// Price ranges in whole currency units: (from, to).
const PRICES: [(Option<i64>, Option<i64>); 5] = [
    (None, Some(100)),
    (Some(100), Some(500)),
    (Some(500), Some(1_000)),
    (Some(1_000), Some(5_000)),
    (Some(5_000), None),
];

impl Extra {
    fn count(&self) -> usize {
        usize::from(self.kind.is_some())
            + usize::from(self.active.is_some())
            + usize::from(self.barcode.is_some())
            + usize::from(self.below_cost)
            + usize::from(self.price.is_some())
            + usize::from(self.brand.is_some())
    }

    fn apply(&self, q: &mut ProductQuery, c: Currency) {
        q.kind = self.kind.map(str::to_owned);
        q.active = self.active;
        q.barcode = self.barcode;
        q.below_cost = self.below_cost;
        q.brand_id = self.brand.as_ref().map(|b| b.0.clone());
        if let Some(&(from, to)) = self.price.and_then(|i| PRICES.get(i)) {
            let unit = 10_i64.pow(u32::from(c.exponent()));
            q.price_min = from.map(|v| v * unit);
            q.price_max = to.map(|v| v * unit - 1);
        }
    }
}

fn price_range_text(st: AppState, at: usize) -> String {
    let n = |v: i64| st.fmt().integer(v);
    match PRICES.get(at) {
        Some((None, Some(to))) => format!("< {}", n(*to)),
        Some((Some(from), None)) => format!("≥ {}", n(*from)),
        Some((Some(from), Some(to))) => format!("{} – {}", n(*from), n(*to)),
        _ => String::new(),
    }
}

/// The selected products, a header first.
fn chosen_rows(
    st: AppState,
    rows: Memo<Vec<ProductRow>>,
    selection: Selection,
) -> Vec<Vec<String>> {
    let chosen = selection.ids.get_untracked();
    let mut out = vec![product_head(st)];
    rows.with_untracked(|v| {
        for p in v.iter().filter(|p| chosen.contains(&id_of(p))) {
            out.push(product_cells(st, p));
        }
    });
    out
}

/// Moves the selected products to a category.
fn move_dialog(
    st: AppState,
    cats: Memo<Vec<Category>>,
    rows: Memo<Vec<ProductRow>>,
    selection: Selection,
    close: impl Fn() + Clone + 'static,
) -> View {
    // usize::MAX: out of any category.
    let chosen = signal(usize::MAX);
    let busy = signal(false);
    let apply = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Some(meta) = st
                .session
                .get_untracked()
                .map(|s| s.meta("product.category"))
            else {
                return;
            };
            let cat = cats.with_untracked(|c| c.get(chosen.get_untracked()).map(|c| c.id.clone()));
            let marked = selection.ids.get_untracked();
            let ids: Vec<ProductId> = rows.with_untracked(|v| {
                v.iter()
                    .filter(|p| marked.contains(&id_of(p)))
                    .filter_map(|p| p.product_id().ok())
                    .collect()
            });
            busy.set(true);
            let close = close.clone();
            let task = spawn(async move {
                let r = st
                    .db()
                    .call(move |s| {
                        nb_store::usecases::products::set_products_category(
                            s,
                            meta,
                            &ids,
                            cat.as_deref(),
                        )
                    })
                    .await;
                busy.set(false);
                match r {
                    Ok(n) => {
                        selection.clear();
                        st.changed();
                        show_toast(
                            st,
                            nb_i18n::tr_args(
                                st.lang(),
                                "products.moved",
                                &[("count", &st.fmt().integer(i64::from(n)))],
                            ),
                        );
                        close();
                    }
                    Err(e) => show_toast(
                        st,
                        super::forms::error_text(st, e.codes.first().map_or("internal", |c| c.1)),
                    ),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let body = view().gap(px(12.0)).child(crate::ui::form::labelled(
        st.t("products.col.category"),
        dropdown(
            move || {
                let mut v = vec![(usize::MAX, st.tn("products.no_category"))];
                cats.with(|c| {
                    for (i, c) in c.iter().enumerate() {
                        v.push((i, c.name.clone()));
                    }
                });
                v
            },
            chosen,
            Some(Icon::Tag),
            st.t("products.col.category"),
            "move-category",
        )
        .w_full(),
        None,
    ));
    let footer = view()
        .child(button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("form.cancel"),
            close.clone(),
        ))
        .child(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Check),
                st.t("pos.apply"),
                apply,
            )
            .test_id("move-apply"),
        );
    let n = selection.count();
    crate::ui::overlay::dialog_frame(
        st,
        st.t("action.change_category"),
        move || {
            nb_i18n::tr_args(
                st.lang(),
                "products.move.help",
                &[("count", &st.fmt().integer(n as i64))],
            )
        },
        body,
        footer,
        close,
    )
}

/// Archives `ids`, then reloads the lists and confirms.
fn archive(st: AppState, ids: Vec<ProductId>) {
    let Some(session) = st.session.get_untracked() else {
        return;
    };
    let n = ids.len();
    let db = st.db();
    let meta = session.meta("product.archive");
    let task = spawn(async move {
        match db.call(move |s| archive_products(s, meta, &ids)).await {
            Ok(_) => {
                st.changed();
                show_toast(
                    st,
                    nb_i18n::tr_args(
                        st.lang(),
                        "products.deleted",
                        &[("count", &st.fmt().integer(n as i64))],
                    ),
                );
            }
            Err(e) => show_toast(
                st,
                super::forms::error_text(st, e.codes.first().map_or("internal", |c| c.1)),
            ),
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

pub fn products(st: AppState, compact: bool) -> View {
    // NB_OPEN=add_product (development): open the form at start, for screenshots.
    // The product whose label is being printed.
    let labeling: Signal<Option<ProductRow>> = signal(None);
    let labeling_open = signal(false);
    // The selected products being moved to a category.
    let moving_open = signal(false);
    let editor: Signal<Option<Target>> =
        signal((std::env::var("NB_OPEN").as_deref() == Ok("add_product")).then_some(Target::New));
    // Opened from the command palette's "Add a product".
    effect(move || match st.intent.get() {
        Some(Intent::AddProduct) => {
            st.intent.set(None);
            editor.set(Some(Target::New));
        }
        Some(Intent::EditProduct(id)) => {
            st.intent.set(None);
            editor.set(Some(Target::Edit(id)));
        }
        _ => {}
    });
    let query = signal(String::new());
    let filter = signal(Filter::All);
    // usize::MAX: every category.
    let category = signal(usize::MAX);
    let sort = signal(None::<Sort>);
    // The favourites only.
    let favs_only = signal(false);
    let extra = signal(Extra::default());
    let selection = Selection::new();
    let paging = Paging::new(25);
    // Other filters or another order start again from the first page.
    effect(move || {
        query.with(|_| ());
        filter.with(|_| ());
        category.with(|_| ());
        sort.with(|_| ());
        favs_only.with(|_| ());
        extra.with(|_| ());
        paging.reset();
    });

    // The categories, then the page of rows for the current filters (SQL).
    let cats = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    s.read(|r| {
                        let o = nb_store::repo::org::the_org(r.conn())?
                            .ok_or(nb_store::Error::NotFound("organization"))?;
                        catalog::categories(r.conn(), o.org_id()?)
                    })
                })
                .await
            }
        },
    );
    let cat_list = computed(move || cats.get().unwrap_or_default());
    let brands = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| nb_store::usecases::catalog::list_brands(s))
                    .await
            }
        },
    );
    let currency_now = move || {
        st.session
            .get_untracked()
            .map_or(Currency::DZD, |s| s.currency)
    };
    // Sent from the categories: their products.
    effect(move || {
        let Some(Intent::ProductsIn(id)) = st.intent.get() else {
            return;
        };
        let Some(at) = cat_list.with(|c: &Vec<Category>| c.iter().position(|c| c.id == id)) else {
            return;
        };
        st.intent.set(None);
        category.set(at);
    });
    let page = resource(
        move || {
            st.session.get()?;
            let (sort, desc) = sort_of(sort.get());
            let category_id = match category.get() {
                usize::MAX => None,
                i => cat_list.with(|c: &Vec<Category>| c.get(i).map(|c| c.id.clone())),
            };
            let mut q = ProductQuery {
                text: query.get(),
                category_id,
                state: match filter.get() {
                    Filter::All => None,
                    Filter::Only(s) => Some(s.stored()),
                },
                sort,
                desc,
                limit: paging.size.get(),
                offset: paging.offset(),
                // Counted apart (below): a page turn reads one page.
                with_counts: false,
                favorites: favs_only.get(),
                ..ProductQuery::default()
            };
            extra.with(|e| e.apply(&mut q, currency_now()));
            Some((q, st.data_rev.get()))
        },
        move |(q, _), _| {
            let db = st.db();
            async move { db.call(move |s| list_products(s, &q)).await }
        },
    );
    // The counts per stock state read every product's stock: only when
    // the search or the category changes, after the page shows.
    let counted = resource(
        move || {
            st.session.get()?;
            let category_id = match category.get() {
                usize::MAX => None,
                i => cat_list.with(|c: &Vec<Category>| c.get(i).map(|c| c.id.clone())),
            };
            Some((query.get(), category_id, st.data_rev.get()))
        },
        move |(text, category_id, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    Ok(list_products(
                        s,
                        &ProductQuery {
                            text,
                            category_id,
                            limit: 0,
                            ..Default::default()
                        },
                    )?
                    .counts)
                })
                .await
            }
        },
    );
    let rows = computed(move || {
        page.with(|p: Option<&ProductPage>| p.map(|p| p.rows.clone()).unwrap_or_default())
    });
    let counts = computed(move || {
        counted.with(|c: Option<&[u32; 4]>| c.map_or([0i64; 4], |c| c.map(i64::from)))
    });
    let total = computed(move || page.with(|p: Option<&ProductPage>| p.map_or(0, |p| p.total)));
    let currency = move || st.session.get().map_or(Currency::DZD, |s| s.currency);

    // ---------------------------------------------------------- quick filters
    // Stock states as cards with their counts (phones: chips).
    let quick = move |f: Filter, key: &'static str, idx: usize, which: Icon, status: Status| {
        let active = computed(move || filter.get() == f);
        crate::ui::filters::stat_tile(
            &format!("tab-{idx}"),
            st.t(key),
            which,
            status,
            move || st.fmt().integer(counts.get()[idx]),
            move || active.get(),
            Some(Box::new(move || {
                filter.set(f);
                selection.clear();
            })),
            compact,
        )
    };
    let quick_row = view()
        .flex_row()
        .gap(px(if compact { 6.0 } else { 16.0 }))
        .child(quick(
            Filter::All,
            "products.filter.all",
            0,
            Icon::Package,
            Status::Accent,
        ))
        .child(quick(
            Filter::Only(StockState::Ok),
            "stock.ok",
            1,
            Icon::CheckCircle,
            Status::Success,
        ))
        .child(quick(
            Filter::Only(StockState::Low),
            "stock.low",
            2,
            Icon::Alert,
            Status::Warning,
        ))
        .child(quick(
            Filter::Only(StockState::Out),
            "stock.out",
            3,
            Icon::XCircle,
            Status::Danger,
        ));
    let quick_row = if compact {
        view().child(scroll().horizontal().child(quick_row))
    } else {
        quick_row
    };

    // ---------------------------------------------------------- toolbar
    let categories = move || {
        let mut v = vec![(usize::MAX, st.tn("products.category.all"))];
        v.extend(
            cat_list
                .get()
                .into_iter()
                .enumerate()
                .map(|(i, c)| (i, c.name)),
        );
        v
    };
    // Optional columns, hidden by bit: category, status, stock.
    let hidden = signal(0u8);
    let columns_menu = move || {
        let item = move |bit: u8, key: &'static str| {
            menu_item()
                .checked(move || Some(hidden.get() & bit == 0))
                .close_on_select(false)
                .on_select(move || hidden.update(|h| *h ^= bit))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .h(px(control::md()))
                        .px(px(10.0))
                        .rounded(px(radius::md()))
                        .cursor_pointer()
                        .hover(|s| s.bg(Tone::Hover.color()))
                        .child(dynamic(move || {
                            if hidden.get() & bit == 0 {
                                icon(Icon::Check, 14.0, || Tone::Primary).into_element()
                            } else {
                                view().w(px(14.0)).into_element()
                            }
                        }))
                        .child(label_line(st.t(key), ty::SM, FontWeight::MEDIUM, Tone::Ink)),
                )
        };
        menu()
            .trigger(menu_button(Some(Icon::Columns), st.t("table.columns")))
            .popup_class("bg-popover border border-border rounded-lg shadow-lg p-1 min-w-[200px]")
            .placement(Placement::Bottom)
            .align(Align::End)
            .child(item(1, "products.col.category"))
            .child(item(2, "col.status"))
            .child(item(4, "products.col.stock"))
    };
    // More filters (owner notes): a menu of kinds, sale, barcode, price,
    // brand; the chips below say what is on.
    let more_filters = move || {
        dynamic(move || {
            use crate::ui::menu::{Act, menu_on};
            let e = extra.get();
            let n = e.count();
            let tick = |on: bool, label: String, f: Box<dyn Fn(&mut Extra)>| {
                Act::new(label, move || extra.update(|x| f(x))).checked(on)
            };
            let kind = vec![
                tick(
                    e.kind.is_none(),
                    st.tn("filter.any"),
                    Box::new(|x| x.kind = None),
                ),
                tick(
                    e.kind == Some("goods"),
                    st.tn("editor.kind.goods"),
                    Box::new(|x| x.kind = Some("goods")),
                ),
                tick(
                    e.kind == Some("service"),
                    st.tn("editor.kind.service"),
                    Box::new(|x| x.kind = Some("service")),
                ),
            ];
            let sale = vec![
                tick(
                    e.active.is_none(),
                    st.tn("filter.any"),
                    Box::new(|x| x.active = None),
                ),
                tick(
                    e.active == Some(true),
                    st.tn("filter.on_sale"),
                    Box::new(|x| x.active = Some(true)),
                ),
                tick(
                    e.active == Some(false),
                    st.tn("filter.stopped"),
                    Box::new(|x| x.active = Some(false)),
                ),
            ];
            let code = vec![
                tick(
                    e.barcode.is_none(),
                    st.tn("filter.any"),
                    Box::new(|x| x.barcode = None),
                ),
                tick(
                    e.barcode == Some(true),
                    st.tn("filter.with_barcode"),
                    Box::new(|x| x.barcode = Some(true)),
                ),
                tick(
                    e.barcode == Some(false),
                    st.tn("filter.without_barcode"),
                    Box::new(|x| x.barcode = Some(false)),
                ),
            ];
            let mut price = vec![tick(
                e.price.is_none(),
                st.tn("filter.any"),
                Box::new(|x| x.price = None),
            )];
            for i in 0..PRICES.len() {
                price.push(tick(
                    e.price == Some(i),
                    price_range_text(st, i),
                    Box::new(move |x| x.price = Some(i)),
                ));
            }
            let mut brand = vec![tick(
                e.brand.is_none(),
                st.tn("filter.any"),
                Box::new(|x| x.brand = None),
            )];
            for b in brands.get().unwrap_or_default() {
                let on = e.brand.as_ref().is_some_and(|x| x.0 == b.id);
                let pair = (b.id.clone(), b.name.clone());
                brand.push(tick(
                    on,
                    b.name,
                    Box::new(move |x| x.brand = Some(pair.clone())),
                ));
            }
            let mut acts = vec![
                Act::sub(Icon::Package, st.tn("filter.kind"), kind),
                Act::sub(Icon::CheckCircle, st.tn("filter.sale"), sale),
                Act::sub(Icon::Barcode, st.tn("products.col.barcode"), code),
                Act::sub(Icon::Coins, st.tn("products.col.price"), price),
            ];
            if brand.len() > 1 {
                acts.push(Act::sub(Icon::Tags, st.tn("filter.brand"), brand));
            }
            acts.push(Act::Line);
            acts.push(
                tick(
                    e.below_cost,
                    st.tn("filter.below_cost"),
                    Box::new(|x| x.below_cost = !x.below_cost),
                )
                .icon(Icon::Alert),
            );
            if n > 0 {
                acts.push(Act::with(
                    Icon::Close,
                    st.tn("table.clear_filters"),
                    move || extra.set(Extra::default()),
                ));
            }
            let on = n > 0;
            let trigger = view()
                .test_id("products-more-filters")
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(control::md()))
                .px(px(12.0))
                .rounded(px(radius::md()))
                .border_1()
                .border_color(if on {
                    Tone::Primary.color()
                } else {
                    Tone::BorderStrong.color()
                })
                .bg(if on {
                    Tone::PrimarySoft.color()
                } else {
                    Tone::Surface.color()
                })
                .cursor_pointer()
                .child(icon(Icon::Filter, 16.0, move || {
                    if on { Tone::Primary } else { Tone::Muted }
                }))
                .child(label_line(
                    st.t("filter.more"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Body,
                ))
                .child(if on {
                    view()
                        .min_w(px(18.0))
                        .h(px(18.0))
                        .px(px(5.0))
                        .rounded_full()
                        .items_center()
                        .justify_center()
                        .bg(Tone::Primary.color())
                        .child(label_line(
                            move || n.to_string(),
                            ty::XS,
                            FontWeight::BOLD,
                            Tone::OnPrimary,
                        ))
                } else {
                    view()
                });
            menu_on(trigger, acts).into_element()
        })
    };
    let tools = move || {
        view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(12.0))
            .child(
                view()
                    .w(if compact {
                        relative(1.0)
                    } else {
                        px(300.0).into()
                    })
                    .flex_shrink(1.0)
                    .child(search_field(
                        query,
                        st.t("products.search"),
                        "product-search",
                    )),
            )
            .child(view().w(px(210.0)).child(dropdown(
                categories,
                category,
                Some(Icon::Tag),
                st.t("products.col.category"),
                "category-filter",
            )))
            .child(
                view()
                    .test_id("products-favorites")
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(control::md()))
                    .px(px(12.0))
                    .rounded(px(radius::md()))
                    .border_1()
                    .border_color(move || {
                        if favs_only.get() {
                            Tone::Warning.color()
                        } else {
                            Tone::BorderStrong.color()
                        }
                    })
                    .bg(move || {
                        if favs_only.get() {
                            Tone::WarningSoft.color()
                        } else {
                            Tone::Surface.color()
                        }
                    })
                    .cursor_pointer()
                    .on_press(move || favs_only.update(|v| *v = !*v))
                    .child(icon(Icon::Star, 16.0, move || {
                        if favs_only.get() {
                            Tone::Warning
                        } else {
                            Tone::Muted
                        }
                    }))
                    .child(label_line(
                        st.t("products.favorites"),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Body,
                    )),
            )
            .child(more_filters())
            .child(spacer())
            .child(if compact {
                view()
            } else {
                view().child(columns_menu())
            })
            .into_element()
    };

    // Shown instead of the toolbar while rows are selected.
    let bulk = move || {
        view()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(16.0))
            .h(px(control::md() + 24.0))
            .bg(Tone::PrimarySoft.color())
            .child(icon(Icon::CheckCircle, 18.0, || Tone::Primary))
            .child(label_line(
                st.t_with("table.selected", move |s| {
                    vec![("count", s.fmt().integer(selection.count() as i64))]
                }),
                ty::SM,
                FontWeight::BOLD,
                Tone::Primary,
            ))
            .child(view().w(px(8.0)))
            .child(
                button(
                    Variant::Secondary,
                    Size::Sm,
                    Some(Icon::Copy),
                    st.t("action.copy"),
                    move || {
                        set_clipboard(crate::ui::menu::tsv(&chosen_rows(st, rows, selection)));
                        show_toast(st, st.tn("action.copied"));
                    },
                )
                .test_id("products-bulk-copy"),
            )
            .child(
                button(
                    Variant::Secondary,
                    Size::Sm,
                    Some(Icon::Tag),
                    st.t("action.change_category"),
                    move || moving_open.set(true),
                )
                .test_id("products-bulk-category"),
            )
            .child(
                button(
                    Variant::Secondary,
                    Size::Sm,
                    Some(Icon::Download),
                    st.t("action.export"),
                    move || {
                        crate::screens::reports::export(
                            st,
                            format!("{}.csv", st.tn("products.file")),
                            chosen_rows(st, rows, selection),
                        )
                    },
                )
                .test_id("products-bulk-export"),
            )
            .child(button(
                Variant::Danger,
                Size::Sm,
                Some(Icon::Trash),
                st.t("action.delete"),
                move || {
                    let gone = selection.ids.get_untracked();
                    let ids: Vec<ProductId> = rows.with(|v| {
                        v.iter()
                            .filter(|p| gone.contains(&id_of(p)))
                            .filter_map(|p| p.product_id().ok())
                            .collect()
                    });
                    selection.clear();
                    archive(st, ids);
                },
            ))
            .child(spacer())
            .child(icon_button(
                || Icon::Close,
                st.t("action.clear"),
                Variant::Ghost,
                move || selection.clear(),
            ))
            .into_element()
    };
    let toolbar = when(move || selection.count() > 0, bulk).otherwise(tools);

    // Active filters as removable chips.
    let chip = move |text_of: Box<dyn Fn() -> String>, clear: Box<dyn Fn()>| {
        view()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .h(px(26.0))
            .ps(px(10.0))
            .pe(px(4.0))
            .rounded_full()
            .bg(Tone::Subtle.color())
            .border_1()
            .border_color(Tone::Border.color())
            .child(label_line(
                text_of,
                ty::XS,
                FontWeight::SEMIBOLD,
                Tone::Body,
            ))
            .child(
                view()
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded_full()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .on_press(clear)
                    .child(icon(Icon::Close, 12.0, || Tone::Muted)),
            )
    };
    let filtered = computed(move || {
        extra.with(|e| e.count() > 0)
            || favs_only.get()
            || filter.get() != Filter::All
            || category.get() != usize::MAX
            || !query.get().trim().is_empty()
    });
    let chips_row = dynamic(move || {
        if !filtered.get() {
            return view().into_element();
        }
        let mut row = view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .px(px(16.0))
            .pb(px(12.0))
            .child(label(
                st.t("table.filtered_by"),
                ty::XS,
                FontWeight::MEDIUM,
                Tone::Muted,
            ));
        if let Filter::Only(s) = filter.get() {
            row = row.child(chip(
                Box::new(move || {
                    format!("{}: {}", st.tn("products.filter.status"), st.tn(s.key()))
                }),
                Box::new(move || filter.set(Filter::All)),
            ));
        }
        let cat = category.get();
        if cat != usize::MAX {
            row = row.child(chip(
                Box::new(move || {
                    format!(
                        "{}: {}",
                        st.tn("products.col.category"),
                        cat_list.with(|c: &Vec<Category>| c
                            .get(cat)
                            .map(|c| c.name.clone())
                            .unwrap_or_default())
                    )
                }),
                Box::new(move || category.set(usize::MAX)),
            ));
        }
        let e = extra.get();
        let mut extras: Vec<(String, Clear)> = Vec::new();
        let mut extra_chip = |text: String, clear: Box<dyn Fn(&mut Extra)>| {
            extras.push((text, clear));
        };
        if let Some(k) = e.kind {
            extra_chip(
                format!(
                    "{}: {}",
                    st.tn("filter.kind"),
                    st.tn(&format!("editor.kind.{k}"))
                ),
                Box::new(|x| x.kind = None),
            );
        }
        if let Some(a) = e.active {
            extra_chip(
                st.tn(if a {
                    "filter.on_sale"
                } else {
                    "filter.stopped"
                }),
                Box::new(|x| x.active = None),
            );
        }
        if let Some(b) = e.barcode {
            extra_chip(
                st.tn(if b {
                    "filter.with_barcode"
                } else {
                    "filter.without_barcode"
                }),
                Box::new(|x| x.barcode = None),
            );
        }
        if let Some(i) = e.price {
            extra_chip(
                format!(
                    "{}: {}",
                    st.tn("products.col.price"),
                    price_range_text(st, i)
                ),
                Box::new(|x| x.price = None),
            );
        }
        if let Some((_, name)) = &e.brand {
            extra_chip(
                format!("{}: {name}", st.tn("filter.brand")),
                Box::new(|x| x.brand = None),
            );
        }
        if e.below_cost {
            extra_chip(
                st.tn("filter.below_cost"),
                Box::new(|x| x.below_cost = false),
            );
        }
        for (text, clear) in extras {
            row = row.child(chip(
                Box::new(move || text.clone()),
                Box::new(move || extra.update(|x| clear(x))),
            ));
        }
        if favs_only.get() {
            row = row.child(chip(
                Box::new(move || st.tn("products.favorites")),
                Box::new(move || favs_only.set(false)),
            ));
        }
        let q = query.get();
        if !q.trim().is_empty() {
            row = row.child(chip(
                Box::new(move || format!("“{}”", q.trim())),
                Box::new(move || query.set(String::new())),
            ));
        }
        row.child(button(
            Variant::Ghost,
            Size::Sm,
            None,
            st.t("table.clear_filters"),
            move || {
                query.set(String::new());
                filter.set(Filter::All);
                category.set(usize::MAX);
                favs_only.set(false);
                extra.set(Extra::default());
            },
        ))
        .into_element()
    });

    // ---------------------------------------------------------- table
    let build_columns = move || {
        let mut columns = vec![
            Column::new("name", st.t("products.col.name"), Width::Flex(3.0, 220.0)).sortable(),
        ];
        let h = hidden.get();
        if !compact && h & 1 == 0 {
            columns.push(
                Column::new(
                    "category",
                    st.t("products.col.category"),
                    Width::Flex(1.4, 140.0),
                )
                .sortable(),
            );
        }
        if !compact && h & 2 == 0 {
            columns.push(Column::new("status", st.t("col.status"), Width::Fixed(140.0)).sortable());
        }
        if !compact && h & 4 == 0 {
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
            columns.push(Column::new("actions", String::new, Width::Fixed(104.0)));
        }
        columns
    };

    let row_of = move |at: Position| computed(move || rows.with(|r| r.get(at.index()).cloned()));
    // One list of what may be done to a product: its ⋯ button and a right
    // click show the same.
    let row_acts: std::rc::Rc<dyn Fn(usize) -> Vec<crate::ui::menu::Act>> =
        std::rc::Rc::new(move |i: usize| {
            use crate::ui::menu::{Act, tsv};
            let Some(p) = rows.with_untracked(|v| v.get(i).cloned()) else {
                return Vec::new();
            };
            let (p1, p2, p3) = (p.id.clone(), p.id.clone(), p.clone());
            let gone = p.product_id().ok();
            let price = Money::from_minor(p.price_minor, currency()).to_plain_string();
            let mut acts = vec![
                Act::with(Icon::Pencil, st.tn("action.edit"), move || {
                    editor.set(Some(Target::Edit(p1.clone())))
                })
                .when(st.can("products.edit")),
                Act::with(Icon::Copy, st.tn("action.duplicate"), move || {
                    editor.set(Some(Target::Copy(p2.clone())))
                })
                .when(st.can("products.edit")),
                Act::with(Icon::Tag, st.tn("products.print_label"), move || {
                    labeling.set(Some(p3.clone()));
                    labeling_open.set(true);
                }),
                {
                    let (id, fav) = (p.id.clone(), p.favorite);
                    Act::with(
                        Icon::Star,
                        st.tn(if fav {
                            "products.unfavorite"
                        } else {
                            "products.favorite"
                        }),
                        move || crate::screens::pos::toggle_favorite(st, id.clone(), !fav),
                    )
                    .when(st.can("products.edit"))
                },
                Act::Line,
                Act::copy(st.tn("action.copy_name"), p.name.clone()),
                Act::copy(st.tn("action.copy_sku"), p.sku.clone()),
                Act::copy(st.tn("action.copy_price"), price.clone()),
            ];
            if let Some(b) = p.barcodes.first() {
                acts.push(Act::copy(st.tn("action.copy_barcode"), b.clone()));
            }
            acts.push(Act::copy(
                st.tn("action.copy_row"),
                tsv(&[product_cells(st, &p)]),
            ));
            acts.push(Act::Line);
            acts.push(
                Act::with(Icon::Trash, st.tn("action.delete"), move || {
                    if let Some(id) = gone {
                        archive(st, vec![id]);
                    }
                })
                .when(st.can("products.archive"))
                .danger(),
            );
            acts
        });
    let menu_acts = row_acts.clone();
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
                                        nb_i18n::bidi::isolate_runs(&p.name, st.lang().direction())
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
                                            format!(
                                                "{} · {}",
                                                p.sku,
                                                st.fmt().quantity(
                                                    Quantity::from_milli(p.on_hand_milli),
                                                    3
                                                )
                                            )
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
                move || p.get().and_then(|p| p.category).unwrap_or_default(),
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
                        .map(|p| st.fmt().quantity(Quantity::from_milli(p.on_hand_milli), 3))
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
                        .map(|p| st.fmt().money(Money::from_minor(p.price_minor, currency())))
                        .unwrap_or_default()
                },
                ty::SM,
                FontWeight::SEMIBOLD,
                || Tone::Ink,
            )
            .into_element(),
            _ => crate::ui::menu::row_actions(st, menu_acts(at.index()), "product-more")
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
            cell.clone(),
            "products-table",
            Some(row_acts.clone()),
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
                            favs_only.set(false);
                            extra.set(Extra::default());
                        },
                    )),
                ))
                .into_element()
        },
    )
    .otherwise(move || dynamic(grid.clone()).into_element());

    let importing = signal(false);
    let actions = if compact {
        view().child(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("products.add"),
                move || editor.set(Some(Target::New)),
            )
            .test_id("products-add"),
        )
    } else {
        view()
            .child(
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Upload),
                    st.t("action.import"),
                    move || importing.set(true),
                )
                .test_id("products-import"),
            )
            .child(crate::screens::common::export_button(
                st,
                "products-export",
                move || {
                    let mut q = ProductQuery {
                        text: query.get_untracked(),
                        category_id: match category.get_untracked() {
                            usize::MAX => None,
                            i => cat_list
                                .with_untracked(|c: &Vec<Category>| c.get(i).map(|c| c.id.clone())),
                        },
                        state: match filter.get_untracked() {
                            Filter::All => None,
                            Filter::Only(s) => Some(s.stored()),
                        },
                        favorites: favs_only.get_untracked(),
                        limit: 100_000,
                        with_counts: false,
                        ..ProductQuery::default()
                    };
                    extra.with_untracked(|e| e.apply(&mut q, currency_now()));
                    crate::screens::common::export_from_db(
                        st,
                        "products.file",
                        move |s| list_products(s, &q),
                        move |p: ProductPage| {
                            let mut out = vec![product_head(st)];
                            out.extend(p.rows.iter().map(|r| product_cells(st, r)));
                            out
                        },
                    );
                },
            ))
            .child(
                button(
                    Variant::Primary,
                    Size::Md,
                    Some(Icon::Plus),
                    st.t("products.add"),
                    move || editor.set(Some(Target::New)),
                )
                .test_id("products-add"),
            )
    };

    let list = view()
        .flex_1()
        .w_full()
        .p(px(page_pad(compact)))
        .gap(px(if compact { 14.0 } else { 20.0 }))
        .child(page_head(
            st,
            Route::Products,
            (!compact).then_some("products.subtitle"),
            actions,
            compact,
        ))
        .child(section_tabs(st, Route::Products))
        .child(quick_row)
        .child(
            card()
                .child(toolbar)
                .child(chips_row)
                .child(
                    view()
                        .border_t_1()
                        .border_color(Tone::Border.color())
                        .child(body),
                )
                .child(pager(st, paging, move || total.get(), compact)),
        )
        .child(wide_modal(importing, compact, move || {
            crate::screens::import::import_dialog(
                st,
                nb_domain::import::Kind::Products,
                move || importing.set(false),
            )
            .into_element()
        }))
        .child(modal(moving_open, compact, move || {
            move_dialog(st, cat_list, rows, selection, move || {
                moving_open.set(false)
            })
            .into_element()
        }))
        .child(modal(labeling_open, compact, move || {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(p) = labeling.get_untracked() {
                let item = crate::screens::printing::LabelItem {
                    name: p.name.clone(),
                    price_minor: p.price_minor,
                    unit: p.units.first().map(|u| u.code.clone()).unwrap_or_default(),
                    barcode: p.barcodes.first().cloned(),
                    sku: p.sku.clone(),
                };
                return crate::screens::printing::label_dialog(st, item, move || {
                    labeling_open.set(false)
                })
                .into_element();
            }
            view().into_element()
        }));
    // The editor takes the page while open; the list (and its filters)
    // waits underneath, as it was.
    view()
        .flex_1()
        // The list scrolls as a page (its table is as tall as its page of
        // rows), with the bar showing where the reader is.
        .child(
            scroll()
                .flex_1()
                .style_when(move || editor.get().is_some(), |s| s.hidden())
                .child(list),
        )
        // The scroll area stays put (a sized box); only its content changes.
        .child(
            scroll()
                .flex_1()
                .style_when(move || editor.get().is_none(), |s| s.hidden())
                .child(dynamic(move || match editor.get() {
                    Some(target) => {
                        product_editor(st, target, compact, move || editor.set(None)).into_element()
                    }
                    None => view().into_element(),
                })),
        )
}

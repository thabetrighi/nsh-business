//! Catalog pages around the products (M5.10): the category tree, the units,
//! the price lists, and the CSV import dialog. Data and rules come from the
//! catalog use cases; this file only shows and collects.

use nb_domain::units::{Dimension, STANDARD_UNITS, standard_factor};
use nb_i18n::tr_args;
use nb_store::usecases::catalog::{
    CategoryInput, CategoryRow, PriceListRow, archive_category, create_price_list, list_categories,
    list_price_lists, save_category,
};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::screens::common::page;
use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::form::field;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, label, label_line};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::popup::{Action, dropdown, row_menu};
use crate::ui::surface::{card, card_header_icon, empty_state};
use crate::ui::tokens::{Status, Tone, ty};

/// Shows the first rule broken by a failed call.
fn toast_error(st: AppState, e: &crate::db::DbError) {
    show_toast(
        st,
        error_text(st, e.codes.first().map_or("internal", |c| c.1)),
    );
}

// ---------------------------------------------------------------- categories

pub fn categories(st: AppState, compact: bool) -> View {
    let list = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_categories(s)).await }
        },
    );
    let rows = computed(move || {
        list.with(|l: Option<&Vec<CategoryRow>>| l.cloned())
            .unwrap_or_default()
    });
    // The dialog: what is edited (None: a new one) and its parent.
    let open = signal(false);
    let editing: Signal<Option<CategoryRow>> = signal(None);
    let parent_of_new: Signal<Option<String>> = signal(None);
    let start = move |row: Option<CategoryRow>, parent: Option<String>| {
        editing.set(row);
        parent_of_new.set(parent);
        open.set(true);
    };
    let add = button(
        Variant::Primary,
        Size::Md,
        Some(Icon::Plus),
        st.t("categories.add"),
        move || start(None, None),
    )
    .test_id("category-add");
    let body = dynamic(move || {
        let all = rows.get();
        if all.is_empty() {
            return card()
                .child(empty_state(
                    Icon::FolderTree,
                    st.t("categories.empty"),
                    st.t("categories.empty.body"),
                    None,
                ))
                .into_element();
        }
        let mut c = card().child(card_header_icon(
            Some(Icon::FolderTree),
            st.t("nav.categories"),
            None,
            view(),
        ));
        for (i, row) in all.iter().cloned().enumerate() {
            c = c.child(category_row(st, row, i, compact, start));
        }
        c.into_element()
    });
    let export = crate::screens::common::export_button(st, "categories-export", move || {
        let mut out = vec![vec![
            st.tn("categories.col.path"),
            st.tn("categories.col.products"),
        ]];
        for r in rows.get_untracked() {
            out.push(vec![r.path.join(" › "), r.products.to_string()]);
        }
        crate::screens::common::export_rows(st, "categories.file", out);
    });
    view()
        .child(page(
            st,
            Route::Categories,
            "categories.subtitle",
            crate::screens::common::actions_row(vec![export, add]),
            view().child(body),
            compact,
        ))
        .child(modal(open, compact, move || {
            category_form(
                st,
                rows.get_untracked(),
                editing.get_untracked(),
                parent_of_new.get_untracked(),
                move || open.set(false),
            )
            .into_element()
        }))
}

fn category_row(
    st: AppState,
    row: CategoryRow,
    i: usize,
    compact: bool,
    start: impl Fn(Option<CategoryRow>, Option<String>) + Copy + 'static,
) -> View {
    let indent = (row.depth.saturating_sub(1)) as f32 * if compact { 14.0 } else { 22.0 };
    let name = row.name.clone();
    let n = row.products;
    let id = row.id.clone();
    let edit_row = row.clone();
    let menu = row_menu(
        move || {
            let (e, sub, del) = (edit_row.clone(), id.clone(), id.clone());
            vec![
                Action::new(Icon::Pencil, st.tn("categories.edit"), move || {
                    start(Some(e.clone()), None)
                }),
                Action::new(Icon::Plus, st.tn("categories.add_sub"), move || {
                    start(None, Some(sub.clone()))
                }),
                Action::new(Icon::Package, st.tn("categories.products_of"), {
                    let id = del.clone();
                    move || {
                        st.intent
                            .set(Some(crate::state::Intent::ProductsIn(id.clone())));
                        st.route.set(Route::Products);
                    }
                }),
                Action::new(Icon::Trash, st.tn("categories.archive"), move || {
                    let Some(session) = st.session.get_untracked() else {
                        return;
                    };
                    let id = del.clone();
                    let db = st.db();
                    let task = spawn(async move {
                        match db
                            .call(move |s| {
                                archive_category(s, session.meta("category.archive"), &id)
                            })
                            .await
                        {
                            Ok(()) => st.changed(),
                            Err(e) => toast_error(st, &e),
                        }
                    });
                    drop(task); // runs to completion: tasks end with their owner, not their handle
                })
                .danger(),
            ]
        },
        st.t("action.row_actions"),
    );
    let (e2, s2, id2, n2) = (
        row.clone(),
        row.id.clone(),
        row.id.clone(),
        row.name.clone(),
    );
    let acts = move || {
        use crate::ui::menu::Act;
        let (e, s, id) = (e2.clone(), s2.clone(), id2.clone());
        vec![
            Act::with(Icon::Pencil, st.tn("categories.edit"), move || {
                start(Some(e.clone()), None)
            }),
            Act::with(Icon::PlusCircle, st.tn("categories.add_sub"), move || {
                start(None, Some(s.clone()))
            }),
            Act::with(Icon::Package, st.tn("categories.products_of"), move || {
                st.intent
                    .set(Some(crate::state::Intent::ProductsIn(id.clone())));
                st.route.set(Route::Products);
            }),
            Act::Line,
            Act::copy(st.tn("action.copy_name"), n2.clone()),
        ]
    };
    let row = view()
        .test_id(&format!("category-{i}"))
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(20.0))
        .py(px(10.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(view().w(px(indent)))
        .child(icon(
            if row.depth > 1 {
                Icon::SubItem
            } else {
                Icon::Folder
            },
            16.0,
            move || {
                if row.depth > 1 {
                    Tone::Faint
                } else {
                    Tone::Primary
                }
            },
        ))
        .child(view().flex_1().min_w(px(0.0)).child(label_line(
            move || name.clone(),
            ty::SM,
            if row.depth == 1 {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            },
            Tone::Ink,
        )))
        .child(badge(
            if n > 0 { Status::Info } else { Status::Neutral },
            None,
            move || {
                tr_args(
                    st.lang(),
                    "categories.products",
                    &[("count", &st.fmt().integer(i64::from(n)))],
                )
            },
        ))
        .child(menu);
    crate::ui::menu::with_menu(row, acts)
}

fn category_form(
    st: AppState,
    all: Vec<CategoryRow>,
    editing: Option<CategoryRow>,
    parent: Option<String>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let name = signal(editing.as_ref().map(|c| c.name.clone()).unwrap_or_default());
    let name_err = signal(String::new());
    let parent_err = signal(String::new());
    // Parent choices: top level, then every other category by its path.
    let own = editing.as_ref().map(|c| c.id.clone());
    let choices: Vec<(Option<String>, String)> = std::iter::once((None, String::new()))
        .chain(
            all.iter()
                .filter(|c| Some(&c.id) != own.as_ref())
                .map(|c| (Some(c.id.clone()), c.path.join(" › "))),
        )
        .collect();
    let current = editing
        .as_ref()
        .and_then(|c| c.parent_id.clone())
        .or(parent);
    let parent_idx = signal(
        choices
            .iter()
            .position(|(id, _)| *id == current)
            .unwrap_or(0),
    );
    let meta = session.meta("category.save");
    let save = {
        let choices = choices.clone();
        let close = close.clone();
        let id = own.clone();
        move || {
            name_err.set(String::new());
            parent_err.set(String::new());
            let input = CategoryInput {
                id: id.clone(),
                name: name.get_untracked(),
                parent_id: choices
                    .get(parent_idx.get_untracked())
                    .and_then(|c| c.0.clone()),
            };
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| save_category(s, meta, input)).await {
                    Ok(row) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "categories.saved",
                                &[("name", &row.path.join(" › "))],
                            ),
                        );
                    }
                    Err(e) => {
                        for (f, code) in &e.codes {
                            let target = if *f == Some("parent") {
                                parent_err
                            } else {
                                name_err
                            };
                            target.set(error_text(st, code));
                        }
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let body = view()
        .gap(px(16.0))
        .child(field(
            st.t("categories.name"),
            name,
            String::new,
            move || name_err.get(),
            KeyboardType::Default,
            "category-name",
        ))
        .child(
            view()
                .gap(px(6.0))
                .child(label(
                    st.t("categories.parent"),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(dropdown(
                    move || {
                        choices
                            .iter()
                            .enumerate()
                            .map(|(i, (id, path))| {
                                (
                                    i,
                                    if id.is_none() {
                                        st.tn("categories.top")
                                    } else {
                                        path.clone()
                                    },
                                )
                            })
                            .collect()
                    },
                    parent_idx,
                    Some(Icon::FolderTree),
                    st.t("categories.parent"),
                    "category-parent",
                ))
                .child(when(
                    move || !parent_err.get().is_empty(),
                    move || {
                        label(
                            move || parent_err.get(),
                            ty::XS,
                            FontWeight::MEDIUM,
                            Tone::Danger,
                        )
                        .into_element()
                    },
                )),
        );
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
                st.t("form.save"),
                save,
            )
            .test_id("category-save"),
        );
    dialog_frame(
        st,
        st.t(if editing.is_some() {
            "categories.edit"
        } else {
            "categories.add"
        }),
        st.t("categories.form.description"),
        body,
        footer,
        close,
    )
}

// ---------------------------------------------------------------- units

fn dimension_key(d: Dimension) -> &'static str {
    match d {
        Dimension::Count => "units.dim.count",
        Dimension::Weight => "units.dim.weight",
        Dimension::Volume => "units.dim.volume",
        Dimension::Length => "units.dim.length",
        Dimension::Area => "units.dim.area",
    }
}

pub fn units(st: AppState, compact: bool) -> View {
    let mut body = view().gap(px(20.0));
    for dim in [
        Dimension::Count,
        Dimension::Weight,
        Dimension::Volume,
        Dimension::Length,
        Dimension::Area,
    ] {
        let mut c = card().child(card_header_icon(
            Some(Icon::Ruler),
            st.t(dimension_key(dim)),
            None,
            view(),
        ));
        let group: Vec<_> = STANDARD_UNITS
            .iter()
            .filter(|u| u.dimension == dim)
            .collect();
        // The dimension's reference unit, to show each unit's size in it.
        let reference = group.iter().find(|u| u.reference == "1").map(|u| u.code);
        for u in group {
            let code = u.code;
            let precision = u.precision;
            let size = match (dim, reference) {
                (Dimension::Count, _) => None,
                (_, Some(r)) if r != code => standard_factor(code, r).map(|f| (f, r)),
                _ => None,
            };
            c = c.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(20.0))
                    .py(px(10.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(view().w(px(56.0)).child(label_line(
                        move || code.to_owned(),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Primary,
                    )))
                    .child(view().flex_1().min_w(px(0.0)).child(label_line(
                        move || st.tn(&format!("unit.{code}")),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Ink,
                    )))
                    .child(label(
                        move || match size {
                            Some((f, r)) => format!(
                                "1 {} = {} {}",
                                code,
                                nb_i18n::bidi::isolate_ltr(&f.to_string()),
                                r
                            ),
                            None => String::new(),
                        },
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    ))
                    .child(badge(Status::Neutral, None, move || {
                        tr_args(
                            st.lang(),
                            "units.decimals",
                            &[("count", &st.fmt().integer(i64::from(precision)))],
                        )
                    })),
            );
        }
        body = body.child(c);
    }
    page(
        st,
        Route::Units,
        "units.subtitle",
        view(),
        body.child(label(
            st.t("units.per_product"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        )),
        compact,
    )
}

// ---------------------------------------------------------------- price lists

pub fn price_lists(st: AppState, compact: bool) -> View {
    let lists = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_price_lists(s)).await }
        },
    );
    let open = signal(false);
    let add = button(
        Variant::Primary,
        Size::Md,
        Some(Icon::Plus),
        st.t("price_lists.add"),
        move || open.set(true),
    )
    .test_id("price-list-add");
    let body = dynamic(move || {
        let rows = lists
            .with(|l: Option<&Vec<PriceListRow>>| l.cloned())
            .unwrap_or_default();
        let mut c = card().child(card_header_icon(
            Some(Icon::Tags),
            st.t("nav.price_lists"),
            Some(Box::new(st.t("price_lists.help"))),
            view(),
        ));
        for r in rows {
            let standard = r.name.is_empty();
            let name = if standard {
                st.tn(&format!("price_list.{}", r.code))
            } else {
                r.name.clone()
            };
            let code = r.code.clone();
            c = c.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(20.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(icon(Icon::Tags, 16.0, || Tone::Primary))
                    .child(view().flex_1().min_w(px(0.0)).child(label_line(
                        move || name.clone(),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    )))
                    .child(label(
                        move || code.clone(),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Faint,
                    ))
                    .child(badge(
                        if standard {
                            Status::Accent
                        } else {
                            Status::Neutral
                        },
                        None,
                        st.t(if standard {
                            "price_lists.standard"
                        } else {
                            "users.custom"
                        }),
                    )),
            );
        }
        c.into_element()
    });
    view()
        .child(page(
            st,
            Route::PriceLists,
            "price_lists.subtitle",
            add,
            view().child(body),
            compact,
        ))
        .child(modal(open, compact, move || {
            let Some(session) = st.session.get_untracked() else {
                return view().into_element();
            };
            let name = signal(String::new());
            let err = signal(String::new());
            let meta = session.meta("price_list.create");
            let close = move || open.set(false);
            let save = move || {
                let n = name.get_untracked();
                let db = st.db();
                let task = spawn(async move {
                    match db.call(move |s| create_price_list(s, meta, &n)).await {
                        Ok(_) => {
                            st.changed();
                            open.set(false);
                        }
                        Err(e) => {
                            err.set(error_text(st, e.codes.first().map_or("internal", |c| c.1)))
                        }
                    }
                });
                drop(task); // runs to completion: tasks end with their owner, not their handle
            };
            dialog_frame(
                st,
                st.t("price_lists.add"),
                st.t("price_lists.help"),
                view().child(field(
                    st.t("price_lists.name"),
                    name,
                    String::new,
                    move || err.get(),
                    KeyboardType::Default,
                    "price-list-name",
                )),
                view()
                    .child(button(
                        Variant::Secondary,
                        Size::Md,
                        None,
                        st.t("form.cancel"),
                        close,
                    ))
                    .child(
                        button(
                            Variant::Primary,
                            Size::Md,
                            Some(Icon::Check),
                            st.t("form.save"),
                            save,
                        )
                        .test_id("price-list-save"),
                    ),
                close,
            )
            .into_element()
        }))
}

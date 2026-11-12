//! Inventory pages (M6.12): stock by product, the movement journal,
//! transfers between warehouses, and stock counts. They show and collect;
//! the inventory use cases do the work.

use nb_core::{Money, Quantity};
use nb_i18n::tr_args;
use nb_store::repo::catalog::{ProductRow, StockState};
use nb_store::repo::org::Warehouse;
use nb_store::usecases::inventory::{
    CountRow, ExpiringLot, MovementQuery, MovementView, StockPage, StockQuery, TransferInput,
    TransferLineInput, TransferRow, cancel_count, close_count, count_detail, expiring_lots,
    list_counts, list_transfers, movements, open_count, receive_transfer, record_count,
    send_transfer, stock_overview,
};
use nsh_core::element::{AnyElement, KeyboardType, Owner, View};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::nav::Route;
use crate::screens::common::{date_time, page};
use crate::screens::forms::error_text;
use crate::screens::picker::product_picker;
use crate::state::AppState;
use crate::ui::form::{field, labelled, row_input, search_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, figure, icon_button, label, label_line};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon, empty_state, kpi};
use crate::ui::table::{Column, Paging, Sort, Width, pager, table};
use crate::ui::tokens::{Status, Tone, radius, ty};

fn first_error(st: AppState, e: &DbError) -> String {
    error_text(st, e.codes.first().map_or("internal", |c| c.1))
}

fn qty_text(st: AppState, milli: i64, unit: &str) -> String {
    format!(
        "{} {unit}",
        st.fmt().quantity(Quantity::from_milli(milli), 3)
    )
}

fn money(st: AppState, minor: i64) -> String {
    let c = st
        .session
        .get()
        .map_or(nb_core::Currency::DZD, |s| s.currency);
    st.fmt().money(Money::from_minor(minor, c))
}

/// The organization's warehouses, for filters and choices.
fn warehouses(st: AppState) -> Memo<Vec<Warehouse>> {
    let r = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    s.read(|r| {
                        let o = nb_store::repo::org::the_org(r.conn())?
                            .ok_or(nb_store::Error::NotFound("organization"))?;
                        nb_store::repo::org::warehouses(r.conn(), o.org_id()?)
                    })
                })
                .await
            }
        },
    );
    computed(move || {
        r.with(|w: Option<&Vec<Warehouse>>| w.cloned())
            .unwrap_or_default()
    })
}

/// A warehouse filter: "all" (usize::MAX) or one.
fn warehouse_filter(
    st: AppState,
    list: Memo<Vec<Warehouse>>,
    chosen: Signal<usize>,
    test_id: &'static str,
) -> View {
    dropdown(
        move || {
            std::iter::once((usize::MAX, st.tn("inventory.all_warehouses")))
                .chain(list.get().into_iter().enumerate().map(|(i, w)| (i, w.name)))
                .collect()
        },
        chosen,
        Some(Icon::Warehouse),
        st.t("inventory.warehouse"),
        test_id,
    )
}

fn state_status(s: StockState) -> (Status, &'static str) {
    match s {
        StockState::Ok => (Status::Success, "status.in_stock"),
        StockState::Low => (Status::Warning, "status.low"),
        StockState::Out => (Status::Danger, "status.out"),
    }
}

// ------------------------------------------------------------------ stock

pub fn stock_page(st: AppState, compact: bool) -> View {
    let whs = warehouses(st);
    let chosen = signal(usize::MAX);
    let text = signal(String::new());
    let state: Signal<Option<StockState>> = signal(None);
    let sort = signal(None::<Sort>);
    let paging = Paging::new(25);
    effect(move || {
        chosen.with(|_| ());
        text.with(|_| ());
        state.with(|_| ());
        paging.reset();
    });
    let page_data = resource(
        move || {
            st.session.get()?;
            let wh = whs.with(|w| w.get(chosen.get()).map(|w| w.id.clone()));
            Some((
                StockQuery {
                    warehouse_id: wh,
                    text: text.get(),
                    state: state.get(),
                    limit: paging.size.get(),
                    offset: paging.offset(),
                },
                st.data_rev.get(),
            ))
        },
        move |(q, _), _| {
            let db = st.db();
            async move { db.call(move |s| stock_overview(s, &q)).await }
        },
    );
    let data = computed(move || {
        page_data
            .with(|p: Option<&StockPage>| p.cloned())
            .unwrap_or_default()
    });
    let kpis = {
        let k = move |name: &'static str, value: Box<dyn Fn() -> String>| {
            kpi(st.t(name), value, None, view(), compact)
        };
        let row = view().flex_row().flex_wrap().gap(px(16.0));
        let cell = move |v: View| {
            if compact {
                v.w(relative(0.47))
            } else {
                v.flex_1()
            }
        };
        row.child(cell(k(
            "inventory.value",
            Box::new(move || money(st, data.get().value_minor)),
        )))
        .child(cell(k(
            "inventory.items",
            Box::new(move || st.fmt().integer(i64::from(data.get().counts[0]))),
        )))
        .child(cell(k(
            "status.low",
            Box::new(move || st.fmt().integer(i64::from(data.get().counts[2]))),
        )))
        .child(cell(k(
            "status.out",
            Box::new(move || st.fmt().integer(i64::from(data.get().counts[3]))),
        )))
    };
    let mut chips = view().flex_row().flex_wrap().gap(px(8.0));
    for (s, key) in [
        (None, "filter.all"),
        (Some(StockState::Low), "status.low"),
        (Some(StockState::Out), "status.out"),
    ] {
        let active = computed(move || state.get() == s);
        chips = chips.child(
            view()
                .h(px(32.0))
                .px(px(12.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .cursor_pointer()
                .border_color(move || {
                    if active.get() {
                        Tone::Primary.color()
                    } else {
                        Tone::Border.color()
                    }
                })
                .bg(move || {
                    if active.get() {
                        Tone::PrimarySoft.color()
                    } else {
                        Tone::Surface.color()
                    }
                })
                .on_press(move || state.set(s))
                .child(label_line(st.t(key), ty::SM, FontWeight::MEDIUM, Tone::Ink)),
        );
    }
    let toolbar = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
        .p(px(12.0))
        .child(view().flex_1().min_w(px(200.0)).child(search_field(
            text,
            st.t("products.search"),
            "stock-search",
        )))
        .child(
            view()
                .w(px(if compact { 160.0 } else { 220.0 }))
                .child(warehouse_filter(st, whs, chosen, "stock-warehouse")),
        )
        .child(chips);
    let rows = computed(move || data.get().rows);
    let cell = move |at: Position, key: &'static str| -> AnyElement {
        let r = computed(move || rows.with(|v| v.get(at.index()).cloned()));
        let text_cell = move |f: Box<dyn Fn() -> String>, tone: Tone| {
            label_line(f, ty::SM, FontWeight::NORMAL, tone).into_element()
        };
        match key {
            "name" => view()
                .gap(px(2.0))
                .child(label_line(
                    move || r.get().map(|r| r.name).unwrap_or_default(),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    move || r.get().map(|r| r.sku).unwrap_or_default(),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .into_element(),
            "category" => text_cell(
                Box::new(move || r.get().and_then(|r| r.category).unwrap_or_default()),
                Tone::Body,
            ),
            "reserved" => text_cell(
                Box::new(move || {
                    r.get()
                        .map(|r| {
                            if r.reserved_milli == 0 {
                                "—".into()
                            } else {
                                qty_text(st, r.reserved_milli, &r.unit)
                            }
                        })
                        .unwrap_or_default()
                }),
                Tone::Muted,
            ),
            "available" => text_cell(
                Box::new(move || {
                    r.get()
                        .map(|r| qty_text(st, r.available_milli, &r.unit))
                        .unwrap_or_default()
                }),
                Tone::Body,
            ),
            "value" => text_cell(
                Box::new(move || {
                    r.get()
                        .map(|r| money(st, r.value_minor))
                        .unwrap_or_default()
                }),
                Tone::Body,
            ),
            "on_hand" => figure(
                move || {
                    r.get()
                        .map(|r| qty_text(st, r.on_hand_milli, &r.unit))
                        .unwrap_or_default()
                },
                ty::SM,
                FontWeight::SEMIBOLD,
                move || {
                    if r.get().is_some_and(|r| r.on_hand_milli < 0) {
                        Tone::Danger
                    } else {
                        Tone::Ink
                    }
                },
            )
            .into_element(),
            _ => dynamic(move || {
                let (s, k) = state_status(r.get().map_or(StockState::Ok, |r| r.state));
                badge(s, None, st.t(k)).into_element()
            })
            .into_element(),
        }
    };
    let grid = dynamic(move || {
        if rows.with(Vec::is_empty) {
            return empty_state(
                Icon::Boxes,
                st.t("inventory.empty"),
                st.t("inventory.empty.body"),
                None,
            )
            .into_element();
        }
        table(
            columns_again(st, compact),
            move || rows.with(Vec::len),
            sort,
            None,
            move |i| Some(i as u64),
            cell,
            "stock-table",
            Some(std::rc::Rc::new(move |i: usize| {
                use crate::ui::menu::{Act, tsv};
                let Some(r) = rows.with_untracked(|v| v.get(i).cloned()) else {
                    return Vec::new();
                };
                vec![
                    Act::copy(st.tn("action.copy_name"), r.name.clone()),
                    Act::copy(st.tn("action.copy_sku"), r.sku.clone()),
                    Act::copy(
                        st.tn("action.copy_row"),
                        tsv(&[vec![
                            r.sku.clone(),
                            r.name.clone(),
                            r.category.clone().unwrap_or_default(),
                            qty_text(st, r.on_hand_milli, &r.unit),
                            qty_text(st, r.available_milli, &r.unit),
                            money(st, r.value_minor),
                        ]]),
                    ),
                ]
            })),
        )
        .into_element()
    });
    let body = view()
        .gap(px(16.0))
        .child(kpis)
        .child(expiry_card(st))
        .child(
            card()
                .child(toolbar)
                .child(
                    view()
                        .border_t_1()
                        .border_color(Tone::Border.color())
                        .child(grid),
                )
                .child(pager(st, paging, move || data.get().total, compact)),
        );
    page(
        st,
        Route::Inventory,
        "inventory.subtitle",
        view(),
        body,
        compact,
    )
}

/// The table's columns (built again for each render of the table).
fn columns_again(st: AppState, compact: bool) -> Vec<Column> {
    let mut columns = vec![Column::new(
        "name",
        st.t("products.col.name"),
        Width::Flex(3.0, 200.0),
    )];
    if !compact {
        columns.push(Column::new(
            "category",
            st.t("products.col.category"),
            Width::Flex(1.2, 120.0),
        ));
        columns
            .push(Column::new("reserved", st.t("inventory.reserved"), Width::Fixed(110.0)).end());
        columns.push(
            Column::new(
                "available",
                st.t("inventory.available"),
                Width::Fixed(120.0),
            )
            .end(),
        );
        columns.push(Column::new("value", st.t("inventory.value"), Width::Fixed(130.0)).end());
    }
    columns.push(Column::new("on_hand", st.t("inventory.on_hand"), Width::Fixed(120.0)).end());
    columns.push(Column::new(
        "status",
        st.t("col.status"),
        Width::Fixed(if compact { 96.0 } else { 120.0 }),
    ));
    columns
}

// ------------------------------------------------------------------ movements

const KINDS: [&str; 12] = [
    "opening",
    "purchase",
    "sale",
    "sale_return",
    "purchase_return",
    "adjust_in",
    "adjust_out",
    "transfer_out",
    "transfer_in",
    "count_gain",
    "count_loss",
    "waste",
];

pub fn movements_page(st: AppState, compact: bool) -> View {
    let whs = warehouses(st);
    let chosen = signal(usize::MAX);
    let kind = signal(usize::MAX);
    let list = resource(
        move || {
            st.session.get()?;
            let wh = whs.with(|w| w.get(chosen.get()).map(|w| w.id.clone()));
            let kinds = KINDS
                .get(kind.get())
                .map(|k| vec![(*k).to_owned()])
                .unwrap_or_default();
            Some((
                MovementQuery {
                    warehouse_id: wh,
                    product_id: None,
                    kinds,
                    limit: 500,
                },
                st.data_rev.get(),
            ))
        },
        move |(q, _), _| {
            let db = st.db();
            async move { db.call(move |s| movements(s, &q)).await }
        },
    );
    let rows = computed(move || {
        list.with(|l: Option<&Vec<MovementView>>| l.cloned())
            .unwrap_or_default()
    });
    let filters = view()
        .flex_row()
        .flex_wrap()
        .gap(px(10.0))
        .p(px(12.0))
        .child(
            view()
                .w(px(220.0))
                .child(warehouse_filter(st, whs, chosen, "movement-warehouse")),
        )
        .child(view().w(px(220.0)).child(dropdown(
            move || {
                std::iter::once((usize::MAX, st.tn("inventory.all_kinds")))
                    .chain(
                        KINDS
                            .iter()
                            .enumerate()
                            .map(|(i, k)| (i, st.tn(&format!("movement.{k}")))),
                    )
                    .collect()
            },
            kind,
            Some(Icon::ArrowsLR),
            st.t("inventory.kind"),
            "movement-kind",
        )));
    let list_view = dynamic(move || {
        let all = rows.get();
        if all.is_empty() {
            return empty_state(
                Icon::ArrowsLR,
                st.t("inventory.no_movements"),
                st.t("inventory.no_movements.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for m in all {
            let positive = m.qty_milli > 0;
            let q = qty_text(st, m.qty_milli.abs(), &m.unit);
            let sign = if positive { "+" } else { "−" };
            let (name, kind, doc, wh, at) = (
                m.name.clone(),
                m.kind.clone(),
                m.doc_number.clone().unwrap_or_default(),
                m.warehouse.clone(),
                m.created_at,
            );
            let kind_status = if positive {
                Status::Success
            } else {
                Status::Neutral
            };
            let mut row = view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .px(px(16.0))
                .py(px(10.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(icon(
                    if positive {
                        Icon::ArrowDown
                    } else {
                        Icon::ArrowUp
                    },
                    16.0,
                    move || if positive { Tone::Success } else { Tone::Muted },
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(2.0))
                        .child(label_line(
                            move || name.clone(),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            move || format!("{} · {} · {}", date_time(st, at), doc, wh),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                );
            if !compact {
                row = row.child(badge(kind_status, None, move || {
                    st.tn(&format!("movement.{kind}"))
                }));
            }
            row = row.child(view().w(px(120.0)).items_end().child(label_line(
                move || format!("{sign}{q}"),
                ty::SM,
                FontWeight::SEMIBOLD,
                if positive { Tone::Success } else { Tone::Ink },
            )));
            v = v.child(row);
        }
        v.into_element()
    });
    let body = card().child(filters).child(list_view);
    page(
        st,
        Route::Movements,
        "movements.subtitle",
        view(),
        body,
        compact,
    )
}

// ------------------------------------------------------------------ transfers

#[derive(Clone, Copy, PartialEq)]
struct Line {
    product: Signal<Option<ProductRow>>,
    qty: Signal<String>,
}

pub fn transfers_page(st: AppState, compact: bool) -> View {
    let list = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_transfers(s, 200)).await }
        },
    );
    let creating = signal(false);
    let receiving: Signal<Option<TransferRow>> = signal(None);
    let receiving_open = signal(false);
    effect(move || receiving_open.set(receiving.get().is_some()));
    effect(move || {
        if !receiving_open.get() {
            receiving.set(None);
        }
    });
    let add = button(
        Variant::Primary,
        Size::Md,
        Some(Icon::Plus),
        st.t("transfers.new"),
        move || creating.set(true),
    )
    .test_id("transfer-new");
    let body = dynamic(move || {
        let all = list
            .with(|l: Option<&Vec<TransferRow>>| l.cloned())
            .unwrap_or_default();
        if all.is_empty() {
            return card()
                .child(empty_state(
                    Icon::Truck,
                    st.t("transfers.empty"),
                    st.t("transfers.empty.body"),
                    None,
                ))
                .into_element();
        }
        let mut c = card();
        for (i, t) in all.into_iter().enumerate() {
            let sent = t.status == "sent";
            let (number, route, at, n) = (
                t.number.clone(),
                format!("{} → {}", t.from_name, t.to_name),
                t.sent_at,
                t.lines.len(),
            );
            let mut row = view()
                .test_id(&format!("transfer-{i}"))
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .px(px(20.0))
                .py(px(12.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(icon(Icon::Truck, 18.0, move || {
                    if sent { Tone::Warning } else { Tone::Success }
                }))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(2.0))
                        .child(label_line(
                            move || format!("{number} · {route}"),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            move || {
                                format!(
                                    "{} · {}",
                                    date_time(st, at),
                                    tr_args(
                                        st.lang(),
                                        "transfers.lines",
                                        &[("count", &st.fmt().integer(n as i64))]
                                    )
                                )
                            },
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .child(badge(
                    if sent {
                        Status::Warning
                    } else {
                        Status::Success
                    },
                    None,
                    st.t(if sent {
                        "transfers.in_transit"
                    } else {
                        "transfers.received"
                    }),
                ));
            if sent {
                row = row.child(
                    button(
                        Variant::Secondary,
                        Size::Sm,
                        Some(Icon::Check),
                        st.t("transfers.receive"),
                        move || receiving.set(Some(t.clone())),
                    )
                    .test_id(&format!("transfer-receive-{i}")),
                );
            }
            c = c.child(row);
        }
        c.into_element()
    });
    view()
        .child(page(
            st,
            Route::Transfers,
            "transfers.subtitle",
            add,
            view().child(body),
            compact,
        ))
        .child(modal(creating, compact, move || {
            new_transfer(st, move || creating.set(false)).into_element()
        }))
        .child(modal(receiving_open, compact, move || {
            match receiving.get_untracked() {
                Some(t) => receive_dialog(st, t, move || receiving_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
}

fn new_transfer(st: AppState, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let owner = Owner::current();
    let whs = warehouses(st);
    let from = signal(0usize);
    let to = signal(1usize);
    let note = signal(String::new());
    let lines: Signal<Vec<Line>> = signal(Vec::new());
    let error = signal(String::new());
    let meta = session.meta("transfer.send");
    let pick = move |p: ProductRow| {
        let l = owner.run(|| Line {
            product: signal(Some(p)),
            qty: signal("1".into()),
        });
        lines.update(|v| v.push(l));
    };
    let save = {
        let close = close.clone();
        move || {
            let w = whs.get_untracked();
            let (Some(a), Some(b)) = (w.get(from.get_untracked()), w.get(to.get_untracked()))
            else {
                return;
            };
            let input = TransferInput {
                from_warehouse: a.id.clone(),
                to_warehouse: b.id.clone(),
                lines: lines
                    .get_untracked()
                    .iter()
                    .filter_map(|l| {
                        l.product.get_untracked().map(|p| TransferLineInput {
                            product_id: p.id,
                            qty: l.qty.get_untracked(),
                            unit: String::new(),
                            tracked: Default::default(),
                        })
                    })
                    .collect(),
                note: note.get_untracked(),
            };
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| send_transfer(s, meta, input)).await {
                    Ok(t) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "transfers.sent", &[("number", &t.number)]),
                        );
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let wh_choice = move |sig: Signal<usize>, key: &'static str, id: &'static str| {
        labelled(
            st.t(key),
            dropdown(
                move || {
                    whs.get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, w)| (i, w.name))
                        .collect()
                },
                sig,
                Some(Icon::Warehouse),
                st.t(key),
                id,
            ),
            None,
        )
    };
    let rows = dynamic(move || {
        let mut v = view().gap(px(8.0));
        for (i, l) in lines.get().into_iter().enumerate() {
            let p = l.product.get_untracked();
            let (name, unit) = p
                .map(|p| {
                    (
                        p.name,
                        p.units.first().map(|u| u.code.clone()).unwrap_or_default(),
                    )
                })
                .unwrap_or_default();
            v = v.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(view().flex_1().min_w(px(0.0)).child(label_line(
                        move || name.clone(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Ink,
                    )))
                    .child(view().w(px(100.0)).child(row_input(
                        l.qty,
                        String::new,
                        KeyboardType::Decimal,
                        format!("transfer-qty-{i}"),
                        || false,
                    )))
                    .child(label_line(
                        move || unit.clone(),
                        ty::SM,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    ))
                    .child(icon_button(
                        || Icon::Trash,
                        st.t("action.delete"),
                        Variant::Ghost,
                        move || {
                            lines.update(|v| {
                                if i < v.len() {
                                    v.remove(i);
                                }
                            })
                        },
                    )),
            );
        }
        v.into_element()
    });
    let body = view()
        .gap(px(16.0))
        .child(
            view()
                .flex_row()
                .gap(px(12.0))
                .child(
                    view()
                        .flex_1()
                        .child(wh_choice(from, "transfers.from", "transfer-from")),
                )
                .child(
                    view()
                        .flex_1()
                        .child(wh_choice(to, "transfers.to", "transfer-to")),
                ),
        )
        .child(labelled(
            st.t("transfers.add_item"),
            product_picker(st, "transfer-search", pick),
            None,
        ))
        .child(rows)
        .child(field(
            st.t("transfers.note"),
            note,
            String::new,
            String::new,
            KeyboardType::Default,
            "transfer-note",
        ))
        .child(when(
            move || !error.get().is_empty(),
            move || {
                label(
                    move || error.get(),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                )
                .into_element()
            },
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
                Some(Icon::Truck),
                st.t("transfers.send"),
                save,
            )
            .test_id("transfer-send"),
        );
    dialog_frame(
        st,
        st.t("transfers.new"),
        st.t("transfers.new.help"),
        body,
        footer,
        close,
    )
}

fn receive_dialog(st: AppState, t: TransferRow, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let meta = session.meta("transfer.receive");
    let error = signal(String::new());
    let got: Vec<(i64, Signal<String>)> = t
        .lines
        .iter()
        .map(|l| {
            (
                l.position,
                signal(Quantity::from_milli(l.sent_milli).to_string()),
            )
        })
        .collect();
    let mut rows = view().gap(px(10.0));
    for (l, (_, sig)) in t.lines.iter().zip(got.iter()) {
        let name = l.name.clone();
        let sent = qty_text(st, l.sent_milli, &l.unit);
        let unit = l.unit.clone();
        rows = rows.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label_line(
                            move || name.clone(),
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            move || tr_args(st.lang(), "transfers.sent_qty", &[("qty", &sent)]),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .child(view().w(px(100.0)).child(row_input(
                    *sig,
                    String::new,
                    KeyboardType::Decimal,
                    format!("receive-qty-{}", l.position),
                    || false,
                )))
                .child(label_line(
                    move || unit.clone(),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        );
    }
    let id = t.id.clone();
    let save = {
        let close = close.clone();
        move || {
            let received: Vec<(i64, String)> =
                got.iter().map(|(p, s)| (*p, s.get_untracked())).collect();
            let id = id.clone();
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db
                    .call(move |s| receive_transfer(s, meta, &id, received))
                    .await
                {
                    Ok(t) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "transfers.received_toast",
                                &[("number", &t.number)],
                            ),
                        );
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let body = view()
        .gap(px(14.0))
        .child(label(
            st.t("transfers.receive.help"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(rows)
        .child(when(
            move || !error.get().is_empty(),
            move || {
                label(
                    move || error.get(),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                )
                .into_element()
            },
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
                st.t("transfers.receive"),
                save,
            )
            .test_id("transfer-receive-save"),
        );
    let title = t.number.clone();
    dialog_frame(
        st,
        move || title.clone(),
        st.t("transfers.receive.title"),
        body,
        footer,
        close,
    )
}

// ------------------------------------------------------------------ counts

pub fn counts_page(st: AppState, compact: bool) -> View {
    let list = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_counts(s, 200)).await }
        },
    );
    let whs = warehouses(st);
    let opening = signal(false);
    let current: Signal<Option<String>> = signal(None);
    let start = button(
        Variant::Primary,
        Size::Md,
        Some(Icon::ClipboardCheck),
        st.t("counts.open"),
        move || opening.set(true),
    )
    .test_id("count-open");
    let listing = dynamic(move || {
        let all = list
            .with(|l: Option<&Vec<CountRow>>| l.cloned())
            .unwrap_or_default();
        if all.is_empty() {
            return card()
                .child(empty_state(
                    Icon::ClipboardCheck,
                    st.t("counts.empty"),
                    st.t("counts.empty.body"),
                    None,
                ))
                .into_element();
        }
        let mut c = card();
        for (i, row) in all.into_iter().enumerate() {
            let (status, key) = match row.status.as_str() {
                "open" => (Status::Info, "counts.status.open"),
                "closed" => (Status::Success, "counts.status.closed"),
                _ => (Status::Neutral, "counts.status.cancelled"),
            };
            let id = row.id.clone();
            let title = format!("{} · {}", row.number, row.warehouse);
            let (at, n) = (row.opened_at, row.counted);
            c = c.child(
                view()
                    .test_id(&format!("count-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(20.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .on_press(move || current.set(Some(id.clone())))
                    .child(icon(Icon::ClipboardCheck, 18.0, || Tone::Primary))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap(px(2.0))
                            .child(label_line(
                                move || title.clone(),
                                ty::SM,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            ))
                            .child(label_line(
                                move || {
                                    format!(
                                        "{} · {}",
                                        date_time(st, at),
                                        tr_args(
                                            st.lang(),
                                            "counts.counted",
                                            &[("count", &st.fmt().integer(i64::from(n)))]
                                        )
                                    )
                                },
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(badge(status, None, st.t(key))),
            );
        }
        c.into_element()
    });
    let open_dialog = move || {
        let Some(session) = st.session.get_untracked() else {
            return view().into_element();
        };
        let chosen = signal(0usize);
        let error = signal(String::new());
        let meta = session.meta("count.open");
        let close = move || opening.set(false);
        let save = move || {
            let Some(w) = whs.get_untracked().get(chosen.get_untracked()).cloned() else {
                return;
            };
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(move |s| open_count(s, meta, nb_store::repo::parse_id(&w.id)?, None))
                    .await;
                match r {
                    Ok(c) => {
                        st.changed();
                        opening.set(false);
                        current.set(Some(c.id));
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        };
        dialog_frame(
            st,
            st.t("counts.open"),
            st.t("counts.open.help"),
            view()
                .gap(px(12.0))
                .child(labelled(
                    st.t("inventory.warehouse"),
                    dropdown(
                        move || {
                            whs.get()
                                .into_iter()
                                .enumerate()
                                .map(|(i, w)| (i, w.name))
                                .collect()
                        },
                        chosen,
                        Some(Icon::Warehouse),
                        st.t("inventory.warehouse"),
                        "count-warehouse",
                    ),
                    None,
                ))
                .child(when(
                    move || !error.get().is_empty(),
                    move || {
                        label(
                            move || error.get(),
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::Danger,
                        )
                        .into_element()
                    },
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
                        st.t("counts.start"),
                        save,
                    )
                    .test_id("count-start"),
                ),
            close,
        )
        .into_element()
    };
    let list_page = page(
        st,
        Route::StockCounts,
        "counts.subtitle",
        start,
        view().child(listing),
        compact,
    );
    view()
        .child(
            view()
                .style_when(move || current.get().is_some(), |s| s.hidden())
                .child(list_page),
        )
        .child(dynamic(move || match current.get() {
            Some(id) => count_sheet(st, id, compact, move || current.set(None)).into_element(),
            None => view().into_element(),
        }))
        .child(modal(opening, compact, open_dialog))
}

/// One count: add what was counted (search or scan), see the differences,
/// then close (books them) or cancel.
fn count_sheet(st: AppState, id: String, compact: bool, back: impl Fn() + Clone + 'static) -> View {
    let detail = {
        let id = id.clone();
        resource(
            move || st.session.get().map(|_| st.data_rev.get()),
            move |_, _| {
                let db = st.db();
                let id = id.clone();
                async move { db.call(move |s| count_detail(s, &id)).await }
            },
        )
    };
    let c = computed(move || detail.with(|d: Option<&CountRow>| d.cloned()));
    let qty = signal("1".to_owned());
    let error = signal(String::new());
    // The pick happens in a result row that goes away with it: the task
    // belongs to this sheet instead, or it would end with the row.
    let sheet = Owner::current();
    let record = {
        let id = id.clone();
        move |p: ProductRow| {
            let Some(session) = st.session.get_untracked() else {
                return;
            };
            let (id, q) = (id.clone(), qty.get_untracked());
            let db = st.db();
            let task = sheet.run(|| {
                spawn(async move {
                    let r = db
                        .call(move |s| {
                            record_count(
                                s,
                                session.meta("count.record"),
                                &id,
                                nb_store::repo::parse_id(&p.id)?,
                                &q,
                                "",
                            )
                        })
                        .await;
                    match r {
                        Ok(_) => {
                            error.set(String::new());
                            st.changed();
                        }
                        Err(e) => error.set(first_error(st, &e)),
                    }
                })
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let finish = {
        let id = id.clone();
        let back = back.clone();
        move |cancel: bool| {
            let Some(session) = st.session.get_untracked() else {
                return;
            };
            let id = id.clone();
            let back = back.clone();
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(move |s| {
                        if cancel {
                            cancel_count(s, session.meta("count.cancel"), &id).map(|_| None)
                        } else {
                            close_count(s, session.meta("count.close"), &id, None).map(Some)
                        }
                    })
                    .await;
                match r {
                    Ok(out) => {
                        st.changed();
                        back();
                        if let Some(o) = out {
                            show_toast(
                                st,
                                tr_args(
                                    st.lang(),
                                    "counts.closed_toast",
                                    &[("count", &st.fmt().integer(i64::from(o.adjusted)))],
                                ),
                            );
                        }
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let open = computed(move || c.get().is_some_and(|c| c.status == "open"));
    let close_btn = {
        let finish = finish.clone();
        button(
            Variant::Primary,
            Size::Md,
            Some(Icon::Check),
            st.t("counts.close"),
            move || finish(false),
        )
        .test_id("count-close")
    };
    let cancel_btn = button(
        Variant::Secondary,
        Size::Md,
        None,
        st.t("counts.cancel"),
        move || finish(true),
    );
    let header = view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .child(
            icon_button(
                move || {
                    if st.rtl() {
                        Icon::ArrowForward
                    } else {
                        Icon::ArrowBack
                    }
                },
                st.t("lock.back"),
                Variant::Secondary,
                back,
            )
            .test_id("count-back"),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(label_line(
                    st.t("nav.stock_counts"),
                    ty::XS,
                    FontWeight::SEMIBOLD,
                    Tone::Primary,
                ))
                .child(label_line(
                    move || {
                        c.get()
                            .map(|c| format!("{} · {}", c.number, c.warehouse))
                            .unwrap_or_default()
                    },
                    ty::H2,
                    FontWeight::BOLD,
                    Tone::Ink,
                )),
        )
        .child(when(open, move || {
            view()
                .flex_row()
                .gap(px(8.0))
                .child(cancel_btn.clone())
                .child(close_btn.clone())
                .into_element()
        }));
    let entry = when(open, move || {
        let record = record.clone();
        card()
            .child(card_header_icon(
                Some(Icon::Barcode),
                st.t("counts.enter"),
                Some(Box::new(st.t("counts.enter.help"))),
                view(),
            ))
            .child(
                view()
                    .flex_row()
                    .items_start()
                    .gap(px(12.0))
                    .p(px(20.0))
                    .child(view().w(px(110.0)).child(field(
                        st.t("counts.qty"),
                        qty,
                        || "1".into(),
                        String::new,
                        KeyboardType::Decimal,
                        "count-qty",
                    )))
                    .child(view().flex_1().min_w(px(0.0)).child(labelled(
                        st.t("counts.product"),
                        product_picker(st, "count-search", record),
                        None,
                    ))),
            )
            .into_element()
    });
    let lines = dynamic(move || {
        let Some(c) = c.get() else {
            return view().into_element();
        };
        let mut card_v = card().child(card_header_icon(
            Some(Icon::ClipboardList),
            st.t("counts.lines"),
            Some(Box::new(move || {
                tr_args(
                    st.lang(),
                    "counts.progress",
                    &[("count", &st.fmt().integer(i64::from(c.counted)))],
                )
            })),
            view(),
        ));
        for (i, l) in c
            .lines
            .into_iter()
            .filter(|l| l.counted_milli.is_some())
            .enumerate()
        {
            let counted = l.counted_milli.unwrap_or(0);
            let diff = counted - l.book_milli;
            let (name, unit) = (l.name.clone(), l.unit.clone());
            let book = qty_text(st, l.book_milli, &l.unit);
            let counted_t = qty_text(st, counted, &l.unit);
            let diff_t = if diff == 0 {
                "=".to_owned()
            } else {
                format!(
                    "{}{}",
                    if diff > 0 { "+" } else { "−" },
                    qty_text(st, diff.abs(), &unit)
                )
            };
            card_v = card_v.child(
                view()
                    .test_id(&format!("count-line-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(20.0))
                    .py(px(10.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(view().flex_1().min_w(px(0.0)).child(label_line(
                        move || name.clone(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Ink,
                    )))
                    .child(
                        view()
                            .w(px(if compact { 80.0 } else { 120.0 }))
                            .items_end()
                            .child(label_line(
                                move || book.clone(),
                                ty::SM,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(
                        view()
                            .w(px(if compact { 80.0 } else { 120.0 }))
                            .items_end()
                            .child(label_line(
                                move || counted_t.clone(),
                                ty::SM,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            )),
                    )
                    .child(
                        view()
                            .w(px(if compact { 80.0 } else { 120.0 }))
                            .items_end()
                            .child(label_line(
                                move || diff_t.clone(),
                                ty::SM,
                                FontWeight::SEMIBOLD,
                                if diff == 0 {
                                    Tone::Muted
                                } else if diff > 0 {
                                    Tone::Success
                                } else {
                                    Tone::Danger
                                },
                            )),
                    ),
            );
        }
        card_v.into_element()
    });
    view()
        .w_full()
        .p(px(crate::ui::surface::page_pad(compact)))
        .gap(px(18.0))
        .child(header)
        .child(when(
            move || !error.get().is_empty(),
            move || {
                view()
                    .p(px(12.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::DangerSoft.color())
                    .child(label(
                        move || error.get(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Danger,
                    ))
                    .into_element()
            },
        ))
        .child(entry)
        .child(lines)
}

/// Lots in stock that expire within 30 days, or have expired (M6.11);
/// hidden when there are none.
fn expiry_card(st: AppState) -> View {
    let lots = resource(
        move || {
            st.session.get()?;
            Some(st.data_rev.get())
        },
        move |_, _| {
            let db = st.db();
            async move { db.call(move |s| expiring_lots(s, 30)).await }
        },
    );
    let list = computed(move || {
        lots.with(|l: Option<&Vec<ExpiringLot>>| l.cloned())
            .unwrap_or_default()
    });
    let card = dynamic(move || {
        let items = list.get();
        if items.is_empty() {
            return view().into_element();
        }
        let mut rows = view();
        for (i, l) in items.into_iter().take(8).enumerate() {
            let (status, key) = if l.days_left < 0 {
                (Status::Danger, "inventory.expired")
            } else {
                (Status::Warning, "inventory.expires_in")
            };
            let days = l.days_left.max(0);
            let qty = Quantity::from_milli(l.qty_milli);
            let mut row = view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .px(px(16.0))
                .py(px(10.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(2.0))
                        .child(label_line(
                            {
                                let n = l.product.clone();
                                move || n.clone()
                            },
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            {
                                let t = format!("{} · {} · {}", l.lot, l.warehouse, l.expiry);
                                move || t.clone()
                            },
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .child(figure(
                    move || st.fmt().quantity(qty, 3),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    || Tone::Ink,
                ))
                .child(badge(status, None, move || {
                    tr_args(st.lang(), key, &[("days", &st.fmt().integer(days))])
                }));
            if i > 0 {
                row = row.border_t_1().border_color(Tone::Border.color());
            }
            rows = rows.child(row);
        }
        card()
            .child(card_header_icon(
                Some(Icon::Bell),
                st.t("inventory.expiring"),
                Some(Box::new(st.t("inventory.expiring.help"))),
                view(),
            ))
            .child(rows.border_t_1().border_color(Tone::Border.color()))
            .into_element()
    });
    view().child(card)
}

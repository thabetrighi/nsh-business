//! The warehouses (owner notes): each one as a card — where it is, who
//! keeps it, what it holds and what runs low — with its stock and its
//! movements one press away; added, edited, and retired when empty. Rules
//! (permissions, an empty warehouse only, never the last) are the use
//! cases'.

use nb_domain::WarehouseId;
use nb_i18n::tr_args;
use nb_store::repo::org::Branch;
use nb_store::usecases::org::{
    WarehouseDetails, WarehouseRow, archive_warehouse, create_warehouse, set_warehouse_details,
    update_warehouse, warehouse_overview,
};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::screens::common::{actions_row, export_button, export_rows, page};
use crate::screens::forms::error_text;
use crate::state::{AppState, Intent};
use crate::ui::filters::{money_tile, stat_row, stat_tile};
use crate::ui::form::{field, labelled};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, figure, label_line};
use crate::ui::menu::{Act, row_actions, with_menu};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, empty_state};
use crate::ui::tokens::{Status, Tone, radius, ty};

/// The negative-stock policies, as stored.
const POLICIES: [&str; 3] = ["forbid", "permission", "allow"];

fn money(st: AppState, minor: i64) -> String {
    let c = st
        .session
        .get()
        .map_or(nb_core::Currency::DZD, |s| s.currency);
    st.fmt().money(nb_core::Money::from_minor(minor, c))
}

fn policy_badge(st: AppState, p: &str) -> View {
    let (status, key) = match p {
        "forbid" => (Status::Success, "warehouses.policy.forbid"),
        "allow" => (Status::Danger, "warehouses.policy.allow"),
        _ => (Status::Warning, "warehouses.policy.permission"),
    };
    badge(status, Some(Icon::Shield), st.t(key))
}

pub fn warehouses_page(st: AppState, compact: bool) -> View {
    let data = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| warehouse_overview(s)).await }
        },
    );
    let rows = computed(move || {
        data.with(|d: Option<&Vec<WarehouseRow>>| d.cloned())
            .unwrap_or_default()
    });
    let branches = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    s.read(|r| {
                        let o = nb_store::repo::org::the_org(r.conn())?
                            .ok_or(nb_store::Error::NotFound("organization"))?;
                        nb_store::repo::org::branches(r.conn(), o.org_id()?)
                    })
                })
                .await
            }
        },
    );
    let branch_list = computed(move || {
        branches
            .with(|b: Option<&Vec<Branch>>| b.cloned())
            .unwrap_or_default()
    });
    // Some(None): a new one; Some(Some(row)): that one.
    let editing: Signal<Option<Option<WarehouseRow>>> = signal(None);
    let open = signal(false);
    effect(move || open.set(editing.get().is_some()));

    let tiles = {
        let sum = move |f: fn(&WarehouseRow) -> i64| rows.with(|r| r.iter().map(f).sum::<i64>());
        stat_row(
            vec![
                stat_tile(
                    "wh-count",
                    st.t("warehouses.count"),
                    Icon::Warehouse,
                    Status::Accent,
                    move || st.fmt().integer(rows.with(|r| r.len() as i64)),
                    || false,
                    None,
                    compact,
                ),
                money_tile(
                    stat_tile(
                        "wh-value",
                        st.t("warehouses.value"),
                        Icon::Coins,
                        Status::Info,
                        move || money(st, sum(|r| r.value_minor)),
                        || false,
                        None,
                        compact,
                    ),
                    compact,
                ),
                stat_tile(
                    "wh-products",
                    st.t("warehouses.products"),
                    Icon::Package,
                    Status::Success,
                    move || st.fmt().integer(sum(|r| i64::from(r.products))),
                    || false,
                    None,
                    compact,
                ),
                stat_tile(
                    "wh-low",
                    st.t("warehouses.low"),
                    Icon::Alert,
                    Status::Warning,
                    move || st.fmt().integer(sum(|r| i64::from(r.low))),
                    || false,
                    None,
                    compact,
                ),
            ],
            compact,
        )
    };

    let cards = dynamic(move || {
        let all = rows.get();
        if all.is_empty() {
            return empty_state(
                Icon::Warehouse,
                st.t("warehouses.empty"),
                st.t("warehouses.empty_hint"),
                None,
            )
            .into_element();
        }
        let mut g = view()
            .grid()
            .grid_template_columns(SharedString::from(if compact {
                "repeat(1, minmax(0px, 1fr))".to_owned()
            } else {
                "repeat(auto-fill, minmax(340px, 1fr))".to_owned()
            }))
            .gap(px(16.0));
        for (i, w) in all.into_iter().enumerate() {
            g = g.child(warehouse_card(st, i, w, editing));
        }
        g.into_element()
    });

    let export = export_button(st, "warehouses-export", move || {
        let mut out = vec![vec![
            st.tn("warehouses.name"),
            st.tn("warehouses.branch"),
            st.tn("warehouses.policy"),
            st.tn("warehouses.address"),
            st.tn("warehouses.phone"),
            st.tn("warehouses.keeper"),
            st.tn("warehouses.products"),
            st.tn("warehouses.low"),
            st.tn("warehouses.value"),
        ]];
        let c = st
            .session
            .get_untracked()
            .map_or(nb_core::Currency::DZD, |s| s.currency);
        for w in rows.get_untracked() {
            out.push(vec![
                w.name,
                w.branch,
                st.tn(&format!("warehouses.policy.{}", w.negative_stock)),
                w.address,
                w.phone,
                w.keeper,
                w.products.to_string(),
                w.low.to_string(),
                nb_core::Money::from_minor(w.value_minor, c).to_plain_string(),
            ]);
        }
        export_rows(st, "warehouses.file", out);
    });
    let add = view().child(when(
        move || st.can("settings.edit"),
        move || {
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("warehouses.add"),
                move || editing.set(Some(None)),
            )
            .test_id("warehouse-add")
            .into_element()
        },
    ));

    view()
        .child(page(
            st,
            Route::Warehouses,
            "warehouses.subtitle",
            actions_row(vec![export, add]),
            view().gap(px(20.0)).child(tiles).child(cards),
            compact,
        ))
        .child(modal(open, compact, move || {
            let current = editing.get_untracked().flatten();
            warehouse_form(st, current, branch_list, move || editing.set(None)).into_element()
        }))
}

fn warehouse_card(
    st: AppState,
    i: usize,
    w: WarehouseRow,
    editing: Signal<Option<Option<WarehouseRow>>>,
) -> View {
    let id = w.id.clone();
    let acts = {
        let (id, w2) = (id.clone(), w.clone());
        let mut v = vec![
            Act::with(Icon::Boxes, st.tn("warehouses.stock"), {
                let id = id.clone();
                move || {
                    st.intent.set(Some(Intent::StockAt(id.clone())));
                    st.route.set(Route::Inventory);
                }
            }),
            Act::with(Icon::History, st.tn("warehouses.moves"), {
                let id = id.clone();
                move || {
                    st.intent.set(Some(Intent::MovesAt(id.clone())));
                    st.route.set(Route::Movements);
                }
            }),
        ];
        if st.can("settings.edit") {
            v.push(Act::with(Icon::Pencil, st.tn("action.edit"), move || {
                editing.set(Some(Some(w2.clone())))
            }));
            v.push(Act::Line);
            v.push(
                Act::with(Icon::Trash, st.tn("warehouses.archive"), move || {
                    let Some(session) = st.session.get_untracked() else {
                        return;
                    };
                    let Ok(wid) = id.parse::<WarehouseId>() else {
                        return;
                    };
                    let db = st.db();
                    let task = spawn(async move {
                        match db
                            .call(move |s| {
                                archive_warehouse(s, session.meta("warehouse.archive"), wid)
                            })
                            .await
                        {
                            Ok(()) => {
                                st.changed();
                                show_toast(st, st.tn("warehouses.archived"));
                            }
                            Err(e) => show_toast(
                                st,
                                error_text(st, e.codes.first().map_or("internal", |c| c.1)),
                            ),
                        }
                    });
                    drop(task); // runs to completion: tasks end with their owner, not their handle
                })
                .danger(),
            );
        }
        v
    };
    let stat = |which: Icon, key: &'static str, value: String, tone: Tone| {
        view()
            .flex_1()
            .min_w(px(0.0))
            .gap(px(4.0))
            .p(px(10.0))
            .rounded(px(radius::md()))
            .bg(Tone::Subtle.color())
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(icon(which, 14.0, move || tone))
                    .child(label_line(
                        st.t(key),
                        ty::XS,
                        FontWeight::MEDIUM,
                        Tone::Muted,
                    )),
            )
            .child(figure(
                move || value.clone(),
                ty::MD,
                FontWeight::BOLD,
                || Tone::Ink,
            ))
    };
    let mut contact = view().gap(px(6.0));
    for (which, text) in [
        (Icon::MapPin, w.address.clone()),
        (Icon::Phone, w.phone.clone()),
        (Icon::User, w.keeper.clone()),
        (Icon::FileText, w.note.clone()),
    ] {
        if text.trim().is_empty() {
            continue;
        }
        contact = contact.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(icon(which, 14.0, || Tone::Muted))
                .child(view().flex_1().min_w(px(0.0)).child(label_line(
                    move || text.clone(),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Body,
                ))),
        );
    }
    let (name, branch) = (w.name.clone(), w.branch.clone());
    let reserved = w.reserved_milli;
    let body = card()
        .test_id(&format!("warehouse-{i}"))
        .p(px(18.0))
        .gap(px(14.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(
                    view()
                        .w(px(42.0))
                        .h(px(42.0))
                        .rounded(px(radius::md()))
                        .items_center()
                        .justify_center()
                        .bg(Tone::PrimarySoft.color())
                        .child(icon(Icon::Warehouse, 20.0, || Tone::Primary)),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label_line(
                            move || name.clone(),
                            ty::LG,
                            FontWeight::BOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            move || branch.clone(),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .child(row_actions(
                    st,
                    acts.clone(),
                    &format!("warehouse-more-{i}"),
                )),
        )
        .child(view().flex_row().child(policy_badge(st, &w.negative_stock)))
        // The value on a line of its own (millions fit), then the counts.
        .child(view().flex_row().child(stat(
            Icon::Coins,
            "warehouses.value",
            money(st, w.value_minor),
            Tone::Primary,
        )))
        .child(
            view()
                .flex_row()
                .gap(px(8.0))
                .child(stat(
                    Icon::Package,
                    "warehouses.products",
                    st.fmt().integer(i64::from(w.products)),
                    Tone::Success,
                ))
                .child(stat(
                    Icon::Alert,
                    "warehouses.low",
                    st.fmt().integer(i64::from(w.low)),
                    if w.low > 0 {
                        Tone::Warning
                    } else {
                        Tone::Muted
                    },
                )),
        )
        .child(label_line(
            move || {
                tr_args(
                    st.lang(),
                    "warehouses.activity",
                    &[
                        ("moves", &st.fmt().integer(i64::from(w.moves_30d))),
                        (
                            "held",
                            &st.fmt()
                                .quantity(nb_core::Quantity::from_milli(reserved), 3),
                        ),
                    ],
                )
            },
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(contact);
    with_menu(body, move || acts.clone())
}

fn warehouse_form(
    st: AppState,
    editing: Option<WarehouseRow>,
    branches: Memo<Vec<Branch>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let get = |f: fn(&WarehouseRow) -> String| editing.as_ref().map(f).unwrap_or_default();
    let name = signal(get(|w| w.name.clone()));
    let address = signal(get(|w| w.address.clone()));
    let phone = signal(get(|w| w.phone.clone()));
    let keeper = signal(get(|w| w.keeper.clone()));
    let note = signal(get(|w| w.note.clone()));
    let policy = signal(
        editing
            .as_ref()
            .and_then(|w| POLICIES.iter().position(|p| *p == w.negative_stock))
            .unwrap_or(1),
    );
    let branch_at = signal(0usize);
    let error = signal(String::new());
    let id = editing.as_ref().map(|w| w.id.clone());
    let is_new = id.is_none();
    let save = {
        let close = close.clone();
        move || {
            error.set(String::new());
            let (n, p) = (
                name.get_untracked(),
                POLICIES[policy.get_untracked().min(2)].to_owned(),
            );
            let details = WarehouseDetails {
                address: address.get_untracked(),
                phone: phone.get_untracked(),
                keeper: keeper.get_untracked(),
                note: note.get_untracked(),
            };
            let branch = branches.with_untracked(|b| {
                b.get(branch_at.get_untracked())
                    .or(b.first())
                    .map(|b| b.id.clone())
            });
            let id = id.clone();
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(move |s| {
                        let wid: WarehouseId = match id {
                            Some(id) => {
                                let wid = nb_store::repo::parse_id(&id)?;
                                update_warehouse(s, session.meta("warehouse.update"), wid, &n, &p)?;
                                wid
                            }
                            None => {
                                let b = nb_store::repo::parse_id(&branch.unwrap_or_default())?;
                                let w = create_warehouse(
                                    s,
                                    session.meta("warehouse.create"),
                                    b,
                                    &n,
                                    &p,
                                )?;
                                nb_store::repo::parse_id(&w.id)?
                            }
                        };
                        set_warehouse_details(s, session.meta("warehouse.details"), wid, &details)
                    })
                    .await;
                match r {
                    Ok(()) => {
                        st.changed();
                        close();
                        show_toast(st, st.tn("warehouses.saved"));
                    }
                    Err(e) => {
                        error.set(error_text(st, e.codes.first().map_or("internal", |c| c.1)))
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let text = |key: &'static str, v: Signal<String>, id: &'static str| {
        field(
            st.t(key),
            v,
            String::new,
            String::new,
            KeyboardType::Default,
            id,
        )
    };
    let mut body = view().gap(px(14.0)).child(field(
        st.t("warehouses.name"),
        name,
        String::new,
        move || error.get(),
        KeyboardType::Default,
        "warehouse-name",
    ));
    if is_new {
        body = body.child(labelled(
            st.t("warehouses.branch"),
            dropdown(
                move || {
                    branches
                        .get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, b)| (i, b.name))
                        .collect()
                },
                branch_at,
                Some(Icon::Building),
                st.t("warehouses.branch"),
                "warehouse-branch",
            ),
            None,
        ));
    }
    body = body
        .child(labelled(
            st.t("warehouses.policy"),
            dropdown(
                move || {
                    POLICIES
                        .iter()
                        .enumerate()
                        .map(|(i, p)| (i, st.tn(&format!("warehouses.policy.{p}"))))
                        .collect()
                },
                policy,
                Some(Icon::Shield),
                st.t("warehouses.policy"),
                "warehouse-policy",
            ),
            Some(Box::new(st.t("warehouses.policy_hint"))),
        ))
        .child(text("warehouses.address", address, "warehouse-address"))
        .child(
            view()
                .flex_row()
                .gap(px(12.0))
                .child(
                    view()
                        .flex_1()
                        .child(text("warehouses.phone", phone, "warehouse-phone")),
                )
                .child(view().flex_1().child(text(
                    "warehouses.keeper",
                    keeper,
                    "warehouse-keeper",
                ))),
        )
        .child(text("warehouses.note", note, "warehouse-note"));
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
            .test_id("warehouse-save"),
        );
    dialog_frame(
        st,
        st.t(if is_new {
            "warehouses.add"
        } else {
            "warehouses.edit"
        }),
        st.t("warehouses.form_hint"),
        body,
        footer,
        close,
    )
}

//! Vehicles and their drivers (M16): each vehicle as a card — on the road
//! or not, its tour, what it carries (its own warehouse) and where it was
//! last seen; loaded and unloaded by transfers, its stock one press away,
//! its position recorded from a map link. Rules (`distribution.fleet`, an
//! empty vehicle only retires) are the use cases'.

use nb_i18n::tr_args;
use nb_store::repo::access::UserRow;
use nb_store::usecases::access::list_users;
use nb_store::usecases::distribution::{
    SILENT_MS, VEHICLE_KINDS, VehicleInput, VehicleRow, archive_vehicle, list_vehicles,
    save_vehicle,
};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::screens::common::{actions_row, date_time, export_button, export_rows, page};
use crate::screens::distribution::{money, open_link, position_form, run, tel_url};
use crate::state::{AppState, Intent};
use crate::ui::filters::{money_tile, stat_row, stat_tile};
use crate::ui::form::{chips, field, labelled};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, figure, label_line};
use crate::ui::menu::{Act, row_actions, with_menu};
use crate::ui::overlay::{dialog_frame, modal};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, empty_state};
use crate::ui::tokens::{Status, Tone, radius, ty};

pub fn vans_page(st: AppState, compact: bool) -> View {
    let data = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_vehicles(s)).await }
        },
    );
    let rows = computed(move || {
        data.with(|d: Option<&Vec<VehicleRow>>| d.cloned())
            .unwrap_or_default()
    });
    let users = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_users(s)).await }
        },
    );
    let us = computed(move || {
        users
            .with(|u: Option<&Vec<UserRow>>| u.cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|u| u.active)
            .collect::<Vec<_>>()
    });
    let editing: Signal<Option<Option<VehicleRow>>> = signal(None);
    let open = signal(false);
    effect(move || open.set(editing.get().is_some()));
    effect(move || {
        if !open.get() {
            editing.set(None);
        }
    });
    let locating: Signal<Option<VehicleRow>> = signal(None);
    let locate_open = signal(false);
    effect(move || locate_open.set(locating.get().is_some()));

    let count =
        move |f: fn(&VehicleRow) -> bool| rows.with(|r| r.iter().filter(|v| f(v)).count() as i64);
    let tiles = stat_row(
        vec![
            stat_tile(
                "fleet-count",
                st.t("fleet.count"),
                Icon::Truck,
                Status::Accent,
                move || st.fmt().integer(count(|_| true)),
                || false,
                None,
                compact,
            ),
            stat_tile(
                "fleet-road",
                st.t("dist.on_road"),
                Icon::Route,
                Status::Info,
                move || {
                    st.fmt()
                        .integer(count(|v| v.tour_status.as_deref() == Some("on_road")))
                },
                || false,
                None,
                compact,
            ),
            money_tile(
                stat_tile(
                    "fleet-value",
                    st.t("fleet.carried"),
                    Icon::Boxes,
                    Status::Success,
                    move || {
                        money(
                            st,
                            rows.with(|r| r.iter().map(|v| v.stock_value_minor).sum()),
                        )
                    },
                    || false,
                    None,
                    compact,
                ),
                compact,
            ),
            stat_tile(
                "fleet-silent",
                st.t("fleet.silent"),
                Icon::Locate,
                Status::Warning,
                move || {
                    let now = crate::state::now_ms();
                    st.fmt().integer(rows.with(|r| {
                        r.iter()
                            .filter(|v| {
                                v.tour_status.as_deref() == Some("on_road")
                                    && v.seen_at.is_none_or(|s| now - s > SILENT_MS)
                            })
                            .count() as i64
                    }))
                },
                || false,
                None,
                compact,
            ),
        ],
        compact,
    );

    let cards = dynamic(move || {
        let all = rows.get();
        if all.is_empty() {
            return empty_state(
                Icon::Truck,
                st.t("fleet.empty"),
                st.t("fleet.empty_hint"),
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
        for (i, v) in all.into_iter().enumerate() {
            g = g.child(vehicle_card(st, i, v, editing, locating));
        }
        g.into_element()
    });

    let export = export_button(st, "fleet-export", move || {
        let mut out = vec![vec![
            st.tn("fleet.name"),
            st.tn("fleet.plate"),
            st.tn("fleet.kind"),
            st.tn("dist.driver"),
            st.tn("fleet.phone"),
            st.tn("fleet.capacity"),
            st.tn("fleet.carried"),
            st.tn("dist.tour"),
        ]];
        let c = st
            .session
            .get_untracked()
            .map_or(nb_core::Currency::DZD, |s| s.currency);
        for v in rows.get_untracked() {
            out.push(vec![
                v.name,
                v.plate,
                st.tn(&format!("fleet.kind.{}", v.kind)),
                v.driver.unwrap_or_default(),
                v.phone,
                v.capacity_kg.to_string(),
                nb_core::Money::from_minor(v.stock_value_minor, c).to_plain_string(),
                v.tour_number.unwrap_or_default(),
            ]);
        }
        export_rows(st, "fleet.file", out);
    });
    let add = view().child(when(
        move || st.can("distribution.fleet"),
        move || {
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("fleet.add"),
                move || editing.set(Some(None)),
            )
            .test_id("vehicle-add")
            .into_element()
        },
    ));

    view()
        .child(page(
            st,
            Route::Vans,
            "fleet.subtitle",
            actions_row(vec![export, add]),
            view().gap(px(20.0)).child(tiles).child(cards),
            compact,
        ))
        .child(modal(open, compact, move || {
            let current = editing.get_untracked().flatten();
            vehicle_form(st, current, us, move || open.set(false)).into_element()
        }))
        .child(modal(locate_open, compact, move || {
            match locating.get_untracked() {
                Some(v) => {
                    position_form(st, v.id, v.name, move || locating.set(None)).into_element()
                }
                None => view().into_element(),
            }
        }))
}

fn kind_icon(kind: &str) -> Icon {
    match kind {
        "car" => Icon::Bag,
        "motorbike" | "bike" => Icon::Zap,
        _ => Icon::Truck,
    }
}

fn vehicle_card(
    st: AppState,
    i: usize,
    v: VehicleRow,
    editing: Signal<Option<Option<VehicleRow>>>,
    locating: Signal<Option<VehicleRow>>,
) -> View {
    let wh = v.warehouse_id.clone();
    let acts = {
        let mut a = vec![
            Act::with(Icon::Boxes, st.tn("fleet.stock"), {
                let wh = wh.clone();
                move || {
                    st.intent.set(Some(Intent::StockAt(wh.clone())));
                    st.route.set(Route::Inventory);
                }
            }),
            Act::with(Icon::Locate, st.tn("dist.position"), {
                let v = v.clone();
                move || locating.set(Some(v.clone()))
            })
            .when(st.can("distribution.drive")),
        ];
        if st.can("stock.transfer") {
            a.push(Act::with(Icon::In, st.tn("fleet.load"), {
                let wh = wh.clone();
                move || {
                    st.intent
                        .set(Some(Intent::TransferWith(None, Some(wh.clone()))));
                    st.route.set(Route::Transfers);
                }
            }));
            a.push(Act::with(Icon::Out, st.tn("fleet.unload"), {
                let wh = wh.clone();
                move || {
                    st.intent
                        .set(Some(Intent::TransferWith(Some(wh.clone()), None)));
                    st.route.set(Route::Transfers);
                }
            }));
        }
        a.push(Act::with(Icon::History, st.tn("fleet.moves"), {
            let wh = wh.clone();
            move || {
                st.intent.set(Some(Intent::MovesAt(wh.clone())));
                st.route.set(Route::Movements);
            }
        }));
        if let Some(p) = v.point {
            a.push(Act::with(Icon::Map, st.tn("dist.open_map"), move || {
                open_link(p.map_url())
            }));
        }
        if let Some(u) = tel_url(&v.phone) {
            a.push(Act::with(Icon::Phone, st.tn("dist.call"), move || {
                open_link(u.clone())
            }));
        }
        if st.can("distribution.fleet") {
            let v2 = v.clone();
            a.push(Act::with(Icon::Pencil, st.tn("action.edit"), move || {
                editing.set(Some(Some(v2.clone())))
            }));
            a.push(Act::Line);
            let id = v.id.clone();
            a.push(
                Act::with(Icon::Trash, st.tn("fleet.archive"), move || {
                    let id = id.clone();
                    run(
                        st,
                        move |db, s| archive_vehicle(db, s.meta("vehicle.archive"), &id),
                        "fleet.archived",
                        |_| {},
                    );
                })
                .danger(),
            );
        }
        a
    };
    let (status, key) = match v.tour_status.as_deref() {
        Some("on_road") => (Status::Info, "fleet.on_road"),
        Some(_) => (Status::Accent, "fleet.planned"),
        None => (Status::Neutral, "fleet.idle"),
    };
    let tour_text = match &v.tour_number {
        Some(n) => format!("{} · {n}", st.tn(key)),
        None => st.tn(key),
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
    let mut facts = view().gap(px(6.0));
    let seen = v
        .seen_at
        .map(|s| tr_args(st.lang(), "dist.seen", &[("at", &date_time(st, s))]));
    for (which, text) in [
        (
            Icon::User,
            v.driver.clone().unwrap_or_else(|| st.tn("dist.no_driver")),
        ),
        (Icon::Phone, v.phone.clone()),
        (
            Icon::Locate,
            seen.unwrap_or_else(|| st.tn("fleet.never_seen")),
        ),
    ] {
        if text.trim().is_empty() {
            continue;
        }
        facts = facts.child(
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
    let (name, sub) = (
        v.name.clone(),
        [v.plate.clone(), st.tn(&format!("fleet.kind.{}", v.kind))]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · "),
    );
    let which = kind_icon(&v.kind);
    let capacity = if v.capacity_kg > 0 {
        tr_args(
            st.lang(),
            "fleet.kg",
            &[("n", &st.fmt().integer(v.capacity_kg))],
        )
    } else {
        "—".into()
    };
    let body = card()
        .test_id(&format!("vehicle-{i}"))
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
                        .child(icon(which, 20.0, || Tone::Primary)),
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
                            move || sub.clone(),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .child(row_actions(st, acts.clone(), &format!("vehicle-more-{i}"))),
        )
        .child(
            view()
                .flex_row()
                .child(badge(status, Some(Icon::Route), move || tour_text.clone())),
        )
        .child(view().flex_row().child(stat(
            Icon::Boxes,
            "fleet.carried",
            money(st, v.stock_value_minor),
            Tone::Primary,
        )))
        .child(
            view()
                .flex_row()
                .gap(px(8.0))
                .child(stat(
                    Icon::Package,
                    "warehouses.products",
                    st.fmt().integer(i64::from(v.products)),
                    Tone::Success,
                ))
                .child(stat(Icon::Scale, "fleet.capacity", capacity, Tone::Info)),
        )
        .child(facts);
    with_menu(body, move || acts.clone())
}

fn vehicle_form(
    st: AppState,
    editing: Option<VehicleRow>,
    users: Memo<Vec<UserRow>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let get = |f: fn(&VehicleRow) -> String| editing.as_ref().map(f).unwrap_or_default();
    let name = signal(get(|v| v.name.clone()));
    let plate = signal(get(|v| v.plate.clone()));
    let phone = signal(get(|v| v.phone.clone()));
    let capacity = signal(get(|v| {
        if v.capacity_kg > 0 {
            v.capacity_kg.to_string()
        } else {
            String::new()
        }
    }));
    let speed = signal(get(|v| v.speed_kmh.to_string()));
    let kind = signal(
        editing
            .as_ref()
            .and_then(|v| VEHICLE_KINDS.iter().position(|k| *k == v.kind))
            .unwrap_or(0),
    );
    let current_driver = editing.as_ref().and_then(|v| v.driver_id.clone());
    // 0: no driver; then the users.
    let driver = signal(0usize);
    effect(move || {
        let i = users.with(|u| {
            current_driver
                .as_ref()
                .and_then(|d| u.iter().position(|x| x.id == *d))
        });
        if let Some(i) = i {
            driver.set(i + 1);
        }
    });
    let error = signal(String::new());
    let id = editing.as_ref().map(|v| v.id.clone());
    let is_new = id.is_none();
    let save = {
        let close = close.clone();
        move || {
            error.set(String::new());
            let num = |s: Signal<String>| -> Option<i64> {
                let t = s.get_untracked();
                let t = t.trim();
                if t.is_empty() {
                    Some(0)
                } else {
                    t.parse().ok()
                }
            };
            let (Some(cap), Some(spd)) = (num(capacity), num(speed)) else {
                error.set(st.tn("error.invalid_value"));
                return;
            };
            let input = VehicleInput {
                id: id.clone(),
                branch_id: None,
                name: name.get_untracked(),
                plate: plate.get_untracked(),
                kind: VEHICLE_KINDS[kind.get_untracked().min(VEHICLE_KINDS.len() - 1)].into(),
                capacity_kg: cap,
                driver_id: match driver.get_untracked() {
                    0 => None,
                    k => users.with_untracked(|u| u.get(k - 1).map(|u| u.id.clone())),
                },
                phone: phone.get_untracked(),
                speed_kmh: spd,
            };
            let close = close.clone();
            run(
                st,
                move |db, s| save_vehicle(db, s.meta("vehicle.save"), input),
                "fleet.saved",
                move |_| close(),
            );
        }
    };
    let text = |key: &'static str, v: Signal<String>, kb: KeyboardType, id: &'static str| {
        field(st.t(key), v, String::new, String::new, kb, id)
    };
    let body = view()
        .gap(px(14.0))
        .child(field(
            st.t("fleet.name"),
            name,
            st.t("fleet.name_placeholder"),
            move || error.get(),
            KeyboardType::Default,
            "vehicle-name",
        ))
        .child(chips(
            st.t("fleet.kind"),
            VEHICLE_KINDS
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    (
                        i,
                        Box::new(st.t_dyn(format!("fleet.kind.{k}"))) as Box<dyn Fn() -> String>,
                    )
                })
                .collect(),
            kind,
        ))
        .child(
            view()
                .flex_row()
                .gap(px(12.0))
                .child(view().flex_1().child(text(
                    "fleet.plate",
                    plate,
                    KeyboardType::Default,
                    "vehicle-plate",
                )))
                .child(view().flex_1().child(text(
                    "fleet.capacity",
                    capacity,
                    KeyboardType::Number,
                    "vehicle-capacity",
                ))),
        )
        .child(labelled(
            st.t("dist.driver"),
            dropdown(
                move || {
                    let mut v = vec![(0usize, st.tn("dist.no_driver"))];
                    v.extend(
                        users
                            .get()
                            .into_iter()
                            .enumerate()
                            .map(|(i, u)| (i + 1, u.name)),
                    );
                    v
                },
                driver,
                Some(Icon::User),
                st.t("dist.driver"),
                "vehicle-driver",
            ),
            Some(Box::new(st.t("fleet.driver_hint"))),
        ))
        .child(
            view()
                .flex_row()
                .gap(px(12.0))
                .child(view().flex_1().child(text(
                    "fleet.phone",
                    phone,
                    KeyboardType::Phone,
                    "vehicle-phone",
                )))
                .child(view().flex_1().child(text(
                    "fleet.speed",
                    speed,
                    KeyboardType::Number,
                    "vehicle-speed",
                ))),
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
            .test_id("vehicle-save"),
        );
    dialog_frame(
        st,
        st.t(if is_new { "fleet.add" } else { "fleet.edit" }),
        st.t("fleet.form_hint"),
        body,
        footer,
        close,
    )
}

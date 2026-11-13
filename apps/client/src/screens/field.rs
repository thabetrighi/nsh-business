//! Field work (M16): the orders waiting for a tour, by zone, ticked and
//! planned in one go; the customers on the map — where each is (a map link
//! pasted), its zone, the days it is visited, what the driver should know;
//! and the zones. Rules (`distribution.plan`, `customers.edit`) are the use
//! cases'.

use nb_i18n::tr_args;
use nb_store::usecases::distribution::{
    CustomerPlace, OpenOrder, RouteCustomer, RouteCustomerQuery, Zone, ZoneInput, archive_zone,
    list_zones, open_orders, route_customers, save_zone, set_customer_place,
};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::screens::common::{actions_row, date_time, export_button, export_rows, page};
use crate::screens::distribution::{
    DAYS, Mark, days_text, map_button, mini_map, money, open_link, planner, run, tel_url,
    today_bit, zone_tone,
};
use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::filters::{stat_row, stat_tile};
use crate::ui::form::{chips, field, labelled, search_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, checkbox, dot, label, label_line, segmented};
use crate::ui::menu::{Act, row_actions};
use crate::ui::overlay::{dialog_frame, modal};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon, empty_state};
use crate::ui::tokens::{Status, Tone, radius, ty};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Orders,
    Customers,
    Zones,
}

pub fn field_page(st: AppState, compact: bool) -> View {
    let tab = signal(Tab::Orders);
    let zones = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_zones(s)).await }
        },
    );
    let zs = computed(move || {
        zones
            .with(|z: Option<&Vec<Zone>>| z.cloned())
            .unwrap_or_default()
    });
    let orders = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| open_orders(s, None)).await }
        },
    );
    let os = computed(move || {
        orders
            .with(|o: Option<&Vec<OpenOrder>>| o.cloned())
            .unwrap_or_default()
    });
    let unplaced = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    route_customers(
                        s,
                        &RouteCustomerQuery {
                            unplaced: true,
                            ..Default::default()
                        },
                    )
                    .map(|v| v.len())
                })
                .await
            }
        },
    );
    let today = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    route_customers(
                        s,
                        &RouteCustomerQuery {
                            days: today_bit(),
                            ..Default::default()
                        },
                    )
                    .map(|v| v.len())
                })
                .await
            }
        },
    );

    let tiles = stat_row(
        vec![
            stat_tile(
                "field-orders",
                st.t("dist.waiting_orders"),
                Icon::ClipboardList,
                Status::Warning,
                move || st.fmt().integer(os.with(|o| o.len() as i64)),
                move || tab.get() == Tab::Orders,
                Some(Box::new(move || tab.set(Tab::Orders))),
                compact,
            ),
            stat_tile(
                "field-value",
                st.t("field.orders_value"),
                Icon::Coins,
                Status::Accent,
                move || money(st, os.with(|o| o.iter().map(|o| o.total_minor).sum())),
                || false,
                None,
                compact,
            ),
            stat_tile(
                "field-today",
                st.t("field.visits_today"),
                Icon::Calendar,
                Status::Info,
                move || {
                    st.fmt()
                        .integer(today.with(|t: Option<&usize>| t.copied()).unwrap_or(0) as i64)
                },
                move || tab.get() == Tab::Customers,
                Some(Box::new(move || tab.set(Tab::Customers))),
                compact,
            ),
            stat_tile(
                "field-unplaced",
                st.t("field.unplaced"),
                Icon::MapPin,
                Status::Danger,
                move || {
                    st.fmt()
                        .integer(unplaced.with(|t: Option<&usize>| t.copied()).unwrap_or(0) as i64)
                },
                || false,
                Some(Box::new(move || tab.set(Tab::Customers))),
                compact,
            ),
        ],
        compact,
    );

    let tabs = segmented(
        vec![
            (
                Tab::Orders,
                Box::new(st.t("field.tab.orders")) as Box<dyn Fn() -> String>,
            ),
            (Tab::Customers, Box::new(st.t("field.tab.customers"))),
            (Tab::Zones, Box::new(st.t("field.tab.zones"))),
        ],
        tab,
    );
    let body = dynamic(move || match tab.get() {
        Tab::Orders => orders_tab(st, os, zs, compact).into_element(),
        Tab::Customers => customers_tab(st, zs, compact).into_element(),
        Tab::Zones => zones_tab(st, zs, compact).into_element(),
    });
    page(
        st,
        Route::FieldOrders,
        "field.subtitle",
        actions_row(vec![]),
        view().gap(px(20.0)).child(tiles).child(tabs).child(body),
        compact,
    )
}

// ---------------------------------------------------------------- orders

fn orders_tab(st: AppState, os: Memo<Vec<OpenOrder>>, zs: Memo<Vec<Zone>>, compact: bool) -> View {
    let picked: Signal<Vec<String>> = signal(Vec::new());
    let planning = signal(false);
    let plan_orders: Signal<Vec<String>> = signal(Vec::new());
    let list = dynamic(move || {
        let all = os.get();
        if all.is_empty() {
            return empty_state(
                Icon::ClipboardCheck,
                st.t("field.no_orders"),
                st.t("field.no_orders_hint"),
                None,
            )
            .into_element();
        }
        // By zone, those with none last.
        let zones = zs.get();
        let mut groups: Vec<(Option<Zone>, Vec<OpenOrder>)> = zones
            .iter()
            .map(|z| (Some(z.clone()), Vec::new()))
            .collect();
        groups.push((None, Vec::new()));
        for o in all {
            let k = o
                .zone_id
                .as_ref()
                .and_then(|z| zones.iter().position(|x| x.id == *z))
                .unwrap_or(zones.len());
            groups[k].1.push(o);
        }
        let mut v = view().gap(px(16.0));
        let mut n = 0usize;
        for (z, orders) in groups.into_iter().filter(|g| !g.1.is_empty()) {
            let ids: Vec<String> = orders.iter().map(|o| o.id.clone()).collect();
            let (title, tone) = match &z {
                Some(z) => (z.name.clone(), zone_tone(z.color)),
                None => (st.tn("field.no_zone"), Tone::Neutral),
            };
            let total: i64 = orders.iter().map(|o| o.total_minor).sum();
            let head_text = format!("{title} · {} · {}", orders.len(), money(st, total));
            let marks: Vec<Mark> = orders
                .iter()
                .enumerate()
                .filter_map(|(k, o)| {
                    o.point.map(|p| Mark {
                        point: p,
                        tone,
                        text: (k + 1).to_string(),
                        vehicle: false,
                        faded: false,
                    })
                })
                .collect();
            let ids_all = ids.clone();
            let mut c = card().p(px(16.0)).gap(px(10.0)).child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(dot(tone, 12.0))
                    .child(view().flex_1().min_w(px(0.0)).child(label_line(
                        move || head_text.clone(),
                        ty::MD,
                        FontWeight::BOLD,
                        Tone::Ink,
                    )))
                    .child(when(
                        move || st.can("distribution.plan"),
                        move || {
                            let ids = ids_all.clone();
                            button(
                                Variant::Secondary,
                                Size::Sm,
                                Some(Icon::Route),
                                st.t("field.plan_zone"),
                                move || {
                                    plan_orders.set(ids.clone());
                                    planning.set(true);
                                },
                            )
                            .into_element()
                        },
                    )),
            );
            if marks.len() > 1 && !compact {
                c = c.child(mini_map(marks, 160.0));
            }
            for (k, o) in orders.into_iter().enumerate() {
                c = c.child(order_row(st, n, k, o, picked));
                n += 1;
            }
            v = v.child(c);
        }
        v.into_element()
    });
    let bar = dynamic(move || {
        let count = picked.with(|p| p.len());
        if count == 0 || !st.can("distribution.plan") {
            return view().into_element();
        }
        let text = tr_args(st.lang(), "field.picked", &[("n", &count.to_string())]);
        view()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .p(px(12.0))
            .rounded(px(radius::md()))
            .bg(Tone::PrimarySoft.color())
            .child(view().flex_1().child(label_line(
                move || text.clone(),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Primary,
            )))
            .child(button(
                Variant::Ghost,
                Size::Sm,
                None,
                st.t("field.clear"),
                move || picked.set(Vec::new()),
            ))
            .child(
                button(
                    Variant::Primary,
                    Size::Sm,
                    Some(Icon::Route),
                    st.t("field.plan_picked"),
                    move || {
                        plan_orders.set(picked.get_untracked());
                        planning.set(true);
                    },
                )
                .test_id("field-plan"),
            )
            .into_element()
    });
    let export = export_button(st, "field-export", move || {
        let mut out = vec![vec![
            st.tn("dist.number"),
            st.tn("dist.customer"),
            st.tn("dist.city"),
            st.tn("dist.zone"),
            st.tn("dist.total"),
            st.tn("dist.since"),
        ]];
        let c = st
            .session
            .get_untracked()
            .map_or(nb_core::Currency::DZD, |s| s.currency);
        for o in os.get_untracked() {
            out.push(vec![
                o.number,
                o.customer,
                o.city,
                o.zone.unwrap_or_default(),
                nb_core::Money::from_minor(o.total_minor, c).to_plain_string(),
                date_time(st, o.created_at),
            ]);
        }
        export_rows(st, "field.file", out);
    });
    view()
        .gap(px(12.0))
        .child(view().flex_row().justify_end().child(export))
        .child(bar)
        .child(list)
        .child(modal(planning, compact, move || {
            planner(
                st,
                Vec::new(),
                plan_orders.get_untracked(),
                move || planning.set(false),
                move |id| {
                    picked.set(Vec::new());
                    st.intent.set(Some(crate::state::Intent::Tour(id)));
                    st.route.set(Route::Tours);
                },
            )
            .into_element()
        }))
}

fn order_row(st: AppState, n: usize, k: usize, o: OpenOrder, picked: Signal<Vec<String>>) -> View {
    let id = o.id.clone();
    let id2 = o.id.clone();
    let (number, customer) = (o.number.clone(), o.customer.clone());
    let sub = [o.city.clone(), date_time(st, o.created_at)]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let total = money(st, o.total_minor);
    let seq = (k + 1).to_string();
    let mut row = view()
        .test_id(&format!("field-order-{n}"))
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .p(px(10.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::Border.color())
        .cursor_pointer()
        .on_press(move || {
            picked.update(|p| {
                if let Some(i) = p.iter().position(|x| *x == id2) {
                    p.remove(i);
                } else {
                    p.push(id2.clone());
                }
            })
        })
        .child(checkbox(
            move || Some(picked.with(|p| p.contains(&id))),
            || {},
        ))
        .child(label_line(
            move || seq.clone(),
            ty::XS,
            FontWeight::BOLD,
            Tone::Muted,
        ))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(label_line(
                    move || customer.clone(),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    move || format!("{number} · {sub}"),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        )
        .child(label_line(
            move || total.clone(),
            ty::SM,
            FontWeight::BOLD,
            Tone::Ink,
        ));
    if o.point.is_none() {
        row = row.child(badge(
            Status::Warning,
            Some(Icon::MapPin),
            st.t("dist.no_place"),
        ));
    }
    row
}

// ---------------------------------------------------------------- customers

fn customers_tab(st: AppState, zs: Memo<Vec<Zone>>, compact: bool) -> View {
    let search = signal(String::new());
    // 0: all; then the zones.
    let zone = signal(0usize);
    // 0 all, 1 today, 2 without a place.
    let show = signal(0usize);
    let editing: Signal<Option<RouteCustomer>> = signal(None);
    let open = signal(false);
    effect(move || open.set(editing.get().is_some()));
    let data = resource(
        move || {
            st.session.get().map(|_| {
                let z = zone.get();
                (
                    st.data_rev.get(),
                    search.get(),
                    if z == 0 {
                        None
                    } else {
                        zs.with(|zs| zs.get(z - 1).map(|z| z.id.clone()))
                    },
                    show.get(),
                )
            })
        },
        move |(_, q, zone, show), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    route_customers(
                        s,
                        &RouteCustomerQuery {
                            search: q,
                            zone_id: zone,
                            days: if show == 1 { today_bit() } else { 0 },
                            unplaced: show == 2,
                            with_orders: false,
                        },
                    )
                })
                .await
            }
        },
    );
    let rows = computed(move || {
        data.with(|d: Option<&Vec<RouteCustomer>>| d.cloned())
            .unwrap_or_default()
    });
    let filters = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
        .child(view().flex_1().min_w(px(200.0)).child(search_field(
            search,
            st.t("dist.search_customers"),
            "field-search",
        )))
        .child(view().min_w(px(160.0)).child(dropdown(
            move || {
                let mut v = vec![(0usize, st.tn("dist.all_zones"))];
                v.extend(
                    zs.get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, z)| (i + 1, z.name)),
                );
                v
            },
            zone,
            Some(Icon::Map),
            st.t("dist.zone"),
            "field-zone",
        )))
        .child(segmented(
            vec![
                (
                    0usize,
                    Box::new(st.t("field.all")) as Box<dyn Fn() -> String>,
                ),
                (1usize, Box::new(st.t("dist.today_visits"))),
                (2usize, Box::new(st.t("field.unplaced"))),
            ],
            show,
        ));
    let map = dynamic(move || {
        let marks: Vec<Mark> = rows.with(|r| {
            r.iter()
                .filter_map(|c| {
                    c.point.map(|p| Mark {
                        point: p,
                        tone: c
                            .zone_id
                            .as_ref()
                            .and_then(|z| {
                                zs.with(|zs| {
                                    zs.iter().find(|x| x.id == *z).map(|x| zone_tone(x.color))
                                })
                            })
                            .unwrap_or(Tone::Neutral),
                        text: String::new(),
                        vehicle: false,
                        faded: false,
                    })
                })
                .collect()
        });
        if marks.len() < 2 {
            return view().into_element();
        }
        card()
            .child(card_header_icon(
                Some(Icon::Map),
                st.t("field.map"),
                Some(Box::new(st.t("field.map_hint"))),
                view(),
            ))
            .child(
                view()
                    .px(px(16.0))
                    .pb(px(16.0))
                    .child(mini_map(marks, if compact { 220.0 } else { 280.0 })),
            )
            .into_element()
    });
    let list = dynamic(move || {
        let all = rows.get();
        if all.is_empty() {
            return empty_state(
                Icon::Users,
                st.t("field.no_customers"),
                st.t("field.no_customers_hint"),
                None,
            )
            .into_element();
        }
        let mut v = view().gap(px(8.0));
        for (i, c) in all.into_iter().enumerate() {
            v = v.child(customer_row(st, i, c, editing));
        }
        v.into_element()
    });
    view()
        .gap(px(16.0))
        .child(filters)
        .child(map)
        .child(list)
        .child(modal(open, compact, move || {
            match editing.get_untracked() {
                Some(c) => place_form(st, c, zs, move || editing.set(None)).into_element(),
                None => view().into_element(),
            }
        }))
}

fn customer_row(
    st: AppState,
    i: usize,
    c: RouteCustomer,
    editing: Signal<Option<RouteCustomer>>,
) -> View {
    let mut acts = Vec::new();
    if st.can("distribution.plan") || st.can("customers.edit") {
        let c2 = c.clone();
        acts.push(Act::with(
            Icon::MapPin,
            st.tn("field.set_place"),
            move || editing.set(Some(c2.clone())),
        ));
    }
    if let Some(p) = c.point {
        acts.push(Act::with(
            Icon::Navigation,
            st.tn("dist.directions"),
            move || open_link(p.directions_url()),
        ));
        acts.push(Act::copy(st.tn("field.copy_place"), p.text()));
    }
    if let Some(u) = tel_url(&c.phone) {
        acts.push(Act::with(Icon::Phone, st.tn("dist.call"), move || {
            open_link(u.clone())
        }));
    }
    let name = c.name.clone();
    let mut sub = vec![c.code.clone(), c.city.clone()];
    if c.visit_days != 0 {
        sub.push(days_text(st, c.visit_days));
    }
    let sub = sub
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let mut facts = view().flex_row().flex_wrap().gap(px(6.0));
    match &c.zone {
        Some(z) => {
            let z = z.clone();
            facts = facts.child(badge(Status::Accent, Some(Icon::Map), move || z.clone()));
        }
        None => {
            facts = facts.child(badge(
                Status::Neutral,
                Some(Icon::Map),
                st.t("field.no_zone"),
            ))
        }
    }
    if c.point.is_none() {
        facts = facts.child(badge(
            Status::Warning,
            Some(Icon::MapPin),
            st.t("dist.no_place"),
        ));
    }
    if c.open_orders > 0 {
        let t = tr_args(
            st.lang(),
            "dist.n_orders",
            &[("n", &c.open_orders.to_string())],
        );
        facts = facts.child(badge(Status::Info, Some(Icon::ClipboardList), move || {
            t.clone()
        }));
    }
    if c.balance_minor > 0 {
        let t = money(st, c.balance_minor);
        facts = facts.child(badge(Status::Danger, Some(Icon::Coins), move || t.clone()));
    }
    view()
        .test_id(&format!("field-customer-{i}"))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .p(px(12.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::Border.color())
        .child(icon(Icon::Store, 18.0, || Tone::Primary))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(4.0))
                .child(label_line(
                    move || name.clone(),
                    ty::MD,
                    FontWeight::BOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    move || sub.clone(),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .child(facts),
        )
        .child(map_button(st, c.point))
        .child(row_actions(st, acts, &format!("field-customer-more-{i}")))
}

fn place_form(
    st: AppState,
    c: RouteCustomer,
    zs: Memo<Vec<Zone>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let position = signal(c.point.map(|p| p.text()).unwrap_or_default());
    let note = signal(c.delivery_note.clone());
    let days = signal(c.visit_days);
    let zone = signal(0usize);
    let current = c.zone_id.clone();
    effect(move || {
        let i = zs.with(|z| {
            current
                .as_ref()
                .and_then(|c| z.iter().position(|x| x.id == *c))
        });
        if let Some(i) = i {
            zone.set(i + 1);
        }
    });
    let error = signal(String::new());
    let save = {
        let close = close.clone();
        let id = c.id.clone();
        move || {
            error.set(String::new());
            let p = position.get_untracked();
            if !p.trim().is_empty()
                && let Err(e) = nb_domain::geo::parse_point(&p)
            {
                error.set(error_text(st, e.code));
                return;
            }
            let place = CustomerPlace {
                zone_id: match zone.get_untracked() {
                    0 => None,
                    k => zs.with_untracked(|z| z.get(k - 1).map(|z| z.id.clone())),
                },
                position: p,
                visit_days: days.get_untracked(),
                delivery_note: note.get_untracked(),
            };
            let id = id.clone();
            let close = close.clone();
            run(
                st,
                move |db, s| set_customer_place(db, s.meta("customer.place"), &id, place),
                "field.place_saved",
                move |_| close(),
            );
        }
    };
    let mut day_row = view().flex_row().flex_wrap().gap(px(6.0));
    for (i, key) in DAYS.iter().enumerate() {
        let bit = 1i64 << i;
        let on = computed(move || days.get() & bit != 0);
        day_row = day_row.child(
            view()
                .test_id(&format!("place-day-{i}"))
                .h(px(32.0))
                .px(px(12.0))
                .justify_center()
                .rounded(px(radius::md()))
                .cursor_pointer()
                .border_1()
                .border_color(move || {
                    if on.get() {
                        Tone::Primary.color()
                    } else {
                        Tone::BorderStrong.color()
                    }
                })
                .bg(move || {
                    if on.get() {
                        Tone::PrimarySoft.color()
                    } else {
                        Tone::Surface.color()
                    }
                })
                .on_press(move || days.update(|d| *d ^= bit))
                .child(label_line(st.t(key), ty::SM, FontWeight::MEDIUM, Tone::Ink)),
        );
    }
    let name = c.name.clone();
    let body = view()
        .gap(px(14.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(icon(Icon::Store, 16.0, || Tone::Muted))
                .child(label_line(
                    move || name.clone(),
                    ty::MD,
                    FontWeight::BOLD,
                    Tone::Ink,
                )),
        )
        .child(labelled(
            st.t("dist.zone"),
            dropdown(
                move || {
                    let mut v = vec![(0usize, st.tn("field.no_zone"))];
                    v.extend(
                        zs.get()
                            .into_iter()
                            .enumerate()
                            .map(|(i, z)| (i + 1, z.name)),
                    );
                    v
                },
                zone,
                Some(Icon::Map),
                st.t("dist.zone"),
                "place-zone",
            ),
            None,
        ))
        .child(field(
            st.t("field.position"),
            position,
            st.t("dist.position_placeholder"),
            move || error.get(),
            KeyboardType::Default,
            "place-position",
        ))
        .child(label(
            st.t("field.position_help"),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(labelled(
            st.t("field.visit_days"),
            day_row,
            Some(Box::new(st.t("field.visit_days_hint"))),
        ))
        .child(field(
            st.t("field.delivery_note"),
            note,
            st.t("field.delivery_note_placeholder"),
            String::new,
            KeyboardType::Default,
            "place-note",
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
                st.t("form.save"),
                save,
            )
            .test_id("place-save"),
        );
    dialog_frame(
        st,
        st.t("field.set_place"),
        st.t("field.place_hint"),
        body,
        footer,
        close,
    )
}

// ---------------------------------------------------------------- zones

fn zones_tab(st: AppState, zs: Memo<Vec<Zone>>, compact: bool) -> View {
    let editing: Signal<Option<Option<Zone>>> = signal(None);
    let open = signal(false);
    effect(move || open.set(editing.get().is_some()));
    effect(move || {
        if !open.get() {
            editing.set(None);
        }
    });
    let add = view().child(when(
        move || st.can("distribution.plan"),
        move || {
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("field.zone_add"),
                move || editing.set(Some(None)),
            )
            .test_id("zone-add")
            .into_element()
        },
    ));
    let list = dynamic(move || {
        let all = zs.get();
        if all.is_empty() {
            return empty_state(
                Icon::Map,
                st.t("field.no_zones"),
                st.t("field.no_zones_hint"),
                None,
            )
            .into_element();
        }
        let mut g = view()
            .grid()
            .grid_template_columns(SharedString::from(if compact {
                "repeat(1, minmax(0px, 1fr))".to_owned()
            } else {
                "repeat(auto-fill, minmax(260px, 1fr))".to_owned()
            }))
            .gap(px(12.0));
        for (i, z) in all.into_iter().enumerate() {
            let mut acts = Vec::new();
            if st.can("distribution.plan") {
                let z2 = z.clone();
                acts.push(Act::with(Icon::Pencil, st.tn("action.edit"), move || {
                    editing.set(Some(Some(z2.clone())))
                }));
                let id = z.id.clone();
                acts.push(
                    Act::with(Icon::Trash, st.tn("field.zone_archive"), move || {
                        let id = id.clone();
                        run(
                            st,
                            move |db, s| archive_zone(db, s.meta("zone.archive"), &id),
                            "field.zone_archived",
                            |_| {},
                        );
                    })
                    .danger(),
                );
            }
            let (name, note) = (z.name.clone(), z.note.clone());
            let count = tr_args(
                st.lang(),
                "field.zone_customers",
                &[("n", &z.customers.to_string())],
            );
            g = g.child(
                card()
                    .test_id(&format!("zone-{i}"))
                    .p(px(16.0))
                    .gap(px(8.0))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .child(dot(zone_tone(z.color), 14.0))
                            .child(view().flex_1().min_w(px(0.0)).child(label_line(
                                move || name.clone(),
                                ty::LG,
                                FontWeight::BOLD,
                                Tone::Ink,
                            )))
                            .child(row_actions(st, acts, &format!("zone-more-{i}"))),
                    )
                    .child(label_line(
                        move || count.clone(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Body,
                    ))
                    .child(label(
                        move || note.clone(),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )),
            );
        }
        g.into_element()
    });
    view()
        .gap(px(12.0))
        .child(view().flex_row().justify_end().child(add))
        .child(list)
        .child(modal(open, compact, move || {
            zone_form(st, editing.get_untracked().flatten(), move || {
                open.set(false)
            })
            .into_element()
        }))
}

fn zone_form(st: AppState, z: Option<Zone>, close: impl Fn() + Clone + 'static) -> View {
    let name = signal(z.as_ref().map(|z| z.name.clone()).unwrap_or_default());
    let note = signal(z.as_ref().map(|z| z.note.clone()).unwrap_or_default());
    let color = signal(
        z.as_ref()
            .map_or(0, |z| usize::try_from(z.color.rem_euclid(6)).unwrap_or(0)),
    );
    let error = signal(String::new());
    let id = z.as_ref().map(|z| z.id.clone());
    let is_new = id.is_none();
    let save = {
        let close = close.clone();
        move || {
            error.set(String::new());
            let input = ZoneInput {
                id: id.clone(),
                name: name.get_untracked(),
                color: color.get_untracked() as i64,
                note: note.get_untracked(),
            };
            let close = close.clone();
            run(
                st,
                move |db, s| save_zone(db, s.meta("zone.save"), input),
                "field.zone_saved",
                move |_| close(),
            );
        }
    };
    let body = view()
        .gap(px(14.0))
        .child(field(
            st.t("field.zone_name"),
            name,
            st.t("field.zone_placeholder"),
            move || error.get(),
            KeyboardType::Default,
            "zone-name",
        ))
        .child(chips(
            st.t("field.zone_color"),
            (0..6usize)
                .map(|i| {
                    (
                        i,
                        Box::new(st.t_dyn(format!("field.color.{i}"))) as Box<dyn Fn() -> String>,
                    )
                })
                .collect(),
            color,
        ))
        .child(field(
            st.t("dist.note"),
            note,
            String::new,
            String::new,
            KeyboardType::Default,
            "zone-note",
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
                st.t("form.save"),
                save,
            )
            .test_id("zone-save"),
        );
    dialog_frame(
        st,
        st.t(if is_new {
            "field.zone_add"
        } else {
            "field.zone_edit"
        }),
        st.t("field.zone_hint"),
        body,
        footer,
        close,
    )
}

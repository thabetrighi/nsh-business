//! The driver's day (M16, owner notes): the same app on the driver's
//! phone, under their own account. Their tour of the day, the stop to do
//! now in large type with large buttons (directions, call, WhatsApp,
//! arrived, delivered, failed, sell from the vehicle, invoice the order),
//! the stops after it, their position sent from a map link, and the end
//! of the day. Only their own tours show; the use cases refuse another's.

use nb_i18n::tr_args;
use nb_store::usecases::distribution::{
    StopRow, TourDetail, TourQuery, arrive_stop, get_tour, list_tours, mark_notified, start_tour,
};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::screens::common::page;
use crate::screens::distribution::{
    clock_in, end_form, km, line, money, open_link, position_form, ratio, run, small_stat,
    stop_badge, stop_form, tel_url, whatsapp_url,
};
use crate::state::{AppState, Intent};
use crate::ui::chart::progress;
use crate::ui::form::field;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, figure, label, label_line};
use crate::ui::overlay::{dialog_frame, modal};
use crate::ui::surface::{card, empty_state};
use crate::ui::tokens::{Status, Tone, radius, ty};

pub fn my_day_page(st: AppState, compact: bool) -> View {
    // The tour on the road first, else the next one planned.
    let tick = signal(0u64);
    let timer = set_interval(std::time::Duration::from_secs(60), move || {
        tick.update(|t| *t += 1)
    });
    cleanup(move || drop(timer));
    let data = resource(
        move || {
            st.session
                .get()
                .map(|s| (st.data_rev.get(), tick.get(), s.actor))
        },
        move |(_, _, me), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    let mine = list_tours(
                        s,
                        &TourQuery {
                            status: "live".into(),
                            driver_id: Some(me.to_string()),
                            limit: 20,
                            ..Default::default()
                        },
                    )?;
                    let pick = mine
                        .iter()
                        .find(|t| t.status == "on_road")
                        .or_else(|| mine.iter().min_by(|a, b| a.day.cmp(&b.day)))
                        .map(|t| t.id.clone());
                    let detail = match pick {
                        Some(id) => Some(get_tour(s, &id)?),
                        None => None,
                    };
                    Ok((detail, mine.len()))
                })
                .await
            }
        },
    );
    let d = computed(move || {
        data.with(|d: Option<&(Option<TourDetail>, usize)>| d.and_then(|d| d.0.clone()))
    });
    // The stop chosen in the list (else the next one).
    let focused: Signal<Option<String>> = signal(None);
    let finishing: Signal<Option<(StopRow, bool)>> = signal(None);
    let finish_open = signal(false);
    effect(move || finish_open.set(finishing.get().is_some()));
    let ending = signal(false);
    let locating = signal(false);
    let starting = signal(false);

    let body = dynamic(move || {
        let Some(d) = d.get() else {
            let mut e = view().gap(px(12.0)).child(empty_state(
                Icon::Navigation,
                st.t("myday.none"),
                st.t("myday.none_hint"),
                None,
            ));
            if st.can("distribution.view") {
                e = e.child(view().items_center().child(button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Route),
                    st.t("myday.all_tours"),
                    move || st.route.set(Route::Tours),
                )));
            }
            return card().p(px(16.0)).child(e).into_element();
        };
        day_body(
            st, d, compact, focused, finishing, ending, locating, starting,
        )
        .into_element()
    });

    view()
        .child(page(
            st,
            Route::MyDay,
            "myday.subtitle",
            view(),
            view().w_full().max_w(px(760.0)).self_center().child(body),
            compact,
        ))
        .child(modal(finish_open, compact, move || {
            match finishing.get_untracked() {
                Some((stop, ok)) => {
                    stop_form(st, stop, ok, move || finishing.set(None)).into_element()
                }
                None => view().into_element(),
            }
        }))
        .child(modal(ending, compact, move || match d.get_untracked() {
            Some(d) => end_form(st, d, false, move || ending.set(false)).into_element(),
            None => view().into_element(),
        }))
        .child(modal(locating, compact, move || match d.get_untracked() {
            Some(d) => position_form(
                st,
                d.tour.vehicle_id.clone(),
                d.tour.vehicle.clone(),
                move || locating.set(false),
            )
            .into_element(),
            None => view().into_element(),
        }))
        .child(modal(starting, compact, move || match d.get_untracked() {
            Some(d) => {
                start_form(st, d.tour.id.clone(), move || starting.set(false)).into_element()
            }
            None => view().into_element(),
        }))
}

/// A large button filling its cell (thumbs on a phone).
fn big(
    variant: Variant,
    which: Icon,
    caption: impl Fn() -> String + 'static,
    id: &'static str,
    run: impl Fn() + 'static,
) -> View {
    view().flex_1().min_w(px(140.0)).child(
        button(variant, Size::Lg, Some(which), caption, run)
            .w_full()
            .test_id(id),
    )
}

#[allow(clippy::too_many_arguments)] // the dialogs the day opens
fn day_body(
    st: AppState,
    d: TourDetail,
    compact: bool,
    focused: Signal<Option<String>>,
    finishing: Signal<Option<(StopRow, bool)>>,
    ending: Signal<bool>,
    locating: Signal<bool>,
    starting: Signal<bool>,
) -> View {
    let t = d.tour.clone();
    let on_road = t.status == "on_road";
    let share = if t.stops == 0 {
        0.0
    } else {
        (t.done + t.failed) as f32 / t.stops as f32
    };
    let title = format!("{} · {}", t.vehicle, t.number);
    let sub = format!("{} · {}", t.day, st.tn(&format!("dist.kind.{}", t.kind)));

    // The tour: progress, and what to do with it now.
    let mut acts = view().flex_row().flex_wrap().gap(px(10.0));
    if t.status == "planned" {
        acts = acts.child(big(
            Variant::Primary,
            Icon::Play,
            st.t("myday.start"),
            "myday-start",
            move || starting.set(true),
        ));
    }
    if on_road {
        acts = acts
            .child(big(
                Variant::Secondary,
                Icon::Locate,
                st.t("myday.send_position"),
                "myday-position",
                move || locating.set(true),
            ))
            .child(big(
                Variant::Secondary,
                Icon::Flag,
                st.t("myday.end"),
                "myday-end",
                move || ending.set(true),
            ));
    }
    if st.can("stock.view") || st.can("inventory.view") {
        let vid = t.vehicle_id.clone();
        acts = acts.child(big(
            Variant::Ghost,
            Icon::Boxes,
            st.t("myday.van_stock"),
            "myday-stock",
            move || {
                let vid = vid.clone();
                run(
                    st,
                    move |db, _| nb_store::usecases::distribution::get_vehicle(db, &vid),
                    "",
                    move |v| {
                        st.intent.set(Some(Intent::StockAt(v.warehouse_id)));
                        st.route.set(Route::Inventory);
                    },
                );
            },
        ));
    }
    let head = card()
        .test_id("myday-tour")
        .p(px(16.0))
        .gap(px(12.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(
                    view()
                        .w(px(44.0))
                        .h(px(44.0))
                        .rounded(px(radius::md()))
                        .items_center()
                        .justify_center()
                        .bg(Tone::PrimarySoft.color())
                        .child(icon(Icon::Truck, 22.0, || Tone::Primary)),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label_line(
                            move || title.clone(),
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
                .child(crate::screens::distribution::tour_badge(st, &t)),
        )
        .child(progress(share, Tone::Success))
        .child(
            view()
                .flex_row()
                .gap(px(8.0))
                .child(small_stat(
                    st,
                    Icon::CheckCircle,
                    "dist.done",
                    ratio(t.done, t.stops),
                    Tone::Success,
                ))
                .child(small_stat(
                    st,
                    Icon::Route,
                    "dist.left_km",
                    km(st, d.left_m),
                    Tone::Info,
                ))
                .child(small_stat(
                    st,
                    Icon::Coins,
                    "dist.collected",
                    money(st, t.collected_minor),
                    Tone::Warning,
                )),
        )
        .child(acts);

    // The stop to do now: the one chosen, else the next.
    let open: Vec<StopRow> = d.stops.iter().filter(|s| s.open()).cloned().collect();
    let chosen = focused
        .get_untracked()
        .and_then(|id| open.iter().find(|s| s.id == id).cloned())
        .or_else(|| open.first().cloned());
    let mut v = view().gap(px(16.0)).child(head);
    match chosen {
        Some(s) => {
            v = v.child(hero(st, &d, s.clone(), on_road, finishing));
            let rest: Vec<StopRow> = open.into_iter().filter(|x| x.id != s.id).collect();
            if !rest.is_empty() {
                let mut list = view().gap(px(6.0));
                for (i, r) in rest.into_iter().enumerate() {
                    list = list.child(next_row(st, i, &d, r, focused));
                }
                v = v.child(
                    card()
                        .p(px(16.0))
                        .gap(px(10.0))
                        .child(label(
                            st.t("myday.after"),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(list),
                );
            }
        }
        None => {
            let s = tr_args(
                st.lang(),
                "myday.finished",
                &[
                    ("done", &t.done.to_string()),
                    ("failed", &t.failed.to_string()),
                    ("collected", &money(st, t.collected_minor)),
                ],
            );
            v = v.child(
                card()
                    .p(px(20.0))
                    .gap(px(10.0))
                    .items_center()
                    .child(icon(Icon::CheckCircle, 36.0, || Tone::Success))
                    .child(label(
                        move || s.clone(),
                        ty::MD,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    )),
            );
        }
    }
    let done: Vec<&StopRow> = d.stops.iter().filter(|s| !s.open()).collect();
    if !done.is_empty() {
        let mut list = view().gap(px(6.0));
        for s in done {
            let name = s.customer.clone();
            let mut row = view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(view().flex_1().min_w(px(0.0)).child(label_line(
                    move || name.clone(),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Body,
                )))
                .child(stop_badge(st, &s.status));
            if s.collected_minor > 0 {
                let m = money(st, s.collected_minor);
                row = row.child(badge(Status::Success, Some(Icon::Coins), move || m.clone()));
            }
            list = list.child(row);
        }
        v = v.child(
            card()
                .p(px(16.0))
                .gap(px(10.0))
                .child(label(
                    st.t("myday.done_list"),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(list),
        );
    }
    let _ = compact;
    v
}

fn hero(
    st: AppState,
    d: &TourDetail,
    s: StopRow,
    on_road: bool,
    finishing: Signal<Option<(StopRow, bool)>>,
) -> View {
    let t = &d.tour;
    let seq = d
        .stops
        .iter()
        .position(|x| x.id == s.id)
        .map_or(0, |p| p + 1);
    let name = s.customer.clone();
    let place = [
        s.address.clone(),
        s.city.clone(),
        s.zone.clone().unwrap_or_default(),
    ]
    .into_iter()
    .filter(|x| !x.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let mut facts = view()
        .flex_row()
        .flex_wrap()
        .gap(px(6.0))
        .child(stop_badge(st, &s.status));
    if let Some(m) = s.eta_min {
        let txt = tr_args(
            st.lang(),
            "dist.eta_short",
            &[
                ("min", &st.fmt().integer(i64::from(m))),
                ("at", &clock_in(st, m)),
            ],
        );
        facts = facts.child(badge(Status::Info, Some(Icon::Clock), move || txt.clone()));
    }
    if let Some(l) = s.leg_m {
        let txt = km(st, l);
        facts = facts.child(badge(Status::Neutral, Some(Icon::Route), move || {
            txt.clone()
        }));
    }
    if s.point.is_none() {
        facts = facts.child(badge(
            Status::Warning,
            Some(Icon::MapPin),
            st.t("dist.no_place"),
        ));
    }
    if s.notified_at.is_some() {
        facts = facts.child(badge(
            Status::Success,
            Some(Icon::Message),
            st.t("dist.notified"),
        ));
    }
    let mut info = view().gap(px(8.0)).child(facts);
    if !s.delivery_note.trim().is_empty() {
        info = info.child(line(Icon::Info, s.delivery_note.clone(), Tone::Warning));
    }
    if let (Some(n), Some(total)) = (s.document_number.clone(), s.document_total_minor) {
        info = info.child(line(
            Icon::Receipt,
            format!("{n} · {}", money(st, total)),
            Tone::Body,
        ));
    }
    if s.balance_minor > 0 {
        let txt = tr_args(
            st.lang(),
            "dist.owes",
            &[("amount", &money(st, s.balance_minor))],
        );
        info = info.child(line(Icon::Coins, txt, Tone::Danger));
    }

    // The buttons: going there first, then what happens there.
    let mut go = view().flex_row().flex_wrap().gap(px(10.0));
    if let Some(p) = s.point {
        go = go.child(big(
            Variant::Primary,
            Icon::Navigation,
            st.t("dist.directions"),
            "myday-go",
            move || open_link(p.directions_url()),
        ));
    }
    if let Some(u) = tel_url(&s.phone) {
        go = go.child(big(
            Variant::Secondary,
            Icon::Phone,
            st.t("dist.call"),
            "myday-call",
            move || open_link(u.clone()),
        ));
    }
    if !s.phone.is_empty() {
        let (phone, cname, number, eta, id) = (
            s.phone.clone(),
            s.customer.clone(),
            s.document_number
                .clone()
                .unwrap_or_else(|| t.number.clone()),
            s.eta_min,
            s.id.clone(),
        );
        go = go.child(big(
            Variant::Secondary,
            Icon::Message,
            st.t("dist.whatsapp"),
            "myday-whatsapp",
            move || {
                let shop = st.shop.get_untracked().map(|s| s.0).unwrap_or_default();
                let msg = match eta {
                    Some(m) if m <= 15 => tr_args(
                        st.lang(),
                        "dist.msg.near",
                        &[("name", &cname), ("shop", &shop)],
                    ),
                    Some(m) => tr_args(
                        st.lang(),
                        "dist.msg.on_way",
                        &[
                            ("name", &cname),
                            ("shop", &shop),
                            ("number", &number),
                            ("at", &clock_in(st, m)),
                        ],
                    ),
                    None => tr_args(
                        st.lang(),
                        "dist.msg.today",
                        &[("name", &cname), ("shop", &shop), ("number", &number)],
                    ),
                };
                if let Some(u) = whatsapp_url(&phone, &msg) {
                    open_link(u);
                }
                let id = id.clone();
                run(
                    st,
                    move |db, se| mark_notified(db, se.meta("tour.notify"), &id),
                    "",
                    |_| {},
                );
            },
        ));
    }
    let mut work = view().flex_row().flex_wrap().gap(px(10.0));
    if on_road && st.can("distribution.drive") {
        if s.status == "pending" {
            let id = s.id.clone();
            work = work.child(big(
                Variant::Secondary,
                Icon::MapPin,
                st.t("dist.arrived"),
                "myday-arrived",
                move || {
                    let id = id.clone();
                    run(
                        st,
                        move |db, se| arrive_stop(db, se.meta("tour.arrive"), &id),
                        "",
                        |_| {},
                    );
                },
            ));
        }
        if st.can("sales.sell") {
            let cid = s.customer_id.clone();
            let doc = s
                .document_id
                .clone()
                .filter(|_| s.document_status.as_deref() == Some("approved"));
            let (key, which): (&'static str, Icon) = if doc.is_some() {
                ("dist.invoice", Icon::Receipt)
            } else {
                ("myday.sell", Icon::Cart)
            };
            work = work.child(big(
                Variant::Secondary,
                which,
                st.t(key),
                "myday-sell",
                move || {
                    match &doc {
                        Some(d) => st.intent.set(Some(Intent::Invoice(d.clone()))),
                        None => st.intent.set(Some(Intent::SellTo(cid.clone()))),
                    }
                    st.route.set(Route::Sell);
                },
            ));
        }
        let s1 = s.clone();
        work = work.child(big(
            Variant::Primary,
            Icon::CheckCircle,
            st.t("dist.deliver"),
            "myday-done",
            move || finishing.set(Some((s1.clone(), true))),
        ));
        let s2 = s.clone();
        work = work.child(big(
            Variant::Danger,
            Icon::XCircle,
            st.t("dist.fail"),
            "myday-fail",
            move || finishing.set(Some((s2.clone(), false))),
        ));
    } else if !on_road {
        work = work.child(label(
            st.t("myday.start_first"),
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Muted,
        ));
    }
    let seq_text = seq.to_string();
    card()
        .test_id("myday-next")
        .p(px(18.0))
        .gap(px(14.0))
        .border_color(Tone::Primary.color())
        .child(label_line(
            st.t("dist.next"),
            ty::XS,
            FontWeight::SEMIBOLD,
            Tone::Primary,
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(
                    view()
                        .w(px(36.0))
                        .h(px(36.0))
                        .rounded_full()
                        .items_center()
                        .justify_center()
                        .bg(Tone::Primary.color())
                        .flex_shrink_0()
                        .child(figure(
                            move || seq_text.clone(),
                            ty::MD,
                            FontWeight::BOLD,
                            || Tone::OnPrimary,
                        )),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label(
                            move || name.clone(),
                            ty::XL,
                            FontWeight::BOLD,
                            Tone::Ink,
                        ))
                        .child(label(
                            move || place.clone(),
                            ty::SM,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                ),
        )
        .child(info)
        .child(go)
        .child(work)
}

fn next_row(
    st: AppState,
    i: usize,
    d: &TourDetail,
    s: StopRow,
    focused: Signal<Option<String>>,
) -> View {
    let seq = d
        .stops
        .iter()
        .position(|x| x.id == s.id)
        .map_or(0, |p| p + 1)
        .to_string();
    let name = s.customer.clone();
    let when = s.eta_min.map(|m| clock_in(st, m)).unwrap_or_default();
    let id = s.id.clone();
    view()
        .test_id(&format!("myday-stop-{i}"))
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .p(px(10.0))
        .min_h(px(48.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::Border.color())
        .cursor_pointer()
        .on_press(move || focused.set(Some(id.clone())))
        .child(figure(
            move || seq.clone(),
            ty::SM,
            FontWeight::BOLD,
            || Tone::Muted,
        ))
        .child(view().flex_1().min_w(px(0.0)).child(label_line(
            move || name.clone(),
            ty::MD,
            FontWeight::SEMIBOLD,
            Tone::Ink,
        )))
        .child(figure(
            move || when.clone(),
            ty::SM,
            FontWeight::MEDIUM,
            || Tone::Muted,
        ))
        .child(icon(
            if st.rtl() {
                Icon::ChevronLeft
            } else {
                Icon::ChevronRight
            },
            16.0,
            || Tone::Muted,
        ))
}

/// Leaving: the odometer, optional.
fn start_form(st: AppState, tour: String, close: impl Fn() + Clone + 'static) -> View {
    let value = signal(String::new());
    let error = signal(String::new());
    let save = {
        let close = close.clone();
        move || {
            let km = match value.get_untracked().trim() {
                "" => None,
                x => match x.replace([' ', '\u{a0}'], "").parse::<i64>() {
                    Ok(k) => Some(k),
                    Err(_) => {
                        error.set(st.tn("error.invalid_value"));
                        return;
                    }
                },
            };
            let id = tour.clone();
            let close = close.clone();
            run(
                st,
                move |db, s| start_tour(db, s.meta("tour.start"), &id, km),
                "dist.started",
                move |_| close(),
            );
        }
    };
    let body = view().gap(px(14.0)).child(field(
        st.t("myday.km_start"),
        value,
        String::new,
        move || error.get(),
        KeyboardType::Number,
        "start-km",
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
                Some(Icon::Play),
                st.t("myday.start"),
                save,
            )
            .test_id("start-save"),
        );
    dialog_frame(
        st,
        st.t("myday.start"),
        st.t("myday.start_hint"),
        body,
        footer,
        close,
    )
}

//! Distribution (M16, owner notes): the day on the road. The board shows
//! what is out, where the vehicles are, what comes next and when, and the
//! alerts (a vehicle near a customer not yet told, a silent one, a late
//! tour, failed stops, cash not counted); a tour opens on its stops, worked
//! one by one by the driver (arrived, delivered with what was collected,
//! failed with why), with a call, a WhatsApp message to the customer and
//! directions a press away. Tours are planned here from the customers of a
//! zone or a visit day and the orders waiting. Rules (who may plan, drive,
//! settle) are the use cases'; this page only hides what one may not do.

use nb_domain::geo::Point;
use nb_i18n::tr_args;
use nb_store::repo::access::UserRow;
use nb_store::usecases::access::list_users;
use nb_store::usecases::distribution::{
    Alert, Board, OpenOrder, RouteCustomer, RouteCustomerQuery, StopDone, StopRow, TourDetail,
    TourInput, TourQuery, TourRow, VehicleRow, Zone, arrive_stop, cancel_tour, complete_stop,
    create_tour, distribution_board, end_tour, fail_stop, get_tour, list_tours, list_vehicles,
    list_zones, mark_notified, move_stop, open_orders, optimize_tour, record_position, remove_stop,
    route_customers, settle_tour, start_tour,
};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::screens::common::{actions_row, date_time, export_button, export_rows, page};
use crate::screens::forms::{error_text, parse_price};
use crate::state::{AppState, Intent};
use crate::ui::chart::progress;
use crate::ui::filters::{money_tile, stat_row, stat_tile};
use crate::ui::form::{chips, field, labelled, search_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, checkbox, dot, figure, icon_button, label, label_line, segmented,
    switch,
};
use crate::ui::menu::{Act, row_actions};
use crate::ui::overlay::{dialog_frame, modal, show_toast, wide_modal};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon, empty_state};
use crate::ui::tokens::{Status, Tone, radius, ty};

// ---------------------------------------------------------------- shared

pub fn money(st: AppState, minor: i64) -> String {
    let c = st
        .session
        .get()
        .map_or(nb_core::Currency::DZD, |s| s.currency);
    st.fmt().money(nb_core::Money::from_minor(minor, c))
}

/// Runs `job` on the store; on success refreshes the data and says `done`,
/// otherwise shows why.
pub fn run<T: Send + 'static>(
    st: AppState,
    job: impl FnOnce(&mut nb_store::Store, crate::db::Session) -> nb_store::Result<T> + Send + 'static,
    done: &'static str,
    after: impl Fn(T) + 'static,
) {
    let Some(session) = st.session.get_untracked() else {
        return;
    };
    let db = st.db();
    let task = st.root.run(move || {
        spawn(async move {
            match db.call(move |s| job(s, session)).await {
                Ok(v) => {
                    st.changed();
                    if !done.is_empty() {
                        show_toast(st, st.tn(done));
                    }
                    after(v);
                }
                Err(e) => show_toast(
                    st,
                    error_text(st, e.codes.first().map_or("internal", |c| c.1)),
                ),
            }
        })
    });
    drop(task); // runs to completion under the root: a row or dialog may go first
}

/// The register of `branch` whose till is open (cash collected on the road
/// goes there).
pub fn open_register(s: &nb_store::Store, branch: &str) -> nb_store::Result<Option<String>> {
    let all = s.read(|r| {
        let o = nb_store::repo::org::the_org(r.conn())?
            .ok_or(nb_store::Error::NotFound("organization"))?;
        nb_store::usecases::org::registers(r.conn(), o.org_id()?)
    })?;
    for reg in all.into_iter().filter(|r| r.branch_id == branch) {
        if nb_store::usecases::sales::current_shift(s, &reg.id)?.is_some() {
            return Ok(Some(reg.id));
        }
    }
    Ok(None)
}

/// Opens a link outside the app (maps, phone, WhatsApp).
pub fn open_link(url: String) {
    drop(open_url(url));
}

/// The number in international form for links (Algerian numbers by
/// default: 0550… → 213550…).
pub fn intl_phone(phone: &str) -> Option<String> {
    let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 6 {
        return None;
    }
    Some(if phone.trim_start().starts_with('+') {
        digits
    } else if let Some(rest) = digits.strip_prefix("00") {
        rest.to_owned()
    } else if let Some(rest) = digits.strip_prefix('0') {
        format!("213{rest}")
    } else {
        digits
    })
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn whatsapp_url(phone: &str, message: &str) -> Option<String> {
    intl_phone(phone).map(|p| format!("https://wa.me/{p}?text={}", url_encode(message)))
}

pub fn tel_url(phone: &str) -> Option<String> {
    let p: String = phone
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    (p.len() >= 6).then(|| format!("tel:{p}"))
}

/// The local time `minutes` from now ("14:35").
pub fn clock_in(st: AppState, minutes: u32) -> String {
    let ms = crate::state::now_ms() + i64::from(minutes) * 60_000;
    let mins = (ms.div_euclid(60_000) + 60).rem_euclid(24 * 60);
    st.fmt().time((mins / 60) as u32, (mins % 60) as u32)
}

pub fn km(st: AppState, metres: u64) -> String {
    if metres < 1_000 {
        tr_args(
            st.lang(),
            "dist.metres",
            &[("n", &st.fmt().integer(metres as i64))],
        )
    } else {
        let rounded = (metres + 50) / 100 * 100;
        tr_args(
            st.lang(),
            "dist.km",
            &[(
                "n",
                &st.fmt()
                    .quantity(nb_core::Quantity::from_milli(rounded as i64), 1),
            )],
        )
    }
}

/// "3 / 5", kept left to right in Arabic too (isolated).
pub fn ratio(a: u32, b: u32) -> String {
    format!("\u{2066}{a} / {b}\u{2069}")
}

/// Today's bit in a visit-days mask (Monday = 1 … Sunday = 64).
pub fn today_bit() -> i64 {
    let days = (crate::state::now_ms() + 3_600_000).div_euclid(86_400_000);
    // 1970-01-01 was a Thursday (index 3 from Monday).
    1 << (days + 3).rem_euclid(7)
}

pub const DAYS: [&str; 7] = [
    "dist.day.mon",
    "dist.day.tue",
    "dist.day.wed",
    "dist.day.thu",
    "dist.day.fri",
    "dist.day.sat",
    "dist.day.sun",
];

pub fn days_text(st: AppState, mask: i64) -> String {
    let names: Vec<String> = (0..7)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| st.tn(DAYS[i]))
        .collect();
    names.join(" · ")
}

pub fn zone_tone(color: i64) -> Tone {
    const T: [Tone; 6] = [
        Tone::Primary,
        Tone::Info,
        Tone::Success,
        Tone::Warning,
        Tone::Danger,
        Tone::Neutral,
    ];
    T[usize::try_from(color.rem_euclid(6)).unwrap_or(0)]
}

pub fn stop_badge(st: AppState, status: &str) -> View {
    let (s, i) = match status {
        "done" => (Status::Success, Icon::CheckCircle),
        "arrived" => (Status::Info, Icon::MapPin),
        "failed" => (Status::Danger, Icon::XCircle),
        "skipped" => (Status::Neutral, Icon::ArrowForward),
        _ => (Status::Warning, Icon::Clock),
    };
    badge(s, Some(i), st.t_dyn(format!("dist.stop.{status}")))
}

pub fn tour_badge(st: AppState, t: &TourRow) -> View {
    let (s, i) = match t.status.as_str() {
        "on_road" => (Status::Info, Icon::Truck),
        "done" if !t.settled() && t.cash_expected_minor > 0 => (Status::Warning, Icon::Coins),
        "done" => (Status::Success, Icon::CheckCircle),
        "cancelled" => (Status::Neutral, Icon::XCircle),
        _ => (Status::Accent, Icon::Calendar),
    };
    let key = if t.status == "done" && !t.settled() && t.cash_expected_minor > 0 {
        "dist.tour.unsettled".to_owned()
    } else {
        format!("dist.tour.{}", t.status)
    };
    badge(s, Some(i), st.t_dyn(key))
}

/// A point on the mini map.
#[derive(Clone)]
pub struct Mark {
    pub point: Point,
    pub tone: Tone,
    /// A number or nothing (vehicles show a truck).
    pub text: String,
    pub vehicle: bool,
    pub faded: bool,
}

/// A plan of places: customers and vehicles where they are relative to
/// each other (north up), no tiles — it works offline. A press on the
/// "map" button opens the real map.
pub fn mini_map(marks: Vec<Mark>, height: f32) -> View {
    let mut frame = view()
        .w_full()
        .h(px(height))
        .rounded(px(radius::md()))
        .bg(Tone::Subtle.color())
        .border_1()
        .border_color(Tone::Border.color())
        .overflow_hidden()
        .direction(LayoutDirection::Ltr);
    // A faint grid.
    for k in 1..4 {
        let f = k as f32 / 4.0;
        frame = frame
            .child(
                view()
                    .absolute()
                    .top(relative(f))
                    .left(px(0.0))
                    .w_full()
                    .h(px(1.0))
                    .bg(Tone::Border.color()),
            )
            .child(
                view()
                    .absolute()
                    .left(relative(f))
                    .top(px(0.0))
                    .h_full()
                    .w(px(1.0))
                    .bg(Tone::Border.color()),
            );
    }
    if marks.is_empty() {
        return frame;
    }
    let lat = |m: &Mark| m.point.lat_e6;
    let lng = |m: &Mark| m.point.lng_e6;
    let (mut a0, mut a1) = (i64::MAX, i64::MIN);
    let (mut o0, mut o1) = (i64::MAX, i64::MIN);
    for m in &marks {
        a0 = a0.min(lat(m));
        a1 = a1.max(lat(m));
        o0 = o0.min(lng(m));
        o1 = o1.max(lng(m));
    }
    let span_a = (a1 - a0).max(2_000) as f32;
    let span_o = (o1 - o0).max(2_000) as f32;
    let pad = 0.1;
    let mut ordered = marks;
    // Vehicles on top.
    ordered.sort_by_key(|m| m.vehicle);
    for m in ordered {
        let x = pad + (1.0 - 2.0 * pad) * ((lng(&m) - o0) as f32 / span_o);
        let y = pad + (1.0 - 2.0 * pad) * (1.0 - (lat(&m) - a0) as f32 / span_a);
        let size = if m.vehicle { 30.0 } else { 22.0 };
        let tone = m.tone;
        let mut d = view()
            .absolute()
            .left(relative(x.clamp(0.0, 1.0)))
            .top(relative(y.clamp(0.0, 1.0)))
            .ml(px(-size / 2.0))
            .mt(px(-size / 2.0))
            .w(px(size))
            .h(px(size))
            .rounded_full()
            .items_center()
            .justify_center()
            .bg(tone.color())
            .border_2()
            .border_color(Tone::Surface.color())
            .shadow_xs();
        if m.faded {
            d = d.opacity(0.45);
        }
        d = if m.vehicle {
            d.child(icon(Icon::Truck, 16.0, || Tone::OnPrimary))
        } else {
            let t = m.text.clone();
            d.child(figure(
                move || t.clone(),
                ty::XS,
                FontWeight::BOLD,
                || Tone::OnPrimary,
            ))
        };
        frame = frame.child(d);
    }
    frame
}

/// A link opening all the places in a map (Google Maps directions through
/// them, up to the ten it takes).
pub fn route_url(from: Option<Point>, stops: &[Point]) -> Option<String> {
    let last = stops.last()?;
    let mut url = format!(
        "https://www.google.com/maps/dir/?api=1&destination={}",
        url_encode(&last.text().replace(' ', ""))
    );
    if let Some(f) = from {
        url.push_str(&format!(
            "&origin={}",
            url_encode(&f.text().replace(' ', ""))
        ));
    }
    let middle: Vec<String> = stops[..stops.len() - 1]
        .iter()
        .take(9)
        .map(|p| p.text().replace(' ', ""))
        .collect();
    if !middle.is_empty() {
        url.push_str(&format!("&waypoints={}", url_encode(&middle.join("|"))));
    }
    Some(url)
}

pub fn small_stat(st: AppState, which: Icon, key: &'static str, value: String, tone: Tone) -> View {
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
}

pub fn line(which: Icon, text: String, tone: Tone) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .min_w(px(0.0))
        .child(icon(which, 14.0, || Tone::Muted))
        .child(view().flex_1().min_w(px(0.0)).child(label_line(
            move || text.clone(),
            ty::SM,
            FontWeight::NORMAL,
            tone,
        )))
}

// ---------------------------------------------------------------- tours page

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    Live,
    Planned,
    Done,
    Unsettled,
    All,
}

impl Filter {
    fn code(self) -> &'static str {
        match self {
            Filter::Live => "live",
            Filter::Planned => "planned",
            Filter::Done => "done",
            Filter::Unsettled => "unsettled",
            Filter::All => "",
        }
    }
}

pub fn tours_page(st: AppState, compact: bool) -> View {
    // The board follows the road: read again every 30 seconds.
    let tick = signal(0u64);
    let timer = set_interval(std::time::Duration::from_secs(30), move || {
        tick.update(|t| *t += 1);
    });
    cleanup(move || drop(timer));
    let board = resource(
        move || st.session.get().map(|_| (st.data_rev.get(), tick.get())),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| distribution_board(s)).await }
        },
    );
    let b = computed(move || {
        board
            .with(|b: Option<&Board>| b.cloned())
            .unwrap_or_default()
    });
    let filter = signal(Filter::Live);
    let mine = signal(false);
    let tours = resource(
        move || {
            st.session
                .get()
                .map(|s| (st.data_rev.get(), filter.get(), mine.get(), s.actor))
        },
        move |(_, f, mine, me), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    list_tours(
                        s,
                        &TourQuery {
                            status: f.code().into(),
                            driver_id: mine.then(|| me.to_string()),
                            limit: 200,
                            ..Default::default()
                        },
                    )
                })
                .await
            }
        },
    );
    let tour_rows = computed(move || {
        tours
            .with(|t: Option<&Vec<TourRow>>| t.cloned())
            .unwrap_or_default()
    });
    let opened: Signal<Option<String>> = signal(None);
    // A tour asked for from another page (just planned there).
    if let Some(Intent::Tour(id)) = st.intent.get_untracked() {
        st.intent.set(None);
        opened.set(Some(id));
    }
    let open_detail = signal(false);
    effect(move || open_detail.set(opened.get().is_some()));
    effect(move || {
        if !open_detail.get() {
            opened.set(None);
        }
    });
    let planning = signal(false);

    let tiles = stat_row(
        vec![
            stat_tile(
                "dist-on-road",
                st.t("dist.on_road"),
                Icon::Truck,
                Status::Info,
                move || {
                    format!(
                        "\u{2066}{} / {}\u{2069}",
                        st.fmt().integer(i64::from(b.with(|b| b.on_road))),
                        st.fmt().integer(i64::from(b.with(|b| b.vehicles)))
                    )
                },
                || false,
                None,
                compact,
            ),
            stat_tile(
                "dist-stops",
                st.t("dist.stops_today"),
                Icon::MapPin,
                Status::Success,
                move || {
                    format!(
                        "\u{2066}{} / {}\u{2069}",
                        st.fmt().integer(i64::from(b.with(|b| b.done_today))),
                        st.fmt().integer(i64::from(b.with(|b| b.stops_today)))
                    )
                },
                || false,
                None,
                compact,
            ),
            money_tile(
                stat_tile(
                    "dist-collected",
                    st.t("dist.collected_today"),
                    Icon::Coins,
                    Status::Accent,
                    move || money(st, b.with(|b| b.collected_today_minor)),
                    || false,
                    None,
                    compact,
                ),
                compact,
            ),
            stat_tile(
                "dist-waiting",
                st.t("dist.waiting_orders"),
                Icon::ClipboardList,
                Status::Warning,
                move || st.fmt().integer(i64::from(b.with(|b| b.open_orders))),
                || false,
                Some(Box::new(move || st.route.set(Route::FieldOrders))),
                compact,
            ),
        ],
        compact,
    );

    let alerts = dynamic(move || {
        let list = b.with(|b| b.alerts.clone());
        if list.is_empty() {
            return view().into_element();
        }
        let mut v = view().gap(px(8.0));
        for (i, a) in list.into_iter().enumerate() {
            v = v.child(alert_row(st, i, a, opened));
        }
        card()
            .child(card_header_icon(
                Some(Icon::BellRing),
                st.t("dist.alerts"),
                Some(Box::new(st.t("dist.alerts_hint"))),
                view(),
            ))
            .child(view().px(px(16.0)).pb(px(16.0)).child(v))
            .into_element()
    });

    let live = dynamic(move || {
        let list = b.with(|b| b.live.clone());
        if list.is_empty() {
            return card()
                .child(empty_state(
                    Icon::Route,
                    st.t("dist.no_live"),
                    st.t("dist.no_live_hint"),
                    None,
                ))
                .into_element();
        }
        // All of today on one plan: vehicles and the stops still to do.
        let mut marks = Vec::new();
        for (k, l) in list.iter().enumerate() {
            let tone = zone_tone(k as i64);
            if let Some(p) = l.detail.vehicle_point {
                marks.push(Mark {
                    point: p,
                    tone,
                    text: String::new(),
                    vehicle: true,
                    faded: false,
                });
            }
            for (n, s) in l.detail.stops.iter().enumerate() {
                if let Some(p) = s.point {
                    marks.push(Mark {
                        point: p,
                        tone,
                        text: (n + 1).to_string(),
                        vehicle: false,
                        faded: !s.open(),
                    });
                }
            }
        }
        let mut cards = view()
            .grid()
            .grid_template_columns(SharedString::from(if compact {
                "repeat(1, minmax(0px, 1fr))".to_owned()
            } else {
                "repeat(auto-fill, minmax(320px, 1fr))".to_owned()
            }))
            .gap(px(16.0));
        for (k, l) in list.into_iter().enumerate() {
            cards = cards.child(live_card(st, k, l.detail, l.next, opened));
        }
        view()
            .gap(px(16.0))
            .child(
                card()
                    .child(card_header_icon(
                        Some(Icon::Map),
                        st.t("dist.plan_today"),
                        Some(Box::new(st.t("dist.plan_today_hint"))),
                        view(),
                    ))
                    .child(
                        view()
                            .px(px(16.0))
                            .pb(px(16.0))
                            .child(mini_map(marks, if compact { 220.0 } else { 300.0 })),
                    ),
            )
            .child(cards)
            .into_element()
    });

    let list = dynamic(move || {
        let rows = tour_rows.get();
        if rows.is_empty() {
            return empty_state(
                Icon::Calendar,
                st.t("dist.no_tours"),
                st.t("dist.no_tours_hint"),
                None,
            )
            .into_element();
        }
        let mut v = view().gap(px(8.0));
        for (i, t) in rows.into_iter().enumerate() {
            v = v.child(tour_line(st, i, t, opened));
        }
        v.into_element()
    });

    let export = export_button(st, "tours-export", move || {
        let mut out = vec![vec![
            st.tn("dist.number"),
            st.tn("dist.day"),
            st.tn("dist.vehicle"),
            st.tn("dist.driver"),
            st.tn("dist.status"),
            st.tn("dist.stops"),
            st.tn("dist.done"),
            st.tn("dist.failed"),
            st.tn("dist.collected"),
            st.tn("dist.counted"),
        ]];
        let c = st
            .session
            .get_untracked()
            .map_or(nb_core::Currency::DZD, |s| s.currency);
        let m = |v: i64| nb_core::Money::from_minor(v, c).to_plain_string();
        for t in tour_rows.get_untracked() {
            out.push(vec![
                t.number.clone(),
                t.day.clone(),
                t.vehicle.clone(),
                t.driver.clone().unwrap_or_default(),
                st.tn(&format!("dist.tour.{}", t.status)),
                t.stops.to_string(),
                t.done.to_string(),
                t.failed.to_string(),
                m(t.collected_minor),
                t.cash_counted_minor.map(m).unwrap_or_default(),
            ]);
        }
        export_rows(st, "dist.tours_file", out);
    });
    let add = view().child(when(
        move || st.can("distribution.plan"),
        move || {
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("dist.new_tour"),
                move || planning.set(true),
            )
            .test_id("tour-new")
            .into_element()
        },
    ));

    let tabs = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(12.0))
        .child(segmented(
            vec![
                (
                    Filter::Live,
                    Box::new(st.t("dist.f.live")) as Box<dyn Fn() -> String>,
                ),
                (Filter::Planned, Box::new(st.t("dist.f.planned"))),
                (Filter::Done, Box::new(st.t("dist.f.done"))),
                (Filter::Unsettled, Box::new(st.t("dist.f.unsettled"))),
                (Filter::All, Box::new(st.t("dist.f.all"))),
            ],
            filter,
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(switch(mine, st.t("dist.mine")))
                .child(label_line(
                    st.t("dist.mine"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Body,
                )),
        );

    view()
        .child(page(
            st,
            Route::Tours,
            "dist.subtitle",
            actions_row(vec![export, add]),
            view()
                .gap(px(20.0))
                .child(tiles)
                .child(alerts)
                .child(live)
                .child(
                    card()
                        .child(card_header_icon(
                            Some(Icon::Route),
                            st.t("dist.tours"),
                            None,
                            view(),
                        ))
                        .child(
                            view()
                                .px(px(16.0))
                                .pb(px(16.0))
                                .gap(px(12.0))
                                .child(tabs)
                                .child(list),
                        ),
                ),
            compact,
        ))
        .child(wide_modal(open_detail, compact, move || {
            match opened.get_untracked() {
                Some(id) => {
                    tour_detail(st, id, compact, move || open_detail.set(false)).into_element()
                }
                None => view().into_element(),
            }
        }))
        .child(modal(planning, compact, move || {
            planner(
                st,
                Vec::new(),
                Vec::new(),
                move || planning.set(false),
                move |id| opened.set(Some(id)),
            )
            .into_element()
        }))
}

fn alert_row(st: AppState, i: usize, a: Alert, opened: Signal<Option<String>>) -> View {
    let (which, status) = match a.kind.as_str() {
        "near" => (Icon::Navigation, Status::Info),
        "silent" => (Icon::Locate, Status::Warning),
        "late" => (Icon::CalendarClock, Status::Danger),
        "failed" => (Icon::XCircle, Status::Danger),
        "unsettled" => (Icon::Coins, Status::Warning),
        _ => (Icon::MapPin, Status::Neutral),
    };
    let value = match a.kind.as_str() {
        "near" => km(st, u64::try_from(a.value).unwrap_or(0)),
        "unsettled" => money(st, a.value),
        _ => st.fmt().integer(a.value),
    };
    let text = tr_args(
        st.lang(),
        &format!("dist.alert.{}", a.kind),
        &[
            ("tour", &a.tour_number),
            ("vehicle", &a.vehicle),
            ("customer", a.customer.as_deref().unwrap_or("")),
            ("value", &value),
        ],
    );
    let mut acts = Vec::new();
    if a.kind == "near" {
        let (phone, customer, stop) = (
            a.phone.clone().unwrap_or_default(),
            a.customer.clone().unwrap_or_default(),
            a.stop_id.clone().unwrap_or_default(),
        );
        if !phone.is_empty() {
            let msg = tr_args(
                st.lang(),
                "dist.msg.near",
                &[
                    ("name", &customer),
                    (
                        "shop",
                        &st.shop.get_untracked().map(|s| s.0).unwrap_or_default(),
                    ),
                ],
            );
            let stop2 = stop.clone();
            acts.push(Act::with(
                Icon::Message,
                st.tn("dist.whatsapp"),
                move || {
                    if let Some(u) = whatsapp_url(&phone, &msg) {
                        open_link(u);
                    }
                    let s = stop2.clone();
                    run(
                        st,
                        move |db, se| mark_notified(db, se.meta("tour.notify"), &s),
                        "",
                        |_| {},
                    );
                },
            ));
        }
        acts.push(Act::with(Icon::Check, st.tn("dist.mark_told"), move || {
            let s = stop.clone();
            run(
                st,
                move |db, se| mark_notified(db, se.meta("tour.notify"), &s),
                "dist.told",
                |_| {},
            );
        }));
    }
    let tour = a.tour_id.clone();
    acts.push(Act::with(Icon::Eye, st.tn("dist.open_tour"), move || {
        opened.set(Some(tour.clone()))
    }));
    view()
        .test_id(&format!("dist-alert-{i}"))
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .p(px(10.0))
        .rounded(px(radius::md()))
        .bg(status.bg().color())
        .child(icon(which, 18.0, move || status.fg()))
        .child(view().flex_1().min_w(px(0.0)).child(label(
            move || text.clone(),
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Ink,
        )))
        .child(row_actions(st, acts, &format!("dist-alert-more-{i}")))
}

fn live_card(
    st: AppState,
    k: usize,
    d: TourDetail,
    next: Option<StopRow>,
    opened: Signal<Option<String>>,
) -> View {
    let t = d.tour.clone();
    let share = if t.stops == 0 {
        0.0
    } else {
        (t.done + t.failed) as f32 / t.stops as f32
    };
    let (title, sub) = (
        format!("{} · {}", t.vehicle, t.number),
        t.driver.clone().unwrap_or_else(|| st.tn("dist.no_driver")),
    );
    let mut body = card()
        .test_id(&format!("live-{k}"))
        .p(px(16.0))
        .gap(px(12.0))
        .cursor_pointer()
        .on_press({
            let id = t.id.clone();
            move || opened.set(Some(id.clone()))
        })
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(dot(zone_tone(k as i64), 12.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label_line(
                            move || title.clone(),
                            ty::MD,
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
                .child(tour_badge(st, &t)),
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
                )),
        );
    match next {
        Some(n) => {
            let eta = n
                .eta_min
                .map(|m| {
                    tr_args(
                        st.lang(),
                        "dist.eta",
                        &[
                            ("min", &st.fmt().integer(i64::from(m))),
                            ("at", &clock_in(st, m)),
                        ],
                    )
                })
                .unwrap_or_else(|| st.tn("dist.eta_unknown"));
            let name = n.customer.clone();
            body = body.child(
                view()
                    .gap(px(4.0))
                    .p(px(10.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::PrimarySoft.color())
                    .child(label_line(
                        st.t("dist.next"),
                        ty::XS,
                        FontWeight::SEMIBOLD,
                        Tone::Primary,
                    ))
                    .child(label_line(
                        move || name.clone(),
                        ty::MD,
                        FontWeight::BOLD,
                        Tone::Ink,
                    ))
                    .child(label_line(
                        move || eta.clone(),
                        ty::SM,
                        FontWeight::NORMAL,
                        Tone::Body,
                    )),
            );
        }
        None => {
            body = body.child(label_line(
                st.t("dist.all_done"),
                ty::SM,
                FontWeight::MEDIUM,
                Tone::Success,
            ));
        }
    }
    if let Some(seen) = d.seen_at {
        let s = tr_args(st.lang(), "dist.seen", &[("at", &date_time(st, seen))]);
        body = body.child(line(Icon::Locate, s, Tone::Muted));
    }
    body
}

fn tour_line(st: AppState, i: usize, t: TourRow, opened: Signal<Option<String>>) -> View {
    let (number, info) = (
        t.number.clone(),
        format!(
            "{} · {} · {}",
            t.day,
            t.vehicle,
            t.driver.clone().unwrap_or_else(|| st.tn("dist.no_driver"))
        ),
    );
    let counts = ratio(t.done, t.stops);
    let id = t.id.clone();
    view()
        .test_id(&format!("tour-{i}"))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .p(px(12.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::Border.color())
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
        .on_press(move || opened.set(Some(id.clone())))
        .child(icon(Icon::Route, 18.0, || Tone::Primary))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(label_line(
                    move || number.clone(),
                    ty::MD,
                    FontWeight::BOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    move || info.clone(),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        )
        .child(figure(
            move || counts.clone(),
            ty::SM,
            FontWeight::SEMIBOLD,
            || Tone::Body,
        ))
        .child(tour_badge(st, &t))
}

// ---------------------------------------------------------------- tour detail

fn tour_detail(
    st: AppState,
    id: String,
    compact: bool,
    close: impl Fn() + Clone + 'static,
) -> View {
    let detail = resource(
        {
            let id = id.clone();
            move || st.session.get().map(|_| (st.data_rev.get(), id.clone()))
        },
        move |(_, id), _| {
            let db = st.db();
            async move { db.call(move |s| get_tour(s, &id)).await }
        },
    );
    let d = computed(move || detail.with(|d: Option<&TourDetail>| d.cloned()));
    // The stop being completed or failed (its form).
    let finishing: Signal<Option<(StopRow, bool)>> = signal(None);
    let finish_open = signal(false);
    effect(move || finish_open.set(finishing.get().is_some()));
    let ending = signal(false);
    let settling = signal(false);
    let locating = signal(false);

    let body = dynamic(move || {
        let Some(d) = d.get() else {
            return view().p(px(24.0)).into_element();
        };
        detail_body(st, d, compact, finishing, ending, settling, locating).into_element()
    });
    let title = {
        let fallback = st.tn("dist.tour");
        move || {
            d.with(|d| d.as_ref().map(|d| d.tour.number.clone()))
                .unwrap_or_else(|| fallback.clone())
        }
    };
    let footer = view().child(
        button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("action.close"),
            close.clone(),
        )
        .test_id("tour-close"),
    );
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(dialog_frame(
            st,
            title,
            st.t("dist.detail_hint"),
            view().child(body),
            footer,
            close,
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
        .child(modal(settling, compact, move || match d.get_untracked() {
            Some(d) => end_form(st, d, true, move || settling.set(false)).into_element(),
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
}

#[allow(clippy::too_many_arguments)] // the dialogs this body opens
fn detail_body(
    st: AppState,
    d: TourDetail,
    compact: bool,
    finishing: Signal<Option<(StopRow, bool)>>,
    ending: Signal<bool>,
    settling: Signal<bool>,
    locating: Signal<bool>,
) -> View {
    let t = d.tour.clone();
    let tid = t.id.clone();
    let on_road = t.status == "on_road";
    let planned = t.status == "planned";
    let live = on_road || planned;

    // The actions of the tour, by status and permission.
    let mut acts = Vec::new();
    if planned && st.can("distribution.drive") {
        let id = tid.clone();
        acts.push(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Play),
                st.t("dist.start"),
                move || {
                    let id = id.clone();
                    run(
                        st,
                        move |db, s| start_tour(db, s.meta("tour.start"), &id, None),
                        "dist.started",
                        |_| {},
                    );
                },
            )
            .test_id("tour-start"),
        );
    }
    if on_road && st.can("distribution.drive") {
        acts.push(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Flag),
                st.t("dist.end"),
                move || ending.set(true),
            )
            .test_id("tour-end"),
        );
        acts.push(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Locate),
                st.t("dist.position"),
                move || locating.set(true),
            )
            .test_id("tour-position"),
        );
    }
    if live && (st.can("distribution.plan") || st.can("distribution.drive")) && d.stops.len() > 1 {
        let id = tid.clone();
        acts.push(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Wand),
                st.t("dist.optimize"),
                move || {
                    let id = id.clone();
                    run(
                        st,
                        move |db, s| optimize_tour(db, s.meta("tour.optimize"), &id),
                        "dist.optimized",
                        |_| {},
                    );
                },
            )
            .test_id("tour-optimize"),
        );
    }
    let points: Vec<Point> = d
        .stops
        .iter()
        .filter(|s| s.open())
        .filter_map(|s| s.point)
        .collect();
    if !points.is_empty() {
        let from = d.vehicle_point;
        acts.push(button(
            Variant::Secondary,
            Size::Md,
            Some(Icon::Map),
            st.t("dist.open_map"),
            move || {
                if let Some(u) = route_url(from, &points) {
                    open_link(u);
                }
            },
        ));
    }
    if t.status == "done" && !t.settled() && st.can("distribution.settle") {
        acts.push(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Coins),
                st.t("dist.settle"),
                move || settling.set(true),
            )
            .test_id("tour-settle"),
        );
    }
    if planned && st.can("distribution.plan") {
        let id = tid.clone();
        acts.push(
            button(
                Variant::Danger,
                Size::Md,
                Some(Icon::XCircle),
                st.t("dist.cancel"),
                move || {
                    let id = id.clone();
                    let why = st.tn("dist.cancelled_note");
                    run(
                        st,
                        move |db, s| cancel_tour(db, s.meta("tour.cancel"), &id, &why),
                        "dist.cancelled",
                        |_| {},
                    );
                },
            )
            .test_id("tour-cancel"),
        );
    }

    let info = format!(
        "{} · {} · {}",
        t.day,
        t.vehicle,
        t.driver.clone().unwrap_or_else(|| st.tn("dist.no_driver"))
    );
    let kind = st.tn(&format!("dist.kind.{}", t.kind));
    let head = view()
        .gap(px(10.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(8.0))
                .child(tour_badge(st, &t))
                .child(badge(Status::Neutral, Some(Icon::Tag), move || {
                    kind.clone()
                }))
                .child(view().flex_1().min_w(px(120.0)).child(label_line(
                    move || info.clone(),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Body,
                ))),
        )
        .child(actions_row(acts));

    let money_row = view()
        .flex_row()
        .flex_wrap()
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
            "dist.route_km",
            km(st, d.route_m),
            Tone::Info,
        ))
        .child(small_stat(
            st,
            Icon::Receipt,
            "dist.orders_value",
            money(st, t.orders_minor),
            Tone::Primary,
        ))
        .child(small_stat(
            st,
            Icon::Coins,
            "dist.collected",
            money(st, t.collected_minor),
            Tone::Warning,
        ));

    let mut marks: Vec<Mark> = d
        .stops
        .iter()
        .enumerate()
        .filter_map(|(n, s)| {
            s.point.map(|p| Mark {
                point: p,
                tone: match s.status.as_str() {
                    "done" => Tone::Success,
                    "failed" => Tone::Danger,
                    "skipped" => Tone::Neutral,
                    _ => Tone::Primary,
                },
                text: (n + 1).to_string(),
                vehicle: false,
                faded: !s.open(),
            })
        })
        .collect();
    if let Some(p) = d.vehicle_point {
        marks.push(Mark {
            point: p,
            tone: Tone::Info,
            text: String::new(),
            vehicle: true,
            faded: false,
        });
    }

    let n_stops = d.stops.len();
    let mut stops = view().gap(px(8.0));
    for (n, s) in d.stops.iter().cloned().enumerate() {
        stops = stops.child(stop_row(st, n, n_stops, s, &t, finishing));
    }
    if d.stops.is_empty() {
        stops = stops.child(label(
            st.t("dist.no_stops"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ));
    }
    let mut v = view().gap(px(16.0)).child(head).child(money_row);
    if !marks.is_empty() {
        v = v.child(mini_map(marks, if compact { 200.0 } else { 260.0 }));
    }
    if let (Some(c), true) = (t.cash_counted_minor, t.status == "done") {
        let diff = c - t.cash_expected_minor;
        let s = tr_args(
            st.lang(),
            "dist.settled_line",
            &[
                ("expected", &money(st, t.cash_expected_minor)),
                ("counted", &money(st, c)),
                ("diff", &money(st, diff)),
            ],
        );
        v = v.child(line(
            Icon::Coins,
            s,
            if diff == 0 {
                Tone::Success
            } else {
                Tone::Danger
            },
        ));
    }
    v.child(stops)
}

fn stop_row(
    st: AppState,
    n: usize,
    count: usize,
    s: StopRow,
    t: &TourRow,
    finishing: Signal<Option<(StopRow, bool)>>,
) -> View {
    let on_road = t.status == "on_road";
    let live = on_road || t.status == "planned";
    let drive = st.can("distribution.drive");
    let plan = st.can("distribution.plan");
    let mut acts = Vec::new();
    if on_road && drive && s.status == "pending" {
        let id = s.id.clone();
        acts.push(Act::with(Icon::MapPin, st.tn("dist.arrived"), move || {
            let id = id.clone();
            run(
                st,
                move |db, se| arrive_stop(db, se.meta("tour.arrive"), &id),
                "",
                |_| {},
            );
        }));
    }
    if on_road && drive && s.open() {
        let s1 = s.clone();
        acts.push(Act::with(
            Icon::CheckCircle,
            st.tn("dist.deliver"),
            move || finishing.set(Some((s1.clone(), true))),
        ));
        let s2 = s.clone();
        acts.push(
            Act::with(Icon::XCircle, st.tn("dist.fail"), move || {
                finishing.set(Some((s2.clone(), false)))
            })
            .danger(),
        );
    }
    if let Some(doc) = s
        .document_id
        .clone()
        .filter(|_| s.document_status.as_deref() == Some("approved"))
        && st.can("sales.sell")
    {
        acts.push(Act::with(Icon::Cart, st.tn("dist.invoice"), move || {
            st.intent.set(Some(Intent::Invoice(doc.clone())));
            st.route.set(Route::Sell);
        }));
    }
    if let Some(u) = tel_url(&s.phone) {
        acts.push(Act::with(Icon::Phone, st.tn("dist.call"), move || {
            open_link(u.clone())
        }));
    }
    if !s.phone.is_empty() && s.open() {
        let eta = s.eta_min;
        let (phone, name, number) = (
            s.phone.clone(),
            s.customer.clone(),
            s.document_number
                .clone()
                .unwrap_or_else(|| t.number.clone()),
        );
        let stop_id = s.id.clone();
        acts.push(Act::with(
            Icon::Message,
            st.tn("dist.whatsapp"),
            move || {
                let shop = st.shop.get_untracked().map(|s| s.0).unwrap_or_default();
                let msg = match eta {
                    Some(m) if m <= 15 => tr_args(
                        st.lang(),
                        "dist.msg.near",
                        &[("name", &name), ("shop", &shop)],
                    ),
                    Some(m) => tr_args(
                        st.lang(),
                        "dist.msg.on_way",
                        &[
                            ("name", &name),
                            ("shop", &shop),
                            ("number", &number),
                            ("at", &clock_in(st, m)),
                        ],
                    ),
                    None => tr_args(
                        st.lang(),
                        "dist.msg.today",
                        &[("name", &name), ("shop", &shop), ("number", &number)],
                    ),
                };
                if let Some(u) = whatsapp_url(&phone, &msg) {
                    open_link(u);
                }
                let id = stop_id.clone();
                run(
                    st,
                    move |db, se| mark_notified(db, se.meta("tour.notify"), &id),
                    "",
                    |_| {},
                );
            },
        ));
    }
    if let Some(p) = s.point {
        acts.push(Act::with(
            Icon::Navigation,
            st.tn("dist.directions"),
            move || open_link(p.directions_url()),
        ));
    }
    if live && (plan || drive) && s.open() {
        if n > 0 {
            let id = s.id.clone();
            acts.push(Act::with(Icon::ArrowUp, st.tn("dist.move_up"), move || {
                let id = id.clone();
                run(
                    st,
                    move |db, se| move_stop(db, se.meta("tour.move"), &id, n - 1),
                    "",
                    |_| {},
                );
            }));
        }
        if n + 1 < count {
            let id = s.id.clone();
            acts.push(Act::with(
                Icon::ArrowDown,
                st.tn("dist.move_down"),
                move || {
                    let id = id.clone();
                    run(
                        st,
                        move |db, se| move_stop(db, se.meta("tour.move"), &id, n + 1),
                        "",
                        |_| {},
                    );
                },
            ));
        }
    }
    if live && plan && s.status == "pending" {
        let id = s.id.clone();
        acts.push(Act::Line);
        acts.push(
            Act::with(Icon::Trash, st.tn("dist.remove_stop"), move || {
                let id = id.clone();
                run(
                    st,
                    move |db, se| remove_stop(db, se.meta("tour.remove_stop"), &id),
                    "",
                    |_| {},
                );
            })
            .danger(),
        );
    }

    let tone = match s.status.as_str() {
        "done" => Tone::Success,
        "failed" => Tone::Danger,
        "skipped" => Tone::Neutral,
        "arrived" => Tone::Info,
        _ => Tone::Primary,
    };
    let seq = (n + 1).to_string();
    let name = s.customer.clone();
    let mut where_ = vec![s.address.clone(), s.city.clone()];
    if let Some(z) = &s.zone {
        where_.push(z.clone());
    }
    let where_: String = where_
        .into_iter()
        .filter(|x| !x.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let mut info = view()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(2.0))
        .child(label_line(
            move || name.clone(),
            ty::MD,
            FontWeight::BOLD,
            Tone::Ink,
        ));
    if !where_.is_empty() {
        info = info.child(label_line(
            move || where_.clone(),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        ));
    }
    let mut facts = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(6.0))
        .child(stop_badge(st, &s.status));
    if let (Some(num), Some(total)) = (s.document_number.clone(), s.document_total_minor) {
        let txt = format!("{num} · {}", money(st, total));
        facts = facts.child(badge(Status::Accent, Some(Icon::Receipt), move || {
            txt.clone()
        }));
    }
    if s.open() {
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
    }
    if s.collected_minor > 0 {
        let txt = money(st, s.collected_minor);
        facts = facts.child(badge(Status::Success, Some(Icon::Coins), move || {
            txt.clone()
        }));
    }
    info = info.child(facts);
    for (txt, which) in [
        (s.delivery_note.clone(), Icon::Info),
        (
            if s.fail_reason.is_empty() || s.fail_reason == "tour_ended" {
                String::new()
            } else {
                s.fail_reason.clone()
            },
            Icon::Alert,
        ),
        (
            if s.receiver.is_empty() {
                String::new()
            } else {
                tr_args(st.lang(), "dist.received_by", &[("name", &s.receiver)])
            },
            Icon::UserCheck,
        ),
        (s.note.clone(), Icon::FileText),
    ] {
        if !txt.trim().is_empty() {
            info = info.child(line(which, txt, Tone::Body));
        }
    }
    view()
        .test_id(&format!("stop-{n}"))
        .flex_row()
        .gap(px(12.0))
        .p(px(12.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(if s.status == "arrived" {
            Tone::Info.color()
        } else {
            Tone::Border.color()
        })
        .child(
            view()
                .w(px(28.0))
                .h(px(28.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(tone.color())
                .flex_shrink_0()
                .child(figure(
                    move || seq.clone(),
                    ty::SM,
                    FontWeight::BOLD,
                    || Tone::OnPrimary,
                )),
        )
        .child(info)
        .child(row_actions(st, acts, &format!("stop-more-{n}")))
}

/// Delivered (who received it, what was collected) or failed (why).
pub fn stop_form(
    st: AppState,
    stop: StopRow,
    ok: bool,
    close: impl Fn() + Clone + 'static,
) -> View {
    let receiver = signal(String::new());
    let collected = signal(String::new());
    let note = signal(String::new());
    let reason = signal(0usize);
    let other = signal(String::new());
    let method = signal(0usize);
    let error = signal(String::new());
    const REASONS: [&str; 6] = [
        "dist.reason.closed",
        "dist.reason.absent",
        "dist.reason.refused",
        "dist.reason.no_money",
        "dist.reason.no_time",
        "dist.reason.other",
    ];
    const METHODS: [&str; 3] = ["cash", "card", "transfer"];
    let save = {
        let close = close.clone();
        let id = stop.id.clone();
        move || {
            error.set(String::new());
            let id = id.clone();
            let close = close.clone();
            if ok {
                let c = st
                    .session
                    .get_untracked()
                    .map_or(nb_core::Currency::DZD, |s| s.currency);
                let amount = match collected.get_untracked().trim() {
                    "" => 0,
                    t => match parse_price(t, c) {
                        Ok(m) => m.minor(),
                        Err(_) => {
                            error.set(st.tn("error.invalid_value"));
                            return;
                        }
                    },
                };
                let done = StopDone {
                    receiver: receiver.get_untracked(),
                    collected_minor: amount,
                    method: METHODS[method.get_untracked().min(2)].into(),
                    register_id: None,
                    note: note.get_untracked(),
                };
                run(
                    st,
                    move |db, s| {
                        let mut done = done;
                        if done.collected_minor > 0 && done.method == "cash" {
                            done.register_id = open_register(db, &s.branch().to_string())?;
                        }
                        complete_stop(db, s.meta("tour.complete"), &id, done)
                    },
                    "dist.delivered",
                    move |_| close(),
                );
            } else {
                let k = reason.get_untracked().min(REASONS.len() - 1);
                let why = if k == REASONS.len() - 1 {
                    other.get_untracked()
                } else {
                    st.tn(REASONS[k])
                };
                run(
                    st,
                    move |db, s| fail_stop(db, s.meta("tour.fail"), &id, &why, false),
                    "dist.failed_saved",
                    move |_| close(),
                );
            }
        }
    };
    let mut body = view().gap(px(14.0));
    let name = stop.customer.clone();
    body = body.child(line(Icon::Store, name, Tone::Ink));
    if ok {
        if let (Some(n), Some(total)) = (stop.document_number.clone(), stop.document_total_minor) {
            body = body.child(line(
                Icon::Receipt,
                format!("{n} · {}", money(st, total)),
                Tone::Body,
            ));
        }
        if stop.balance_minor > 0 {
            let s = tr_args(
                st.lang(),
                "dist.owes",
                &[("amount", &money(st, stop.balance_minor))],
            );
            body = body.child(line(Icon::Coins, s, Tone::Warning));
        }
        body = body
            .child(field(
                st.t("dist.receiver"),
                receiver,
                String::new,
                String::new,
                KeyboardType::Default,
                "stop-receiver",
            ))
            .child(field(
                st.t("dist.collected"),
                collected,
                String::new,
                move || error.get(),
                KeyboardType::Decimal,
                "stop-collected",
            ))
            .child(chips(
                st.t("dist.method"),
                METHODS
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        (
                            i,
                            Box::new(st.t_dyn(format!("pay.{m}"))) as Box<dyn Fn() -> String>,
                        )
                    })
                    .collect(),
                method,
            ))
            .child(field(
                st.t("dist.note"),
                note,
                String::new,
                String::new,
                KeyboardType::Default,
                "stop-note",
            ));
    } else {
        body = body
            .child(chips(
                st.t("dist.reason"),
                REASONS
                    .iter()
                    .enumerate()
                    .map(|(i, k)| (i, Box::new(st.t(k)) as Box<dyn Fn() -> String>))
                    .collect(),
                reason,
            ))
            .child(when(
                move || reason.get() == REASONS.len() - 1,
                move || {
                    field(
                        st.t("dist.reason_other"),
                        other,
                        String::new,
                        move || error.get(),
                        KeyboardType::Default,
                        "stop-reason",
                    )
                    .into_element()
                },
            ));
    }
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
                if ok {
                    Variant::Primary
                } else {
                    Variant::Danger
                },
                Size::Md,
                Some(if ok { Icon::Check } else { Icon::XCircle }),
                st.t(if ok { "dist.deliver" } else { "dist.fail" }),
                save,
            )
            .test_id("stop-save"),
        );
    dialog_frame(
        st,
        st.t(if ok {
            "dist.deliver_title"
        } else {
            "dist.fail_title"
        }),
        st.t(if ok {
            "dist.deliver_hint"
        } else {
            "dist.fail_hint"
        }),
        body,
        footer,
        close,
    )
}

/// Ending a tour (its odometer) or settling it (the cash counted).
pub fn end_form(
    st: AppState,
    d: TourDetail,
    settle: bool,
    close: impl Fn() + Clone + 'static,
) -> View {
    let value = signal(String::new());
    let note = signal(String::new());
    let error = signal(String::new());
    let t = d.tour.clone();
    let left = d.stops.iter().filter(|s| s.open()).count();
    let save = {
        let close = close.clone();
        let id = t.id.clone();
        move || {
            error.set(String::new());
            let id = id.clone();
            let close = close.clone();
            let v = value.get_untracked();
            if settle {
                let c = st
                    .session
                    .get_untracked()
                    .map_or(nb_core::Currency::DZD, |s| s.currency);
                let Ok(m) = parse_price(v.trim(), c) else {
                    error.set(st.tn("error.invalid_value"));
                    return;
                };
                let n = note.get_untracked();
                run(
                    st,
                    move |db, s| settle_tour(db, s.meta("tour.settle"), &id, m.minor(), &n),
                    "dist.settled",
                    move |_| close(),
                );
            } else {
                let km = match v.trim() {
                    "" => None,
                    x => match x.replace([' ', '\u{a0}'], "").parse::<i64>() {
                        Ok(k) => Some(k),
                        Err(_) => {
                            error.set(st.tn("error.invalid_value"));
                            return;
                        }
                    },
                };
                run(
                    st,
                    move |db, s| end_tour(db, s.meta("tour.end"), &id, km),
                    "dist.ended",
                    move |_| close(),
                );
            }
        }
    };
    let mut body = view().gap(px(14.0));
    if settle {
        let s = tr_args(
            st.lang(),
            "dist.expected",
            &[("amount", &money(st, t.cash_expected_minor))],
        );
        body = body
            .child(line(Icon::Coins, s, Tone::Ink))
            .child(field(
                st.t("dist.counted"),
                value,
                String::new,
                move || error.get(),
                KeyboardType::Decimal,
                "settle-counted",
            ))
            .child(field(
                st.t("dist.note"),
                note,
                String::new,
                String::new,
                KeyboardType::Default,
                "settle-note",
            ));
    } else {
        if left > 0 {
            let s = tr_args(
                st.lang(),
                "dist.left_skipped",
                &[("n", &st.fmt().integer(left as i64))],
            );
            body = body.child(line(Icon::Alert, s, Tone::Warning));
        }
        let s = tr_args(
            st.lang(),
            "dist.collected_line",
            &[("amount", &money(st, t.collected_minor))],
        );
        body = body.child(line(Icon::Coins, s, Tone::Body)).child(field(
            st.t("dist.km_end"),
            value,
            String::new,
            move || error.get(),
            KeyboardType::Number,
            "end-km",
        ));
    }
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
                Some(if settle { Icon::Coins } else { Icon::Flag }),
                st.t(if settle { "dist.settle" } else { "dist.end" }),
                save,
            )
            .test_id("end-save"),
        );
    dialog_frame(
        st,
        st.t(if settle {
            "dist.settle_title"
        } else {
            "dist.end_title"
        }),
        st.t(if settle {
            "dist.settle_hint"
        } else {
            "dist.end_hint"
        }),
        body,
        footer,
        close,
    )
}

/// Where the vehicle is now: coordinates or a map link pasted from the
/// driver's phone.
pub fn position_form(
    st: AppState,
    vehicle: String,
    name: String,
    close: impl Fn() + Clone + 'static,
) -> View {
    let text = signal(String::new());
    let error = signal(String::new());
    let save = {
        let close = close.clone();
        move || {
            let p = text.get_untracked();
            let v = vehicle.clone();
            let close = close.clone();
            error.set(String::new());
            if let Err(e) = nb_domain::geo::parse_point(&p) {
                error.set(error_text(st, e.code));
                return;
            }
            run(
                st,
                move |db, s| record_position(db, s.meta("vehicle.position"), &v, &p, None),
                "dist.position_saved",
                move |_| close(),
            );
        }
    };
    let body = view()
        .gap(px(14.0))
        .child(line(Icon::Truck, name, Tone::Ink))
        .child(field(
            st.t("dist.position_field"),
            text,
            st.t("dist.position_placeholder"),
            move || error.get(),
            KeyboardType::Default,
            "position-text",
        ))
        .child(label(
            st.t("dist.position_help"),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
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
                Some(Icon::Locate),
                st.t("form.save"),
                save,
            )
            .test_id("position-save"),
        );
    dialog_frame(
        st,
        st.t("dist.position"),
        st.t("dist.position_hint"),
        body,
        footer,
        close,
    )
}

// ---------------------------------------------------------------- planner

/// Plans a tour: a vehicle, a driver, a day and its stops, from the
/// customers of a zone or visit day and the orders waiting. `customers`
/// and `orders` come ticked (from the field orders page).
pub fn planner(
    st: AppState,
    customers: Vec<String>,
    orders: Vec<String>,
    close: impl Fn() + Clone + 'static,
    created: impl Fn(String) + Clone + 'static,
) -> View {
    let vehicles = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_vehicles(s)).await }
        },
    );
    let vs = computed(move || {
        vehicles
            .with(|v: Option<&Vec<VehicleRow>>| v.cloned())
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

    let vehicle = signal(0usize);
    // 0: the vehicle's own driver; then the users in order.
    let driver = signal(0usize);
    let day = signal(String::new());
    let kind = signal(0usize);
    const KINDS: [&str; 3] = ["delivery", "van", "presell"];
    // 0: all zones.
    let zone = signal(0usize);
    // 0: any day; 1: today's customers.
    let which_day = signal(if customers.is_empty() && orders.is_empty() {
        1usize
    } else {
        0
    });
    let search = signal(String::new());
    let optimize = signal(true);
    let picked_c: Signal<Vec<String>> = signal(customers);
    let picked_o: Signal<Vec<String>> = signal(orders);
    let error = signal(String::new());

    let zone_id = move || {
        let z = zone.get();
        if z == 0 {
            None
        } else {
            zs.with(|zs| zs.get(z - 1).map(|z| z.id.clone()))
        }
    };
    let candidates = resource(
        move || {
            st.session
                .get()
                .map(|_| (st.data_rev.get(), zone_id(), which_day.get(), search.get()))
        },
        move |(_, zone, days, q), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    route_customers(
                        s,
                        &RouteCustomerQuery {
                            search: q,
                            zone_id: zone,
                            days: if days == 1 { today_bit() } else { 0 },
                            ..Default::default()
                        },
                    )
                })
                .await
            }
        },
    );
    let cs = computed(move || {
        candidates
            .with(|c: Option<&Vec<RouteCustomer>>| c.cloned())
            .unwrap_or_default()
    });
    let waiting = resource(
        move || st.session.get().map(|_| (st.data_rev.get(), zone_id())),
        move |(_, zone), _| {
            let db = st.db();
            async move { db.call(move |s| open_orders(s, zone.as_deref())).await }
        },
    );
    let os = computed(move || {
        waiting
            .with(|o: Option<&Vec<OpenOrder>>| o.cloned())
            .unwrap_or_default()
    });

    let save = {
        let close = close.clone();
        let created = created.clone();
        move || {
            error.set(String::new());
            let Some(v) =
                vs.with_untracked(|v| v.get(vehicle.get_untracked()).map(|v| v.id.clone()))
            else {
                error.set(st.tn("dist.need_vehicle"));
                return;
            };
            let drv = match driver.get_untracked() {
                0 => None,
                k => us.with_untracked(|u| u.get(k - 1).map(|u| u.id.clone())),
            };
            let input = TourInput {
                vehicle_id: v,
                driver_id: drv,
                day: day.get_untracked().trim().to_owned(),
                kind: KINDS[kind.get_untracked().min(2)].into(),
                customers: picked_c.get_untracked(),
                orders: picked_o.get_untracked(),
                optimize: optimize.get_untracked(),
                note: String::new(),
            };
            let close = close.clone();
            let created = created.clone();
            run(
                st,
                move |db, s| create_tour(db, s.meta("tour.create"), input),
                "dist.planned",
                move |d: TourDetail| {
                    close();
                    created(d.tour.id);
                },
            );
        }
    };

    let pick_row = |checked: Box<dyn Fn() -> bool>,
                    toggle: Box<dyn Fn()>,
                    title: String,
                    sub: String,
                    tid: String| {
        let checked = std::rc::Rc::new(checked);
        let c2 = checked.clone();
        view()
            .test_id(&tid)
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .py(px(8.0))
            .px(px(10.0))
            .rounded(px(radius::md()))
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Hover.color()))
            .on_press(toggle)
            .child(checkbox(move || Some(c2()), || {}))
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(label_line(
                        move || title.clone(),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    ))
                    .child(label_line(
                        move || sub.clone(),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )),
            )
    };

    let orders_list = dynamic(move || {
        let list = os.get();
        if list.is_empty() {
            return label(
                st.t("dist.no_waiting"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        }
        let mut v = view().gap(px(2.0));
        for (i, o) in list.into_iter().enumerate() {
            let id = o.id.clone();
            let id2 = o.id.clone();
            let sub = format!(
                "{} · {} · {}",
                o.customer,
                o.zone.clone().unwrap_or_default(),
                money(st, o.total_minor)
            );
            v = v.child(pick_row(
                Box::new(move || picked_o.with(|p| p.contains(&id))),
                Box::new(move || {
                    picked_o.update(|p| {
                        if let Some(k) = p.iter().position(|x| *x == id2) {
                            p.remove(k);
                        } else {
                            p.push(id2.clone());
                        }
                    })
                }),
                o.number.clone(),
                sub,
                format!("plan-order-{i}"),
            ));
        }
        v.into_element()
    });
    let customers_list = dynamic(move || {
        let list = cs.get();
        if list.is_empty() {
            return label(
                st.t("dist.no_candidates"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        }
        let mut v = view().gap(px(2.0));
        for (i, c) in list.into_iter().take(150).enumerate() {
            let id = c.id.clone();
            let id2 = c.id.clone();
            let mut sub = vec![c.city.clone()];
            if let Some(z) = &c.zone {
                sub.push(z.clone());
            }
            if c.point.is_none() {
                sub.push(st.tn("dist.no_place"));
            }
            if c.open_orders > 0 {
                sub.push(tr_args(
                    st.lang(),
                    "dist.n_orders",
                    &[("n", &c.open_orders.to_string())],
                ));
            }
            let sub = sub
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            v = v.child(pick_row(
                Box::new(move || picked_c.with(|p| p.contains(&id))),
                Box::new(move || {
                    picked_c.update(|p| {
                        if let Some(k) = p.iter().position(|x| *x == id2) {
                            p.remove(k);
                        } else {
                            p.push(id2.clone());
                        }
                    })
                }),
                c.name.clone(),
                sub,
                format!("plan-customer-{i}"),
            ));
        }
        v.into_element()
    });
    let pick_all = button(
        Variant::Ghost,
        Size::Sm,
        Some(Icon::CheckCircle),
        st.t("dist.pick_all"),
        move || {
            let all: Vec<String> = cs.get_untracked().into_iter().map(|c| c.id).collect();
            picked_c.update(|p| {
                for id in all {
                    if !p.contains(&id) {
                        p.push(id);
                    }
                }
            });
        },
    )
    .test_id("plan-all");

    let counts = move || {
        tr_args(
            st.lang(),
            "dist.picked",
            &[
                ("orders", &picked_o.with(|p| p.len()).to_string()),
                ("customers", &picked_c.with(|p| p.len()).to_string()),
            ],
        )
    };
    let body = view()
        .gap(px(14.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(12.0))
                .child(view().flex_1().min_w(px(200.0)).child(labelled(
                    st.t("dist.vehicle"),
                    dropdown(
                        move || {
                            vs.get()
                                .into_iter()
                                .enumerate()
                                .map(|(i, v)| (i, format!("{} · {}", v.name, v.plate)))
                                .collect()
                        },
                        vehicle,
                        Some(Icon::Truck),
                        st.t("dist.vehicle"),
                        "plan-vehicle",
                    ),
                    None,
                )))
                .child(view().flex_1().min_w(px(200.0)).child(labelled(
                    st.t("dist.driver"),
                    dropdown(
                        move || {
                            let mut v = vec![(0usize, st.tn("dist.vehicle_driver"))];
                            v.extend(
                                us.get()
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, u)| (i + 1, u.name)),
                            );
                            v
                        },
                        driver,
                        Some(Icon::User),
                        st.t("dist.driver"),
                        "plan-driver",
                    ),
                    None,
                ))),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(12.0))
                .child(view().flex_1().min_w(px(200.0)).child(field(
                    st.t("dist.day"),
                    day,
                    st.t("dist.day_placeholder"),
                    String::new,
                    KeyboardType::Default,
                    "plan-day",
                )))
                .child(
                    view().flex_1().min_w(px(200.0)).child(chips(
                        st.t("dist.kind"),
                        KINDS
                            .iter()
                            .enumerate()
                            .map(|(i, k)| {
                                (
                                    i,
                                    Box::new(st.t_dyn(format!("dist.kind.{k}")))
                                        as Box<dyn Fn() -> String>,
                                )
                            })
                            .collect(),
                        kind,
                    )),
                ),
        )
        .child(
            view()
                .gap(px(8.0))
                .child(label(
                    st.t("dist.waiting_orders"),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(orders_list),
        )
        .child(
            view()
                .gap(px(8.0))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(view().flex_1().child(label(
                            st.t("dist.customers"),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        )))
                        .child(pick_all),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(8.0))
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
                            "plan-zone",
                        )))
                        .child(segmented(
                            vec![
                                (
                                    0usize,
                                    Box::new(st.t("dist.any_day")) as Box<dyn Fn() -> String>,
                                ),
                                (1usize, Box::new(st.t("dist.today_visits"))),
                            ],
                            which_day,
                        ))
                        .child(view().flex_1().min_w(px(160.0)).child(search_field(
                            search,
                            st.t("dist.search_customers"),
                            "plan-search",
                        ))),
                )
                .child(
                    view()
                        .max_h(px(280.0))
                        .child(scroll().child(customers_list)),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(switch(optimize, st.t("dist.optimize_on_save")))
                .child(label_line(
                    st.t("dist.optimize_on_save"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Body,
                )),
        )
        .child(label_line(
            counts,
            ty::SM,
            FontWeight::SEMIBOLD,
            Tone::Primary,
        ))
        .child(dynamic(move || {
            let e = error.get();
            if e.is_empty() {
                view().into_element()
            } else {
                label(move || e.clone(), ty::SM, FontWeight::MEDIUM, Tone::Danger).into_element()
            }
        }));
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
                Some(Icon::Route),
                st.t("dist.plan_save"),
                save,
            )
            .test_id("plan-save"),
        );
    dialog_frame(
        st,
        st.t("dist.new_tour"),
        st.t("dist.new_tour_hint"),
        body,
        footer,
        close,
    )
}

/// An icon button opening a place in the map (used by the field pages).
pub fn map_button(st: AppState, p: Option<Point>) -> View {
    match p {
        Some(p) => icon_button(
            || Icon::Map,
            st.t("dist.open_map"),
            Variant::Ghost,
            move || open_link(p.map_url()),
        ),
        None => view(),
    }
}

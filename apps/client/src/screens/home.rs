//! Home: the store at a glance (M13) — sales, invoices, average basket and
//! what customers owe, with their trend against the period before; sales
//! over time, payment methods, the latest invoices, sales by category and
//! what runs out. Everything is read from the records
//! (`nb_store::usecases::reports::dashboard`).

use std::rc::Rc;

use nb_core::Money;
use nb_store::usecases::reports::{Dashboard, Period, dashboard};
use nsh_core::element::View;
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::screens::common::date_time;
use crate::state::AppState;
use crate::ui::chart::{Series, area_chart, bar_list, donut, legend, legend_row, sparkline};
use crate::ui::icon::Icon;
use crate::ui::kit::{Size, Variant, avatar, badge, button, figure, label, label_line, segmented};
use crate::ui::surface::{
    card, card_body, card_header_icon, delta, empty_state, kpi, page_header_full, page_pad,
};
use crate::ui::tokens::{Status, Tone, sp, ty};

fn money(st: AppState, minor: i64) -> Money {
    let c = st
        .session
        .get_untracked()
        .map_or(nb_core::Currency::DZD, |s| s.currency);
    Money::from_minor(minor, c)
}

/// Change in basis points between two totals (display only).
fn change_bps(cur: i64, prev: i64) -> i64 {
    if prev == 0 {
        0
    } else {
        (cur - prev).saturating_mul(10_000) / prev
    }
}

fn period_key(p: Period) -> &'static str {
    match p {
        Period::Week => "period.7d",
        Period::Month => "period.30d",
        Period::Year => "period.12m",
    }
}

pub fn home(st: AppState, compact: bool) -> View {
    let period = signal(Period::Month);
    let gap = if compact { 16.0 } else { 20.0 };
    let data = resource(
        move || st.session.get().map(|_| (period.get(), st.data_rev.get())),
        move |(p, _), _| {
            let db = st.db();
            async move { db.call(move |s| dashboard(s, p)).await }
        },
    );
    let board = computed(move || data.with(|d: Option<&Dashboard>| d.cloned()));
    let periods = move || {
        segmented(
            [Period::Week, Period::Month, Period::Year]
                .into_iter()
                .map(|p| (p, Box::new(st.t(period_key(p))) as Box<dyn Fn() -> String>))
                .collect(),
            period,
        )
    };
    let actions = if compact {
        view()
    } else {
        view().child(periods()).child(when(
            move || st.can("reports.view"),
            move || {
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::ChartLine),
                    st.t("reports.open"),
                    move || st.route.set(Route::Reports),
                )
                .into_element()
            },
        ))
    };
    let mut page = view()
        .w_full()
        .p(px(page_pad(compact)))
        .gap(px(gap))
        .child(page_header_full(
            Some(Icon::Dashboard),
            None,
            st.t("dashboard.title"),
            Some(Box::new(st.t("dashboard.subtitle"))),
            actions,
            compact,
        ));
    if compact {
        page = page.child(scroll().horizontal().child(periods()));
    }
    page.child(dynamic(move || {
        let Some(d) = board.get() else {
            return view().into_element();
        };
        let p = period.get_untracked();
        view()
            .gap(px(gap))
            .child(kpis(st, &d, compact))
            .child(
                row(compact, gap)
                    .child(column(compact, 2.0).child(sales_card(st, &d, p, compact)))
                    .child(column(compact, 1.0).child(payments_card(st, &d))),
            )
            .child(
                row(compact, gap)
                    .child(column(compact, 2.0).child(invoices_card(st, &d, compact)))
                    .child(
                        column(compact, 1.0)
                            .gap(px(gap))
                            .child(categories_card(st, &d))
                            .child(low_stock_card(st, &d)),
                    ),
            )
            .into_element()
    }))
}

/// A column of a dashboard row: shares the width on desktop, full width on phones.
fn column(compact: bool, grow: f32) -> View {
    if compact {
        view()
    } else {
        view().flex_grow(grow).basis(px(0.0)).min_w(px(0.0))
    }
}

fn row(compact: bool, gap: f32) -> View {
    if compact {
        view().gap(px(gap))
    } else {
        view().flex_row().items_start().gap(px(gap))
    }
}

fn kpis(st: AppState, d: &Dashboard, compact: bool) -> View {
    let rtl = st.rtl();
    let total: i64 = d.current.iter().sum();
    let total_prev: i64 = d.previous.iter().sum();
    let n_orders: i64 = d.orders.iter().sum();
    let basket = if n_orders == 0 { 0 } else { total / n_orders };
    let basket_prev = if d.orders_previous == 0 {
        0
    } else {
        total_prev / d.orders_previous
    };
    let credit = d.credit_minor;
    let owing = i64::from(d.customers_owing);
    let pct = move |bps: i64| {
        let f = st.fmt();
        move || f.percent_signed(bps)
    };
    let d_total = change_bps(total, total_prev);
    let d_orders = change_bps(n_orders, d.orders_previous);
    let d_basket = change_bps(basket, basket_prev);
    let spark_w = if compact { 110.0 } else { 160.0 };
    let trend = |bps: i64, prev: i64| (prev > 0).then(|| delta(bps, pct(bps)));
    let tiles = [
        kpi(
            st.t("kpi.revenue"),
            move || st.fmt().money_whole(money(st, total)),
            trend(d_total, total_prev),
            sparkline(d.current.clone(), spark_w, 36.0, Tone::Primary, rtl),
            compact,
        ),
        kpi(
            st.t("kpi.orders"),
            move || st.fmt().integer(n_orders),
            trend(d_orders, d.orders_previous),
            sparkline(d.orders.clone(), spark_w, 36.0, Tone::Primary, rtl),
            compact,
        ),
        kpi(
            st.t("kpi.basket"),
            move || st.fmt().money_whole(money(st, basket)),
            trend(d_basket, basket_prev),
            view().child(label(
                st.t("kpi.vs_prev"),
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            )),
            compact,
        ),
        kpi(
            st.t("kpi.credit"),
            move || st.fmt().money_whole(money(st, credit)),
            None,
            view().flex_row().child(badge(
                if owing > 0 {
                    Status::Warning
                } else {
                    Status::Neutral
                },
                Some(Icon::Users),
                st.t_with("kpi.customers_owing", move |s| {
                    vec![("count", s.fmt().integer(owing))]
                }),
            )),
            compact,
        ),
    ];
    let mut grid = view()
        .flex_row()
        .flex_wrap()
        .gap(px(if compact { 12.0 } else { 20.0 }));
    for t in tiles {
        grid = grid.child(if compact {
            t.min_w(px(150.0)).basis(relative(0.45))
        } else {
            t
        });
    }
    grid
}

/// A bucket's label: its day, or its month for the year.
fn labels(st: AppState, d: &Dashboard, p: Period) -> Vec<String> {
    let f = st.fmt();
    d.labels
        .iter()
        .map(|iso| {
            let m: u32 = iso.get(5..7).and_then(|x| x.parse().ok()).unwrap_or(1);
            let day: u32 = iso.get(8..10).and_then(|x| x.parse().ok()).unwrap_or(1);
            if p == Period::Year {
                f.month_short(m).to_owned()
            } else {
                f.day_month(m, day)
            }
        })
        .collect()
}

fn sales_card(st: AppState, d: &Dashboard, p: Period, compact: bool) -> View {
    let total: i64 = d.current.iter().sum();
    let series = vec![
        Series {
            name: st.tn("chart.current"),
            values: d.current.clone(),
            filled: true,
        },
        Series {
            name: st.tn("chart.previous"),
            values: d.previous.clone(),
            filled: false,
        },
    ];
    let names: Vec<String> = series.iter().map(|s| s.name.clone()).collect();
    let names2 = names.clone();
    let y = Rc::new(move |v: i64| st.fmt().compact(v / 100));
    card()
        .min_w(px(0.0))
        .child(card_header_icon(
            Some(Icon::ChartLine),
            st.t("chart.sales"),
            Some(Box::new(st.t("chart.sales_hint"))),
            if compact {
                view()
            } else {
                view().child(legend(names))
            },
        ))
        .child(
            card_body()
                .gap(px(8.0))
                .child(if compact {
                    view().child(legend(names2))
                } else {
                    view()
                })
                .child(figure(
                    move || st.fmt().money_whole(money(st, total)),
                    ty::H2,
                    FontWeight::BOLD,
                    || Tone::Ink,
                ))
                .child(area_chart(
                    series,
                    labels(st, d, p),
                    if compact { 180.0 } else { 248.0 },
                    st.rtl(),
                    y,
                )),
        )
}

fn method_key(code: &str) -> String {
    format!("pay.{code}")
}

fn payments_card(st: AppState, d: &Dashboard) -> View {
    let total: i64 = d.methods.iter().map(|p| p.net_minor).sum();
    let tones = [
        Tone::Primary,
        Tone::Info,
        Tone::Warning,
        Tone::Success,
        Tone::Danger,
    ];
    let parts = d
        .methods
        .iter()
        .zip(tones)
        .map(|(p, t)| (p.net_minor, t))
        .collect::<Vec<_>>();
    let mut legend_list = view().gap(px(12.0)).w_full();
    for (m, tone) in d.methods.iter().zip(tones) {
        let v = m.net_minor;
        let share = v.saturating_mul(10_000) / total.max(1);
        let key = method_key(&m.key);
        legend_list = legend_list.child(legend_row(
            tone,
            move || st.tn(&key),
            move || {
                let f = st.fmt();
                format!("{} · {}", f.compact(v / 100), percent_plain(&f, share))
            },
        ));
    }
    let body = if parts.is_empty() {
        card_body().child(label(
            st.t("dashboard.nothing_yet"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
    } else {
        card_body()
            .items_center()
            .gap(px(20.0))
            .child(donut(
                parts,
                168.0,
                move || st.fmt().compact(total / 100),
                st.t("pay.total"),
            ))
            .child(legend_list)
    };
    card()
        .child(card_header_icon(
            Some(Icon::Wallet),
            st.t("chart.payments"),
            None,
            view(),
        ))
        .child(body)
}

fn percent_plain(f: &nb_i18n::Format, bps: i64) -> String {
    // Unsigned share: drop the sign the signed formatter adds.
    nb_i18n::bidi::isolate_ltr(
        nb_i18n::bidi::strip_controls(&f.percent_signed(bps)).trim_start_matches('+'),
    )
}

fn invoices_card(st: AppState, d: &Dashboard, compact: bool) -> View {
    let mut table = view();
    if d.recent.is_empty() {
        table = table.child(empty_state(
            Icon::Receipt,
            st.t("sales.empty"),
            st.t("dashboard.nothing_yet"),
            None,
        ));
    }
    for (i, s) in d.recent.iter().enumerate() {
        let credit = s.methods.iter().any(|m| m == "credit");
        let (status, key) = if s.returned_minor >= s.total_minor && s.total_minor > 0 {
            (Status::Neutral, "dashboard.returned")
        } else if credit {
            (Status::Warning, "pay.credit")
        } else {
            (Status::Success, "status.paid")
        };
        let number = s.number.clone();
        let when = date_time(st, s.created_at);
        let total = s.total_minor;
        let n = number.clone();
        let who = view()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .flex_1()
            .min_w(px(0.0))
            .px(px(12.0))
            .child(avatar(move || n.clone(), 30.0, status))
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(label_line(
                        move || number.clone(),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    ))
                    .child(label_line(
                        move || when.clone(),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )),
            );
        table = table.child(
            view()
                .test_id(&format!("home-sale-{i}"))
                .flex_row()
                .items_center()
                .py(px(sp(if compact { 12.0 } else { 10.0 })))
                .px(px(8.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(who)
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .px(px(8.0))
                        .child(badge(status, None, st.t(key)))
                        .child(figure(
                            move || st.fmt().money(money(st, total)),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            || Tone::Ink,
                        )),
                ),
        );
    }
    card()
        .min_w(px(0.0))
        .child(card_header_icon(
            Some(Icon::Receipt),
            st.t("recent.title"),
            None,
            view().child(button(
                Variant::Ghost,
                Size::Sm,
                None,
                st.t("dashboard.see_all"),
                move || st.route.set(Route::Sales),
            )),
        ))
        .child(table)
}

fn categories_card(st: AppState, d: &Dashboard) -> View {
    let rows: Vec<(String, i64, String)> = d
        .categories
        .iter()
        .map(|c| {
            let name = if c.name.is_empty() {
                st.tn("reports.uncategorized")
            } else {
                c.name.clone()
            };
            (name, c.net_minor, st.fmt().compact(c.net_minor / 100))
        })
        .collect();
    let body = if rows.is_empty() {
        card_body().child(label(
            st.t("dashboard.nothing_yet"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
    } else {
        card_body().child(bar_list(rows, Tone::Primary))
    };
    card()
        .child(card_header_icon(
            Some(Icon::Layers),
            st.t("chart.by_category"),
            None,
            view(),
        ))
        .child(body)
}

fn low_stock_card(st: AppState, d: &Dashboard) -> View {
    let mut list = view();
    if d.low_stock.is_empty() {
        list = list.child(view().px(px(20.0)).py(px(12.0)).child(label(
            st.t("dashboard.stock_fine"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        )));
    }
    for r in &d.low_stock {
        let (status, word) = if r.out() {
            (Status::Danger, "stock.out")
        } else {
            (Status::Warning, "stock.low")
        };
        let name = r.name.clone();
        let (left, min) = (r.on_hand_milli, r.min_stock_milli.unwrap_or(0));
        list = list.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(20.0))
                .py(px(sp(10.0)))
                .border_t_1()
                .border_color(Tone::Border.color())
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
                            st.t_with("dashboard.left", move |s| {
                                let q =
                                    |m: i64| s.fmt().quantity(nb_core::Quantity::from_milli(m), 3);
                                vec![("qty", format!("{} / {}", q(left), q(min)))]
                            }),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .child(badge(status, None, st.t(word))),
        );
    }
    card()
        .child(card_header_icon(
            Some(Icon::Alert),
            st.t("dashboard.low_stock_title"),
            None,
            view().child(button(
                Variant::Ghost,
                Size::Sm,
                None,
                st.t("dashboard.see_all"),
                move || st.route.set(Route::StockReports),
            )),
        ))
        .child(list)
}

//! Home: the store at a glance — KPIs with trends, sales over time, payment
//! methods, recent invoices with their status, sales by category and what
//! runs out. SAMPLE DATA (`sample.rs`) until the modules feed projections.

use std::rc::Rc;

use nb_core::{Currency, Money};
use nsh_core::element::View;
use nsh_core::prelude::*;

use super::sample::{
    self, CATEGORY_SALES, INVOICES, InvoiceStatus, LOW_STOCK, PAYMENTS, Period, pick,
};
use crate::nav::Route;
use crate::state::AppState;
use crate::ui::chart::{Series, area_chart, bar_list, donut, legend, legend_row, sparkline};
use crate::ui::icon::Icon;
use crate::ui::kit::{Size, Variant, avatar, badge, button, figure, label, label_line, segmented};
use crate::ui::popup::{Action, row_menu};
use crate::ui::surface::{
    card, card_body, card_header_icon, delta, kpi, page_header_full, page_pad,
};
use crate::ui::tokens::{Status, Tone, sp, ty};

const DZD: Currency = Currency::DZD;

fn dzd(minor: i64) -> Money {
    Money::from_minor(minor, DZD)
}

/// Change in basis points between two totals (display only).
fn change_bps(cur: i64, prev: i64) -> i64 {
    if prev == 0 {
        0
    } else {
        (cur - prev).saturating_mul(10_000) / prev
    }
}

pub fn home(st: AppState, compact: bool) -> View {
    let period = signal(Period::Month);
    let gap = if compact { 16.0 } else { 20.0 };

    let periods = segmented(
        [Period::Week, Period::Month, Period::Year]
            .into_iter()
            .map(|p| (p, Box::new(st.t(p.key())) as Box<dyn Fn() -> String>))
            .collect(),
        period,
    );
    let actions = if compact {
        view()
    } else {
        view().child(periods).child(button(
            Variant::Secondary,
            Size::Md,
            Some(Icon::Download),
            st.t("action.export"),
            || {},
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
        page = page.child(
            scroll().horizontal().child(segmented(
                [Period::Week, Period::Month, Period::Year]
                    .into_iter()
                    .map(|p| (p, Box::new(st.t(p.key())) as Box<dyn Fn() -> String>))
                    .collect(),
                period,
            )),
        );
    }
    page.child(dynamic(move || {
        kpis(st, period.get(), compact).into_element()
    }))
    .child(
        row(compact, gap)
            .child(column(compact, 2.0).child(dynamic(move || {
                sales_card(st, period.get(), compact).into_element()
            })))
            .child(column(compact, 1.0).child(payments_card(st))),
    )
    .child(
        row(compact, gap)
            .child(column(compact, 2.0).child(invoices_card(st, compact)))
            .child(
                column(compact, 1.0)
                    .gap(px(gap))
                    .child(categories_card(st))
                    .child(low_stock_card(st)),
            ),
    )
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

fn kpis(st: AppState, period: Period, compact: bool) -> View {
    let (cur, prev) = sample::sales(period);
    let rtl = st.rtl();
    let total: i64 = cur.iter().sum();
    let total_prev: i64 = prev.iter().sum();
    // Invoices: about one per 1 450 DA sold.
    let orders: Vec<i64> = cur.iter().map(|v| v / 145_000).collect();
    let orders_prev: i64 = prev.iter().map(|v| v / 145_000).sum();
    let n_orders: i64 = orders.iter().sum();
    let basket = if n_orders == 0 { 0 } else { total / n_orders };
    let basket_prev = if orders_prev == 0 {
        0
    } else {
        total_prev / orders_prev
    };
    let credit: i64 = 2_875_050;

    let pct = move |bps: i64| {
        let f = st.fmt();
        move || f.percent_signed(bps)
    };
    let d_total = change_bps(total, total_prev);
    let d_orders = change_bps(n_orders, orders_prev);
    let d_basket = change_bps(basket, basket_prev);
    let spark_w = if compact { 110.0 } else { 160.0 };

    let tiles = [
        kpi(
            st.t("kpi.revenue"),
            move || st.fmt().money_whole(dzd(total)),
            Some(delta(d_total, pct(d_total))),
            sparkline(cur.clone(), spark_w, 36.0, Tone::Primary, rtl),
            compact,
        ),
        kpi(
            st.t("kpi.orders"),
            move || st.fmt().integer(n_orders),
            Some(delta(d_orders, pct(d_orders))),
            sparkline(orders, spark_w, 36.0, Tone::Primary, rtl),
            compact,
        ),
        kpi(
            st.t("kpi.basket"),
            move || st.fmt().money_whole(dzd(basket)),
            Some(delta(d_basket, pct(d_basket))),
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
            move || st.fmt().money_whole(dzd(credit * 100)),
            None,
            view().flex_row().child(badge(
                Status::Warning,
                Some(Icon::Users),
                st.t_with("kpi.customers_owing", |s| {
                    vec![("count", s.fmt().integer(14))]
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

fn sales_card(st: AppState, period: Period, compact: bool) -> View {
    let (cur, prev) = sample::sales(period);
    let fmt = st.fmt();
    let x = sample::labels(period, &fmt);
    let total: i64 = cur.iter().sum();
    let series = vec![
        Series {
            name: st.tn("chart.current"),
            values: cur,
            filled: true,
        },
        Series {
            name: st.tn("chart.previous"),
            values: prev,
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
                    move || st.fmt().money_whole(dzd(total)),
                    ty::H2,
                    FontWeight::BOLD,
                    || Tone::Ink,
                ))
                .child(area_chart(
                    series,
                    x,
                    if compact { 180.0 } else { 248.0 },
                    st.rtl(),
                    y,
                )),
        )
}

fn payments_card(st: AppState) -> View {
    let total: i64 = PAYMENTS.iter().map(|p| p.1).sum();
    let tones = [Tone::Primary, Tone::Info, Tone::Warning];
    let parts = PAYMENTS.iter().zip(tones).map(|(p, t)| (p.1, t)).collect();
    let mut legend_list = view().gap(px(12.0)).w_full();
    for ((key, v), tone) in PAYMENTS.into_iter().zip(tones) {
        let share = v.saturating_mul(10_000) / total.max(1);
        legend_list = legend_list.child(legend_row(tone, st.t(key), move || {
            let f = st.fmt();
            format!("{} · {}", f.compact(v / 100), percent_plain(&f, share))
        }));
    }
    card()
        .child(card_header_icon(
            Some(Icon::Wallet),
            st.t("chart.payments"),
            None,
            view(),
        ))
        .child(
            card_body()
                .items_center()
                .gap(px(20.0))
                .child(donut(
                    parts,
                    168.0,
                    move || st.fmt().compact(total / 100),
                    st.t("pay.total"),
                ))
                .child(legend_list),
        )
}

fn percent_plain(f: &nb_i18n::Format, bps: i64) -> String {
    // Unsigned share: drop the sign the signed formatter adds.
    nb_i18n::bidi::isolate_ltr(
        nb_i18n::bidi::strip_controls(&f.percent_signed(bps)).trim_start_matches('+'),
    )
}

fn status_of(s: InvoiceStatus) -> (Status, Icon) {
    match s {
        InvoiceStatus::Paid => (Status::Success, Icon::Check),
        InvoiceStatus::Partial => (Status::Info, Icon::Clock),
        InvoiceStatus::Unpaid => (Status::Neutral, Icon::Clock),
        InvoiceStatus::Overdue => (Status::Danger, Icon::Alert),
    }
}

fn invoices_card(st: AppState, compact: bool) -> View {
    let head = |key: &'static str, w: Option<f32>, end: bool| {
        let v = view().px(px(12.0));
        let v = match w {
            Some(w) => v.w(px(w)).flex_shrink_0(),
            None => v.flex_1().min_w(px(0.0)),
        };
        let v = if end { v.items_end() } else { v.items_start() };
        v.child(label_line(
            st.t(key),
            ty::XS,
            FontWeight::SEMIBOLD,
            Tone::Muted,
        ))
    };
    let mut table = view();
    if !compact {
        table = table.child(
            view()
                .flex_row()
                .items_center()
                .h(px(sp(36.0)))
                .px(px(8.0))
                .bg(Tone::Subtle.color())
                .border_t_1()
                .border_b_1()
                .border_color(Tone::Border.color())
                .child(head("col.customer", None, false))
                .child(head("col.date", Some(120.0), false))
                .child(head("col.status", Some(130.0), false))
                .child(head("col.amount", Some(140.0), true))
                .child(view().w(px(40.0))),
        );
    }
    for inv in INVOICES.iter() {
        let (status, ic) = status_of(inv.status);
        let name = move || pick(st.lang(), inv.ar, inv.fr);
        let when = move || {
            let f = st.fmt();
            let (m, d) = sample::day_back(inv.back_days);
            format!("{} · {}", f.day_month(m, d), f.time(inv.hour, inv.minute))
        };
        let amount = move || st.fmt().money(dzd(inv.total));
        let who = view()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .flex_1()
            .min_w(px(0.0))
            .px(px(12.0))
            .child(avatar(name, 30.0, Status::Neutral))
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(label_line(name, ty::SM, FontWeight::SEMIBOLD, Tone::Ink))
                    .child(label_line(
                        move || inv.number.to_owned(),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )),
            );
        let menu = row_menu(
            move || {
                vec![
                    Action::new(Icon::Eye, st.tn("action.view"), || {}),
                    Action::new(Icon::Printer, st.tn("action.print"), || {}),
                    Action::new(Icon::Copy, st.tn("action.duplicate"), || {}),
                ]
            },
            st.t("action.row_actions"),
        );
        let r = if compact {
            view()
                .flex_row()
                .items_center()
                .py(px(sp(12.0)))
                .px(px(8.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(who)
                .child(
                    view()
                        .items_end()
                        .gap(px(4.0))
                        .px(px(8.0))
                        .child(figure(amount, ty::SM, FontWeight::SEMIBOLD, || Tone::Ink))
                        .child(badge(status, None, st.t(inv.status.key()))),
                )
        } else {
            view()
                .flex_row()
                .items_center()
                .h(px(sp(56.0)))
                .px(px(8.0))
                .border_b_1()
                .border_color(Tone::Border.color())
                .hover(|s| s.bg(Tone::Subtle.color()))
                .child(who)
                .child(view().w(px(120.0)).px(px(12.0)).child(figure(
                    when,
                    ty::SM,
                    FontWeight::NORMAL,
                    || Tone::Muted,
                )))
                .child(view().w(px(130.0)).px(px(12.0)).child(badge(
                    status,
                    Some(ic),
                    st.t(inv.status.key()),
                )))
                .child(view().w(px(140.0)).px(px(12.0)).items_end().child(figure(
                    amount,
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    || Tone::Ink,
                )))
                .child(view().w(px(40.0)).items_center().child(menu))
        };
        table = table.child(r);
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

fn categories_card(st: AppState) -> View {
    let rows = move || {
        let f = st.fmt();
        CATEGORY_SALES
            .iter()
            .map(|(ar, fr, v)| (pick(st.lang(), ar, fr), *v, f.compact(v / 100)))
            .collect::<Vec<_>>()
    };
    card()
        .child(card_header_icon(
            Some(Icon::Layers),
            st.t("chart.by_category"),
            None,
            view(),
        ))
        .child(card_body().child(dynamic(move || {
            bar_list(rows(), Tone::Primary).into_element()
        })))
}

fn low_stock_card(st: AppState) -> View {
    let mut list = view();
    for (ar, fr, left, min) in LOW_STOCK {
        let status = if left == 0 {
            Status::Danger
        } else {
            Status::Warning
        };
        let word = if left == 0 { "stock.out" } else { "stock.low" };
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
                            move || pick(st.lang(), ar, fr),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            st.t_with("dashboard.left", move |s| {
                                vec![(
                                    "qty",
                                    format!("{} / {}", s.fmt().integer(left), s.fmt().integer(min)),
                                )]
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
                move || st.route.set(Route::Products),
            )),
        ))
        .child(list)
}

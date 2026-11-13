//! Home: the store at a glance (M13) — sales, invoices, average basket and
//! what customers owe, with their trend against the period before; sales
//! over time, payment methods, the latest invoices, sales by category and
//! what runs out. Everything is read from the records
//! (`nb_store::usecases::reports::dashboard`).

use std::rc::Rc;

use nb_core::Money;
use nb_i18n::tr_args;
use nb_store::usecases::debts::{Ageing, ageing};
use nb_store::usecases::reports::{
    Alerts, Dashboard, PartyRow, Period, alerts, dashboard, payables,
};

use crate::screens::home_layout::{Card, Home, Item, WIDTHS};
use crate::ui::icon::icon;
use crate::ui::kit::icon_button;
use crate::ui::menu::{Act, menu_on};
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
    let arranging = signal(false);
    // Read only for the cards shown.
    let shown = move |c: Card| {
        Home::parse(&st.biz.home.get())
            .items
            .iter()
            .any(|i| i.card == c && !i.hidden)
    };
    let alerts_data = resource(
        move || (shown(Card::Alerts) && st.session.get().is_some()).then(|| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| alerts(s)).await }
        },
    );
    let ageing_data = resource(
        move || (shown(Card::Debts) && st.session.get().is_some()).then(|| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| ageing(s)).await }
        },
    );
    let payables_data = resource(
        move || (shown(Card::Payables) && st.session.get().is_some()).then(|| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| payables(s)).await }
        },
    );
    let extra = Extra {
        alerts: computed(move || alerts_data.with(|a: Option<&Alerts>| a.cloned())),
        ageing: computed(move || ageing_data.with(|a: Option<&Ageing>| a.cloned())),
        payables: computed(move || payables_data.with(|a: Option<&Vec<PartyRow>>| a.cloned())),
    };
    let periods = move || {
        segmented(
            [Period::Week, Period::Month, Period::Year]
                .into_iter()
                .map(|p| (p, Box::new(st.t(period_key(p))) as Box<dyn Fn() -> String>))
                .collect(),
            period,
        )
    };
    let arrange = move || {
        button(
            Variant::Secondary,
            Size::Md,
            Some(Icon::LayoutDashboard),
            move || {
                st.tn(if arranging.get() {
                    "home.arrange.done"
                } else {
                    "home.arrange"
                })
            },
            move || arranging.update(|v| *v = !*v),
        )
        .test_id("home-arrange")
    };
    let actions = if compact {
        view().child(arrange())
    } else {
        view().child(periods()).child(arrange()).child(when(
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
        let editing = arranging.get();
        let h = Home::parse(&st.biz.home.get());
        let mut v = view().gap(px(gap));
        if editing {
            v = v.child(edit_bar(st));
        }
        if compact {
            // One under the other on a phone.
            for (at, item) in h.rows(editing).into_iter().flatten() {
                v = v.child(frame(
                    st,
                    at,
                    &item,
                    editing,
                    card_of(st, item.card, &d, p, compact, &extra),
                ));
            }
            return v.into_element();
        }
        for row in h.rows(editing) {
            let mut r = view().flex_row().items_start().gap(px(gap));
            for (at, item) in row {
                r = r.child(
                    view()
                        .flex_grow(f32::from(item.width))
                        .basis(px(0.0))
                        .min_w(px(0.0))
                        .child(frame(
                            st,
                            at,
                            &item,
                            editing,
                            card_of(st, item.card, &d, p, compact, &extra),
                        )),
                );
            }
            v = v.child(r);
        }
        v.into_element()
    }))
}

/// What some cards read besides the dashboard (each only when shown).
#[derive(Clone, Copy)]
struct Extra {
    alerts: Memo<Option<Alerts>>,
    ageing: Memo<Option<Ageing>>,
    payables: Memo<Option<Vec<PartyRow>>>,
}

/// A card's content.
fn card_of(st: AppState, c: Card, d: &Dashboard, p: Period, compact: bool, x: &Extra) -> View {
    match c {
        Card::Revenue => kpi_tiles(st, d, compact).0,
        Card::Orders => kpi_tiles(st, d, compact).1,
        Card::Basket => kpi_tiles(st, d, compact).2,
        Card::Credit => kpi_tiles(st, d, compact).3,
        Card::Sales => sales_card(st, d, p, compact),
        Card::Payments => payments_card(st, d),
        Card::Invoices => invoices_card(st, d, compact),
        Card::Categories => categories_card(st, d),
        Card::LowStock => low_stock_card(st, d),
        Card::Alerts => alerts_card(st, x.alerts),
        Card::Quick => quick_card(st),
        Card::Debts => debts_card(st, x.ageing),
        Card::Payables => payables_card(st, x.payables),
    }
}

/// A card while the dashboard is arranged: dimmed when hidden, its menu
/// (earlier, later, width, hide, remove) over it.
fn frame(st: AppState, at: usize, item: &Item, editing: bool, content: View) -> View {
    if !editing {
        return content;
    }
    let change = move |f: Rc<dyn Fn(&mut Home)>| {
        move || {
            let mut h = Home::parse(&st.biz.home.get_untracked());
            f(&mut h);
            st.biz.home.set(h.to_json());
        }
    };
    let widths = WIDTHS
        .iter()
        .map(|w| {
            let w = *w;
            Act::new(
                st.tn(&format!("home.width.{w}")),
                change(Rc::new(move |h: &mut Home| h.set_width(at, w))),
            )
            .checked(item.width == w)
        })
        .collect();
    let acts = vec![
        Act::with(
            Icon::ArrowUp,
            st.tn("home.earlier"),
            change(Rc::new(move |h: &mut Home| h.shift(at, true))),
        ),
        Act::with(
            Icon::ArrowDown,
            st.tn("home.later"),
            change(Rc::new(move |h: &mut Home| h.shift(at, false))),
        ),
        Act::sub(Icon::Columns, st.tn("home.width"), widths),
        Act::with(
            if item.hidden { Icon::Eye } else { Icon::EyeOff },
            st.tn(if item.hidden {
                "pos.panel.show"
            } else {
                "pos.panel.hide"
            }),
            change(Rc::new(move |h: &mut Home| h.toggle(at))),
        ),
        Act::with(
            Icon::Trash,
            st.tn("pos.panel.remove"),
            change(Rc::new(move |h: &mut Home| h.remove(at))),
        )
        .danger(),
    ];
    let name = st.tn(&format!("home.card.{}", item.card.code()));
    view()
        .test_id(&format!("home-card-{}", item.card.code()))
        .rounded(px(14.0))
        .border_2()
        .border_color(Tone::Primary.color())
        .overflow_hidden()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(10.0))
                .py(px(4.0))
                .bg(Tone::PrimarySoft.color())
                .child(icon(Icon::Grip, 14.0, || Tone::Primary))
                .child(view().flex_1().child(label_line(
                    move || name.clone(),
                    ty::SM,
                    FontWeight::BOLD,
                    Tone::Primary,
                )))
                .child(menu_on(
                    icon_button(
                        || Icon::Settings,
                        st.t("pos.panel.options"),
                        Variant::Ghost,
                        || {},
                    )
                    .test_id(&format!("home-opts-{}", item.card.code())),
                    acts,
                )),
        )
        .child(
            view()
                .p(px(6.0))
                .opacity(if item.hidden { 0.4 } else { 1.0 })
                .child(content),
        )
}

/// Arranging the dashboard: add a card, back to the default, done.
fn edit_bar(st: AppState) -> View {
    let h = Home::parse(&st.biz.home.get_untracked());
    let adds: Vec<Act> = h
        .missing()
        .into_iter()
        .map(|c| {
            Act::new(st.tn(&format!("home.card.{}", c.code())), move || {
                let mut h = Home::parse(&st.biz.home.get_untracked());
                h.add(c);
                st.biz.home.set(h.to_json());
            })
        })
        .collect();
    view()
        .test_id("home-edit-bar")
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
        .p(px(12.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(Tone::Primary.color())
        .bg(Tone::PrimarySoft.color())
        .child(icon(Icon::LayoutDashboard, 18.0, || Tone::Primary))
        .child(view().flex_1().child(label(
            st.t("home.arrange.hint"),
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Body,
        )))
        .child(menu_on(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Plus),
                st.t("home.add"),
                || {},
            )
            .test_id("home-add"),
            adds,
        ))
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Undo),
                st.t("home.reset"),
                move || st.biz.home.set(Home::default().to_json()),
            )
            .test_id("home-reset"),
        )
}

/// What wants attention (the bell's list, as a card).
fn alerts_card(st: AppState, a: Memo<Option<Alerts>>) -> View {
    let body = dynamic(move || {
        let Some(a) = a.get() else {
            return view().into_element();
        };
        let n = |v: u32| st.fmt().integer(i64::from(v));
        let mut items: Vec<(Icon, Status, String, Route)> = Vec::new();
        if a.out_of_stock > 0 {
            items.push((
                Icon::XCircle,
                Status::Danger,
                tr_args(st.lang(), "alerts.out", &[("count", &n(a.out_of_stock))]),
                Route::Products,
            ));
        }
        if a.low_stock > 0 {
            items.push((
                Icon::Alert,
                Status::Warning,
                tr_args(st.lang(), "alerts.low", &[("count", &n(a.low_stock))]),
                Route::Inventory,
            ));
        }
        if a.customers_overdue > 0 {
            items.push((
                Icon::Coins,
                Status::Warning,
                tr_args(
                    st.lang(),
                    "alerts.overdue",
                    &[("count", &n(a.customers_overdue))],
                ),
                Route::Debts,
            ));
        }
        if a.suppliers_owed > 0 {
            items.push((
                Icon::Truck,
                Status::Info,
                tr_args(
                    st.lang(),
                    "alerts.payables",
                    &[("count", &n(a.suppliers_owed))],
                ),
                Route::PartyReports,
            ));
        }
        let mut v = view().gap(px(4.0)).px(px(12.0)).pb(px(12.0));
        if items.is_empty() {
            v = v.child(label(
                st.t("alerts.none"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        for (ic, status, text, r) in items {
            v = v.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .p(px(8.0))
                    .rounded(px(10.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .on_press(move || st.route.set(r))
                    .child(badge(status, Some(ic), String::new))
                    .child(view().flex_1().child(label_line(
                        move || text.clone(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Ink,
                    ))),
            );
        }
        v.into_element()
    });
    card()
        .child(card_header_icon(
            Some(Icon::Bell),
            st.t("action.notifications"),
            None,
            view(),
        ))
        .child(body)
}

/// Ways to start.
fn quick_card(st: AppState) -> View {
    let go = |ic: Icon, key: &'static str, id: &'static str, run: Rc<dyn Fn()>| {
        view()
            .test_id(id)
            .flex_grow(1.0)
            .basis(px(120.0))
            .items_center()
            .gap(px(6.0))
            .p(px(12.0))
            .rounded(px(12.0))
            .bg(Tone::Subtle.color())
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Hover.color()))
            .on_press(move || run())
            .child(icon(ic, 22.0, || Tone::Primary))
            .child(label_line(
                st.t(key),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            ))
    };
    let mut grid = view()
        .flex_row()
        .flex_wrap()
        .gap(px(10.0))
        .px(px(12.0))
        .pb(px(12.0));
    if st.shows(Route::Sell) {
        grid = grid.child(go(
            Icon::Cart,
            "home.quick.sell",
            "quick-sell",
            Rc::new(move || st.route.set(Route::Sell)),
        ));
    }
    if st.can("products.edit") {
        grid = grid.child(go(
            Icon::PackagePlus,
            "home.quick.product",
            "quick-product",
            Rc::new(move || {
                st.intent.set(Some(crate::state::Intent::AddProduct));
                st.route.set(Route::Products);
            }),
        ));
    }
    if st.shows(Route::Purchases) {
        grid = grid.child(go(
            Icon::Truck,
            "home.quick.purchase",
            "quick-purchase",
            Rc::new(move || st.route.set(Route::Purchases)),
        ));
    }
    if st.shows(Route::Customers) {
        grid = grid.child(go(
            Icon::Users,
            "home.quick.customers",
            "quick-customers",
            Rc::new(move || st.route.set(Route::Customers)),
        ));
    }
    if st.shows(Route::Expenses) {
        grid = grid.child(go(
            Icon::Banknote,
            "home.quick.expense",
            "quick-expense",
            Rc::new(move || st.route.set(Route::Expenses)),
        ));
    }
    if st.shows(Route::Reports) {
        grid = grid.child(go(
            Icon::ChartLine,
            "home.quick.reports",
            "quick-reports",
            Rc::new(move || st.route.set(Route::Reports)),
        ));
    }
    card()
        .child(card_header_icon(
            Some(Icon::Zap),
            st.t("home.card.quick"),
            None,
            view(),
        ))
        .child(grid)
}

/// A list of names and amounts (the customers who owe most, the
/// suppliers owed most).
fn ranking(st: AppState, rows: Vec<(String, String, i64)>, empty: &'static str, go: Route) -> View {
    let mut v = view().px(px(12.0)).pb(px(12.0));
    if rows.is_empty() {
        return v.child(label(st.t(empty), ty::SM, FontWeight::NORMAL, Tone::Muted));
    }
    for (name, note, m) in rows.into_iter().take(6) {
        let n = name.clone();
        v = v.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .py(px(8.0))
                .border_b_1()
                .border_color(Tone::Border.color())
                .cursor_pointer()
                .on_press(move || st.route.set(go))
                .child(avatar(move || n.clone(), 28.0, Status::Accent))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label_line(
                            move || name.clone(),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            move || note.clone(),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .child(figure(
                    move || st.fmt().money(money(st, m)),
                    ty::SM,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        );
    }
    v
}

fn debts_card(st: AppState, a: Memo<Option<Ageing>>) -> View {
    let body = dynamic(move || {
        let Some(a) = a.get() else {
            return view().into_element();
        };
        let mut rows: Vec<_> = a.rows.into_iter().filter(|r| r.balance_minor > 0).collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.balance_minor));
        let list = rows
            .into_iter()
            .map(|r| {
                let late = if r.overdue_minor > 0 {
                    tr_args(
                        st.lang(),
                        "home.overdue",
                        &[("amount", &st.fmt().money(money(st, r.overdue_minor)))],
                    )
                } else {
                    r.phone.clone()
                };
                (r.name, late, r.balance_minor)
            })
            .collect();
        ranking(st, list, "home.debts.none", Route::Debts).into_element()
    });
    card()
        .child(card_header_icon(
            Some(Icon::Coins),
            st.t("home.card.debts"),
            None,
            view(),
        ))
        .child(body)
}

fn payables_card(st: AppState, a: Memo<Option<Vec<PartyRow>>>) -> View {
    let body = dynamic(move || {
        let Some(mut rows) = a.get() else {
            return view().into_element();
        };
        rows.retain(|r| r.total_minor > 0);
        rows.sort_by_key(|r| std::cmp::Reverse(r.total_minor));
        let list = rows
            .into_iter()
            .map(|r| {
                let note = tr_args(
                    st.lang(),
                    "home.invoices_open",
                    &[("count", &st.fmt().integer(r.count))],
                );
                (r.name, note, r.total_minor)
            })
            .collect();
        ranking(st, list, "home.payables.none", Route::PartyReports).into_element()
    });
    card()
        .child(card_header_icon(
            Some(Icon::Truck),
            st.t("home.card.payables"),
            None,
            view(),
        ))
        .child(body)
}

fn kpi_tiles(st: AppState, d: &Dashboard, compact: bool) -> (View, View, View, View) {
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
    let [a, b, c, e] = [
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
    (a, b, c, e)
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

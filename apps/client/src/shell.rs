//! App shell (DESIGN.md §5): sidebar + top bar on wide screens, header +
//! bottom navigation on phones. The sidebar sits at the reading start, so it
//! moves to the other side when the language changes.

use nb_i18n::Lang;
use nsh_core::element::SharedString;
use nsh_core::prelude::*;

use crate::screens::home::home;
use crate::screens::products::products;
use crate::state::{AppState, toggle_appearance};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{icon_button, label, label_line, primary_button, size};
use crate::ui::overlay::{modal, toast_layer};
use crate::ui::palette::Tone;

/// Width below which the phone layout is used.
const COMPACT_MAX: f32 = 760.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Home,
    Sell,
    Sales,
    Products,
    Inventory,
    Customers,
    Purchases,
    Cash,
    Reports,
    Settings,
}

impl Route {
    fn key(self) -> &'static str {
        match self {
            Route::Home => "nav.home",
            Route::Sell => "nav.sell",
            Route::Sales => "nav.sales",
            Route::Products => "nav.products",
            Route::Inventory => "nav.inventory",
            Route::Customers => "nav.customers",
            Route::Purchases => "nav.purchases",
            Route::Cash => "nav.cash",
            Route::Reports => "nav.reports",
            Route::Settings => "nav.settings",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Route::Home => Icon::Home,
            Route::Sell => Icon::Cart,
            Route::Sales => Icon::Receipt,
            Route::Products => Icon::Package,
            Route::Inventory => Icon::Boxes,
            Route::Customers => Icon::Users,
            Route::Purchases => Icon::Truck,
            Route::Cash => Icon::Wallet,
            Route::Reports => Icon::Chart,
            Route::Settings => Icon::Settings,
        }
    }
}

const NAV_GROUPS: [(&str, &[Route]); 3] = [
    (
        "nav.group.daily",
        &[Route::Home, Route::Sales, Route::Customers],
    ),
    (
        "nav.group.stock",
        &[Route::Products, Route::Inventory, Route::Purchases],
    ),
    ("nav.group.money", &[Route::Cash, Route::Reports]),
];

pub fn shell(st: AppState) -> impl IntoElement {
    // NB_ROUTE (development): start on a given screen, e.g. for screenshots.
    let start = match std::env::var("NB_ROUTE").as_deref() {
        Ok("products") => Route::Products,
        Ok("customers") => Route::Customers,
        _ => Route::Home,
    };
    let route = signal(start);
    let compact = computed(|| window().width.0 < COMPACT_MAX);
    view()
        .flex_1()
        .direction(move || st.direction())
        .bg(Tone::Paper.color())
        .text_color(Tone::Ink.color())
        .child(
            when(compact, move || phone(st, route).into_element())
                .otherwise(move || desktop(st, route).into_element()),
        )
        .child(dynamic(move || {
            toast_layer(st, compact.get()).into_element()
        }))
}

fn content(st: AppState, route: Signal<Route>, compact: bool) -> impl IntoElement {
    // Screens with their own scrolling (tables) fill the area; the rest scroll as a page.
    dynamic(move || match route.get() {
        Route::Home | Route::Sell => scroll().flex_1().child(home(st, compact)).into_element(),
        Route::Products => products(st, compact).into_element(),
        other => scroll()
            .flex_1()
            .child(coming_soon(st, other))
            .into_element(),
    })
}

fn coming_soon(st: AppState, r: Route) -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(12.0))
        .p_6()
        .child(icon(r.icon(), 40.0, || Tone::Muted))
        .child(label(
            st.t(r.key()),
            size::TITLE,
            FontWeight::SEMIBOLD,
            Tone::Ink,
        ))
        .child(label(
            st.t("placeholder.coming"),
            size::BODY,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
}

// ---------------------------------------------------------------- desktop

fn desktop(st: AppState, route: Signal<Route>) -> impl IntoElement {
    view().flex_1().flex_row().child(sidebar(st, route)).child(
        view()
            .flex_1()
            .child(top_bar(st))
            .child(content(st, route, false)),
    )
}

fn sidebar(st: AppState, route: Signal<Route>) -> impl IntoElement {
    let mut nav = view().gap(px(20.0)).px_3();
    for (group, routes) in NAV_GROUPS {
        let mut g = view().gap(px(2.0)).child(view().px_3().pb_1().child(label(
            st.t(group),
            size::CAPTION,
            FontWeight::MEDIUM,
            Tone::Muted,
        )));
        for &r in routes {
            g = g.child(nav_item(st, route, r));
        }
        nav = nav.child(g);
    }
    view()
        .w(px(248.0))
        .bg(Tone::Surface.color())
        .border_e(px(1.0))
        .border_e_color(Tone::Line.color())
        .py_4()
        .gap(px(20.0))
        // Brand and store.
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px_4()
                .child(
                    view()
                        .w(px(36.0))
                        .h(px(36.0))
                        .rounded(px(8.0))
                        .items_center()
                        .justify_center()
                        .bg(Tone::Primary.color())
                        .child(icon(Icon::Store, 20.0, || Tone::OnPrimary)),
                )
                .child(
                    view()
                        .flex_1()
                        .child(label(
                            st.t("app.name"),
                            size::LEAD,
                            FontWeight::BOLD,
                            Tone::Ink,
                        ))
                        .child(label(
                            store_name(st),
                            size::CAPTION,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                ),
        )
        // The one primary action of the app.
        .child(view().px_4().child(primary_button(
            st.t("dashboard.start_sale"),
            Some(Icon::Cart),
            move || route.set(Route::Sell),
        )))
        .child(nav)
        .child(view().flex_1())
        .child(view().px_3().child(nav_item(st, route, Route::Settings)))
}

fn nav_item(st: AppState, route: Signal<Route>, r: Route) -> impl IntoElement {
    let active = computed(move || route.get() == r);
    view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .h(px(40.0))
        .px_3()
        .rounded(px(8.0))
        .style_when(active, |s| s.bg(Tone::Accent.color()))
        .active(|s| s.bg(Tone::Accent.color()))
        .on_press(move || route.set(r))
        .child(dynamic(move || {
            icon(r.icon(), 20.0, move || {
                if active.get() {
                    Tone::Primary
                } else {
                    Tone::Muted
                }
            })
            .into_element()
        }))
        .child(
            text(st.t(r.key()))
                .text_size(px(size::BODY))
                .font_weight(FontWeight::NORMAL)
                .style_when(active, |s| s.font_weight(FontWeight::SEMIBOLD))
                .text_color(move || {
                    if active.get() {
                        Tone::Primary.color()
                    } else {
                        Tone::Ink.color()
                    }
                }),
        )
}

fn top_bar(st: AppState) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .h(px(64.0))
        .px_6()
        .border_b_1()
        .border_color(Tone::Line.color())
        .child(search_box(st))
        .child(view().flex_1())
        .child(sync_status(st))
        .child(language_switch(st))
        .child(icon_button(
            || if dark() { Icon::Sun } else { Icon::Moon },
            st.t("settings.theme"),
            toggle_appearance,
        ))
        .child(user_chip(st))
}

fn search_box(st: AppState) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(40.0))
        .w(px(420.0))
        .px_3()
        .rounded(px(8.0))
        .bg(Tone::Surface.color())
        .border_1()
        .border_color(Tone::Line.color())
        .child(icon(Icon::Search, 18.0, || Tone::Muted))
        .child(view().flex_1().child(label(
            st.t("search.placeholder"),
            size::BODY,
            FontWeight::NORMAL,
            Tone::Muted,
        )))
        .child(
            view()
                .px_2()
                .py(px(2.0))
                .rounded(px(4.0))
                .bg(Tone::Paper.color())
                .border_1()
                .border_color(Tone::Line.color())
                .child(label(
                    st.t("search.shortcut"),
                    size::CAPTION,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )),
        )
}

/// Local-first made visible: selling works offline, and pending work is counted.
fn sync_status(st: AppState) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .h(px(32.0))
        .px_3()
        .rounded_full()
        .border_1()
        .border_color(Tone::Line.color())
        .child(icon(Icon::Offline, 16.0, || Tone::Warn))
        .child(label(
            st.t("sync.offline"),
            size::CAPTION,
            FontWeight::MEDIUM,
            Tone::Warn,
        ))
}

fn language_switch(st: AppState) -> impl IntoElement {
    let chip = move |lang: Lang, caption: &'static str| {
        let active = computed(move || st.lang.get() == lang);
        view()
            .h(px(28.0))
            .px_2()
            .justify_center()
            .rounded(px(6.0))
            .style_when(active, |s| s.bg(Tone::Surface.color()))
            .on_press(move || st.lang.set(lang))
            .label(lang.native_name())
            .child(
                text(caption)
                    .text_size(px(size::CAPTION))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || {
                        if active.get() {
                            Tone::Primary.color()
                        } else {
                            Tone::Muted.color()
                        }
                    }),
            )
    };
    view()
        .flex_row()
        .direction(LayoutDirection::Ltr)
        .gap(px(2.0))
        .p(px(3.0))
        .rounded(px(8.0))
        .bg(Tone::Line.color())
        .child(chip(Lang::Ar, "ع"))
        .child(chip(Lang::Fr, "FR"))
        .child(chip(Lang::En, "EN"))
}

fn user_name(st: AppState) -> impl Fn() -> String + 'static {
    move || {
        if st.lang() == Lang::Ar {
            "أحمد بن علي".into()
        } else {
            "Ahmed Benali".into()
        }
    }
}

fn store_name(st: AppState) -> impl Fn() -> String + 'static {
    move || {
        if st.lang() == Lang::Ar {
            "متجر النور — الوادي".into()
        } else {
            "Magasin En-Nour — El Oued".into()
        }
    }
}

fn user_chip(st: AppState) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .ps_2()
        .child(
            view()
                .w(px(34.0))
                .h(px(34.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(Tone::Accent.color())
                .child(icon(Icon::User, 18.0, || Tone::Primary)),
        )
        .child(
            view()
                .child(label(
                    user_name(st),
                    size::BODY,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(label(
                    st.t("user.role.owner"),
                    size::CAPTION,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        )
}

// ---------------------------------------------------------------- phone

fn phone(st: AppState, route: Signal<Route>) -> impl IntoElement {
    view()
        .flex_1()
        .child(
            view()
                .pt_safe_area()
                .bg(Tone::Surface.color())
                .border_b_1()
                .border_color(Tone::Line.color())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .h(px(56.0))
                        .px_4()
                        .child(icon(Icon::Store, 22.0, || Tone::Primary))
                        .child(view().flex_1().min_w(px(0.0)).child(label_line(
                            store_name(st),
                            size::LEAD,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        )))
                        .child(icon(Icon::Offline, 20.0, || Tone::Warn))
                        .child(icon_button(
                            || Icon::Languages,
                            st.t("settings.language"),
                            move || {
                                let next = match st.lang.get_untracked() {
                                    Lang::Ar => Lang::Fr,
                                    Lang::Fr => Lang::En,
                                    Lang::En => Lang::Ar,
                                };
                                st.lang.set(next);
                            },
                        ))
                        .child(icon_button(
                            || if dark() { Icon::Sun } else { Icon::Moon },
                            st.t("settings.theme"),
                            toggle_appearance,
                        )),
                ),
        )
        .child(content(st, route, true))
        .child(bottom_nav(st, route))
}

/// Every section not in the bottom bar, as a sheet of large targets.
fn more_sheet(st: AppState, route: Signal<Route>, open: Signal<bool>) -> impl IntoElement {
    let mut grid = view().flex_row().flex_wrap().px_4().pb_4();
    for r in [
        Route::Products,
        Route::Inventory,
        Route::Purchases,
        Route::Cash,
        Route::Reports,
        Route::Settings,
    ] {
        grid = grid.child(
            view()
                .w(relative(1.0 / 3.0))
                .items_center()
                .gap(px(8.0))
                .py_4()
                .rounded(px(12.0))
                .active(|s| s.bg(Tone::Accent.color()))
                .on_press(move || {
                    route.set(r);
                    open.set(false);
                })
                .child(
                    view()
                        .w(px(48.0))
                        .h(px(48.0))
                        .rounded(px(12.0))
                        .items_center()
                        .justify_center()
                        .bg(Tone::Accent.color())
                        .child(icon(r.icon(), 24.0, || Tone::Primary)),
                )
                .child(label(
                    st.t(r.key()),
                    size::BODY,
                    FontWeight::MEDIUM,
                    Tone::Ink,
                )),
        );
    }
    view()
        .gap(px(8.0))
        .child(view().px_6().pt_3().child(label(
            st.t("nav.more"),
            size::TITLE,
            FontWeight::BOLD,
            Tone::Ink,
        )))
        .child(grid)
}

fn bottom_nav(st: AppState, route: Signal<Route>) -> impl IntoElement {
    let more_open = signal(false);
    let in_more = computed(move || {
        !matches!(
            route.get(),
            Route::Home | Route::Sell | Route::Sales | Route::Customers
        )
    });
    let more = view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .h(px(60.0))
        .on_press(move || more_open.set(true))
        .child(dynamic(move || {
            icon(Icon::More, 22.0, move || {
                if in_more.get() {
                    Tone::Primary
                } else {
                    Tone::Muted
                }
            })
            .into_element()
        }))
        .child(
            text(st.t("nav.more"))
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if in_more.get() {
                        Tone::Primary.color()
                    } else {
                        Tone::Muted.color()
                    }
                }),
        );
    let tab = move |r: Route| {
        let active = computed(move || route.get() == r);
        view()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(4.0))
            .h(px(60.0))
            .on_press(move || route.set(r))
            .child(dynamic(move || {
                icon(r.icon(), 22.0, move || {
                    if active.get() {
                        Tone::Primary
                    } else {
                        Tone::Muted
                    }
                })
                .into_element()
            }))
            .child(
                text(st.t(r.key()))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if active.get() {
                            Tone::Primary.color()
                        } else {
                            Tone::Muted.color()
                        }
                    }),
            )
    };
    // Selling is the centre of the phone app: a larger, filled button.
    let sell = view()
        .flex_1()
        .items_center()
        .justify_center()
        .h(px(60.0))
        .child(
            view()
                .w(px(52.0))
                .h(px(52.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(Tone::Primary.color())
                .active(|s| s.opacity(0.85))
                .label({
                    let f = st.t("nav.sell");
                    move || SharedString::from(f())
                })
                .on_press(move || route.set(Route::Sell))
                .child(icon(Icon::Cart, 24.0, || Tone::OnPrimary)),
        );
    view()
        .pb_safe_area()
        .bg(Tone::Surface.color())
        .border_t_1()
        .border_color(Tone::Line.color())
        .child(
            view()
                .flex_row()
                .items_center()
                .child(tab(Route::Home))
                .child(tab(Route::Sales))
                .child(sell)
                .child(tab(Route::Customers))
                .child(more),
        )
        .child(modal(more_open, true, move || {
            more_sheet(st, route, more_open).into_element()
        }))
}

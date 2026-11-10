//! App shell (DESIGN.md v2 §Layout): on wide screens a quiet sidebar and a
//! top bar with search and the main action; on phones a header and a bottom
//! navigation. The sidebar sits at the reading start, so it moves to the
//! other side when the language changes.

use nb_i18n::Lang;
use nsh_core::element::{SharedString, View};
use nsh_core::prelude::*;

use crate::command_palette::command_palette;
use crate::screens::{design::design, home::home, products::products, settings::settings};
use crate::state::AppState;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, avatar, badge, button, icon_button, kbd, label, label_line, spacer,
};
use crate::ui::overlay::{modal, toast_layer};
use crate::ui::surface::{empty_state, page_pad};
use crate::ui::tokens::{Status, Tone, radius, sp, ty};

/// Width below which the phone layout is used.
const COMPACT_MAX: f32 = 760.0;
const SIDEBAR_W: f32 = 232.0;

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
    Design,
}

impl Route {
    pub fn key(self) -> &'static str {
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
            Route::Design => "nav.design",
        }
    }

    pub fn icon(self) -> Icon {
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
            Route::Design => Icon::Palette,
        }
    }

    /// NB_ROUTE (development): start on a given screen, e.g. for screenshots.
    pub fn from_env() -> Option<Self> {
        Some(match std::env::var("NB_ROUTE").ok()?.as_str() {
            "products" => Route::Products,
            "customers" => Route::Customers,
            "settings" => Route::Settings,
            "design" => Route::Design,
            _ => return None,
        })
    }
}

const NAV_GROUPS: [(&str, &[Route]); 3] = [
    (
        "nav.group.daily",
        &[Route::Home, Route::Sell, Route::Sales, Route::Customers],
    ),
    (
        "nav.group.stock",
        &[Route::Products, Route::Inventory, Route::Purchases],
    ),
    ("nav.group.money", &[Route::Cash, Route::Reports]),
];

pub fn shell(st: AppState) -> impl IntoElement {
    let compact = computed(|| window().width.0 < COMPACT_MAX);
    view()
        .flex_1()
        .direction(move || st.direction())
        .bg(Tone::Canvas.color())
        .text_color(Tone::Body.color())
        .child(
            when(compact, move || phone(st).into_element())
                .otherwise(move || desktop(st).into_element()),
        )
        .child(dynamic(move || {
            toast_layer(st, compact.get()).into_element()
        }))
        .child(dynamic(move || {
            command_palette(st, compact.get()).into_element()
        }))
}

fn content(st: AppState, compact: bool) -> impl IntoElement {
    // Screens with their own scrolling (tables) fill the area; the rest scroll as a page.
    dynamic(move || match st.route.get() {
        Route::Home | Route::Sell => scroll().flex_1().child(home(st, compact)).into_element(),
        Route::Products => products(st, compact).into_element(),
        Route::Settings => scroll()
            .flex_1()
            .child(settings(st, compact))
            .into_element(),
        Route::Design => scroll().flex_1().child(design(st, compact)).into_element(),
        other => scroll()
            .flex_1()
            .child(coming_soon(st, other, compact))
            .into_element(),
    })
}

fn coming_soon(st: AppState, r: Route, compact: bool) -> View {
    view()
        .flex_1()
        .p(px(page_pad(compact)))
        .pt(px(80.0))
        .child(empty_state(
            r.icon(),
            st.t(r.key()),
            st.t("placeholder.coming"),
            None,
        ))
}

fn store_name(st: AppState) -> impl Fn() -> String + 'static {
    move || {
        if st.lang() == Lang::Ar {
            "متجر النور".into()
        } else {
            "Magasin En-Nour".into()
        }
    }
}

fn store_place(st: AppState) -> impl Fn() -> String + 'static {
    move || {
        if st.lang() == Lang::Ar {
            "الوادي".into()
        } else {
            "El Oued".into()
        }
    }
}

pub fn user_name(st: AppState) -> impl Fn() -> String + 'static {
    move || {
        if st.lang() == Lang::Ar {
            "أحمد بن علي".into()
        } else {
            "Ahmed Benali".into()
        }
    }
}

// ---------------------------------------------------------------- desktop

fn desktop(st: AppState) -> View {
    view().flex_1().flex_row().child(sidebar(st)).child(
        view()
            .flex_1()
            .min_w(px(0.0))
            .child(top_bar(st))
            .child(content(st, false)),
    )
}

fn store_switcher(st: AppState) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .mx(px(10.0))
        .px(px(8.0))
        .h(px(48.0))
        .rounded(px(radius::md()))
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
        .child(
            view()
                .w(px(30.0))
                .h(px(30.0))
                .rounded(px(radius::md()))
                .items_center()
                .justify_center()
                .bg(Tone::Primary.color())
                .child(icon(Icon::Store, 16.0, || Tone::OnPrimary)),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(label_line(
                    store_name(st),
                    ty::SM,
                    FontWeight::BOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    store_place(st),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        )
        .child(icon(Icon::ChevronsUpDown, 14.0, || Tone::Faint))
}

fn sidebar(st: AppState) -> View {
    let mut nav = view().gap(px(18.0)).px(px(10.0));
    for (group, routes) in NAV_GROUPS {
        let mut g = view()
            .gap(px(1.0))
            .child(view().px(px(10.0)).pb(px(4.0)).child(label(
                st.t(group),
                ty::XS,
                FontWeight::SEMIBOLD,
                Tone::Faint,
            )));
        for &r in routes {
            g = g.child(nav_item(st, r));
        }
        nav = nav.child(g);
    }
    view()
        .w(px(SIDEBAR_W))
        .bg(Tone::Sidebar.color())
        .border_e(px(1.0))
        .border_e_color(Tone::Border.color())
        .pt(px(10.0))
        .pb(px(12.0))
        .gap(px(14.0))
        .child(store_switcher(st))
        .child(scroll().flex_1().child(nav))
        .child(
            view()
                .px(px(10.0))
                .gap(px(1.0))
                .child(nav_item(st, Route::Design))
                .child(nav_item(st, Route::Settings)),
        )
}

fn nav_item(st: AppState, r: Route) -> View {
    let active = computed(move || st.route.get() == r);
    view()
        .test_id(match r {
            Route::Settings => "nav-settings",
            Route::Products => "nav-products",
            _ => "nav-item",
        })
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(sp(32.0)))
        .px(px(10.0))
        .rounded(px(radius::md()))
        .cursor_pointer()
        .style_when(active, |s| s.bg(Tone::Hover.color()))
        .hover(|s| s.bg(Tone::Hover.color()))
        .on_press(move || st.route.set(r))
        .child(dynamic(move || {
            icon(r.icon(), 16.0, move || {
                if active.get() {
                    Tone::Primary
                } else {
                    Tone::Muted
                }
            })
            .into_element()
        }))
        .child(
            label_line(st.t(r.key()), ty::SM, FontWeight::MEDIUM, Tone::Body)
                .style_when(active, |s| s.font_weight(FontWeight::SEMIBOLD))
                .text_color(move || {
                    if active.get() {
                        Tone::Primary.color()
                    } else {
                        Tone::Body.color()
                    }
                }),
        )
}

fn top_bar(st: AppState) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .h(px(56.0))
        .px(px(page_pad(false)))
        .border_b_1()
        .border_color(Tone::Border.color())
        .child(search_box(st))
        .child(spacer())
        .child(sync_status(st))
        .child(icon_button(
            || Icon::Bell,
            st.t("action.notifications"),
            Variant::Ghost,
            || {},
        ))
        .child(icon_button(
            || Icon::Help,
            st.t("action.help"),
            Variant::Ghost,
            || {},
        ))
        .child(
            icon_button(
                || if dark() { Icon::Sun } else { Icon::Moon },
                st.t("settings.theme"),
                Variant::Ghost,
                move || st.toggle_appearance(),
            )
            .test_id("toggle-theme"),
        )
        .child(view().w(px(6.0)))
        .child(button(
            Variant::Primary,
            Size::Md,
            Some(Icon::Plus),
            st.t("dashboard.start_sale"),
            move || st.route.set(Route::Sell),
        ))
        .child(view().w(px(6.0)))
        .child(avatar(user_name(st), 32.0, Status::Accent))
}

fn search_box(st: AppState) -> View {
    view()
        .test_id("open-palette")
        .on_press(move || st.palette.set(true))
        .cursor_pointer()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(34.0))
        .w(px(380.0))
        .flex_shrink(1.0)
        .px(px(10.0))
        .rounded(px(radius::md()))
        .bg(Tone::Subtle.color())
        .border_1()
        .border_color(Tone::Border.color())
        .hover(|s| s.border_color(Tone::BorderStrong.color()))
        .child(icon(Icon::Search, 16.0, || Tone::Muted))
        .child(view().flex_1().min_w(px(0.0)).child(label_line(
            st.t("search.placeholder"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Faint,
        )))
        .child(kbd(st.t("search.shortcut")))
}

/// Local-first made visible: selling works offline, and pending work is counted.
fn sync_status(st: AppState) -> View {
    view()
        .pe(px(6.0))
        .label({
            let f = st.t("sync.offline");
            move || SharedString::from(f())
        })
        .child(badge(
            Status::Warning,
            Some(Icon::Offline),
            st.t("sync.offline"),
        ))
}

// ---------------------------------------------------------------- phone

fn phone(st: AppState) -> View {
    let title = move || st.tn(st.route.get().key());
    view()
        .flex_1()
        .child(
            view()
                .pt_safe_area()
                .bg(Tone::Canvas.color())
                .border_b_1()
                .border_color(Tone::Border.color())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .h(px(56.0))
                        .px(px(12.0))
                        .child(
                            view()
                                .w(px(30.0))
                                .h(px(30.0))
                                .rounded(px(radius::md()))
                                .items_center()
                                .justify_center()
                                .bg(Tone::Primary.color())
                                .child(icon(Icon::Store, 16.0, || Tone::OnPrimary)),
                        )
                        .child(view().flex_1().min_w(px(0.0)).ps(px(8.0)).child(label_line(
                            title,
                            ty::LG,
                            FontWeight::BOLD,
                            Tone::Ink,
                        )))
                        .child(icon(Icon::Offline, 18.0, || Tone::Warning))
                        .child(icon_button(
                            || Icon::Search,
                            st.t("search.placeholder"),
                            Variant::Ghost,
                            move || st.palette.set(true),
                        ))
                        .child(icon_button(
                            || if dark() { Icon::Sun } else { Icon::Moon },
                            st.t("settings.theme"),
                            Variant::Ghost,
                            move || st.toggle_appearance(),
                        )),
                ),
        )
        .child(content(st, true))
        .child(bottom_nav(st))
}

/// Every section not in the bottom bar, as a sheet of large targets.
fn more_sheet(st: AppState, open: Signal<bool>) -> View {
    let mut grid = view().flex_row().flex_wrap().px(px(12.0)).pb(px(12.0));
    for r in [
        Route::Products,
        Route::Inventory,
        Route::Purchases,
        Route::Cash,
        Route::Reports,
        Route::Settings,
        Route::Design,
    ] {
        grid = grid.child(
            view()
                .w(relative(1.0 / 3.0))
                .items_center()
                .gap(px(8.0))
                .py(px(14.0))
                .rounded(px(radius::lg()))
                .active(|s| s.bg(Tone::Hover.color()))
                .on_press(move || {
                    st.route.set(r);
                    open.set(false);
                })
                .child(
                    view()
                        .w(px(48.0))
                        .h(px(48.0))
                        .rounded(px(radius::lg()))
                        .items_center()
                        .justify_center()
                        .bg(Tone::PrimarySoft.color())
                        .child(icon(r.icon(), 22.0, || Tone::Primary)),
                )
                .child(label(st.t(r.key()), ty::SM, FontWeight::MEDIUM, Tone::Ink)),
        );
    }
    view()
        .gap(px(4.0))
        .child(view().px(px(24.0)).pt(px(12.0)).pb(px(4.0)).child(label(
            st.t("nav.more"),
            ty::XL,
            FontWeight::BOLD,
            Tone::Ink,
        )))
        .child(grid)
}

fn bottom_nav(st: AppState) -> View {
    let more_open = signal(false);
    let in_more = computed(move || {
        !matches!(
            st.route.get(),
            Route::Home | Route::Sell | Route::Sales | Route::Customers
        )
    });
    let tab = move |r: Option<Route>, which: Icon, caption: Box<dyn Fn() -> String>| {
        let active = computed(move || match r {
            Some(r) => st.route.get() == r,
            None => in_more.get(),
        });
        view()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(3.0))
            .h(px(58.0))
            .on_press(move || match r {
                Some(r) => st.route.set(r),
                None => more_open.set(true),
            })
            .child(dynamic(move || {
                icon(which, 22.0, move || {
                    if active.get() {
                        Tone::Primary
                    } else {
                        Tone::Muted
                    }
                })
                .into_element()
            }))
            .child(
                text(caption)
                    .text_size(px(11.0))
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
    // Selling is the centre of the phone app: a larger, filled button.
    let sell = view()
        .flex_1()
        .items_center()
        .justify_center()
        .h(px(58.0))
        .child(
            view()
                .w(px(50.0))
                .h(px(50.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(Tone::Primary.color())
                .shadow_md()
                .active(|s| s.opacity(0.85))
                .label({
                    let f = st.t("nav.sell");
                    move || SharedString::from(f())
                })
                .on_press(move || st.route.set(Route::Sell))
                .child(icon(Icon::Cart, 22.0, || Tone::OnPrimary)),
        );
    view()
        .pb_safe_area()
        .bg(Tone::Canvas.color())
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(
            view()
                .flex_row()
                .items_center()
                .child(tab(
                    Some(Route::Home),
                    Icon::Home,
                    Box::new(st.t("nav.home")),
                ))
                .child(tab(
                    Some(Route::Sales),
                    Icon::Receipt,
                    Box::new(st.t("nav.sales")),
                ))
                .child(sell)
                .child(tab(
                    Some(Route::Customers),
                    Icon::Users,
                    Box::new(st.t("nav.customers")),
                ))
                .child(tab(None, Icon::More, Box::new(st.t("nav.more")))),
        )
        .child(modal(more_open, true, move || {
            more_sheet(st, more_open).into_element()
        }))
}

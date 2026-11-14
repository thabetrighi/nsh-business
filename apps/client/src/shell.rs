//! App shell (DESIGN.md v2 §Layout): on wide screens a quiet sidebar and a
//! top bar with search and the main action; on phones a header and a bottom
//! navigation. The sidebar sits at the reading start, so it moves to the
//! other side when the language changes.

use nsh_core::element::{SharedString, View};
use nsh_core::prelude::*;

use crate::command_palette::command_palette;
use crate::nav::{BOTTOM, Route, SECTIONS, Section, TOP};
use crate::screens::common::coming_soon;
use crate::screens::lock::lock_screen;
use crate::screens::{design::design, home::home, products::products, settings::settings};
use crate::state::AppState;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, figure, icon_button, kbd, label, label_line, spacer,
};
use crate::ui::overlay::{modal, toast_layer};
use crate::ui::surface::page_pad;
use crate::ui::tokens::{Status, Tone, radius, sp, ty};

/// Width below which the phone layout is used.
const COMPACT_MAX: f32 = 760.0;
const SIDEBAR_W: f32 = 248.0;

pub fn shell(st: AppState) -> impl IntoElement {
    let compact = computed(|| window().width.0 < COMPACT_MAX);
    // A driver who signs in lands on their day (the same app, their account).
    let landed: Signal<Option<String>> = signal(None);
    effect(move || {
        let who = st.user.get().map(|u| u.user_id);
        if who.is_none() || who == landed.get_untracked() {
            return;
        }
        landed.set(who);
        if st.can("distribution.drive") && !st.can("distribution.plan") && st.shows(Route::MyDay) {
            st.route.set(Route::MyDay);
        }
    });
    view()
        .flex_1()
        .direction(move || st.direction())
        .bg(Tone::Canvas.color())
        .text_color(Tone::Body.color())
        .child(
            when(
                move || st.needs_setup.get(),
                move || crate::screens::setup::setup_wizard(st).into_element(),
            )
            .otherwise(move || {
                when(
                    move || st.needs_sign_in.get() && st.user.get().is_none(),
                    move || lock_screen(st).into_element(),
                )
                .otherwise(move || {
                    when(compact, move || phone(st).into_element())
                        .otherwise(move || desktop(st).into_element())
                        .into_element()
                })
                .into_element()
            }),
        )
        .child(dynamic(move || {
            toast_layer(st, compact.get()).into_element()
        }))
        .child(dynamic(move || {
            command_palette(st, compact.get()).into_element()
        }))
        .child(dynamic(move || {
            crate::ui::overlay::modal(st.shortcuts, compact.get(), move || {
                crate::topbar::shortcuts_dialog(st, move || st.shortcuts.set(false)).into_element()
            })
            .into_element()
        }))
}

fn content(st: AppState, compact: bool) -> impl IntoElement {
    // Screens with their own scrolling (tables) fill the area; the rest scroll as a page.
    // Only the route rebuilds a screen: what a screen reads while it is built
    // (the session, the data) must not throw away its state (an open editor).
    dynamic(move || {
        let route = st.route.get();
        untracked(move || page_of(st, route, compact))
    })
}

fn page_of(st: AppState, route: Route, compact: bool) -> AnyElement {
    match route {
        Route::Home => scroll().flex_1().child(home(st, compact)).into_element(),
        Route::Products => products(st, compact).into_element(),
        Route::Settings => settings(st, compact).into_element(),
        Route::Inventory => scroll()
            .flex_1()
            .child(crate::screens::inventory::stock_page(st, compact))
            .into_element(),
        Route::Movements => scroll()
            .flex_1()
            .child(crate::screens::inventory::movements_page(st, compact))
            .into_element(),
        Route::Transfers => scroll()
            .flex_1()
            .child(crate::screens::inventory::transfers_page(st, compact))
            .into_element(),
        Route::StockCounts => scroll()
            .flex_1()
            .child(crate::screens::inventory::counts_page(st, compact))
            .into_element(),
        Route::Categories => scroll()
            .flex_1()
            .child(crate::screens::catalog::categories(st, compact))
            .into_element(),
        Route::Units => scroll()
            .flex_1()
            .child(crate::screens::catalog::units(st, compact))
            .into_element(),
        Route::PriceLists => scroll()
            .flex_1()
            .child(crate::screens::catalog::price_lists(st, compact))
            .into_element(),
        Route::Customers => scroll()
            .flex_1()
            .child(crate::screens::customers::customers_page(st, compact))
            .into_element(),
        Route::Debts => scroll()
            .flex_1()
            .child(crate::screens::customers::debts_page(st, compact))
            .into_element(),
        Route::Installments => scroll()
            .flex_1()
            .child(crate::screens::customers::installments_page(st, compact))
            .into_element(),
        Route::Expenses => scroll()
            .flex_1()
            .child(crate::screens::expenses::expenses_page(st, compact))
            .into_element(),
        Route::Suppliers => scroll()
            .flex_1()
            .child(crate::screens::purchases::suppliers_page(st, compact))
            .into_element(),
        Route::Reports => scroll()
            .flex_1()
            .child(crate::screens::reports::reports_page(st, compact))
            .into_element(),
        Route::StockReports => scroll()
            .flex_1()
            .child(crate::screens::reports::stock_reports_page(st, compact))
            .into_element(),
        Route::FinanceReports => scroll()
            .flex_1()
            .child(crate::screens::reports::finance_reports_page(st, compact))
            .into_element(),
        Route::Shifts => scroll()
            .flex_1()
            .child(crate::screens::reports::shifts_page(st, compact))
            .into_element(),
        Route::PartyReports => scroll()
            .flex_1()
            .child(crate::screens::reports::party_reports_page(st, compact))
            .into_element(),
        Route::PurchaseOrders => scroll()
            .flex_1()
            .child(crate::screens::purchases::orders_page(st, compact))
            .into_element(),
        Route::Purchases => scroll()
            .flex_1()
            .child(crate::screens::purchases::purchases_page(st, compact))
            .into_element(),
        Route::Sales => scroll()
            .flex_1()
            .child(crate::screens::sales::sales_page(st, compact))
            .into_element(),
        Route::Returns => scroll()
            .flex_1()
            .child(crate::screens::sales::returns_page(st, compact))
            .into_element(),
        Route::Quotes => scroll()
            .flex_1()
            .child(crate::screens::sales::documents_page(st, compact))
            .into_element(),
        // The till fills the window: its parts scroll on their own.
        Route::Sell => crate::screens::pos::pos_page(st, compact).into_element(),
        Route::Warehouses => scroll()
            .flex_1()
            .child(crate::screens::warehouses::warehouses_page(st, compact))
            .into_element(),
        Route::PriceCheck => crate::screens::price_check::price_check(st, compact).into_element(),
        Route::Vans => scroll()
            .flex_1()
            .child(crate::screens::fleet::vans_page(st, compact))
            .into_element(),
        Route::FieldOrders => scroll()
            .flex_1()
            .child(crate::screens::field::field_page(st, compact))
            .into_element(),
        Route::MyDay => scroll()
            .flex_1()
            .child(crate::screens::my_day::my_day_page(st, compact))
            .into_element(),
        Route::Tours => scroll()
            .flex_1()
            .child(crate::screens::distribution::tours_page(st, compact))
            .into_element(),
        Route::Journal => scroll()
            .flex_1()
            .child(crate::screens::accounting::journal_page(st, compact))
            .into_element(),
        Route::Accounts => scroll()
            .flex_1()
            .child(crate::screens::accounting::accounts_page(st, compact))
            .into_element(),
        Route::Ledger => scroll()
            .flex_1()
            .child(crate::screens::accounting::ledger_page(st, compact))
            .into_element(),
        Route::TrialBalance => scroll()
            .flex_1()
            .child(crate::screens::accounting::trial_balance_page(st, compact))
            .into_element(),
        Route::Design => scroll().flex_1().child(design(st, compact)).into_element(),
        other => scroll()
            .flex_1()
            .child(coming_soon(st, other, compact))
            .into_element(),
    }
}

fn store_name(st: AppState) -> impl Fn() -> String + 'static {
    move || st.shop.get().map(|s| s.0).unwrap_or_default()
}

fn store_place(st: AppState) -> impl Fn() -> String + 'static {
    move || st.shop.get().map(|s| s.1).unwrap_or_default()
}

pub fn user_name(st: AppState) -> impl Fn() -> String + 'static {
    move || st.user.get().map(|u| u.name).unwrap_or_default()
}

// ---------------------------------------------------------------- desktop

fn desktop(st: AppState) -> View {
    view().flex_1().flex_row().child(sidebar(st)).child(
        view()
            .flex_1()
            .min_w(px(0.0))
            .child(dynamic(move || {
                if st.bare.get() {
                    view().into_element()
                } else {
                    top_bar(st).into_element()
                }
            }))
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

/// The sidebar: full, or icons only (a section's pages in a menu).
fn sidebar(st: AppState) -> View {
    view().child(dynamic(move || {
        if st.focus.get() {
            return view().into_element();
        }
        if st.biz.sidebar_narrow.get() {
            narrow_sidebar(st).into_element()
        } else {
            full_sidebar(st).into_element()
        }
    }))
}

/// Icons only: the top pages, each section as its icon (its pages in a
/// menu), settings at the bottom.
fn narrow_sidebar(st: AppState) -> View {
    let item = move |r: Route| {
        let active = computed(move || st.route.get() == r);
        view()
            .test_id(&format!("nav-{}", r.code()))
            .w(px(40.0))
            .h(px(40.0))
            .items_center()
            .justify_center()
            .rounded(px(radius::md()))
            .cursor_pointer()
            .style_when(active, |s| s.bg(Tone::Hover.color()))
            .hover(|s| s.bg(Tone::Hover.color()))
            .label(move || SharedString::from(st.tn(r.key())))
            .on_press(move || st.route.set(r))
            .child(dynamic(move || {
                icon(r.icon(), 18.0, move || {
                    if active.get() {
                        Tone::Primary
                    } else {
                        Tone::Muted
                    }
                })
                .into_element()
            }))
    };
    let mut nav = view().gap(px(4.0)).items_center();
    for &r in TOP {
        nav = nav.child(item(r));
    }
    nav = nav.child(
        view()
            .w(px(28.0))
            .h(px(1.0))
            .my(px(6.0))
            .bg(Tone::Border.color()),
    );
    for (i, section) in SECTIONS.iter().enumerate() {
        let holds = computed(move || section.routes.contains(&st.route.get()));
        nav = nav.child(when(
            move || section.routes.iter().any(|r| st.shows(*r)),
            move || {
                let trigger = view()
                    .test_id(&format!("section-{i}"))
                    .w(px(40.0))
                    .h(px(40.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(radius::md()))
                    .cursor_pointer()
                    .style_when(holds, |s| s.bg(Tone::Hover.color()))
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .label(move || SharedString::from(st.tn(section.key)))
                    .child(dynamic(move || {
                        icon(section.icon, 18.0, move || {
                            if holds.get() {
                                Tone::Primary
                            } else {
                                Tone::Muted
                            }
                        })
                        .into_element()
                    }));
                // Its pages beside it: the section's name, then each page
                // with its icon (the one shown marked).
                let mut acts = vec![crate::ui::menu::Act::Title(st.tn(section.key))];
                for &r in section.routes.iter().filter(|r| st.shows(**r)) {
                    let mut a = crate::ui::menu::Act::with(r.icon(), st.tn(r.key()), move || {
                        st.route.set(r)
                    });
                    if st.route.get_untracked() == r {
                        a = a.hint("●");
                    }
                    acts.push(a);
                }
                let side = if st.rtl() {
                    nsh_core::element::Placement::Left
                } else {
                    nsh_core::element::Placement::Right
                };
                crate::ui::menu::menu_at(trigger, acts, side, nsh_core::element::Align::Start)
                    .into_element()
            },
        ));
    }
    view()
        .flex_1()
        .min_h(px(0.0))
        .w(px(64.0))
        .bg(Tone::Sidebar.color())
        .border_e(px(1.0))
        .border_e_color(Tone::Border.color())
        .py(px(10.0))
        .gap(px(10.0))
        .items_center()
        .child(
            view()
                .w(px(36.0))
                .h(px(36.0))
                .rounded(px(radius::md()))
                .items_center()
                .justify_center()
                .bg(Tone::Primary.color())
                .child(icon(Icon::Store, 18.0, || Tone::OnPrimary)),
        )
        .child(scroll().flex_1().child(nav.pb(px(12.0))))
        .child(
            view()
                .gap(px(4.0))
                .pt(px(8.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(item(Route::Design))
                .child(item(Route::Settings)),
        )
}

fn full_sidebar(st: AppState) -> View {
    // The section of the open page is expanded when the page changes.
    effect(move || {
        if let Some((i, _)) = st.route.get().section() {
            let bit = 1u32 << i;
            if st.open_sections.get_untracked() & bit == 0 {
                st.open_sections.update(|m| *m |= bit);
            }
        }
    });
    let mut nav = view().gap(px(1.0)).px(px(10.0));
    for &r in TOP {
        nav = nav.child(nav_item(st, r, false));
    }
    nav = nav.child(view().px(px(10.0)).pt(px(16.0)).pb(px(6.0)).child(label(
        st.t("nav.workspace"),
        ty::XS,
        FontWeight::SEMIBOLD,
        Tone::Faint,
    )));
    for (i, section) in SECTIONS.iter().enumerate() {
        // A section shows when one of its pages does (activity, permissions).
        nav = nav.child(when(
            move || section.routes.iter().any(|r| st.shows(*r)),
            move || section_group(st, i, section).into_element(),
        ));
    }
    // The wrapper in `sidebar` is a column: this fills its height.
    view()
        .flex_1()
        .min_h(px(0.0))
        .w(px(SIDEBAR_W))
        .bg(Tone::Sidebar.color())
        .border_e(px(1.0))
        .border_e_color(Tone::Border.color())
        .pt(px(10.0))
        .pb(px(10.0))
        .gap(px(10.0))
        .child(store_switcher(st))
        .child(scroll().flex_1().child(nav.pb(px(12.0))))
        .child(
            view()
                .px(px(10.0))
                .pt(px(8.0))
                .gap(px(1.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(nav_item(st, Route::Design, false))
                .child(nav_item(st, Route::Settings, false)),
        )
}

/// Pages with something waiting, shown as a count beside their name.
fn nav_count(_r: Route) -> Option<(i64, Status)> {
    None
}

/// A sidebar section: a header that expands to its pages.
fn section_group(st: AppState, i: usize, section: &'static Section) -> View {
    let bit = 1u32 << i;
    let open = computed(move || st.open_sections.get() & bit != 0);
    let holds = computed(move || section.routes.contains(&st.route.get()));
    let header = view()
        .test_id(&format!("section-{i}"))
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(sp(32.0)))
        .px(px(10.0))
        .rounded(px(radius::md()))
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
        .on_press(move || st.open_sections.update(|m| *m ^= bit))
        .child(dynamic(move || {
            icon(section.icon, 16.0, move || {
                if holds.get() {
                    Tone::Primary
                } else {
                    Tone::Muted
                }
            })
            .into_element()
        }))
        .child(
            view().flex_1().min_w(px(0.0)).child(
                label_line(st.t(section.key), ty::SM, FontWeight::MEDIUM, Tone::Body)
                    .style_when(holds, |s| s.font_weight(FontWeight::SEMIBOLD))
                    .text_color(move || {
                        if holds.get() {
                            Tone::Ink.color()
                        } else {
                            Tone::Body.color()
                        }
                    }),
            ),
        )
        .child(dynamic(move || {
            let which = if open.get() {
                Icon::ChevronDown
            } else if st.rtl() {
                Icon::ChevronLeft
            } else {
                Icon::ChevronRight
            };
            icon(which, 14.0, || Tone::Faint).into_element()
        }));
    let children = when(open, move || {
        let mut list = view()
            .ms(px(18.0))
            .ps(px(10.0))
            .border_s(px(1.0))
            .border_s_color(Tone::Border.color())
            .gap(px(1.0))
            .py(px(2.0));
        for &r in section.routes {
            list = list.child(when(
                move || st.shows(r),
                move || nav_item(st, r, true).into_element(),
            ));
        }
        list.into_element()
    });
    view().gap(px(1.0)).child(header).child(children)
}

fn nav_item(st: AppState, r: Route, nested: bool) -> View {
    let active = computed(move || st.route.get() == r);
    let mut item = view()
        .test_id(&format!("nav-{}", r.code()))
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(sp(if nested { 30.0 } else { 32.0 })))
        .px(px(10.0))
        .rounded(px(radius::md()))
        .cursor_pointer()
        .style_when(active, |s| s.bg(Tone::Hover.color()))
        .hover(|s| s.bg(Tone::Hover.color()))
        .on_press(move || st.route.set(r));
    {
        let size = if nested { 14.0 } else { 16.0 };
        item = item.child(dynamic(move || {
            icon(r.icon(), size, move || {
                if active.get() {
                    Tone::Primary
                } else {
                    Tone::Muted
                }
            })
            .into_element()
        }));
    }
    item = item.child(
        view().flex_1().min_w(px(0.0)).child(
            label_line(st.t(r.key()), ty::SM, FontWeight::MEDIUM, Tone::Body)
                .style_when(active, |s| s.font_weight(FontWeight::SEMIBOLD))
                .text_color(move || {
                    if active.get() {
                        Tone::Primary.color()
                    } else if nested {
                        Tone::Muted.color()
                    } else {
                        Tone::Body.color()
                    }
                }),
        ),
    );
    if let Some((n, status)) = nav_count(r) {
        item = item.child(
            view()
                .px(px(6.0))
                .h(px(18.0))
                .justify_center()
                .rounded_full()
                .bg(status.bg().color())
                .child(figure(
                    move || st.fmt().integer(n),
                    11.0,
                    FontWeight::SEMIBOLD,
                    move || status.fg(),
                )),
        );
    }
    item
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
        .child(
            icon_button(
                || Icon::PanelStart,
                st.t("action.toggle_sidebar"),
                Variant::Ghost,
                move || st.biz.sidebar_narrow.update(|v| *v = !*v),
            )
            .test_id("toggle-sidebar"),
        )
        .child(search_box(st))
        .child(spacer())
        .child(sync_status(st))
        .child(crate::topbar::notifications(st))
        .child(crate::topbar::help(st))
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
        .child(when(
            move || st.needs_sign_in.get(),
            move || {
                icon_button(
                    || Icon::Lock,
                    st.t("action.lock"),
                    Variant::Ghost,
                    move || st.lock(),
                )
                .test_id("lock-now")
                .into_element()
            },
        ))
        .child(dynamic(move || {
            // Its entries follow the language, the theme and who is there.
            st.lang.get();
            st.user.get();
            st.prefs.theme.get();
            st.biz.status_icon_only.get();
            untracked(move || crate::topbar::user_menu(st)).into_element()
        }))
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
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .pe(px(6.0))
        .child(when(
            move || st.unsound.get(),
            move || {
                badge(Status::Danger, Some(Icon::Alert), st.t("storage.unsound"))
                    .test_id("storage-unsound")
                    .into_element()
            },
        ))
        .label(move || SharedString::from(st.tn(sync_state(st).2)))
        // Its words may go (the person's choice): the icon stays, its name
        // in the tooltip; a press switches.
        .child(dynamic(move || {
            let (status, which, key) = sync_state(st);
            let v = if st.biz.status_icon_only.get() {
                view()
                    .w(px(30.0))
                    .h(px(30.0))
                    .rounded_full()
                    .items_center()
                    .justify_center()
                    .bg(status.bg().color())
                    .child(icon(which, 15.0, move || status.fg()))
            } else {
                view().child(badge(status, Some(which), st.t(key)))
            };
            v.test_id("sync-status")
                .cursor_pointer()
                .on_press(move || st.biz.status_icon_only.update(|v| *v = !*v))
                .into_element()
        }))
}

/// What the sync pill says: only on this device (no server), sending,
/// failing, waiting, or in step with the server.
fn sync_state(st: AppState) -> (Status, Icon, &'static str) {
    let v = st.sync.get();
    if !v.linked {
        (Status::Warning, Icon::Offline, "sync.offline")
    } else if v.busy {
        (Status::Info, Icon::Refresh, "sync.busy")
    } else if !v.error.is_empty() {
        (Status::Danger, Icon::Alert, "sync.failing")
    } else if v
        .last_ok_at
        .is_some_and(|t| crate::state::now_ms() - t > crate::sync::STALE_MS)
    {
        // A week without reaching the server (ADR-0007).
        (Status::Danger, Icon::Alert, "sync.stale")
    } else if v.pending > 0 {
        (Status::Warning, Icon::CloudUpload, "sync.waiting")
    } else {
        (Status::Success, Icon::Synced, "sync.synced")
    }
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

/// Every page, by section, as a scrolling sheet of large targets.
fn more_sheet(st: AppState, open: Signal<bool>) -> View {
    let go = move |r: Route| {
        st.route.set(r);
        open.set(false);
    };
    let tile = move |r: Route| {
        view().w(relative(0.5)).p(px(4.0)).child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .h(px(48.0))
                .px(px(10.0))
                .rounded(px(radius::md()))
                .bg(Tone::Subtle.color())
                .active(|s| s.bg(Tone::Hover.color()))
                .on_press(move || go(r))
                .child(icon(r.icon(), 18.0, || Tone::Primary))
                .child(view().flex_1().min_w(px(0.0)).child(label_line(
                    st.t(r.key()),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Ink,
                ))),
        )
    };
    let mut body = view().px(px(12.0)).pb(px(16.0)).gap(px(12.0));
    for section in SECTIONS.iter().chain(std::iter::once(&Section {
        key: "nav.sec.system",
        icon: Icon::Settings,
        routes: BOTTOM,
    })) {
        let routes: Vec<Route> = section
            .routes
            .iter()
            .copied()
            .filter(|r| st.shows(*r))
            .collect();
        if routes.is_empty() {
            continue;
        }
        let mut grid = view().flex_row().flex_wrap();
        for r in routes {
            grid = grid.child(tile(r));
        }
        body = body.child(
            view()
                .gap(px(4.0))
                .child(view().px(px(6.0)).child(label(
                    st.t(section.key),
                    ty::XS,
                    FontWeight::SEMIBOLD,
                    Tone::Muted,
                )))
                .child(grid),
        );
    }
    view()
        .gap(px(4.0))
        .child(view().px(px(18.0)).pt(px(10.0)).pb(px(6.0)).child(label(
            st.t("nav.more"),
            ty::XL,
            FontWeight::BOLD,
            Tone::Ink,
        )))
        .child(scroll().max_h(px(560.0)).child(body))
}

fn bottom_nav(st: AppState) -> View {
    let more_open = signal(false);
    let in_more = computed(move || {
        !matches!(
            st.route.get(),
            Route::Home | Route::Sell | Route::Sales | Route::Customers | Route::MyDay
        )
    });
    let tab = move |r: Option<Route>, which: Icon, caption: Box<dyn Fn() -> String>| {
        let active = computed(move || match r {
            Some(r) => st.route.get() == r,
            None => in_more.get(),
        });
        view()
            .test_id(&format!("bottom-{}", r.map_or("more", Route::code)))
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
                // A driver's second tab is their day; others', the sales.
                .child(view().flex_1().child(dynamic(move || {
                    if st.can("distribution.drive") && st.shows(Route::MyDay) {
                        tab(
                            Some(Route::MyDay),
                            Icon::Navigation,
                            Box::new(st.t("nav.my_day")),
                        )
                        .into_element()
                    } else {
                        tab(
                            Some(Route::Sales),
                            Icon::Receipt,
                            Box::new(st.t("nav.sales")),
                        )
                        .into_element()
                    }
                })))
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

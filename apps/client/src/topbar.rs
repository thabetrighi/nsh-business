//! The top bar's own menus (owner notes): the bell with what wants
//! attention (products out of stock or low, customers past their days to
//! pay, suppliers owed, a database to look at), each a way to the page
//! that deals with it; help (the command palette, the keyboard shortcuts,
//! about); and the person's own menu under their avatar (who they are,
//! appearance, language, settings, lock).

use nb_i18n::{Lang, tr_args};
use nb_store::usecases::reports::{Alerts, alerts};
use nsh_base::{PopoverEvent, popover};
use nsh_core::element::{Align, Placement, View};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::state::AppState;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{avatar, label, label_line};
use crate::ui::menu::{Act, POPUP, menu_on};
use crate::ui::overlay::dialog_frame;
use crate::ui::tokens::Status;
use crate::ui::tokens::{Tone, control, radius, ty};

/// A square button of the top bar (what opens a menu or a popover).
fn bar_button(st: AppState, ic: Icon, title: &'static str, id: &str) -> View {
    let side = control::md();
    view()
        .test_id(id)
        .w(px(side))
        .h(px(side))
        .items_center()
        .justify_center()
        .rounded(px(radius::md()))
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
        .label({
            let t = st.t(title);
            move || SharedString::from(t())
        })
        .child(icon(ic, 18.0, || Tone::Muted))
}

/// The bell: a count of what wants attention, the list on a press.
pub fn notifications(st: AppState) -> View {
    let data = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| alerts(s)).await }
        },
    );
    let now = computed(move || data.with(|d: Option<&Alerts>| d.cloned().unwrap_or_default()));
    let open = signal(false);
    let count = computed(move || now.get().count() + u32::from(st.unsound.get()));
    let trigger = view()
        .relative()
        .child(bar_button(
            st,
            Icon::Bell,
            "action.notifications",
            "open-notifications",
        ))
        .child(when(
            move || count.get() > 0,
            move || {
                let badge = view()
                    .test_id("notifications-count")
                    .absolute()
                    .top(px(2.0));
                // At the button's end corner, whichever way the text reads.
                let badge = if st.rtl() {
                    badge.left(px(2.0))
                } else {
                    badge.right(px(2.0))
                };
                badge
                    .min_w(px(16.0))
                    .h(px(16.0))
                    .px(px(4.0))
                    .rounded_full()
                    .items_center()
                    .justify_center()
                    .bg(Tone::Danger.color())
                    .child(label_line(
                        move || st.fmt().integer(i64::from(count.get())),
                        ty::XS,
                        FontWeight::BOLD,
                        Tone::OnPrimary,
                    ))
                    .into_element()
            },
        ));
    let go = move |r: Route| {
        open.set(false);
        st.route.set(r);
    };
    let content = dynamic(move || {
        let a = now.get();
        let money = move |m: i64| {
            st.fmt().money(nb_core::Money::from_minor(
                m,
                st.session
                    .get_untracked()
                    .map_or(nb_core::Currency::DZD, |s| s.currency),
            ))
        };
        let mut items: Vec<(Icon, Tone, String, String, Route)> = Vec::new();
        let n = |v: u32| st.fmt().integer(i64::from(v));
        if st.unsound.get() {
            items.push((
                Icon::Alert,
                Tone::Danger,
                st.tn("storage.unsound"),
                st.tn("alerts.unsound.body"),
                Route::Settings,
            ));
        }
        if a.out_of_stock > 0 {
            items.push((
                Icon::XCircle,
                Tone::Danger,
                tr_args(st.lang(), "alerts.out", &[("count", &n(a.out_of_stock))]),
                st.tn("alerts.out.body"),
                Route::Products,
            ));
        }
        if a.low_stock > 0 {
            items.push((
                Icon::Alert,
                Tone::Warning,
                tr_args(st.lang(), "alerts.low", &[("count", &n(a.low_stock))]),
                st.tn("alerts.low.body"),
                Route::Inventory,
            ));
        }
        if a.customers_overdue > 0 {
            items.push((
                Icon::Coins,
                Tone::Warning,
                tr_args(
                    st.lang(),
                    "alerts.overdue",
                    &[("count", &n(a.customers_overdue))],
                ),
                money(a.overdue_minor),
                Route::Debts,
            ));
        }
        if a.suppliers_owed > 0 {
            items.push((
                Icon::Truck,
                Tone::Info,
                tr_args(
                    st.lang(),
                    "alerts.payables",
                    &[("count", &n(a.suppliers_owed))],
                ),
                money(a.payables_minor),
                Route::PartyReports,
            ));
        }
        let mut list = view().gap(px(2.0));
        if items.is_empty() {
            list = list.child(
                view()
                    .items_center()
                    .gap(px(8.0))
                    .py(px(24.0))
                    .child(icon(Icon::CheckCircle, 28.0, || Tone::Success))
                    .child(label(
                        st.t("alerts.none"),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Muted,
                    )),
            );
        }
        for (i, (ic, tone, title, body, r)) in items.into_iter().enumerate() {
            list = list.child(
                view()
                    .test_id(&format!("alert-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .p(px(10.0))
                    .rounded(px(radius::md()))
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .on_press(move || go(r))
                    .child(
                        view()
                            .w(px(34.0))
                            .h(px(34.0))
                            .rounded_full()
                            .items_center()
                            .justify_center()
                            .bg(Tone::Subtle.color())
                            .child(icon(ic, 17.0, move || tone)),
                    )
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
                                move || body.clone(),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(icon(
                        if crate::ui::tokens::rtl() {
                            Icon::ChevronLeft
                        } else {
                            Icon::ChevronRight
                        },
                        14.0,
                        || Tone::Faint,
                    )),
            );
        }
        view()
            .w(px(340.0))
            .gap(px(6.0))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .px(px(10.0))
                    .pt(px(6.0))
                    .child(view().flex_1().child(label_line(
                        st.t("action.notifications"),
                        ty::MD,
                        FontWeight::BOLD,
                        Tone::Ink,
                    )))
                    .child(label_line(
                        move || {
                            let c = st.fmt().integer(i64::from(count.get()));
                            tr_args(st.lang(), "alerts.count", &[("count", &c)])
                        },
                        ty::XS,
                        FontWeight::MEDIUM,
                        Tone::Muted,
                    )),
            )
            .child(list)
            .into_element()
    });
    view().child(
        popover()
            .open(move || Some(open.get()))
            .on_open_change(move |e: &PopoverEvent| {
                if let PopoverEvent::OpenChange(v, _) = e {
                    open.set(*v);
                }
            })
            .placement(Placement::Bottom)
            .align(Align::End)
            .popup_class(POPUP)
            .trigger(trigger)
            .child(view().test_id("notifications").p(px(4.0)).child(content)),
    )
}

/// Help: the palette, the shortcuts, about.
pub fn help(st: AppState) -> View {
    let acts = vec![
        Act::with(Icon::Search, st.tn("help.palette"), move || {
            st.palette.set(true)
        })
        .hint("Ctrl K"),
        Act::with(Icon::Keyboard, st.tn("help.shortcuts"), move || {
            st.shortcuts.set(true)
        }),
        Act::Line,
        Act::with(Icon::Info, st.tn("help.about"), move || {
            st.settings_tab.set(9);
            st.route.set(Route::Settings);
        }),
    ];
    menu_on(bar_button(st, Icon::Help, "action.help", "open-help"), acts)
}

/// The person's own menu, under their avatar.
pub fn user_menu(st: AppState) -> View {
    let name = crate::shell::user_name(st);
    let trigger = view()
        .test_id("open-user")
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .p(px(2.0))
        .rounded_full()
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
        .child(avatar(crate::shell::user_name(st), 32.0, Status::Accent))
        .child(icon(Icon::ChevronDown, 14.0, || Tone::Muted));
    let langs: Vec<Act> = [Lang::Ar, Lang::Fr, Lang::En]
        .into_iter()
        .map(|l| {
            Act::new(l.native_name().to_owned(), move || st.lang.set(l))
                .checked(st.lang.get_untracked() == l)
        })
        .collect();
    let who = name();
    let mut acts = vec![
        Act::Title(if who.is_empty() {
            st.tn("user.this_device")
        } else {
            who
        }),
        Act::with(Icon::Brush, st.tn("user.appearance"), move || {
            st.settings_tab.set(1);
            st.route.set(Route::Settings);
        }),
        Act::with(
            if nsh_core::prelude::dark() {
                Icon::Sun
            } else {
                Icon::Moon
            },
            st.tn(if nsh_core::prelude::dark() {
                "user.light"
            } else {
                "user.dark"
            }),
            move || st.toggle_appearance(),
        ),
        Act::sub(Icon::Languages, st.tn("user.language"), langs),
        Act::with(Icon::Offline, st.tn("user.status_text"), move || {
            st.biz.status_icon_only.update(|v| *v = !*v)
        })
        .checked(!st.biz.status_icon_only.get_untracked()),
        Act::Line,
        Act::with(Icon::Settings, st.tn("nav.settings"), move || {
            st.route.set(Route::Settings)
        }),
        Act::with(Icon::Shield, st.tn("user.users"), move || {
            st.settings_tab.set(7);
            st.route.set(Route::Settings);
        })
        .when(st.can("users.manage")),
    ];
    if st.needs_sign_in.get_untracked() {
        acts.push(Act::Line);
        acts.push(Act::with(Icon::Lock, st.tn("action.lock"), move || st.lock()).hint("Ctrl L"));
    }
    menu_on(trigger, acts)
}

/// The keyboard shortcuts, by place.
pub fn shortcuts_dialog(st: AppState, close: impl Fn() + Clone + 'static) -> View {
    let groups: [(&str, &[(&str, &str)]); 2] = [
        (
            "help.keys.app",
            &[
                ("Ctrl K", "help.palette"),
                ("Ctrl L", "action.lock"),
                ("F11", "pos.fullscreen"),
            ],
        ),
        (
            "help.keys.pos",
            &[
                ("F2", "pos.key.search"),
                ("F9", "pos.pay"),
                ("Ctrl+Enter", "pos.pay_now"),
                ("F1", "pos.sales"),
                ("F3", "pos.key.held"),
                ("F4", "pos.key.free"),
                ("F5", "pos.key.discount"),
                ("F6", "pos.edit_line"),
                ("F7", "pos.key.hold"),
                ("F8", "pos.key.clear"),
                ("F10", "pos.key.customer"),
                ("F12", "pos.return"),
            ],
        ),
    ];
    let mut body = view().gap(px(16.0));
    for (title, keys) in groups {
        let mut g =
            view()
                .gap(px(4.0))
                .child(label_line(st.t(title), ty::SM, FontWeight::BOLD, Tone::Ink));
        for (k, what) in keys {
            let what = *what;
            let k = *k;
            g = g.child(
                view()
                    .flex_row()
                    .items_center()
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(Tone::Border.color())
                    .child(view().flex_1().child(label_line(
                        st.t(what),
                        ty::SM,
                        FontWeight::NORMAL,
                        Tone::Body,
                    )))
                    .child(crate::ui::kit::kbd(move || k.to_owned())),
            );
        }
        body = body.child(g);
    }
    dialog_frame(
        st,
        st.t("help.shortcuts"),
        st.t("help.shortcuts.desc"),
        body,
        view(),
        close,
    )
}

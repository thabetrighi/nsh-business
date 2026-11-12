//! The till's panels that a layout sets up one by one (owner notes): which
//! buttons a panel shows and in what order, whether they read as an icon,
//! a word or both; payment in one control (at once in cash, through the
//! payment screen, with the receipt); a strip with the cart as a badge,
//! its total and what to do with it; the till and its shift.

use std::rc::Rc;

use crate::screens::pos_layout::{Panel, Widget};

use super::*;

/// What a button of a panel shows: (icon, text).
fn shows(labels: u8) -> (bool, bool) {
    (labels != 2, labels != 1)
}

/// One of the buttons a panel may show: its icon, name, key, test id,
/// whether it may be used now, and what it does.
struct Button {
    icon: Icon,
    key: &'static str,
    hint: &'static str,
    id: String,
    allowed: bool,
    run: Rc<dyn Fn()>,
}

/// The cart's actions, by code (`pos_layout::Widget::Actions`'s items).
fn action(st: AppState, t: Till, code: &str, n: usize) -> Option<Button> {
    let id = |base: &str| {
        if n == 0 {
            format!("side-{base}")
        } else {
            format!("side-{base}-{n}")
        }
    };
    let b = |icon, key, hint, base: &str, run: Rc<dyn Fn()>| Button {
        icon,
        key,
        hint,
        id: id(base),
        allowed: true,
        run,
    };
    Some(match code {
        "discount" => Button {
            allowed: st.can("sales.discount"),
            ..b(
                Icon::Percent,
                "pos.key.discount",
                "F5",
                "discount",
                Rc::new(move || {
                    if !t.cart.with_untracked(Vec::is_empty) {
                        t.discount_open.set(true);
                    }
                }),
            )
        },
        "customer" => b(
            Icon::User,
            "pos.key.customer",
            "F10",
            "customer",
            Rc::new(move || t.customer_open.set(true)),
        ),
        "hold" => b(
            Icon::Pause,
            "pos.key.hold",
            "F7",
            "hold",
            Rc::new(move || hold(st, t)),
        ),
        "held" => b(
            Icon::History,
            "pos.key.held",
            "F3",
            "held",
            Rc::new(move || t.recalling.set(true)),
        ),
        "misc" => b(
            Icon::PackagePlus,
            "pos.key.free",
            "F4",
            "free",
            Rc::new(move || t.free_open.set(true)),
        ),
        "return" => Button {
            allowed: st.can("sales.return") || st.can("sales.sell"),
            ..b(
                Icon::Undo,
                "pos.key.return",
                "F12",
                "return",
                Rc::new(move || t.returning.set(true)),
            )
        },
        "clear" => b(
            Icon::Trash,
            "pos.key.clear",
            "F8",
            "clear",
            Rc::new(move || t.clear()),
        ),
        "sales" => b(
            Icon::Receipt,
            "pos.key.sales",
            "F1",
            "sales",
            Rc::new(move || t.sales_open.set(true)),
        ),
        "prebill" => b(
            Icon::Printer,
            "pos.prebill",
            "",
            "prebill",
            Rc::new(move || {
                if !t.cart.with_untracked(Vec::is_empty) {
                    t.prebill_open.set(true);
                }
            }),
        ),
        "keys" => b(
            Icon::Keyboard,
            "pos.keys",
            "",
            "keys",
            Rc::new(move || t.keys_open.set(true)),
        ),
        _ => return None,
    })
}

/// A count over a button (the carts on hold).
fn count_badge(n: impl Fn() -> usize + 'static) -> View {
    let n = Rc::new(n);
    let shown = n.clone();
    view().child(when(
        move || shown() > 0,
        move || {
            let n = n.clone();
            view()
                .min_w(px(18.0))
                .h(px(18.0))
                .px(px(5.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(Tone::Primary.color())
                .child(label_line(
                    move || n().to_string(),
                    ty::XS,
                    FontWeight::BOLD,
                    Tone::OnPrimary,
                ))
                .into_element()
        },
    ))
}

/// A large button of a panel, as its labels say.
fn tile_button(st: AppState, t: Till, b: Button, labels: u8, code: &str) -> View {
    let (with_icon, with_text) = shows(labels);
    let run = b.run.clone();
    let key = b.key;
    let hint = b.hint;
    let mut v = view()
        .test_id(&b.id)
        .flex_grow(1.0)
        .basis(px(if with_text { 140.0 } else { 56.0 }))
        .min_h(px(crate::ui::tokens::control::lg() + 18.0))
        .p(px(8.0))
        .gap(px(2.0))
        .items_center()
        .justify_center()
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::Border.color())
        .bg(Tone::Surface.color())
        .label(move || SharedString::from(st.tn(key)))
        .opacity(if b.allowed { 1.0 } else { 0.45 });
    if b.allowed {
        v = v
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Subtle.color()))
            .on_press(move || run());
    }
    let mut top = view().flex_row().items_center().gap(px(6.0));
    if with_icon {
        top = top.child(icon(b.icon, if with_text { 18.0 } else { 22.0 }, || {
            Tone::Ink
        }));
    }
    if with_text {
        top = top.child(label_line(
            st.t(key),
            ty::SM,
            FontWeight::SEMIBOLD,
            Tone::Ink,
        ));
    }
    if code == "held" {
        top = top.child(count_badge(move || t.held.with(Vec::len)));
    }
    v = v.child(top);
    if with_text && !hint.is_empty() {
        v = v.child(label_line(
            move || hint.to_owned(),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Faint,
        ));
    }
    v
}

/// The cart's actions as large buttons: those the panel shows, in order.
pub(super) fn actions_panel(st: AppState, t: Till, p: &Panel, n: usize) -> View {
    let mut v = view().p(px(10.0)).flex_row().flex_wrap().gap(px(8.0));
    for code in p.shown_items() {
        if let Some(b) = action(st, t, &code, n) {
            v = v.child(tile_button(st, t, b, p.labels, &code));
        }
    }
    v
}

/// Payment in one control: its parts as the panel says (at once in cash,
/// through the payment screen, with the receipt); the first is the large
/// one.
pub(super) fn pay_panel(st: AppState, t: Till, p: &Panel, in_sheet: bool) -> View {
    let items = p.shown_items();
    let (with_icon, with_text) = shows(p.labels);
    let id = |base: &str| {
        if in_sheet {
            format!("sheet-{base}")
        } else {
            format!("pos-{base}")
        }
    };
    let mut row = view().flex_row().gap(px(2.0));
    for (i, code) in items.iter().enumerate() {
        let (ic, key, hint, test, run): (Icon, &'static str, &'static str, String, Rc<dyn Fn()>) =
            match code.as_str() {
                "direct" => (
                    Icon::Banknote,
                    "pos.pay_now",
                    "Ctrl+Enter",
                    id("pay-now"),
                    Rc::new(move || t.pay_now(st, false)),
                ),
                "dialog" => (
                    Icon::Wallet,
                    "pos.pay_dialog",
                    "F9",
                    id("pay"),
                    Rc::new(move || t.pay(st, false)),
                ),
                "print" => (
                    Icon::Printer,
                    "pos.pay_print",
                    "",
                    id("pay-print"),
                    Rc::new(move || t.pay(st, true)),
                ),
                _ => continue,
            };
        let main = i == 0;
        // The first part says what it is; the others are icons unless
        // the panel reads as words.
        let text = with_text && (main || !with_icon);
        let mut part = view()
            .test_id(&test)
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .h(px(crate::ui::tokens::control::lg() + 6.0))
            .px(px(if text { 16.0 } else { 0.0 }))
            .rounded(px(radius::md()))
            .bg(if main {
                Tone::Primary.color()
            } else {
                Tone::PrimarySoft.color()
            })
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .label(move || SharedString::from(st.tn(key)))
            .on_press(move || run());
        part = if main {
            part.flex_1()
        } else if text {
            part.flex_shrink_0()
        } else {
            part.w(px(crate::ui::tokens::control::lg() + 10.0))
        };
        let ink = if main { Tone::OnPrimary } else { Tone::Primary };
        if with_icon || !text {
            part = part.child(icon(ic, if main { 20.0 } else { 19.0 }, move || ink));
        }
        if text {
            part = part.child(label_line(
                st.t(key),
                if main { ty::MD } else { ty::SM },
                FontWeight::BOLD,
                ink,
            ));
            if main && !hint.is_empty() {
                part = part.child(label_line(
                    move || hint.to_owned(),
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::OnPrimary,
                ));
            }
        }
        row = row.child(part);
    }
    row
}

/// The strip along the bottom: the cart as a badge (open it), the
/// customer, the carts on hold, the till, the total, clear, pay at once.
pub(super) fn bottom_bar(st: AppState, t: Till, p: &Panel) -> View {
    let (with_icon, with_text) = shows(p.labels);
    let total = computed(move || t.totals(currency(st)).map_or(0, |x| x.total.minor()));
    let lines = computed(move || t.cart.with(Vec::len));
    let chip = |id: &'static str, ic: Icon, text: Box<dyn Fn() -> String>, run: Rc<dyn Fn()>| {
        let mut c = view()
            .test_id(id)
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .h(px(crate::ui::tokens::control::lg()))
            .px(px(12.0))
            .rounded(px(radius::md()))
            .bg(Tone::Subtle.color())
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Hover.color()))
            .on_press(move || run());
        if with_icon {
            c = c.child(icon(ic, 18.0, || Tone::Muted));
        }
        if with_text {
            c = c.child(label_line(text, ty::SM, FontWeight::SEMIBOLD, Tone::Ink));
        } else {
            let text = Rc::new(text);
            c = c.label(move || SharedString::from(text()));
        }
        c
    };
    let mut bar = view()
        .test_id("pos-cart-bar")
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(radius::lg()))
        .bg(Tone::Surface.color())
        .border_1()
        .border_color(Tone::Border.color());
    let mut ending = false;
    for code in p.shown_items() {
        bar = match code.as_str() {
            "count" => bar.child(
                view()
                    .test_id("bar-cart")
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(crate::ui::tokens::control::lg()))
                    .px(px(12.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::PrimarySoft.color())
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.85))
                    .label(st.t("pos.bar.open_cart"))
                    .on_press(move || t.cart_open.set(true))
                    .child(icon(Icon::Cart, 18.0, || Tone::Primary))
                    .child(
                        view()
                            .min_w(px(22.0))
                            .h(px(22.0))
                            .px(px(6.0))
                            .rounded_full()
                            .items_center()
                            .justify_center()
                            .bg(Tone::Primary.color())
                            .child(figure(
                                move || st.fmt().integer(lines.get() as i64),
                                ty::XS,
                                FontWeight::BOLD,
                                || Tone::OnPrimary,
                            )),
                    )
                    .child(when(
                        move || with_text,
                        move || {
                            label_line(
                                st.t("pos.bar.lines"),
                                ty::SM,
                                FontWeight::SEMIBOLD,
                                Tone::Primary,
                            )
                            .into_element()
                        },
                    )),
            ),
            "customer" => bar.child(chip(
                "bar-customer",
                Icon::User,
                Box::new(move || {
                    t.customer
                        .get()
                        .map(|c| c.name)
                        .unwrap_or_else(|| st.tn("pos.counter_customer"))
                }),
                Rc::new(move || t.customer_open.set(true)),
            )),
            "held" => bar.child(
                chip(
                    "bar-held",
                    Icon::History,
                    Box::new(move || st.tn("pos.key.held")),
                    Rc::new(move || t.recalling.set(true)),
                )
                .child(count_badge(move || t.held.with(Vec::len))),
            ),
            "till" => bar.child(chip(
                "bar-till",
                Icon::Wallet,
                Box::new(move || till_line(st, t)),
                Rc::new(move || t.sales_open.set(true)),
            )),
            "total" => {
                ending = true;
                bar.child(view().flex_1()).child(
                    view()
                        .test_id("bar-total")
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(label_line(
                            st.t("pos.total"),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Muted,
                        ))
                        .child(figure(
                            move || money(st, total.get()),
                            ty::XL + 6.0,
                            FontWeight::BOLD,
                            || Tone::Ink,
                        )),
                )
            }
            "clear" => bar.child(
                icon_button(
                    || Icon::Trash,
                    st.t("pos.key.clear"),
                    Variant::Ghost,
                    move || t.clear(),
                )
                .test_id("bar-clear"),
            ),
            "pay" => {
                if !ending {
                    ending = true;
                    bar = bar.child(view().flex_1());
                }
                bar.child(view().min_w(px(200.0)).child(pay_panel(
                    st,
                    t,
                    &Panel::new(Widget::Pay).look(0, p.labels),
                    false,
                )))
            }
            _ => bar,
        };
    }
    bar
}

/// "Till 1 · shift S-0003" (what the till's chip and bar show).
fn till_line(st: AppState, t: Till) -> String {
    t.till
        .with(|s| {
            s.as_ref().map(|(reg, shift)| {
                let name = if reg.name.is_empty() {
                    tr_args(st.lang(), "pos.till.code", &[("code", &reg.code)])
                } else {
                    reg.name.clone()
                };
                match shift {
                    Some(sh) => tr_args(
                        st.lang(),
                        "pos.till.line",
                        &[("till", &name), ("number", &sh.number)],
                    ),
                    None => name,
                }
            })
        })
        .unwrap_or_default()
}

/// "Ahmed · since 08:30" (who works the till, since when).
fn since_line(st: AppState, t: Till) -> String {
    let who = crate::shell::user_name(st)();
    let opened = t.till.with(|s| {
        s.as_ref()
            .and_then(|(_, sh)| sh.as_ref().map(|sh| sh.opened_at))
    });
    let since = opened.map(|ms| clock_of(st, ms)).unwrap_or_default();
    tr_args(
        st.lang(),
        "pos.till.since",
        &[("who", &who), ("time", &since)],
    )
}

/// The time of day of `ms` at the store (UTC+1, Algeria, as the lock).
fn clock_of(st: AppState, ms: i64) -> String {
    let mins = (ms / 60_000 + 60).rem_euclid(24 * 60);
    st.fmt().time(
        u32::try_from(mins / 60).unwrap_or(0),
        u32::try_from(mins % 60).unwrap_or(0),
    )
}

/// The till, its shift and what was taken, closing it.
pub(super) fn session_panel(st: AppState, t: Till, p: &Panel) -> View {
    let items = p.shown_items();
    let has = |c: &str| items.iter().any(|i| i == c);
    let (with_icon, with_text) = shows(p.labels);
    let mut v = card().p(px(14.0)).gap(px(10.0));
    if has("info") {
        v = v.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    view()
                        .w(px(38.0))
                        .h(px(38.0))
                        .rounded(px(radius::md()))
                        .items_center()
                        .justify_center()
                        .bg(Tone::SuccessSoft.color())
                        .child(icon(Icon::Wallet, 18.0, || Tone::Success)),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label_line(
                            move || till_line(st, t),
                            ty::SM,
                            FontWeight::BOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            move || {
                                let who = crate::shell::user_name(st)();
                                let since = t
                                    .till
                                    .with(|s| {
                                        s.as_ref().and_then(|(_, sh)| {
                                            sh.as_ref().map(|sh| clock_of(st, sh.opened_at))
                                        })
                                    })
                                    .unwrap_or_default();
                                tr_args(
                                    st.lang(),
                                    "pos.till.since",
                                    &[("who", &who), ("time", &since)],
                                )
                            },
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                ),
        );
    }
    if has("totals") {
        let row = |key: &'static str, f: Box<dyn Fn(&ShiftRow) -> i64>| {
            let f = Rc::new(f);
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(label_line(
                    st.t(key),
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )))
                .child(figure(
                    move || {
                        let f = f.clone();
                        money(
                            st,
                            t.till.with(|s| {
                                s.as_ref()
                                    .and_then(|(_, sh)| sh.as_ref().map(|sh| f(sh)))
                                    .unwrap_or(0)
                            }),
                        )
                    },
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    || Tone::Ink,
                ))
        };
        v = v.child(
            view()
                .gap(px(4.0))
                .p(px(10.0))
                .rounded(px(radius::md()))
                .bg(Tone::Subtle.color())
                .child(row("pos.till.opening", Box::new(|s| s.opening_minor)))
                .child(row("pos.till.cash", Box::new(|s| s.cash_sales_minor)))
                .child(row(
                    "pos.till.other",
                    Box::new(|s| s.card_minor + s.transfer_minor + s.credit_minor),
                ))
                .child(row(
                    "pos.till.drawer",
                    Box::new(|s| {
                        s.opening_minor + s.cash_sales_minor + s.cash_in_minor + s.collections_minor
                            - s.cash_out_minor
                            - s.refunds_minor
                    }),
                )),
        );
    }
    let mut buttons = view().flex_row().gap(px(8.0));
    let btn = |id: &'static str, ic: Icon, key: &'static str, danger: bool, run: Rc<dyn Fn()>| {
        let mut b = view()
            .test_id(id)
            .flex_1()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .h(px(crate::ui::tokens::control::md()))
            .rounded(px(radius::md()))
            .border_1()
            .border_color(if danger {
                Tone::Danger.color()
            } else {
                Tone::Border.color()
            })
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Subtle.color()))
            .label(move || SharedString::from(st.tn(key)))
            .on_press(move || run());
        let tone = if danger { Tone::Danger } else { Tone::Ink };
        if with_icon {
            b = b.child(icon(ic, 16.0, move || tone));
        }
        if with_text {
            b = b.child(label_line(st.t(key), ty::SM, FontWeight::SEMIBOLD, tone));
        }
        b
    };
    if has("sales") {
        buttons = buttons.child(btn(
            "session-sales",
            Icon::Receipt,
            "pos.sales",
            false,
            Rc::new(move || t.sales_open.set(true)),
        ));
    }
    if has("close") {
        buttons = buttons.child(btn(
            "session-close",
            Icon::Lock,
            "pos.close_till",
            true,
            Rc::new(move || t.closing.set(true)),
        ));
    }
    v.child(buttons)
}

/// The till's chip in the page's head: which till, which shift, since
/// when and who; a press opens what may be done at the till.
pub(super) fn till_chip(st: AppState, t: Till, compact: bool) -> View {
    let trigger = view()
        .test_id("pos-till-chip")
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(crate::ui::tokens::control::md() + 6.0))
        .px(px(10.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::Border.color())
        .bg(Tone::Surface.color())
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Subtle.color()))
        .child(
            view()
                .w(px(8.0))
                .h(px(8.0))
                .rounded_full()
                .bg(Tone::Success.color()),
        )
        .child(icon(Icon::Wallet, 16.0, || Tone::Muted))
        // No reactive part in a menu's trigger: the words are there or not.
        .child(if compact {
            view()
        } else {
            view()
                .child(label_line(
                    move || till_line(st, t),
                    ty::SM,
                    FontWeight::BOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    move || since_line(st, t),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
        })
        .child(icon(Icon::ChevronDown, 14.0, || Tone::Muted));
    let acts = vec![
        crate::ui::menu::Act::Title(till_line(st, t)),
        crate::ui::menu::Act::with(Icon::Receipt, st.tn("pos.sales"), move || {
            t.sales_open.set(true)
        })
        .hint("F1"),
        crate::ui::menu::Act::with(Icon::History, st.tn("pos.key.held"), move || {
            t.recalling.set(true)
        })
        .hint("F3"),
        crate::ui::menu::Act::with(Icon::Undo, st.tn("pos.return"), move || {
            t.returning.set(true)
        })
        .hint("F12")
        .when(st.can("sales.return") || st.can("sales.sell")),
        crate::ui::menu::Act::with(Icon::PackagePlus, st.tn("pos.free"), move || {
            t.free_open.set(true)
        })
        .hint("F4"),
        crate::ui::menu::Act::Line,
        crate::ui::menu::Act::with(Icon::Keyboard, st.tn("pos.keys"), move || {
            t.keys_open.set(true)
        }),
        crate::ui::menu::Act::with(Icon::Lock, st.tn("pos.close_till"), move || {
            t.closing.set(true)
        })
        .danger(),
    ];
    crate::ui::menu::menu_on(trigger, acts)
}

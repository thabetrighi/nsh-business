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
        "misc" => Button {
            allowed: st.can("sales.sell"),
            ..b(
                Icon::PackagePlus,
                "pos.key.free",
                "F4",
                "free",
                Rc::new(move || t.free_open.set(true)),
            )
        },
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
        "close" => Button {
            allowed: st.can("cash.close_shift"),
            ..b(
                Icon::Lock,
                "pos.close_till",
                "",
                "close",
                Rc::new(move || t.closing.set(true)),
            )
        },
        "edit" => Button {
            allowed: st.can("settings.edit"),
            ..b(
                Icon::Pencil,
                "pos.layouts.edit_this",
                "",
                "edit",
                Rc::new(move || edit_layout(st, t)),
            )
        },
        "theme" => b(
            Icon::SunMoon,
            "pos.head.theme",
            "",
            "theme",
            Rc::new(move || st.toggle_appearance()),
        ),
        "fullscreen" => b(
            Icon::Maximize,
            "pos.fullscreen",
            "",
            "fullscreen",
            Rc::new(move || t.toggle_full()),
        ),
        "layouts" => b(
            Icon::LayoutTemplate,
            "pos.layouts",
            "",
            "layouts",
            Rc::new(move || t.board.layouts_open.set(true)),
        ),
        _ => return None,
    })
}

/// Edits the layout in use (a ready-made one as a copy of its own).
pub(super) fn edit_layout(st: AppState, t: Till) {
    let k = super::board::Board::kept(st);
    let l = k.current();
    let ready = k.chosen.is_empty() || k.chosen.starts_with(crate::screens::pos_layout::BUILTIN);
    let name = if ready {
        nb_i18n::tr_args(
            st.lang(),
            "templates.copy_of",
            &[("name", &st.tn(&format!("pos.layout.{}", l.name)))],
        )
    } else {
        l.name.clone()
    };
    t.board.edit(l, name);
}

/// The test id of a head button (the ones the tests know by name).
fn head_id(code: &str) -> String {
    match code {
        "misc" => "pos-free".into(),
        "held" => "pos-held".into(),
        "return" => "pos-return".into(),
        "sales" => "pos-sales".into(),
        "layouts" => "pos-customize".into(),
        "edit" => "pos-layout-edit".into(),
        "keys" => "pos-keys".into(),
        "fullscreen" => "pos-fullscreen".into(),
        "close" => "till-close".into(),
        "quick" => "pos-quick".into(),
        other => format!("pos-head-{other}"),
    }
}

/// The app's pages from the till (owner notes: in full screen, or with
/// the sidebar hidden): the top ones, then each section's, as the user
/// may open them.
pub(super) fn quick_acts(st: AppState, t: Till) -> Vec<crate::ui::menu::Act> {
    use crate::nav::{Route, SECTIONS, TOP};
    use crate::ui::menu::Act;
    let go = move |r: Route| {
        move || {
            if t.full.get_untracked() {
                t.toggle_full();
            }
            st.route.set(r)
        }
    };
    let mut acts = vec![Act::Title(st.tn("pos.head.quick"))];
    for &r in TOP.iter().filter(|r| **r != Route::Sell && st.shows(**r)) {
        acts.push(Act::with(r.icon(), st.tn(r.key()), go(r)));
    }
    for section in SECTIONS {
        let items: Vec<Act> = section
            .routes
            .iter()
            .filter(|r| st.shows(**r))
            .map(|&r| Act::with(r.icon(), st.tn(r.key()), go(r)))
            .collect();
        if !items.is_empty() {
            acts.push(Act::sub(section.icon, st.tn(section.key), items));
        }
    }
    acts.push(Act::Line);
    for r in [Route::Design, Route::Settings] {
        if st.shows(r) {
            acts.push(Act::with(r.icon(), st.tn(r.key()), go(r)));
        }
    }
    acts
}

/// The till's head beside its chip (owner notes): the buttons the layout
/// chooses, in its order, one look and one height; those the user may not
/// use do not show.
pub(super) fn head_buttons(st: AppState, t: Till, compact: bool) -> View {
    view().child(dynamic(move || {
        let l = t.board.shown(st);
        let labels = l.head_labels;
        let mut row = view().flex_row().flex_wrap().items_center().gap(px(6.0));
        for code in l.head_shown() {
            let code = code.as_str();
            // Words beside the icon: always (2), or for closing the till (0).
            let words = !compact && (labels == 2 || (labels == 0 && code == "close"));
            let id = head_id(code);
            // One height for all, the till's chip's.
            let side = crate::ui::tokens::control::lg();
            let make = |ic: Icon, key: &'static str, run: Rc<dyn Fn()>| -> View {
                if words {
                    button(
                        Variant::Secondary,
                        Size::Md,
                        Some(ic),
                        st.t(key),
                        move || run(),
                    )
                    .h(px(side))
                } else {
                    icon_button(move || ic, st.t(key), Variant::Secondary, move || run())
                        .w(px(side))
                        .h(px(side))
                }
            };
            let v = match code {
                "quick" => crate::ui::menu::menu_on(
                    make(Icon::Apps, "pos.head.quick", Rc::new(|| {})).test_id(&id),
                    quick_acts(st, t),
                ),
                "held" => {
                    let n = t.held.get().len();
                    if n == 0 {
                        continue;
                    }
                    let caption = move || {
                        tr_args(
                            st.lang(),
                            "pos.held.count",
                            &[("count", &st.fmt().integer(n as i64))],
                        )
                    };
                    if compact || labels == 1 {
                        view()
                            .relative()
                            .child(
                                icon_button(
                                    || Icon::Clock,
                                    caption,
                                    Variant::Secondary,
                                    move || t.recalling.set(true),
                                )
                                .w(px(side))
                                .h(px(side))
                                .test_id(&id),
                            )
                            .child(
                                view()
                                    .absolute()
                                    .top(px(-6.0))
                                    .right(px(-6.0))
                                    .child(count_badge(move || n)),
                            )
                    } else {
                        button(
                            Variant::Secondary,
                            Size::Md,
                            Some(Icon::Clock),
                            caption,
                            move || t.recalling.set(true),
                        )
                        .h(px(side))
                        .test_id(&id)
                    }
                }
                other => {
                    let Some(btn) = action(st, t, other, 0) else {
                        continue;
                    };
                    if !btn.allowed {
                        continue;
                    }
                    make(btn.icon, btn.key, btn.run.clone()).test_id(&id)
                }
            };
            row = row.child(v);
        }
        row.into_element()
    }))
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

/// A panel's colours: its background, its text, its quieter text.
#[derive(Clone, Copy)]
pub(super) struct Paint {
    pub bg: Tone,
    pub fg: Tone,
    pub muted: Tone,
}

/// The colours of `COLORS[color]`, or `own` for 0.
pub(super) fn paint(color: u8, own: Paint) -> Paint {
    let on = |bg| Paint {
        bg,
        fg: Tone::OnPrimary,
        muted: Tone::OnPrimary,
    };
    match color {
        1 => on(Tone::Primary),
        2 => Paint {
            bg: Tone::Ink,
            fg: Tone::Surface,
            muted: Tone::Faint,
        },
        3 => on(Tone::Success),
        4 => Paint {
            bg: Tone::WarningSoft,
            fg: Tone::Ink,
            muted: Tone::Body,
        },
        5 => on(Tone::Danger),
        6 => Paint {
            bg: Tone::PrimarySoft,
            fg: Tone::Ink,
            muted: Tone::Muted,
        },
        7 => Paint {
            bg: Tone::Surface,
            fg: Tone::Ink,
            muted: Tone::Muted,
        },
        _ => own,
    }
}

/// The usual surface of a panel.
pub(super) const PLAIN: Paint = Paint {
    bg: Tone::Surface,
    fg: Tone::Ink,
    muted: Tone::Muted,
};

/// Content along a row placed as the panel says.
fn justify(v: View, align: u8) -> View {
    match align {
        1 => v.justify_center(),
        2 => v.justify_end(),
        _ => v,
    }
}

/// A button of a panel, as its labels, style (0 tiles, 1 a row of small
/// buttons, 2 a list) and colour say.
fn tile_button(st: AppState, t: Till, b: Button, p: &Panel, code: &str) -> View {
    let (with_icon, with_text) = shows(p.labels);
    let style = p.style;
    let colors = paint(p.color, PLAIN);
    let run = b.run.clone();
    let key = b.key;
    let hint = b.hint;
    let mut v = view()
        .test_id(&b.id)
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::Border.color())
        .bg(colors.bg.color())
        .label(move || SharedString::from(st.tn(key)))
        .opacity(if b.allowed { 1.0 } else { 0.45 });
    v = match style {
        // A row of small buttons.
        1 => v
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(px(crate::ui::tokens::control::md() + 4.0))
            .px(px(if with_text { 12.0 } else { 0.0 }))
            .min_w(px(crate::ui::tokens::control::md() + 4.0))
            .justify_center(),
        // A list, one per line.
        2 => v
            .w_full()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .h(px(crate::ui::tokens::control::md() + 8.0))
            .px(px(12.0)),
        _ => v
            .flex_grow(1.0)
            .basis(px(if with_text { 140.0 } else { 56.0 }))
            .min_h(px(crate::ui::tokens::control::lg() + 18.0))
            .p(px(8.0))
            .gap(px(2.0))
            .items_center()
            .justify_center(),
    };
    if b.allowed {
        v = v
            .cursor_pointer()
            .hover(|s| s.opacity(0.85))
            .on_press(move || run());
    }
    let fg = colors.fg;
    let mut top = view().flex_row().items_center().gap(px(6.0));
    if style == 2 {
        top = top.flex_1();
    }
    if with_icon {
        top = top.child(icon(
            b.icon,
            if with_text || style == 1 { 18.0 } else { 22.0 },
            move || fg,
        ));
    }
    if with_text {
        top = top.child(label_line(st.t(key), ty::SM, FontWeight::SEMIBOLD, fg));
    }
    if code == "held" {
        top = top.child(count_badge(move || t.held.with(Vec::len)));
    }
    v = v.child(top);
    if with_text && !hint.is_empty() && style != 1 {
        let muted = colors.muted;
        v = v.child(label_line(
            move || hint.to_owned(),
            ty::XS,
            FontWeight::NORMAL,
            muted,
        ));
    }
    v
}

/// The cart's actions as buttons: those the panel shows, in order.
pub(super) fn actions_panel(st: AppState, t: Till, p: &Panel, n: usize) -> View {
    let mut v = view()
        .p(px(if p.style == 1 { 8.0 } else { 10.0 }))
        .gap(px(8.0));
    v = if p.style == 2 {
        v
    } else {
        justify(v.flex_row().flex_wrap(), p.align)
    };
    for code in p.shown_items() {
        if let Some(b) = action(st, t, &code, n) {
            v = v.child(tile_button(st, t, b, p, &code));
        }
    }
    v
}

/// The total, large: the word, the number of lines, the customer, as the
/// panel says; its colour and alignment.
pub(super) fn big_total(st: AppState, t: Till, p: &Panel) -> View {
    let items = p.shown_items();
    let has = |c: &str| items.iter().any(|i| i == c);
    let colors = paint(
        p.color,
        Paint {
            bg: Tone::Ink,
            fg: Tone::Surface,
            muted: Tone::Faint,
        },
    );
    let total = computed(move || t.totals(currency(st)).map_or(0, |x| x.total.minor()));
    let (fg, muted) = (colors.fg, colors.muted);
    let mut words = view().gap(px(2.0));
    if has("label") {
        words = words.child(label_line(st.t("pos.total"), ty::LG, FontWeight::BOLD, fg));
    }
    if has("count") {
        words = words.child(label_line(
            move || {
                let n = t.cart.with(Vec::len) as i64;
                tr_args(
                    st.lang(),
                    "pos.cart.count",
                    &[("count", &st.fmt().integer(n))],
                )
            },
            ty::SM,
            FontWeight::MEDIUM,
            muted,
        ));
    }
    if has("customer") {
        words = words.child(label_line(
            move || {
                t.customer
                    .get()
                    .map(|c| c.name)
                    .unwrap_or_else(|| st.tn("pos.counter_customer"))
            },
            ty::SM,
            FontWeight::MEDIUM,
            muted,
        ));
    }
    let number = figure(
        move || money(st, total.get()),
        38.0,
        FontWeight::BOLD,
        move || fg,
    );
    let v = view()
        .test_id("pos-total")
        .px(px(18.0))
        .py(px(14.0))
        .gap(px(12.0))
        .rounded(px(radius::lg()))
        .bg(colors.bg.color());
    match p.align {
        // Centred: the words on one line beside the number.
        1 => v
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap(px(24.0))
            .child(words.flex_row().items_center().gap(px(14.0)))
            .child(number),
        // The number first.
        2 => v
            .flex_row()
            .items_center()
            .child(number)
            .child(view().flex_1())
            .child(words.items_end()),
        _ => v
            .flex_row()
            .items_center()
            .child(words.flex_1())
            .child(number),
    }
}

/// The carts on hold as badges: each its label, lines and total; a press
/// takes it back (this cart is held first), its × drops it. Hold this
/// cart at the end.
pub(super) fn held_bar(st: AppState, t: Till, p: &Panel) -> View {
    let items = p.shown_items();
    let has = |c: &str| items.iter().any(|i| i == c);
    let (title, lines, total, hold_btn) = (has("title"), has("lines"), has("total"), has("hold"));
    let colors = paint(p.color, PLAIN);
    let (with_icon, with_text) = shows(p.labels);
    let align = p.align;
    let list = dynamic(move || {
        let held = t.held.get();
        let mut row = view().flex_row().flex_wrap().items_center().gap(px(8.0));
        row = justify(row, align);
        if held.is_empty() {
            row = row.child(label_line(
                st.t("pos.held.none"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        for (i, h) in held.into_iter().enumerate() {
            let id = h.id.clone();
            let drop_id = h.id.clone();
            let name = h.label.clone();
            let n = i64::from(h.lines);
            let mut badge = view()
                .test_id(&format!("held-badge-{i}"))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .h(px(crate::ui::tokens::control::md() + 4.0))
                .ps(px(12.0))
                .pe(px(4.0))
                .rounded_full()
                .bg(Tone::PrimarySoft.color())
                .border_1()
                .border_color(Tone::Primary.color())
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .label(st.t("pos.held.take"))
                .on_press(move || recall(st, t, id.clone(), || {}))
                .child(icon(Icon::Clock, 15.0, || Tone::Primary))
                .child(label_line(
                    move || name.clone(),
                    ty::SM,
                    FontWeight::BOLD,
                    Tone::Primary,
                ));
            if lines {
                badge = badge.child(label_line(
                    move || {
                        tr_args(
                            st.lang(),
                            "pos.cart.count",
                            &[("count", &st.fmt().integer(n))],
                        )
                    },
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                ));
            }
            if total {
                badge = badge.child(figure(
                    move || money(st, h.total_minor),
                    ty::SM,
                    FontWeight::BOLD,
                    || Tone::Ink,
                ));
            }
            badge = badge.child(
                view()
                    .test_id(&format!("held-drop-{i}"))
                    .w(px(24.0))
                    .h(px(24.0))
                    .rounded_full()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .label(st.t("pos.held.drop"))
                    .on_press(move || drop_held(st, t, drop_id.clone()))
                    .child(icon(Icon::Close, 13.0, || Tone::Muted)),
            );
            row = row.child(badge);
        }
        row.into_element()
    });
    let mut bar = view()
        .test_id("pos-held-bar")
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(radius::lg()))
        .bg(colors.bg.color())
        .border_1()
        .border_color(Tone::Border.color());
    if title {
        let fg = colors.fg;
        bar = bar.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(icon(Icon::History, 16.0, move || fg))
                .child(label_line(
                    st.t("pos.key.held"),
                    ty::SM,
                    FontWeight::BOLD,
                    fg,
                )),
        );
    }
    bar = bar.child(view().flex_1().min_w(px(0.0)).child(list));
    if hold_btn {
        let mut b = view()
            .test_id("held-bar-hold")
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(px(crate::ui::tokens::control::md() + 4.0))
            .px(px(12.0))
            .rounded(px(radius::md()))
            .bg(Tone::Primary.color())
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .label(st.t("pos.key.hold"))
            .on_press(move || hold(st, t));
        if with_icon {
            b = b.child(icon(Icon::Pause, 16.0, || Tone::OnPrimary));
        }
        if with_text {
            b = b.child(label_line(
                st.t("pos.key.hold"),
                ty::SM,
                FontWeight::BOLD,
                Tone::OnPrimary,
            ));
        }
        bar = bar.child(b);
    }
    bar
}

/// Drops a cart on hold (taken and let go).
fn drop_held(st: AppState, t: Till, id: String) {
    let Some(session) = st.session.get_untracked() else {
        return;
    };
    let meta = session.meta("cart.drop");
    let db = st.db();
    let task = spawn(async move {
        match db.call(move |s| take_held(s, meta, &id)).await {
            Ok(_) => {
                t.held_rev.update(|n| *n += 1);
                show_toast(st, st.tn("pos.held.dropped"));
            }
            Err(e) => show_toast(st, first_error(st, &e)),
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

/// The last sale made here: its number, total, the change given, its
/// receipt again.
pub(super) fn last_sale(st: AppState, t: Till, p: &Panel) -> View {
    let items = p.shown_items();
    let has = |c: &str| items.iter().any(|i| i == c);
    let (number, total, change, print) = (has("number"), has("total"), has("change"), has("print"));
    let colors = paint(p.color, PLAIN);
    let (fg, muted) = (colors.fg, colors.muted);
    let (with_icon, with_text) = shows(p.labels);
    let body = dynamic(move || {
        let Some(r) = t.last.get() else {
            return label_line(st.t("pos.last.none"), ty::SM, FontWeight::NORMAL, muted)
                .into_element();
        };
        let mut v = view().gap(px(6.0));
        if number {
            let n = r.number.clone();
            v = v.child(label_line(move || n.clone(), ty::SM, FontWeight::BOLD, fg));
        }
        let line = |key: &'static str, m: i64, big: bool| {
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(label_line(
                    st.t(key),
                    ty::XS,
                    FontWeight::MEDIUM,
                    muted,
                )))
                .child(figure(
                    move || money(st, m),
                    if big { ty::LG } else { ty::SM },
                    FontWeight::BOLD,
                    move || fg,
                ))
        };
        if total {
            v = v.child(line("pos.total", r.total_minor, false));
        }
        if change {
            v = v.child(line("pos.last.change", r.change_minor, true));
        }
        if print {
            let n = r.number.clone();
            let mut b = view()
                .test_id("last-print")
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .h(px(crate::ui::tokens::control::md()))
                .rounded(px(radius::md()))
                .border_1()
                .border_color(Tone::Border.color())
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .label(st.t("pos.last.print"))
                .on_press(move || {
                    t.printing.set(Some(n.clone()));
                    t.printing_open.set(true);
                });
            if with_icon {
                b = b.child(icon(Icon::Printer, 16.0, move || fg));
            }
            if with_text {
                b = b.child(label_line(
                    st.t("pos.last.print"),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    fg,
                ));
            }
            v = v.child(b);
        }
        v.into_element()
    });
    view()
        .test_id("pos-last-sale")
        .gap(px(8.0))
        .p(px(14.0))
        .rounded(px(radius::lg()))
        .bg(colors.bg.color())
        .border_1()
        .border_color(Tone::Border.color())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(icon(Icon::Receipt, 16.0, move || muted))
                .child(label_line(
                    st.t("pos.last.title"),
                    ty::XS,
                    FontWeight::BOLD,
                    muted,
                )),
        )
        .child(body)
}

/// The time and the date, the till and who works it.
pub(super) fn clock_panel(st: AppState, t: Till, p: &Panel) -> View {
    let items = p.shown_items();
    let has = |c: &str| items.iter().any(|i| i == c);
    let colors = paint(p.color, PLAIN);
    let (fg, muted) = (colors.fg, colors.muted);
    // The minute, kept current.
    let now = signal(crate::state::now_ms());
    let tick = spawn(async move {
        loop {
            timer(std::time::Duration::from_secs(20)).await;
            now.set(crate::state::now_ms());
        }
    });
    drop(tick); // ends with the panel (its owner)
    let mut row = view()
        .test_id("pos-clock")
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(18.0))
        .px(px(16.0))
        .py(px(10.0))
        .rounded(px(radius::lg()))
        .bg(colors.bg.color())
        .border_1()
        .border_color(Tone::Border.color());
    row = justify(row, p.align);
    if has("time") {
        row = row.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(icon(Icon::Clock, 18.0, move || muted))
                .child(figure(
                    move || clock_of(st, now.get()),
                    ty::XL,
                    FontWeight::BOLD,
                    move || fg,
                )),
        );
    }
    if has("date") {
        row = row.child(label_line(
            move || {
                // The store's day (UTC+1, Algeria, as the clock).
                let days = (now.get() / 60_000 + 60).div_euclid(24 * 60);
                let d = nb_domain::installment::add_days(
                    nb_domain::fiscal::Date {
                        y: 1970,
                        m: 1,
                        d: 1,
                    },
                    days,
                );
                st.fmt().date(d.y, d.m, d.d)
            },
            ty::MD,
            FontWeight::SEMIBOLD,
            fg,
        ));
    }
    if has("till") {
        row = row.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(icon(Icon::Wallet, 16.0, move || muted))
                .child(label_line(
                    move || till_line(st, t),
                    ty::SM,
                    FontWeight::MEDIUM,
                    fg,
                )),
        );
    }
    if has("cashier") {
        row = row.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(icon(Icon::User, 16.0, move || muted))
                .child(label_line(
                    crate::shell::user_name(st),
                    ty::SM,
                    FontWeight::MEDIUM,
                    fg,
                )),
        );
    }
    row
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
    let line = till_line_only(st, t);
    match t.van.get() {
        Some((van, _)) => tr_args(st.lang(), "pos.van_line", &[("till", &line), ("van", &van)]),
        None => line,
    }
}

fn till_line_only(st: AppState, t: Till) -> String {
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
        .h(px(crate::ui::tokens::control::lg()))
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

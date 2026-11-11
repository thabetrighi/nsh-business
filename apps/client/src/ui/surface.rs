//! Page structure: page headers, cards with headers, KPI tiles, empty
//! states, settings rows. Every screen is built from these, so pages share
//! one rhythm: 32 pt page padding on desktop, 24 pt between cards.

use nsh_core::element::View;
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::kit::{badge, figure, label, label_line};
use super::tokens::{Status, Tone, radius, sp, ty};

/// Page padding.
pub fn page_pad(compact: bool) -> f32 {
    if compact { 16.0 } else { 32.0 }
}

/// A page header with its icon and the section it is in (a breadcrumb).
pub fn page_header_full(
    which: Option<Icon>,
    eyebrow: Option<Box<dyn Fn() -> String>>,
    title: impl Fn() -> String + 'static,
    subtitle: Option<Box<dyn Fn() -> String>>,
    actions: View,
    compact: bool,
) -> View {
    if compact {
        // Phones: the line under the title on its own, the actions under it.
        let mut col = view().w_full().gap(px(10.0));
        if let Some(s) = subtitle {
            col = col.child(label(s, ty::SM, FontWeight::NORMAL, Tone::Muted));
        }
        return col.child(actions.flex_row().flex_wrap().items_center().gap(px(8.0)));
    }
    // Tajawal sets lines tight: the header's lines keep some air.
    let mut heading = view().flex_1().min_w(px(0.0)).gap(px(5.0));
    if let Some(e) = eyebrow {
        heading = heading.child(label_line(e, ty::XS, FontWeight::SEMIBOLD, Tone::Primary));
    }
    heading = heading.child(label_line(title, ty::H2, FontWeight::BOLD, Tone::Ink));
    if let Some(s) = subtitle {
        heading = heading.child(label(s, ty::SM, FontWeight::NORMAL, Tone::Muted));
    }
    let mut row = view().flex_row().items_center().gap(px(14.0));
    if let Some(i) = which {
        row = row.child(
            view()
                .w(px(44.0))
                .h(px(44.0))
                .rounded(px(radius::lg()))
                .items_center()
                .justify_center()
                .bg(Tone::PrimarySoft.color())
                .border_1()
                .border_color(Tone::Border.color())
                .child(icon(i, 22.0, || Tone::Primary)),
        );
    }
    row.child(heading)
        .child(actions.flex_row().items_center().gap(px(8.0)))
}

/// A card: white surface, hairline border, soft corners.
pub fn card() -> View {
    view()
        .bg(Tone::Surface.color())
        .border_1()
        .border_color(Tone::Border.color())
        .rounded(px(radius::lg()))
        .shadow_xs()
        .overflow_hidden()
}

/// The top of a card: a title, an optional hint, actions at the end.
pub fn card_header(
    title: impl Fn() -> String + 'static,
    hint: Option<Box<dyn Fn() -> String>>,
    actions: View,
) -> View {
    card_header_icon(None, title, hint, actions)
}

/// A card header led by an icon in a soft tile.
pub fn card_header_icon(
    which: Option<Icon>,
    title: impl Fn() -> String + 'static,
    hint: Option<Box<dyn Fn() -> String>>,
    actions: View,
) -> View {
    let mut t = view()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(2.0))
        .child(label_line(title, ty::LG, FontWeight::SEMIBOLD, Tone::Ink));
    if let Some(h) = hint {
        t = t.child(label(h, ty::SM, FontWeight::NORMAL, Tone::Muted));
    }
    let mut row = view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(20.0))
        .pt(px(sp(18.0)))
        .pb(px(sp(14.0)));
    if let Some(i) = which {
        row = row.child(
            view()
                .w(px(32.0))
                .h(px(32.0))
                .rounded(px(radius::md()))
                .items_center()
                .justify_center()
                .bg(Tone::Subtle.color())
                .border_1()
                .border_color(Tone::Border.color())
                .child(icon(i, 16.0, || Tone::Body)),
        );
    }
    row.child(t)
        .child(actions.flex_row().items_center().gap(px(6.0)))
}

/// The body of a card, padded.
pub fn card_body() -> View {
    view().px(px(20.0)).pb(px(sp(20.0)))
}

/// A change against a previous period, in basis points: ↑ 12,4 % in green,
/// ↓ in red, and neutral while it rounds to 0,0 %.
pub fn delta(bps: i64, text_of: impl Fn() -> String + 'static) -> View {
    let (status, which) = match bps {
        b if b >= 5 => (Status::Success, Some(Icon::TrendUp)),
        b if b <= -5 => (Status::Danger, Some(Icon::TrendDown)),
        _ => (Status::Neutral, None),
    };
    badge(status, which, text_of)
}

/// A KPI tile: name, figure, change, and a chart or hint under it.
pub fn kpi(
    name: impl Fn() -> String + 'static,
    value: impl Fn() -> String + 'static,
    change: Option<View>,
    footer: View,
    compact: bool,
) -> View {
    let mut top =
        view()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(view().flex_1().child(label_line(
                name,
                ty::SM,
                FontWeight::MEDIUM,
                Tone::Muted,
            )));
    if let Some(c) = change {
        top = top.child(c);
    }
    card()
        .flex_1()
        .min_w(px(if compact { 150.0 } else { 200.0 }))
        .p(px(sp(if compact { 14.0 } else { 20.0 })))
        .gap(px(sp(10.0)))
        .child(top)
        .child(figure(
            value,
            if compact { ty::LG } else { ty::H1 },
            FontWeight::BOLD,
            || Tone::Ink,
        ))
        .child(footer)
}

/// What a list shows when it has nothing: an icon, a sentence, an action.
pub fn empty_state(
    which: Icon,
    title: impl Fn() -> String + 'static,
    body: impl Fn() -> String + 'static,
    action: Option<View>,
) -> View {
    let mut v = view()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .p(px(40.0))
        .child(
            view()
                .w(px(48.0))
                .h(px(48.0))
                .rounded(px(radius::lg()))
                .items_center()
                .justify_center()
                .bg(Tone::Subtle.color())
                .border_1()
                .border_color(Tone::Border.color())
                .child(icon(which, 22.0, || Tone::Muted)),
        )
        .child(label(title, ty::LG, FontWeight::SEMIBOLD, Tone::Ink).text_center())
        .child(
            label(body, ty::SM, FontWeight::NORMAL, Tone::Muted)
                .text_center()
                .max_w(px(380.0)),
        );
    if let Some(a) = action {
        v = v.child(view().pt(px(6.0)).child(a));
    }
    v
}

/// A settings row: name and help at the start, the control at the end
/// (stacked on phones).
pub fn setting_row(
    name: impl Fn() -> String + 'static,
    help: impl Fn() -> String + 'static,
    control: View,
    compact: bool,
) -> View {
    let text_part = view()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(2.0))
        .child(label(name, ty::MD, FontWeight::SEMIBOLD, Tone::Ink))
        .child(label(help, ty::SM, FontWeight::NORMAL, Tone::Muted));
    let row = view().gap(px(12.0)).py(px(sp(16.0))).px(px(20.0));
    if compact {
        row.child(text_part).child(control)
    } else {
        row.flex_row()
            .items_center()
            .child(text_part)
            .child(view().w(px(300.0)).items_end().child(control))
    }
}

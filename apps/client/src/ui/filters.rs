//! The figures above a list that also filter it (owner notes: one look on
//! every page): an icon in its colour, a name, a count; the one chosen is
//! outlined and checked. A pill on phones. A figure that does not filter
//! (a value) has the same look without the press.

use nsh_core::element::View;
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::kit::{figure, label_line};
use super::tokens::{Status, Tone, radius, sp, ty};

/// A figure tile: `active` outlines it; `on_press` (if any) makes it a
/// filter.
#[allow(clippy::too_many_arguments)] // a tile's parts, named at each call
pub fn stat_tile(
    id: &str,
    name: impl Fn() -> String + 'static,
    which: Icon,
    status: Status,
    value: impl Fn() -> String + 'static,
    active: impl Fn() -> bool + Copy + 'static,
    on_press: Option<Box<dyn Fn()>>,
    compact: bool,
) -> View {
    let mut base = view().test_id(id);
    if let Some(f) = on_press {
        base = base.cursor_pointer().on_press(f);
    }
    let border = move || {
        if active() {
            Tone::Primary.color()
        } else {
            Tone::Border.color()
        }
    };
    if compact {
        return base
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(px(34.0))
            .px(px(12.0))
            .rounded_full()
            .border_1()
            .border_color(border)
            .bg(move || {
                if active() {
                    Tone::PrimarySoft.color()
                } else {
                    Tone::Surface.color()
                }
            })
            .child(icon(which, 14.0, move || status.fg()))
            .child(label_line(name, ty::SM, FontWeight::MEDIUM, Tone::Body))
            .child(figure(value, ty::XS, FontWeight::SEMIBOLD, || Tone::Muted));
    }
    base.flex_1()
        .min_w(px(160.0))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .p(px(sp(14.0)))
        .rounded(px(radius::lg()))
        .bg(Tone::Surface.color())
        .border_1()
        .border_color(border)
        .shadow_xs()
        .hover(|s| s.border_color(Tone::BorderStrong.color()))
        .child(
            view()
                .w(px(36.0))
                .h(px(36.0))
                .rounded(px(radius::md()))
                .items_center()
                .justify_center()
                .bg(status.bg().color())
                .child(icon(which, 18.0, move || status.fg())),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(label_line(name, ty::SM, FontWeight::MEDIUM, Tone::Muted))
                .child(figure(value, ty::XL, FontWeight::BOLD, || Tone::Ink)),
        )
        .child(when(active, || {
            icon(Icon::CheckCircle, 18.0, || Tone::Primary).into_element()
        }))
}

/// A tile holding an amount of money: wide enough for millions.
pub fn money_tile(tile: View, compact: bool) -> View {
    if compact { tile } else { tile.min_w(px(250.0)) }
}

/// Tiles in a row (scrolling sideways on phones).
pub fn stat_row(tiles: Vec<View>, compact: bool) -> View {
    let mut row = view()
        .flex_row()
        .flex_wrap()
        .gap(px(if compact { 6.0 } else { 16.0 }));
    if compact {
        row = view().flex_row().gap(px(6.0));
    }
    for t in tiles {
        row = row.child(t);
    }
    if compact {
        view().child(scroll().horizontal().child(row))
    } else {
        row
    }
}

/// A filter that is on or off, beside the search (favourites, held
/// stock…): outlined in the accent while on.
pub fn toggle_filter(
    id: &str,
    which: Icon,
    name: impl Fn() -> String + 'static,
    on: Signal<bool>,
) -> View {
    view()
        .test_id(id)
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .h(px(super::tokens::control::md()))
        .px(px(12.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(move || {
            if on.get() {
                Tone::Primary.color()
            } else {
                Tone::BorderStrong.color()
            }
        })
        .bg(move || {
            if on.get() {
                Tone::PrimarySoft.color()
            } else {
                Tone::Surface.color()
            }
        })
        .cursor_pointer()
        .on_press(move || on.update(|v| *v = !*v))
        .child(icon(which, 16.0, move || {
            if on.get() { Tone::Primary } else { Tone::Muted }
        }))
        .child(label_line(name, ty::SM, FontWeight::MEDIUM, Tone::Body))
}

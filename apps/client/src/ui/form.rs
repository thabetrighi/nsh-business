//! Form controls: a labelled text field with help or error, a search field,
//! and choice chips. Errors say what is wrong and how to fix it.

use nsh_core::element::{KeyboardType, View, input};
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::kit::label;
use super::tokens::{Tone, control, fs, radius, ty};

fn input_box() -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(control::md() + 2.0))
        .px(px(10.0))
        .rounded(px(radius::md()))
        .bg(Tone::Surface.color())
        .border_1()
        .shadow_xs()
}

/// A text field with a label above and an error below (empty error: none).
pub fn field(
    caption: impl Fn() -> String + 'static,
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    error: impl Fn() -> String + Clone + 'static,
    keyboard: KeyboardType,
    test_id: &'static str,
) -> View {
    let err_for_border = error.clone();
    let err_for_text = error.clone();
    let err_shown = error.clone();
    view()
        .gap(px(6.0))
        .child(label(caption, ty::SM, FontWeight::SEMIBOLD, Tone::Ink))
        .child(
            input_box()
                .border_color(move || {
                    if err_for_border().is_empty() {
                        Tone::BorderStrong.color()
                    } else {
                        Tone::Danger.color()
                    }
                })
                .child(
                    input()
                        .test_id(test_id)
                        .flex_1()
                        .value(move || value.get())
                        .on_change(move |t: &str| value.set(t.to_owned()))
                        .placeholder(placeholder)
                        .keyboard_type(keyboard)
                        .text_size(px(fs(ty::MD)))
                        .text_color(Tone::Ink.color()),
                ),
        )
        .child(when(
            move || !err_shown().is_empty(),
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(icon(Icon::Info, 14.0, || Tone::Danger))
                    .child(label(
                        err_for_text.clone(),
                        ty::XS,
                        FontWeight::MEDIUM,
                        Tone::Danger,
                    ))
                    .into_element()
            },
        ))
}

/// A search field with a magnifier, for filtering a list in place.
pub fn search_field(
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    test_id: &'static str,
) -> View {
    input_box()
        .border_color(Tone::BorderStrong.color())
        .child(icon(Icon::Search, 16.0, || Tone::Muted))
        .child(
            input()
                .test_id(test_id)
                .flex_1()
                .placeholder(placeholder)
                .value(move || value.get())
                .on_change(move |t: &str| value.set(t.to_owned()))
                .text_size(px(fs(ty::SM)))
                .text_color(Tone::Ink.color()),
        )
}

/// One-of-many choice as chips (better than a dropdown on touch).
pub fn chips(
    caption: impl Fn() -> String + 'static,
    options: Vec<(usize, Box<dyn Fn() -> String>)>,
    selected: Signal<usize>,
) -> View {
    let mut row = view().flex_row().flex_wrap().gap(px(8.0));
    for (id, text_of) in options {
        let active = computed(move || selected.get() == id);
        row = row.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(control::sm() + 4.0))
                .px(px(12.0))
                .rounded(px(radius::md()))
                .cursor_pointer()
                .border_1()
                .border_color(move || {
                    if active.get() {
                        Tone::Primary.color()
                    } else {
                        Tone::BorderStrong.color()
                    }
                })
                .bg(move || {
                    if active.get() {
                        Tone::PrimarySoft.color()
                    } else {
                        Tone::Surface.color()
                    }
                })
                .on_press(move || selected.set(id))
                .child(
                    text(text_of)
                        .text_size(px(fs(ty::SM)))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if active.get() {
                                Tone::Primary.color()
                            } else {
                                Tone::Body.color()
                            }
                        }),
                ),
        );
    }
    view()
        .gap(px(8.0))
        .child(label(caption, ty::SM, FontWeight::SEMIBOLD, Tone::Ink))
        .child(row)
}

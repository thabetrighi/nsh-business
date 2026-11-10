//! Form controls: a labelled text field with its error, and touch-sized
//! choice chips. Errors say what is wrong and how to fix it (DESIGN.md §7).

use nsh_core::element::{KeyboardType, input};
use nsh_core::prelude::*;

use super::kit::{label, size};
use super::palette::Tone;

/// A text field with a label above and an error below (empty error: none).
pub fn field(
    caption: impl Fn() -> String + 'static,
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    error: impl Fn() -> String + Clone + 'static,
    keyboard: KeyboardType,
    test_id: &'static str,
) -> impl IntoElement {
    let err_for_border = error.clone();
    let err_for_text = error.clone();
    view()
        .gap(px(4.0))
        .child(label(caption, size::BODY, FontWeight::MEDIUM, Tone::Ink))
        .child(
            input()
                .test_id(test_id)
                .value(move || value.get())
                .on_change(move |t: &str| value.set(t.to_owned()))
                .placeholder(placeholder)
                .keyboard_type(keyboard)
                .h(px(44.0))
                .px_3()
                .rounded(px(8.0))
                .bg(Tone::Surface.color())
                .border_1()
                .border_color(move || {
                    if err_for_border().is_empty() {
                        Tone::Line.color()
                    } else {
                        Tone::Owed.color()
                    }
                })
                .text_size(px(size::LEAD))
                .text_color(Tone::Ink.color()),
        )
        .child(label(
            err_for_text,
            size::CAPTION,
            FontWeight::NORMAL,
            Tone::Owed,
        ))
}

/// One-of-many choice as large chips (better than a dropdown on touch).
pub fn chips(
    caption: impl Fn() -> String + 'static,
    options: Vec<(usize, Box<dyn Fn() -> String>)>,
    selected: Signal<usize>,
) -> impl IntoElement {
    let mut row = view().flex_row().flex_wrap().gap(px(8.0));
    for (id, text_of) in options {
        let active = computed(move || selected.get() == id);
        row = row.child(
            view()
                .h(px(36.0))
                .px_3()
                .justify_center()
                .rounded(px(18.0))
                .border_1()
                .border_color(move || {
                    if active.get() {
                        Tone::Primary.color()
                    } else {
                        Tone::Line.color()
                    }
                })
                .bg(move || {
                    if active.get() {
                        Tone::Accent.color()
                    } else {
                        Tone::Surface.color()
                    }
                })
                .on_press(move || selected.set(id))
                .child(
                    text(text_of)
                        .text_size(px(size::BODY))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if active.get() {
                                Tone::Primary.color()
                            } else {
                                Tone::Ink.color()
                            }
                        }),
                ),
        );
    }
    view()
        .gap(px(8.0))
        .child(label(caption, size::BODY, FontWeight::MEDIUM, Tone::Ink))
        .child(row)
}

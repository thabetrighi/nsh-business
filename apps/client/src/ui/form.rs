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
    field_inner(caption, value, placeholder, error, keyboard, test_id, false)
}

/// A [`field`] whose text is hidden (passwords).
pub fn secret_field(
    caption: impl Fn() -> String + 'static,
    value: Signal<String>,
    error: impl Fn() -> String + Clone + 'static,
    test_id: &'static str,
) -> View {
    field_inner(
        caption,
        value,
        String::new,
        error,
        KeyboardType::Default,
        test_id,
        true,
    )
}

fn field_inner(
    caption: impl Fn() -> String + 'static,
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    error: impl Fn() -> String + Clone + 'static,
    keyboard: KeyboardType,
    test_id: &'static str,
    masked: bool,
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
                        .masked(masked)
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

/// A bare text input (no label), for rows of a table-like editor. Its test
/// id may be built at run time (`price-2`).
pub fn row_input(
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    keyboard: KeyboardType,
    test_id: String,
    invalid: impl Fn() -> bool + 'static,
) -> View {
    input_box()
        .border_color(move || {
            if invalid() {
                Tone::Danger.color()
            } else {
                Tone::BorderStrong.color()
            }
        })
        .child(
            input()
                .test_id(&test_id)
                .flex_1()
                .value(move || value.get())
                .on_change(move |t: &str| value.set(t.to_owned()))
                .placeholder(placeholder)
                .keyboard_type(keyboard)
                .text_size(px(fs(ty::SM)))
                .text_color(Tone::Ink.color()),
        )
}

/// A labelled multi-line text field.
pub fn text_area(
    caption: impl Fn() -> String + 'static,
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    height: f32,
    test_id: &'static str,
) -> View {
    view()
        .gap(px(6.0))
        .child(label(caption, ty::SM, FontWeight::SEMIBOLD, Tone::Ink))
        .child(
            view()
                .h(px(height))
                .p(px(10.0))
                .rounded(px(radius::md()))
                .bg(Tone::Surface.color())
                .border_1()
                .border_color(Tone::BorderStrong.color())
                .child(
                    input()
                        .test_id(test_id)
                        .flex_1()
                        .multiline(true)
                        .value(move || value.get())
                        .on_change(move |t: &str| value.set(t.to_owned()))
                        .placeholder(placeholder)
                        .text_size(px(fs(ty::SM)))
                        .text_color(Tone::Ink.color()),
                ),
        )
}

/// A caption above a control (dropdowns, chips), with an optional help line.
pub fn labelled(
    caption: impl Fn() -> String + 'static,
    control: View,
    help: Option<Box<dyn Fn() -> String>>,
) -> View {
    let mut v = view()
        .gap(px(6.0))
        .child(label(caption, ty::SM, FontWeight::SEMIBOLD, Tone::Ink))
        .child(control);
    if let Some(h) = help {
        v = v.child(label(h, ty::XS, FontWeight::NORMAL, Tone::Muted));
    }
    v
}

/// A group of a form's fields under a heading with its icon (identity,
/// contact, credit…): long forms read as a few short ones.
pub fn section(
    icon_of: super::icon::Icon,
    title: impl Fn() -> String + 'static,
    body: View,
) -> View {
    view()
        .gap(px(12.0))
        .p(px(16.0))
        .rounded(px(radius::lg()))
        .border_1()
        .border_color(Tone::Border.color())
        .bg(Tone::Surface.color())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    view()
                        .w(px(26.0))
                        .h(px(26.0))
                        .rounded(px(radius::md()))
                        .items_center()
                        .justify_center()
                        .bg(Tone::PrimarySoft.color())
                        .child(super::icon::icon(icon_of, 14.0, || Tone::Primary)),
                )
                .child(label(title, ty::SM, FontWeight::BOLD, Tone::Ink)),
        )
        .child(body.gap(px(12.0)))
}

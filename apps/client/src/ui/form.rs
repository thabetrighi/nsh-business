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
    field_inner(
        caption,
        value,
        placeholder,
        error,
        keyboard,
        test_id,
        false,
        false,
        false,
    )
}

/// A phone number: digits and spaces stay left to right in an Arabic
/// interface (else `0550 12 34 56` shows as `56 34 12 0550`).
pub fn phone_field(
    caption: impl Fn() -> String + 'static,
    value: Signal<String>,
    test_id: &'static str,
) -> View {
    field_inner(
        caption,
        value,
        || "0550 12 34 56".to_owned(),
        String::new,
        KeyboardType::Phone,
        test_id,
        false,
        false,
        true,
    )
}

/// A [`field`] that takes the keyboard when it shows (what is typed first).
pub fn focused_field(
    caption: impl Fn() -> String + 'static,
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    error: impl Fn() -> String + Clone + 'static,
    keyboard: KeyboardType,
    test_id: &'static str,
) -> View {
    field_inner(
        caption,
        value,
        placeholder,
        error,
        keyboard,
        test_id,
        false,
        true,
        false,
    )
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
        false,
        false,
    )
}

/// A password field with an eye that shows or hides what is typed, as the
/// well-known apps do. `on_submit`: the keyboard's return (sign in).
pub fn password_field(
    caption: impl Fn() -> String + 'static,
    value: Signal<String>,
    error: impl Fn() -> String + Clone + 'static,
    keyboard: KeyboardType,
    test_id: &'static str,
    show_a11y: impl Fn() -> String + 'static,
) -> View {
    let shown = signal(false);
    let err_for_border = error.clone();
    let err_for_text = error.clone();
    let err_shown = error.clone();
    view()
        .gap(px(6.0))
        .child(caption_row(caption, is_required(test_id)))
        .child(
            input_box()
                .pe(px(2.0))
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
                        .keyboard_type(keyboard)
                        .masked(move || !shown.get())
                        .text_size(px(fs(ty::MD)))
                        .text_color(Tone::Ink.color()),
                )
                .child(
                    view()
                        .w(px(36.0))
                        .h(px(36.0))
                        .items_center()
                        .justify_center()
                        .rounded(px(radius::md()))
                        .cursor_pointer()
                        .label(move || SharedString::from(show_a11y()))
                        .on_press(move || shown.update(|s| *s = !*s))
                        .child(dynamic(move || {
                            icon(
                                if shown.get() { Icon::EyeOff } else { Icon::Eye },
                                18.0,
                                || Tone::Muted,
                            )
                            .into_element()
                        })),
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

#[allow(clippy::too_many_arguments)] // a field's parts, named at each call
fn field_inner(
    caption: impl Fn() -> String + 'static,
    value: Signal<String>,
    placeholder: impl Fn() -> String + 'static,
    error: impl Fn() -> String + Clone + 'static,
    keyboard: KeyboardType,
    test_id: &'static str,
    masked: bool,
    focus: bool,
    ltr: bool,
) -> View {
    let err_for_border = error.clone();
    let err_for_text = error.clone();
    let err_shown = error.clone();
    view()
        .gap(px(6.0))
        .child(caption_row(caption, is_required(test_id)))
        .child(
            input_box()
                .border_color(move || {
                    if err_for_border().is_empty() {
                        Tone::BorderStrong.color()
                    } else {
                        Tone::Danger.color()
                    }
                })
                .direction(move || {
                    if ltr {
                        LayoutDirection::Ltr
                    } else if super::tokens::rtl() {
                        LayoutDirection::Rtl
                    } else {
                        LayoutDirection::Ltr
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
                        .auto_focus(focus)
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

/// The fields a form cannot be saved without (by test id): their caption
/// carries a red star.
const REQUIRED: &[&str] = &[
    "account-code",
    "account-edit-name",
    "account-name",
    "adj-amount",
    "adj-reason",
    "approval-pin",
    "cat-name",
    "category-name",
    "count-qty",
    "customer-name",
    "doc-cancel-reason",
    "exp-total-input",
    "free-name",
    "free-price",
    "label-count",
    "line-name",
    "line-price",
    "line-qty",
    "money-name",
    "order-reason",
    "plan-amount",
    "plan-count",
    "pos-layout-name",
    "pr-reason",
    "price-list-name",
    "printer-name",
    "product-name",
    "product-price",
    "rc-amount",
    "reopen-reason",
    "ret-reason",
    "reverse-reason",
    "setup-name",
    "setup-owner",
    "sp-amount",
    "till-counted",
    "tpl-name",
    "user-name",
    "warehouse-name",
];

/// Whether the field `test_id` must be filled.
pub fn is_required(test_id: &str) -> bool {
    REQUIRED.contains(&test_id)
}

/// A field's caption, with a red star when it must be filled.
pub fn caption_row(caption: impl Fn() -> String + 'static, required: bool) -> View {
    // A field without a caption (its placeholder says it) has no star.
    let named = !untracked(&caption).is_empty();
    let mut row = view().flex_row().items_center().gap(px(3.0)).child(label(
        caption,
        ty::SM,
        FontWeight::SEMIBOLD,
        Tone::Ink,
    ));
    if required && named {
        row = row.child(
            label(|| "*".to_owned(), ty::SM, FontWeight::BOLD, Tone::Danger)
                .test_id("required-mark"),
        );
    }
    row
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

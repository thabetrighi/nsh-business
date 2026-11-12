//! Overlays: the adaptive modal (centred dialog on wide screens, bottom
//! sheet on phones), the dialog's standard frame (header, body, footer) and
//! the toast that confirms an action in its own words.

use std::time::Duration;

use nsh_base::dialog;
use nsh_core::element::View;
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::kit::{Variant, icon_button, label};
use super::tokens::{Tone, radius, sp, ty};
use crate::state::AppState;

/// Dialog widths.
pub const DIALOG_W: f32 = 520.0;

/// A modal shown while `open` is true. `compact` picks the phone form (a sheet
/// rising from the bottom edge); otherwise a dialog centred on a dimmed
/// backdrop. Escape, the back button and a press on the backdrop close it.
pub fn modal(
    open: Signal<bool>,
    compact: bool,
    content: impl Fn() -> AnyElement + 'static,
) -> impl IntoElement {
    modal_sized(open, compact, false, content)
}

/// A [`modal`] for work that needs room (an editor with its preview):
/// most of the window on a desktop, the full sheet on a phone.
pub fn wide_modal(
    open: Signal<bool>,
    compact: bool,
    content: impl Fn() -> AnyElement + 'static,
) -> impl IntoElement {
    modal_sized(open, compact, true, content)
}

fn modal_sized(
    open: Signal<bool>,
    compact: bool,
    wide: bool,
    content: impl Fn() -> AnyElement + 'static,
) -> impl IntoElement {
    let close = move || open.set(false);
    when(open, move || {
        // The dimmed layer is the container itself (so it always paints); a
        // transparent full-size layer under the panel takes backdrop presses.
        let dim = rgb(0x0A0D12).opacity(0.42);
        let backdrop = view()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .on_press(close);
        let layer = view().flex_1().bg(dim).child(backdrop);
        let layer = if compact {
            layer.child(
                view()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .max_h(relative(if wide { 0.97 } else { 0.92 }))
                    .rounded_t(px(radius::xl().max(14.0)))
                    .bg(Tone::Surface.color())
                    .pb_safe_area_plus(px(8.0))
                    .child(
                        // Grab handle: the sheet can be dismissed by tapping outside.
                        view().items_center().pt(px(8.0)).child(
                            view()
                                .w(px(36.0))
                                .h(px(4.0))
                                .rounded_full()
                                .bg(Tone::BorderStrong.color()),
                        ),
                    )
                    .child(content()),
            )
        } else {
            let panel = if wide {
                view().w(relative(0.94)).max_w(px(1240.0)).h(relative(0.92))
            } else {
                view().w(px(DIALOG_W)).max_h(relative(0.9))
            };
            layer.items_center().justify_center().child(
                panel
                    .rounded(px(radius::xl()))
                    .bg(Tone::Surface.color())
                    .border_1()
                    .border_color(Tone::Border.color())
                    .shadow_2xl()
                    .overflow_hidden()
                    .child(content()),
            )
        };
        dialog().on_cancel(close).child(layer).into_element()
    })
}

/// The standard dialog: title and description with a close button, a
/// scrolling body, and a footer with the actions at the reading end.
pub fn dialog_frame(
    st: AppState,
    title: impl Fn() -> String + 'static,
    description: impl Fn() -> String + 'static,
    body: View,
    footer: View,
    on_close: impl Fn() + 'static,
) -> View {
    // Shrinks to the sheet or panel around it, so the body scrolls and the
    // footer's actions stay on screen whatever the form's length.
    view()
        .flex_shrink(1.0)
        .min_h(px(0.0))
        .child(
            view()
                .flex_row()
                .items_start()
                .gap(px(12.0))
                .px(px(24.0))
                .pt(px(sp(20.0)))
                .pb(px(sp(16.0)))
                .border_b_1()
                .border_color(Tone::Border.color())
                .child(
                    view()
                        .flex_1()
                        .gap(px(4.0))
                        .child(label(title, ty::XL, FontWeight::BOLD, Tone::Ink))
                        .child(label(description, ty::SM, FontWeight::NORMAL, Tone::Muted)),
                )
                .child(
                    icon_button(
                        || Icon::Close,
                        st.t("action.close"),
                        Variant::Ghost,
                        on_close,
                    )
                    .test_id("dialog-close"),
                ),
        )
        .child(
            scroll()
                .flex_shrink(1.0)
                .min_h(px(0.0))
                .child(body.px(px(24.0)).py(px(sp(20.0)))),
        )
        .child(
            footer
                .flex_row()
                .items_center()
                .justify_end()
                .gap(px(8.0))
                .px(px(24.0))
                .py(px(sp(14.0)))
                .bg(Tone::Subtle.color())
                .border_t_1()
                .border_color(Tone::Border.color()),
        )
}

/// Shows `message` as a toast (replaces a visible one). The toast layer at
/// the root hides it after a few seconds.
pub fn show_toast(st: AppState, message: String) {
    st.toast.set(Some(message));
}

/// The toast layer, placed once at the root, above the bottom navigation.
pub fn toast_layer(st: AppState, compact: bool) -> impl IntoElement {
    // The hide timer belongs to this root-level layer, not to the screen that
    // raised the toast (a closing form would cancel a timer it owned). Each
    // new message re-runs the effect, which replaces the previous timer.
    effect(move || {
        if let Some(message) = st.toast.get() {
            let timer = set_timeout(Duration::from_millis(2600), move || {
                if st.toast.get_untracked().as_deref() == Some(message.as_str()) {
                    st.toast.set(None);
                }
            });
            drop(timer); // runs to completion: tasks end with their owner, not their handle
        }
    });
    when(
        move || st.toast.with(Option::is_some),
        move || {
            view()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(if compact { 88.0 } else { 28.0 }))
                .items_center()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .ps(px(12.0))
                        .pe(px(16.0))
                        .h(px(44.0))
                        .rounded(px(radius::lg()))
                        .bg(Tone::Inverse.color())
                        .shadow_lg()
                        .child(icon(Icon::CheckCircle, 18.0, || Tone::InverseSuccess))
                        .child(label(
                            move || st.toast.get().unwrap_or_default(),
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::OnInverse,
                        )),
                )
                .into_element()
        },
    )
}

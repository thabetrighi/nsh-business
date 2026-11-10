//! Overlays: the adaptive modal (centred card on wide screens, bottom sheet on
//! phones) and the toast that confirms an action in its own words.

use std::time::Duration;

use nsh_base::dialog;
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::kit::{label, size};
use super::palette::Tone;
use crate::state::AppState;

/// A modal shown while `open` is true. `compact` picks the phone form (a sheet
/// rising from the bottom edge); otherwise a card centred on a dimmed backdrop.
/// Escape, the back button and a press on the backdrop close it.
pub fn modal(
    open: Signal<bool>,
    compact: bool,
    content: impl Fn() -> AnyElement + 'static,
) -> impl IntoElement {
    let close = move || open.set(false);
    when(open, move || {
        // The dimmed layer is the container itself (so it always paints); a
        // transparent full-size layer under the panel takes backdrop presses.
        let dim = rgb(0x0B1015).opacity(0.45);
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
                    .rounded_t(px(16.0))
                    .bg(Tone::Surface.color())
                    .pb_safe_area_plus(px(12.0))
                    .child(
                        // Grab handle: the sheet can be swiped down or dismissed by tapping outside.
                        view().items_center().pt_2().child(
                            view()
                                .w(px(40.0))
                                .h(px(4.0))
                                .rounded_full()
                                .bg(Tone::Line.color()),
                        ),
                    )
                    .child(content()),
            )
        } else {
            layer.items_center().justify_center().child(
                view()
                    .w(px(560.0))
                    .rounded(px(14.0))
                    .bg(Tone::Surface.color())
                    .border_1()
                    .border_color(Tone::Line.color())
                    .shadow_xl()
                    .child(content()),
            )
        };
        dialog().on_cancel(close).child(layer).into_element()
    })
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
                .bottom(px(if compact { 88.0 } else { 32.0 }))
                .items_center()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .px_4()
                        .py(px(10.0))
                        .rounded_full()
                        .bg(Tone::Ink.color())
                        .shadow_lg()
                        .child(icon(Icon::Synced, 18.0, || Tone::Paper))
                        .child(label(
                            move || st.toast.get().unwrap_or_default(),
                            size::BODY,
                            FontWeight::MEDIUM,
                            Tone::Paper,
                        )),
                )
                .into_element()
        },
    )
}

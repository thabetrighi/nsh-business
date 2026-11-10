//! Small styled building blocks over nsh-base and nsh-core: text roles,
//! buttons, rules. Every screen composes these; no screen styles raw views
//! for these roles itself.

use nsh_core::element::SharedString;
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::palette::Tone;

/// Type scale from DESIGN.md §4 (points).
pub mod size {
    pub const CAPTION: f32 = 12.0;
    pub const BODY: f32 = 14.0;
    pub const LEAD: f32 = 16.0;
    pub const TITLE: f32 = 20.0;
    pub const HEADLINE: f32 = 26.0;
    pub const FIGURE: f32 = 40.0;
}

/// Text in a role: size, weight, tone.
pub fn label(
    content: impl Fn() -> String + 'static,
    size: f32,
    weight: FontWeight,
    tone: Tone,
) -> impl IntoElement {
    text(content)
        .text_size(px(size))
        .font_weight(weight)
        .text_color(tone.color())
}

/// Text in a role, kept to one line (truncated) — for names in dense rows.
pub fn label_line(
    content: impl Fn() -> String + 'static,
    size: f32,
    weight: FontWeight,
    tone: Tone,
) -> impl IntoElement {
    text(content)
        .text_size(px(size))
        .font_weight(weight)
        .number_of_lines(1)
        .text_color(tone.color())
}

/// A money or quantity figure: tabular digits so columns line up.
pub fn figure(
    content: impl Fn() -> String + 'static,
    size: f32,
    weight: FontWeight,
    tone: impl Fn() -> Tone + 'static,
) -> impl IntoElement {
    text(content)
        .text_size(px(size))
        .font_weight(weight)
        .tabular_nums()
        .text_color(move || tone().color())
}

/// A hairline rule between ledger rows.
pub fn rule() -> impl IntoElement {
    view().h(px(1.0)).bg(Tone::Line.color())
}

/// The main action: filled in the brand blue.
pub fn primary_button(
    caption: impl Fn() -> String + 'static,
    leading: Option<Icon>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let mut row = view()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .h(px(44.0))
        .px_4()
        .rounded(px(8.0))
        .bg(Tone::Primary.color())
        .active(|s| s.opacity(0.85))
        .on_press(on_press);
    if let Some(i) = leading {
        row = row.child(icon(i, 18.0, || Tone::OnPrimary));
    }
    row.child(label(
        caption,
        size::BODY,
        FontWeight::SEMIBOLD,
        Tone::OnPrimary,
    ))
}

/// A quiet action in a list row: outlined, compact.
pub fn quiet_button(
    caption: impl Fn() -> String + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .h(px(32.0))
        .px_3()
        .justify_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(Tone::Line.color())
        .bg(Tone::Surface.color())
        .active(|s| s.bg(Tone::Accent.color()))
        .on_press(on_press)
        .child(label(
            caption,
            size::BODY,
            FontWeight::MEDIUM,
            Tone::Primary,
        ))
}

/// A square icon-only button with an accessible label.
pub fn icon_button(
    which: impl Fn() -> Icon + 'static,
    a11y: impl Fn() -> String + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let glyph = move || {
        let i = which();
        icon(i, 20.0, || Tone::Ink)
    };
    view()
        .w(px(40.0))
        .h(px(40.0))
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .active(|s| s.bg(Tone::Accent.color()))
        .label(move || SharedString::from(a11y()))
        .on_press(on_press)
        .child(dynamic(glyph))
}

/// Initials in a tinted circle, for people in lists: two Latin initials,
/// or the first letter of an Arabic name (Arabic initials are not customary).
pub fn initials(name: impl Fn() -> String + 'static, tone: Tone) -> impl IntoElement {
    let letters = move || {
        let n = nb_i18n::bidi::strip_controls(&name());
        let arabic = n.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
        let take = if arabic { 1 } else { 2 };
        n.split_whitespace()
            .take(take)
            .filter_map(|w| w.chars().next())
            .collect::<String>()
    };
    view()
        .w(px(36.0))
        .h(px(36.0))
        .rounded_full()
        .items_center()
        .justify_center()
        .bg(Tone::Accent.color())
        .child(label(letters, size::BODY, FontWeight::SEMIBOLD, tone))
}

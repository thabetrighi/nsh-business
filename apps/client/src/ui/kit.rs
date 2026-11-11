//! Building blocks of the design system (DESIGN.md v2): text roles, figures,
//! buttons, badges, avatars, segmented controls, switches, checkboxes.
//! Screens compose these; they do not style raw views for these roles.

use nsh_core::LineHeight;
use nsh_core::element::{SharedString, Text, View};
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::tokens::{FIGURE_FONT, Status, Tone, control, fs, radius, ty};

// ------------------------------------------------------------------ text

/// Text in a role: size (from [`ty`], scaled by the text-size setting),
/// weight, tone.
pub fn label(
    content: impl Fn() -> String + 'static,
    size: f32,
    weight: FontWeight,
    tone: Tone,
) -> Text {
    text(content)
        .text_size(px(fs(size)))
        // Wrapped lines keep their marks apart (Tajawal's own spacing
        // lets Arabic lines touch).
        .line_height(LineHeight::Multiple(1.5))
        .font_weight(weight)
        .text_color(tone.color())
}

/// Like [`label`], kept to one line (truncated) — for names in dense rows.
pub fn label_line(
    content: impl Fn() -> String + 'static,
    size: f32,
    weight: FontWeight,
    tone: Tone,
) -> Text {
    label(content, size, weight, tone).number_of_lines(1)
}

/// A fixed string in a role.
pub fn caption(s: impl Into<String>, size: f32, weight: FontWeight, tone: Tone) -> Text {
    let s: String = s.into();
    label(move || s.clone(), size, weight, tone)
}

/// A money or quantity figure: equal-width digits in the figure font, so
/// columns line up whatever the interface font.
pub fn figure(
    content: impl Fn() -> String + 'static,
    size: f32,
    weight: FontWeight,
    tone: impl Fn() -> Tone + 'static,
) -> Text {
    text(content)
        .font_family(FIGURE_FONT)
        .text_size(px(fs(size)))
        .font_weight(weight)
        .tabular_nums()
        .number_of_lines(1)
        .text_color(move || tone().color())
}

// ------------------------------------------------------------------ buttons

/// What a button is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// The one main action of a view: filled accent.
    Primary,
    /// Other actions: white with a border and a soft shadow.
    Secondary,
    /// Toolbar and inline actions: no frame until hovered.
    Ghost,
    /// Deleting, voiding: filled red.
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Sm,
    Md,
    Lg,
}

impl Size {
    fn height(self) -> f32 {
        match self {
            Size::Sm => control::sm(),
            Size::Md => control::md(),
            Size::Lg => control::lg(),
        }
    }
    fn pad(self) -> f32 {
        match self {
            Size::Sm => 10.0,
            Size::Md => 12.0,
            Size::Lg => 16.0,
        }
    }
    fn text(self) -> f32 {
        match self {
            Size::Sm => ty::SM,
            Size::Md => ty::SM,
            Size::Lg => ty::MD,
        }
    }
    fn icon(self) -> f32 {
        match self {
            Size::Sm => 14.0,
            Size::Md => 16.0,
            Size::Lg => 18.0,
        }
    }
}

fn frame(variant: Variant, size: Size) -> View {
    let base = view()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .h(px(size.height()))
        .px(px(size.pad()))
        .rounded(px(radius::md()))
        .cursor_pointer();
    match variant {
        Variant::Primary => base
            .bg(Tone::Primary.color())
            .shadow_xs()
            .hover(|s| s.opacity(0.92))
            .active(|s| s.opacity(0.82)),
        Variant::Danger => base
            .bg(Tone::Danger.color())
            .shadow_xs()
            .hover(|s| s.opacity(0.92))
            .active(|s| s.opacity(0.82)),
        Variant::Secondary => base
            .bg(Tone::Surface.color())
            .border_1()
            .border_color(Tone::BorderStrong.color())
            .shadow_xs()
            .hover(|s| s.bg(Tone::Subtle.color()))
            .active(|s| s.bg(Tone::Hover.color())),
        Variant::Ghost => base
            .hover(|s| s.bg(Tone::Hover.color()))
            .active(|s| s.bg(Tone::Border.color())),
    }
}

fn ink(variant: Variant) -> Tone {
    match variant {
        Variant::Primary | Variant::Danger => Tone::OnPrimary,
        Variant::Secondary | Variant::Ghost => Tone::Ink,
    }
}

/// A button with a caption and an optional leading icon.
pub fn button(
    variant: Variant,
    size: Size,
    leading: Option<Icon>,
    caption: impl Fn() -> String + 'static,
    on_press: impl Fn() + 'static,
) -> View {
    let ink = ink(variant);
    let mut b = frame(variant, size).on_press(on_press);
    if let Some(i) = leading {
        let tone = if variant == Variant::Ghost || variant == Variant::Secondary {
            Tone::Muted
        } else {
            ink
        };
        b = b.child(icon(i, size.icon(), move || tone));
    }
    b.child(label_line(caption, size.text(), FontWeight::SEMIBOLD, ink))
}

/// A button whose caption is followed by a chevron: it opens a menu or list.
pub fn menu_button(leading: Option<Icon>, caption: impl Fn() -> String + 'static) -> View {
    let mut b = frame(Variant::Secondary, Size::Md);
    if let Some(i) = leading {
        b = b.child(icon(i, 16.0, || Tone::Muted));
    }
    b.child(label_line(caption, ty::SM, FontWeight::SEMIBOLD, Tone::Ink))
        .child(icon(Icon::ChevronDown, 14.0, || Tone::Muted))
}

/// A square icon-only button with an accessible label.
pub fn icon_button(
    which: impl Fn() -> Icon + 'static,
    a11y: impl Fn() -> String + 'static,
    variant: Variant,
    on_press: impl Fn() + 'static,
) -> View {
    let side = control::md();
    let tone = match variant {
        Variant::Primary | Variant::Danger => Tone::OnPrimary,
        _ => Tone::Muted,
    };
    frame(variant, Size::Md)
        .w(px(side))
        .px(px(0.0))
        .label(move || SharedString::from(a11y()))
        .on_press(on_press)
        .child(dynamic(move || {
            icon(which(), 16.0, move || tone).into_element()
        }))
}

// ------------------------------------------------------------------ badges

/// A status pill: soft background, colored text, optional icon. Used in
/// tables and lists for states (paid, overdue, low stock…).
pub fn badge(
    status: Status,
    leading: Option<Icon>,
    content: impl Fn() -> String + 'static,
) -> View {
    let mut b = view()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .h(px(fs(22.0)))
        .px(px(8.0))
        .rounded(px(radius::sm()))
        .bg(status.bg().color())
        .self_start();
    if let Some(i) = leading {
        b = b.child(icon(i, 12.0, move || status.fg()));
    }
    b.child(label_line(
        content,
        ty::XS,
        FontWeight::SEMIBOLD,
        status.fg(),
    ))
}

/// A small colored dot before a word (inline statuses, legends).
pub fn dot(tone: Tone, size: f32) -> View {
    view()
        .w(px(size))
        .h(px(size))
        .rounded_full()
        .bg(tone.color())
        .flex_shrink_0()
}

/// A keyboard shortcut hint.
pub fn kbd(content: impl Fn() -> String + 'static) -> View {
    view()
        .h(px(20.0))
        .px(px(6.0))
        .justify_center()
        .rounded(px(radius::sm()))
        .bg(Tone::Surface.color())
        .border_1()
        .border_color(Tone::Border.color())
        .child(label(content, 11.0, FontWeight::MEDIUM, Tone::Muted).font_family(FIGURE_FONT))
}

/// Initials in a tinted circle: two Latin initials, or the first letter of
/// an Arabic name (Arabic initials are not customary).
pub fn avatar(name: impl Fn() -> String + 'static, size: f32, status: Status) -> View {
    let letters = move || {
        let n = nb_i18n::bidi::strip_controls(&name());
        let arabic = n.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
        let take = if arabic { 1 } else { 2 };
        n.split_whitespace()
            .take(take)
            .filter_map(|w| w.chars().next())
            .collect::<String>()
            .to_uppercase()
    };
    view()
        .w(px(size))
        .h(px(size))
        .rounded_full()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .bg(status.bg().color())
        .child(label(
            letters,
            size * 0.4,
            FontWeight::SEMIBOLD,
            status.fg(),
        ))
}

/// A hairline between groups.
pub fn divider() -> View {
    view().h(px(1.0)).bg(Tone::Border.color())
}

/// Takes the free room in a row.
pub fn spacer() -> View {
    view().flex_1()
}

// ------------------------------------------------------------------ choices

/// One of a few options as a segmented control (filters, view modes).
pub fn segmented<T: Copy + PartialEq + 'static>(
    options: Vec<(T, Box<dyn Fn() -> String>)>,
    selected: Signal<T>,
) -> View {
    let mut track = view()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .p(px(2.0))
        .rounded(px(radius::md()))
        .bg(Tone::Subtle.color())
        .border_1()
        .border_color(Tone::Border.color())
        .self_start();
    for (value, caption) in options {
        let active = computed(move || selected.get() == value);
        track = track.child(
            view()
                .h(px(control::sm()))
                .px(px(12.0))
                .justify_center()
                .rounded(px(radius::md() - 2.0))
                .cursor_pointer()
                .style_when(active, |s| s.bg(Tone::Surface.color()).shadow_xs())
                .hover(|s| s.opacity(0.9))
                .on_press(move || selected.set(value))
                .child(
                    text(caption)
                        .text_size(px(fs(ty::SM)))
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(1)
                        .text_color(move || {
                            if active.get() {
                                Tone::Ink.color()
                            } else {
                                Tone::Muted.color()
                            }
                        }),
                ),
        );
    }
    track
}

/// An on/off switch.
pub fn switch(on: Signal<bool>, a11y: impl Fn() -> String + 'static) -> View {
    let w = 34.0;
    let knob = 16.0;
    view()
        .w(px(w))
        .h(px(20.0))
        .p(px(2.0))
        .rounded_full()
        .flex_row()
        .direction(LayoutDirection::Ltr)
        .cursor_pointer()
        .bg(move || {
            if on.get() {
                Tone::Primary.color()
            } else {
                Tone::BorderStrong.color()
            }
        })
        .label(move || SharedString::from(a11y()))
        .on_press(move || on.update(|v| *v = !*v))
        .child(view().w(move || px(if on.get() { w - knob - 4.0 } else { 0.0 })))
        .child(
            view()
                .w(px(knob))
                .h(px(knob))
                .rounded_full()
                .bg(rgb(0xFFFFFF))
                .shadow_xs(),
        )
}

/// A checkbox (three states: off, on, mixed for "some rows").
pub fn checkbox(
    state: impl Fn() -> Option<bool> + Clone + 'static,
    on_press: impl Fn() + 'static,
) -> View {
    let s1 = state.clone();
    let s2 = state.clone();
    view()
        .w(px(16.0))
        .h(px(16.0))
        .rounded(px(4.0))
        .items_center()
        .justify_center()
        .cursor_pointer()
        .border_1()
        .border_color(move || {
            if s1() == Some(false) {
                Tone::BorderStrong.color()
            } else {
                Tone::Primary.color()
            }
        })
        .bg(move || {
            if s2() == Some(false) {
                Tone::Surface.color()
            } else {
                Tone::Primary.color()
            }
        })
        .on_press(on_press)
        .child(dynamic(move || match state() {
            Some(true) => icon(Icon::Check, 12.0, || Tone::OnPrimary).into_element(),
            None => view()
                .w(px(8.0))
                .h(px(2.0))
                .rounded_full()
                .bg(Tone::OnPrimary.color())
                .into_element(),
            Some(false) => view().into_element(),
        }))
}

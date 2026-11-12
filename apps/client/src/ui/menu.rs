//! Menus (owner notes: one look everywhere): what may be done to a row of
//! any list — a "⋯" button, and the same menu on a right click (a held
//! finger on touch screens) — and every other menu of the app: one
//! floating list in the design system's look, each entry with its icon, a
//! check where it is a setting, a chevron where it opens more, the
//! destructive ones in red after a line. Drawn by the app (nsh-base's
//! menus), not the platform's: the same on every system, and testable.

use std::rc::Rc;

use nsh_base::{context_menu, menu, menu_item, menu_separator, submenu};
use nsh_core::element::{Align, Placement, SharedString, View};
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::kit::label_line;
use super::tokens::{Tone, control, radius, ty};
use crate::state::AppState;

/// The floating list's look (the same as the dropdowns').
pub const POPUP: &str = "bg-popover border border-border rounded-lg shadow-lg p-1 min-w-[200px]";

/// One entry of a menu.
#[derive(Clone)]
pub enum Act {
    Do {
        icon: Option<Icon>,
        label: String,
        enabled: bool,
        danger: bool,
        /// A setting: checked or not.
        checked: Option<bool>,
        /// A shortcut or a value, at the end.
        hint: Option<String>,
        /// Keeps the menu open (settings toggled in a row).
        stay: bool,
        run: Rc<dyn Fn()>,
    },
    /// Opens more entries beside it.
    Sub {
        icon: Option<Icon>,
        label: String,
        items: Vec<Act>,
    },
    /// A group's title.
    Title(String),
    Line,
}

impl Act {
    pub fn new(label: String, run: impl Fn() + 'static) -> Self {
        Act::Do {
            icon: None,
            label,
            enabled: true,
            danger: false,
            checked: None,
            hint: None,
            stay: false,
            run: Rc::new(run),
        }
    }

    /// An entry with its icon.
    pub fn with(icon: Icon, label: String, run: impl Fn() + 'static) -> Self {
        Act::new(label, run).icon(icon)
    }

    /// Copies `text` (shown as `label`).
    pub fn copy(label: String, text: String) -> Self {
        Act::with(Icon::Copy, label, move || set_clipboard(text.clone()))
    }

    pub fn sub(icon: Icon, label: String, items: Vec<Act>) -> Self {
        Act::Sub {
            icon: Some(icon),
            label,
            items,
        }
    }

    pub fn icon(mut self, i: Icon) -> Self {
        match &mut self {
            Act::Do { icon, .. } | Act::Sub { icon, .. } => *icon = Some(i),
            _ => {}
        }
        self
    }

    /// Greyed out unless `on` (a permission the user lacks).
    pub fn when(mut self, on: bool) -> Self {
        if let Act::Do { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }

    /// Destructive: in red, after a line.
    pub fn danger(mut self) -> Self {
        if let Act::Do { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        if let Act::Do { checked, .. } = &mut self {
            *checked = Some(on);
        }
        self
    }

    pub fn hint(mut self, h: impl Into<String>) -> Self {
        if let Act::Do { hint, .. } = &mut self {
            *hint = Some(h.into());
        }
        self
    }
}

/// The destructive entries last, after a line.
fn ordered(acts: Vec<Act>) -> Vec<Act> {
    let (bad, mut good): (Vec<Act>, Vec<Act>) = acts
        .into_iter()
        .partition(|a| matches!(a, Act::Do { danger: true, .. }));
    // No line left hanging at the end.
    while matches!(good.last(), Some(Act::Line)) {
        good.pop();
    }
    if !bad.is_empty() && !good.is_empty() {
        good.push(Act::Line);
    }
    good.extend(bad);
    good
}

fn row(
    ic: Option<Icon>,
    label: String,
    tone: Tone,
    checked: Option<bool>,
    hint: Option<String>,
    more: bool,
    enabled: bool,
) -> View {
    let mut r = view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(control::md()))
        .px(px(10.0))
        .rounded(px(radius::md()))
        .opacity(if enabled { 1.0 } else { 0.45 })
        .hover(|s| s.bg(Tone::Hover.color()));
    if enabled {
        r = r.cursor_pointer();
    }
    let icon_tone = if tone == Tone::Danger {
        Tone::Danger
    } else {
        Tone::Muted
    };
    r = match (checked, ic) {
        (Some(on), _) => r.child(
            view()
                .w(px(16.0))
                .items_center()
                .opacity(if on { 1.0 } else { 0.0 })
                .child(icon(Icon::Check, 15.0, || Tone::Primary)),
        ),
        (None, Some(i)) => r.child(icon(i, 16.0, move || icon_tone)),
        (None, None) => r.child(view().w(px(16.0))),
    };
    r = r.child(view().flex_1().min_w(px(0.0)).child(label_line(
        move || label.clone(),
        ty::SM,
        FontWeight::MEDIUM,
        tone,
    )));
    if let Some(h) = hint {
        r = r.child(label_line(
            move || h.clone(),
            ty::XS,
            FontWeight::MEDIUM,
            Tone::Faint,
        ));
    }
    if more {
        let side = if super::tokens::rtl() {
            Icon::ChevronLeft
        } else {
            Icon::ChevronRight
        };
        r = r.child(icon(side, 14.0, || Tone::Muted));
    }
    r
}

/// Adds `acts` to a menu, a context menu or a submenu (`push`).
fn fill<M>(mut m: M, acts: Vec<Act>, push: &dyn Fn(M, View) -> M) -> M {
    for a in ordered(acts) {
        m = match a {
            Act::Line => push(
                m,
                view().child(
                    menu_separator().child(view().h(px(1.0)).my(px(4.0)).bg(Tone::Border.color())),
                ),
            ),
            Act::Title(t) => push(
                m,
                view()
                    .px(px(10.0))
                    .pt(px(6.0))
                    .pb(px(4.0))
                    .child(label_line(
                        move || t.clone(),
                        ty::XS,
                        FontWeight::SEMIBOLD,
                        Tone::Faint,
                    )),
            ),
            Act::Do {
                icon,
                label,
                enabled,
                danger,
                checked,
                hint,
                stay,
                run,
            } => {
                let tone = if danger { Tone::Danger } else { Tone::Ink };
                let mut it = menu_item()
                    .disabled(!enabled)
                    .close_on_select(!stay)
                    .accessibility_label(SharedString::from(label.clone()))
                    .on_select(move || run());
                if let Some(c) = checked {
                    it = it.checked(Some(c));
                }
                push(
                    m,
                    view().child(it.child(row(icon, label, tone, checked, hint, false, enabled))),
                )
            }
            Act::Sub { icon, label, items } => {
                let s = submenu().trigger(row(icon, label, Tone::Ink, None, None, true, true));
                let s = fill(s, items, &|s: nsh_base::SubmenuElement, v: View| s.child(v));
                push(m, view().child(s))
            }
        };
    }
    m
}

/// A menu `trigger` opens, below it, `acts` made as it is built.
pub fn menu_on(trigger: View, acts: Vec<Act>) -> View {
    menu_at(trigger, acts, Placement::Bottom, Align::End)
}

/// As [`menu_on`], placed as asked (a sidebar's flyout opens beside it).
pub fn menu_at(trigger: View, acts: Vec<Act>, placement: Placement, align: Align) -> View {
    let m = menu()
        .trigger(trigger)
        .popup_class(POPUP)
        .placement(placement)
        .align(align);
    let m = fill(m, acts, &|m: nsh_base::MenuElement, v: View| m.child(v));
    view().child(m)
}

/// The "⋯" button opening `acts()`.
pub fn more_button(st: AppState, acts: impl Fn() -> Vec<Act> + 'static, test_id: &str) -> View {
    let side = control::sm();
    let trigger = view()
        .test_id(test_id)
        .w(px(side))
        .h(px(side))
        .items_center()
        .justify_center()
        .rounded(px(radius::md()))
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
        .label(st.t("action.more"))
        .child(icon(Icon::More, 16.0, || Tone::Muted));
    menu_on(trigger, acts())
}

/// The same menu on a right click on `row` (a held finger on touch
/// screens).
pub fn with_menu(row: View, acts: impl Fn() -> Vec<Act> + 'static) -> View {
    let m = context_menu().trigger(row).popup_class(POPUP);
    let m = fill(m, acts(), &|m: nsh_base::ContextMenuElement, v: View| {
        m.child(v)
    });
    view().child(m)
}

/// Rows as text to paste in a spreadsheet: cells by tabs, rows by lines.
pub fn tsv(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|r| {
            r.iter()
                .map(|c| c.replace(['\t', '\n'], " "))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_copy_as_tab_separated_text() {
        let rows = vec![
            vec!["SKU".to_owned(), "Nom".into()],
            vec!["A\t1".into(), "Lait\n1L".into()],
        ];
        assert_eq!(super::tsv(&rows), "SKU\tNom\nA 1\tLait 1L");
    }

    #[test]
    fn destructive_entries_come_last_after_a_line() {
        let acts = vec![
            Act::new("Delete".into(), || {}).danger(),
            Act::new("Open".into(), || {}),
            Act::Line,
        ];
        let names: Vec<String> = ordered(acts)
            .into_iter()
            .map(|a| match a {
                Act::Do { label, .. } => label,
                Act::Line => "—".into(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(names, ["Open", "—", "Delete"]);
    }
}

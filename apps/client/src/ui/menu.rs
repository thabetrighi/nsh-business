//! Row actions (owner notes): what may be done to a row of any list — a
//! "⋯" button, and the same menu on a right click — copying included.
//! The menus are the platform's own (on desktops; elsewhere the button
//! opens nothing and the row's own buttons remain).

use std::rc::Rc;

use nsh_core::element::View;
use nsh_core::element::desktop::{MenuItem, popup_menu, popup_menu_at};
use nsh_core::prelude::*;

use super::icon::Icon;
use super::kit::{Variant, icon_button};
use crate::state::AppState;

/// One action of a row's menu.
#[derive(Clone)]
pub enum Act {
    Do {
        label: String,
        enabled: bool,
        run: Rc<dyn Fn()>,
    },
    Line,
}

impl Act {
    pub fn new(label: String, run: impl Fn() + 'static) -> Self {
        Act::Do {
            label,
            enabled: true,
            run: Rc::new(run),
        }
    }

    /// Copies `text` (shown as `label`).
    pub fn copy(label: String, text: String) -> Self {
        Act::new(label, move || set_clipboard(text.clone()))
    }

    /// Greyed out unless `on` (a permission the user lacks).
    pub fn when(self, on: bool) -> Self {
        match self {
            Act::Do { label, run, .. } => Act::Do {
                label,
                enabled: on,
                run,
            },
            l => l,
        }
    }
}

pub fn menu_items(acts: Vec<Act>) -> Vec<MenuItem> {
    acts.into_iter()
        .map(|a| match a {
            Act::Do {
                label,
                enabled,
                run,
            } => MenuItem::new(label)
                .disabled(!enabled)
                .on_select(move || run()),
            Act::Line => MenuItem::separator(),
        })
        .collect()
}

/// The "⋯" button opening `acts()` at the pointer.
pub fn more_button(st: AppState, acts: impl Fn() -> Vec<Act> + 'static, test_id: &str) -> View {
    icon_button(
        || Icon::MoreVertical,
        st.t("action.more"),
        Variant::Ghost,
        move || drop(popup_menu(menu_items(acts()))),
    )
    .test_id(test_id)
}

/// The same menu on a right click on `row`.
pub fn with_menu(row: View, acts: impl Fn() -> Vec<Act> + 'static) -> View {
    row.on_context_menu(move |e: &ContextMenuEvent| {
        drop(popup_menu_at(menu_items(acts()), e.page_position))
    })
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
    #[test]
    fn rows_copy_as_tab_separated_text() {
        let rows = vec![
            vec!["SKU".to_owned(), "Nom".into()],
            vec!["A\t1".into(), "Lait\n1L".into()],
        ];
        assert_eq!(super::tsv(&rows), "SKU\tNom\nA 1\tLait 1L");
    }
}

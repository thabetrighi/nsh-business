//! Keeps nsh-base's components (dropdowns, menus, the command palette) in
//! the same look as the app: the theme file with the user's accent, font
//! and corner radius over it.

use nsh_base::{ScrollbarLook, ScrollbarMode, ScrollbarTheme, Theme, set_scrollbar_theme};
use nsh_core::prelude::*;

use super::tokens::{Look, Tone, radius};

/// Applies `look` to nsh-base's theme. Call after [`super::tokens::install`].
pub fn apply(look: Look) {
    let mut theme = match Theme::from_json(include_str!("../../theme.json")) {
        Ok(t) => t,
        Err(e) => {
            log::error!("theme.json: {e:?}");
            return;
        }
    };
    for name in ["primary", "ring", "sidebar-primary", "sidebar-ring"] {
        theme.set_color(name, Tone::Primary.color());
    }
    for name in ["primary-foreground", "sidebar-primary-foreground"] {
        theme.set_color(name, Tone::OnPrimary.color());
    }
    theme.set_font("sans", look.font.family());
    theme.set_number("radius", radius::md());
    set_tokens(theme);
    scrollbars();
}

/// Vertical scroll views draw a thin bar of their own, at the reading end,
/// always visible: the platform's overlay indicator (GTK) hides until the
/// view scrolls, so a long page did not show that it scrolls. Strips of
/// tabs and chips keep the platform's.
fn scrollbars() {
    let mut vertical = ScrollbarLook::default();
    vertical.class = Some("absolute top-0 bottom-0 end-0 w-[10px] p-[2px]".into());
    vertical.thumb_class =
        Some("w-full rounded-full bg-input hover:bg-muted-foreground transition-all".into());
    let mut theme = ScrollbarTheme::default();
    theme.mode = ScrollbarMode::Always;
    theme.min_thumb_length = px(32.0);
    theme.vertical = Some(vertical);
    set_scrollbar_theme(Some(theme));
}

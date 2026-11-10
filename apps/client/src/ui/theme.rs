//! Keeps nsh-base's components (dropdowns, menus, the command palette) in
//! the same look as the app: the theme file with the user's accent, font
//! and corner radius over it.

use nsh_base::Theme;
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
}

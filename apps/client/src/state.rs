//! App-wide UI state: language and appearance. Switching either updates the
//! whole interface at once, without a restart.

use nb_i18n::{Direction, Format, Lang, tr, tr_args};
use nsh_core::prelude::*;

/// Something a screen should do when it opens (e.g. from the command palette).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    AddProduct,
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub lang: Signal<Lang>,
    /// The confirmation shown at the bottom of the screen (`ui::overlay::show_toast`).
    pub toast: Signal<Option<String>>,
    /// The command palette is open.
    pub palette: Signal<bool>,
    /// A pending request for the screen being opened.
    pub intent: Signal<Option<Intent>>,
}

impl AppState {
    pub fn new() -> Self {
        let lang = locale()
            .as_str()
            .split(['-', '_'])
            .next()
            .and_then(Lang::from_tag)
            .unwrap_or(Lang::Ar);
        Self {
            lang: signal(lang),
            toast: signal(None),
            palette: signal(false),
            intent: signal(None),
        }
    }

    pub fn lang(&self) -> Lang {
        self.lang.get()
    }

    pub fn fmt(&self) -> Format {
        Format::new(self.lang.get())
    }

    pub fn rtl(&self) -> bool {
        self.lang.get().direction() == Direction::Rtl
    }

    pub fn direction(&self) -> LayoutDirection {
        if self.rtl() {
            LayoutDirection::Rtl
        } else {
            LayoutDirection::Ltr
        }
    }

    /// A reactive translated string.
    pub fn t(self, key: &'static str) -> impl Fn() -> String + 'static {
        move || tr(self.lang.get(), key).into_owned()
    }

    /// A reactive translated string with arguments computed on each update.
    pub fn t_with(
        self,
        key: &'static str,
        args: impl Fn(&AppState) -> Vec<(&'static str, String)> + 'static,
    ) -> impl Fn() -> String + 'static {
        move || {
            let a = args(&self);
            let pairs: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
            tr_args(self.lang.get(), key, &pairs)
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Toggles light and dark, starting from the current appearance.
pub fn toggle_appearance() {
    set_color_scheme(if dark() {
        Appearance::Light
    } else {
        Appearance::Dark
    });
}

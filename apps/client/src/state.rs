//! App-wide UI state: language, appearance settings, navigation. Changing a
//! setting updates the whole interface at once, without a restart, and is
//! remembered on the device.

use nb_i18n::{Digits, Direction, Format, Lang, tr, tr_args};
use nsh_core::prelude::*;
use nsh_core::storage::Storage;

use crate::nav::Route;
use crate::ui::tokens::{Accent, Corners, Density, Look, TextSize, UiFont};

/// Something a screen should do when it opens (e.g. from the command palette).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    AddProduct,
}

/// Light, dark, or as the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System];
    pub fn code(self) -> &'static str {
        match self {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.code() == s)
    }
    pub fn appearance(self) -> Appearance {
        match self {
            ThemeMode::System => Appearance::System,
            ThemeMode::Light => Appearance::Light,
            ThemeMode::Dark => Appearance::Dark,
        }
    }
}

/// The user's display preferences, saved on the device.
#[derive(Clone, Copy)]
pub struct Prefs {
    pub accent: Signal<Accent>,
    pub text: Signal<TextSize>,
    pub density: Signal<Density>,
    pub corners: Signal<Corners>,
    pub font: Signal<UiFont>,
    pub theme: Signal<ThemeMode>,
    pub digits: Signal<Digits>,
}

const K_ACCENT: &str = "nb.look.accent";
const K_TEXT: &str = "nb.look.text";
const K_DENSITY: &str = "nb.look.density";
const K_CORNERS: &str = "nb.look.corners";
const K_FONT: &str = "nb.look.font";
const K_THEME: &str = "nb.look.theme";
const K_DIGITS: &str = "nb.look.digits";
const K_LANG: &str = "nb.lang";

fn stored<T>(key: &str, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
    Storage::get_item(key).as_deref().and_then(parse)
}

fn save(key: &str, value: &str) {
    // A full storage only loses the preference, never the session.
    if Storage::set_item(key, value).is_err() {
        log::warn!("settings storage is full; {key} not saved");
    }
}

impl Prefs {
    fn load() -> Self {
        Self {
            accent: signal(stored(K_ACCENT, Accent::from_code).unwrap_or_default()),
            text: signal(stored(K_TEXT, TextSize::from_code).unwrap_or_default()),
            density: signal(stored(K_DENSITY, Density::from_code).unwrap_or_default()),
            corners: signal(stored(K_CORNERS, Corners::from_code).unwrap_or_default()),
            font: signal(stored(K_FONT, UiFont::from_code).unwrap_or_default()),
            theme: signal(stored(K_THEME, ThemeMode::from_code).unwrap_or_default()),
            digits: signal(
                stored(K_DIGITS, |s| match s {
                    "arab" => Some(Digits::ArabicIndic),
                    "latn" => Some(Digits::Latin),
                    _ => None,
                })
                .unwrap_or_default(),
            ),
        }
    }

    /// The look the tokens follow (reading it subscribes).
    pub fn look(&self) -> Look {
        Look {
            accent: self.accent.get(),
            text: self.text.get(),
            density: self.density.get(),
            corners: self.corners.get(),
            font: self.font.get(),
        }
    }

    /// Back to the defaults.
    pub fn reset(&self) {
        self.accent.set(Accent::default());
        self.text.set(TextSize::default());
        self.density.set(Density::default());
        self.corners.set(Corners::default());
        self.font.set(UiFont::default());
        self.theme.set(ThemeMode::default());
        self.digits.set(Digits::default());
    }
}

/// A text setting saved on the device whenever it changes.
pub fn persisted_text(key: &'static str, default: &str) -> Signal<String> {
    let s = signal(Storage::get_item(key).unwrap_or_else(|| default.to_owned()));
    effect(move || save(key, &s.get()));
    s
}

/// A choice (by index) saved on the device.
pub fn persisted_index(key: &'static str, default: usize) -> Signal<usize> {
    let s = signal(stored(key, |v| v.parse().ok()).unwrap_or(default));
    effect(move || save(key, &s.get().to_string()));
    s
}

/// An on/off setting saved on the device.
pub fn persisted_flag(key: &'static str, default: bool) -> Signal<bool> {
    let s = signal(stored(key, |v| v.parse().ok()).unwrap_or(default));
    effect(move || save(key, if s.get() { "true" } else { "false" }));
    s
}

/// Store and device settings. Kept on the device for now; they move to the
/// organization's database with M2 (and sync from there).
#[derive(Clone, Copy)]
pub struct BizPrefs {
    pub start_page: Signal<usize>,
    pub confirm_delete: Signal<bool>,
    pub show_cents: Signal<bool>,
    pub store_name: Signal<String>,
    pub activity: Signal<usize>,
    pub phone: Signal<String>,
    pub address: Signal<String>,
    pub nif: Signal<String>,
    pub nis: Signal<String>,
    pub rc: Signal<String>,
    pub ai: Signal<String>,
    pub prices_incl_tax: Signal<bool>,
    pub tva_rate: Signal<usize>,
    pub stamp_duty: Signal<bool>,
    pub tax_on_receipt: Signal<bool>,
    pub receipt_width: Signal<usize>,
    pub printer: Signal<usize>,
    pub print_logo: Signal<bool>,
    pub auto_print: Signal<bool>,
    pub copies: Signal<usize>,
    pub scale: Signal<usize>,
    pub scanner: Signal<bool>,
    pub drawer: Signal<bool>,
}

impl BizPrefs {
    fn load() -> Self {
        Self {
            start_page: persisted_index("nb.app.start", 0),
            confirm_delete: persisted_flag("nb.app.confirm_delete", true),
            show_cents: persisted_flag("nb.fmt.cents", true),
            store_name: persisted_text("nb.store.name", ""),
            activity: persisted_index("nb.store.activity", 0),
            phone: persisted_text("nb.store.phone", ""),
            address: persisted_text("nb.store.address", ""),
            nif: persisted_text("nb.store.nif", ""),
            nis: persisted_text("nb.store.nis", ""),
            rc: persisted_text("nb.store.rc", ""),
            ai: persisted_text("nb.store.ai", ""),
            prices_incl_tax: persisted_flag("nb.tax.included", true),
            tva_rate: persisted_index("nb.tax.rate", 0),
            stamp_duty: persisted_flag("nb.tax.stamp", true),
            tax_on_receipt: persisted_flag("nb.tax.on_receipt", true),
            receipt_width: persisted_index("nb.print.width", 1),
            printer: persisted_index("nb.print.printer", 0),
            print_logo: persisted_flag("nb.print.logo", true),
            auto_print: persisted_flag("nb.print.auto", false),
            copies: persisted_index("nb.print.copies", 0),
            scale: persisted_index("nb.device.scale", 0),
            scanner: persisted_flag("nb.device.scanner", true),
            drawer: persisted_flag("nb.device.drawer", false),
        }
    }
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub lang: Signal<Lang>,
    pub prefs: Prefs,
    pub biz: BizPrefs,
    pub route: Signal<Route>,
    /// Sidebar sections that are expanded (bit per section).
    pub open_sections: Signal<u32>,
    /// The settings page's open tab.
    pub settings_tab: Signal<usize>,
    /// The confirmation shown at the bottom of the screen (`ui::overlay::show_toast`).
    pub toast: Signal<Option<String>>,
    /// The command palette is open.
    pub palette: Signal<bool>,
    /// A pending request for the screen being opened.
    pub intent: Signal<Option<Intent>>,
}

impl AppState {
    pub fn new() -> Self {
        let lang = stored(K_LANG, Lang::from_tag).unwrap_or_else(|| {
            locale()
                .as_str()
                .split(['-', '_'])
                .next()
                .and_then(Lang::from_tag)
                .unwrap_or(Lang::Ar)
        });
        let st = Self {
            lang: signal(lang),
            prefs: Prefs::load(),
            biz: BizPrefs::load(),
            route: signal(Route::Home),
            open_sections: signal(0),
            settings_tab: signal(0),
            toast: signal(None),
            palette: signal(false),
            intent: signal(None),
        };
        if st.biz.start_page.get_untracked() == 1 {
            st.route.set(Route::Sell);
        }
        st
    }

    /// Saves every setting when it changes, and applies the appearance.
    pub fn persist(self) {
        let p = self.prefs;
        effect(move || save(K_LANG, self.lang.get().code()));
        effect(move || save(K_ACCENT, p.accent.get().code()));
        effect(move || save(K_TEXT, p.text.get().code()));
        effect(move || save(K_DENSITY, p.density.get().code()));
        effect(move || save(K_CORNERS, p.corners.get().code()));
        effect(move || save(K_FONT, p.font.get().code()));
        effect(move || {
            let mode = p.theme.get();
            save(K_THEME, mode.code());
            set_color_scheme(mode.appearance());
        });
        effect(move || {
            save(
                K_DIGITS,
                match p.digits.get() {
                    Digits::ArabicIndic => "arab",
                    Digits::Latin => "latn",
                },
            )
        });
    }

    pub fn lang(&self) -> Lang {
        self.lang.get()
    }

    pub fn fmt(&self) -> Format {
        Format {
            lang: self.lang.get(),
            digits: self.prefs.digits.get(),
        }
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

    /// A translated string now (for one-off labels built inside a reactive scope).
    pub fn tn(self, key: &str) -> String {
        tr(self.lang.get(), key).into_owned()
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

    /// Light ↔ dark, starting from what is shown.
    pub fn toggle_appearance(self) {
        self.prefs.theme.set(if dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        });
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

//! Languages, translations and locale-aware formatting for NSH Business.
//!
//! - [`Lang`]: Arabic (RTL), French, English; direction follows the language.
//! - [`tr`] / [`tr_args`]: messages from `locales/*.ftl` (`key = value`, `{name}` placeholders),
//!   compiled into the binary; a missing key falls back to English, then to the key itself.
//! - [`Format`]: money, quantities, numbers and dates per language, already bidi-safe.
//! - [`bidi`]: isolation of mixed-direction runs for renderers with limited bidi.

pub mod bidi;
mod format;
mod messages;

pub use bidi::Direction;
pub use format::{Digits, Format};
pub use messages::{all_keys, missing_keys, tr, tr_args};

/// A supported interface language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Ar,
    Fr,
    En,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Ar, Lang::Fr, Lang::En];

    /// BCP 47 code.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ar => "ar",
            Lang::Fr => "fr",
            Lang::En => "en",
        }
    }

    /// The language's own name, as shown in the language picker.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Ar => "العربية",
            Lang::Fr => "Français",
            Lang::En => "English",
        }
    }

    pub fn direction(self) -> Direction {
        match self {
            Lang::Ar => Direction::Rtl,
            Lang::Fr | Lang::En => Direction::Ltr,
        }
    }

    /// From a BCP 47 tag such as `ar-DZ` or `fr-FR`; unknown languages give `None`.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        match tag.split(['-', '_']).next()?.to_ascii_lowercase().as_str() {
            "ar" => Some(Lang::Ar),
            "fr" => Some(Lang::Fr),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
}

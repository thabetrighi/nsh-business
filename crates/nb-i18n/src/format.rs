//! Locale-aware display of money, quantities, numbers and dates.
//!
//! Output is display-only and bidi-safe: numbers are isolated (LRI…PDI) so
//! they keep their order inside Arabic text on every renderer.

use nb_core::{Currency, Money, Quantity};

use crate::Lang;
use crate::bidi::isolate_ltr;

/// Which digit shapes to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Digits {
    /// 0-9: the usage in Algeria and the Maghreb (default).
    #[default]
    Latin,
    /// ٠-٩: Arabic-Indic digits, as used in the Mashriq.
    ArabicIndic,
}

/// Formatting rules for one language.
#[derive(Debug, Clone, Copy)]
pub struct Format {
    pub lang: Lang,
    pub digits: Digits,
}

/// Thousands separator for ar/fr. U+00A0 (not U+202F): the bundled UI font has
/// no U+202F glyph, and the web would fall back to a near-zero width.
const GROUP_SPACE: char = '\u{00A0}';

impl Format {
    pub fn new(lang: Lang) -> Self {
        Self {
            lang,
            digits: Digits::Latin,
        }
    }

    fn group_sep(&self) -> char {
        match self.lang {
            Lang::Ar | Lang::Fr => GROUP_SPACE,
            Lang::En => ',',
        }
    }

    fn decimal_sep(&self) -> char {
        match self.lang {
            Lang::Fr => ',',
            Lang::Ar | Lang::En => '.',
        }
    }

    fn shape_digits(&self, s: String) -> String {
        match self.digits {
            Digits::Latin => s,
            Digits::ArabicIndic => s
                .chars()
                .map(|c| match c.to_digit(10) {
                    Some(d) => char::from_u32(0x0660 + d).unwrap_or(c),
                    None => c,
                })
                .collect(),
        }
    }

    /// `1234567` -> `1 234 567` (grouped, unsigned digits only).
    fn group(&self, digits: &str) -> String {
        let sep = self.group_sep();
        let n = digits.len();
        let mut out = String::with_capacity(n + n / 3);
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (n - i).is_multiple_of(3) {
                out.push(sep);
            }
            out.push(c);
        }
        out
    }

    /// The bare number of an amount, grouped and with the currency's decimals: `1 234.50`.
    pub fn amount(&self, m: Money) -> String {
        self.amount_with(m, false)
    }

    /// Like [`Self::money`] but with an explicit `+` for positive amounts, for
    /// ledger lines. The sign sits inside the number's isolate, so it stays on
    /// the number's left in Arabic too.
    pub fn money_signed(&self, m: Money) -> String {
        self.with_symbol(self.amount_with(m, true), m)
    }

    fn amount_with(&self, m: Money, plus: bool) -> String {
        let plain = m.to_plain_string();
        let (neg, plain) = match plain.strip_prefix('-') {
            Some(rest) => (true, rest.to_owned()),
            None => (false, plain),
        };
        let (int, frac) = plain.split_once('.').unwrap_or((&plain, ""));
        let mut s = String::new();
        if neg {
            s.push('\u{2212}');
        } else if plus && !m.is_zero() {
            s.push('+');
        }
        s.push_str(&self.group(int));
        if !frac.is_empty() {
            s.push(self.decimal_sep());
            s.push_str(frac);
        }
        isolate_ltr(&self.shape_digits(s))
    }

    /// The currency's local symbol.
    pub fn currency_symbol(&self, c: Currency) -> String {
        match (c.code(), self.lang) {
            ("DZD", Lang::Ar) => "دج".to_owned(),
            ("DZD", _) => "DA".to_owned(),
            ("EUR", _) => "€".to_owned(),
            ("USD", _) => "$".to_owned(),
            (code, _) => code.to_owned(),
        }
    }

    /// An amount with its currency: `1 234.50 دج`, `1 234,50 DA`, `DA 1,234.50`.
    pub fn money(&self, m: Money) -> String {
        self.with_symbol(self.amount(m), m)
    }

    fn with_symbol(&self, amount: String, m: Money) -> String {
        let symbol = self.currency_symbol(m.currency());
        match self.lang {
            Lang::Ar | Lang::Fr => format!("{amount}\u{00A0}{symbol}"),
            Lang::En => format!("{symbol}\u{00A0}{amount}"),
        }
    }

    /// A quantity with at most `dp` decimals and no trailing zeros: `1.25`, `3`.
    pub fn quantity(&self, q: Quantity, dp: u32) -> String {
        let s = q.round_to(dp).to_string();
        let (neg, s) = match s.strip_prefix('-') {
            Some(rest) => (true, rest.to_owned()),
            None => (false, s),
        };
        let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
        let mut out = String::new();
        if neg {
            out.push('-');
        }
        out.push_str(&self.group(int));
        if !frac.is_empty() {
            out.push(self.decimal_sep());
            out.push_str(frac);
        }
        isolate_ltr(&self.shape_digits(out))
    }

    /// A count: `12 345`.
    pub fn integer(&self, n: i64) -> String {
        let digits = n.unsigned_abs().to_string();
        let sign = if n < 0 { "-" } else { "" };
        isolate_ltr(&self.shape_digits(format!("{sign}{}", self.group(&digits))))
    }

    /// A calendar date as `dd/mm/yyyy` (the usage in Algeria and France).
    pub fn date(&self, year: i32, month: u32, day: u32) -> String {
        let s = match self.lang {
            Lang::Ar | Lang::Fr => format!("{day:02}/{month:02}/{year:04}"),
            Lang::En => format!("{day:02}/{month:02}/{year:04}"),
        };
        isolate_ltr(&self.shape_digits(s))
    }

    /// A time of day as `HH:MM`.
    pub fn time(&self, hour: u32, minute: u32) -> String {
        isolate_ltr(&self.shape_digits(format!("{hour:02}:{minute:02}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bidi::strip_controls;

    fn plain(s: String) -> String {
        strip_controls(&s)
            .replace(GROUP_SPACE, " ")
            .replace('\u{2212}', "-")
    }

    #[test]
    fn money_per_language() {
        let m = Money::from_minor(123_450, Currency::DZD);
        assert_eq!(plain(Format::new(Lang::Ar).money(m)), "1 234.50 دج");
        assert_eq!(plain(Format::new(Lang::Fr).money(m)), "1 234,50 DA");
        assert_eq!(plain(Format::new(Lang::En).money(m)), "DA 1,234.50");
        assert_eq!(
            plain(Format::new(Lang::Ar).money(Money::from_minor(-5, Currency::DZD))),
            "-0.05 دج"
        );
        assert_eq!(
            plain(Format::new(Lang::Fr).money(Money::from_minor(100_000_000, Currency::DZD))),
            "1 000 000,00 DA"
        );
    }

    #[test]
    fn digits_and_quantities() {
        let ar = Format {
            lang: Lang::Ar,
            digits: Digits::ArabicIndic,
        };
        assert_eq!(
            plain(ar.money(Money::from_minor(12_050, Currency::DZD))),
            "١٢٠.٥٠ دج"
        );
        let fr = Format::new(Lang::Fr);
        assert_eq!(plain(fr.quantity(Quantity::from_milli(1_250), 3)), "1,25");
        assert_eq!(plain(fr.quantity(Quantity::whole(12_000), 0)), "12 000");
        assert_eq!(plain(fr.integer(-1234)), "-1 234");
        assert_eq!(plain(fr.date(2026, 11, 10)), "10/11/2026");
        assert_eq!(plain(fr.time(9, 5)), "09:05");
    }

    #[test]
    fn signed_ledger_amounts() {
        let ar = Format::new(Lang::Ar);
        let m = |x| Money::from_minor(x, Currency::DZD);
        assert_eq!(plain(ar.money_signed(m(3_915_000))), "+39 150.00 دج");
        assert_eq!(plain(ar.money_signed(m(-185_000))), "-1 850.00 دج");
        assert_eq!(plain(ar.money_signed(m(0))), "0.00 دج");
    }

    #[test]
    fn numbers_are_isolated() {
        let s = Format::new(Lang::Ar).money(Money::from_minor(100, Currency::DZD));
        assert!(s.starts_with('\u{2066}') && s.contains('\u{2069}'));
    }
}

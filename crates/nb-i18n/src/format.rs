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

    /// An amount rounded to whole units (half away from zero), with its
    /// currency: `1 284 500 دج`. For headline figures where centimes are noise.
    pub fn money_whole(&self, m: Money) -> String {
        let per = m.currency().minor_per_major().max(1);
        let minor = m.minor();
        let half = if minor < 0 { -(per / 2) } else { per / 2 };
        let whole = minor.saturating_add(half) / per;
        let digits = whole.unsigned_abs().to_string();
        let mut s = String::new();
        if whole < 0 {
            s.push('\u{2212}');
        }
        s.push_str(&self.group(&digits));
        self.with_symbol(isolate_ltr(&self.shape_digits(s)), m)
    }

    /// A short amount for chart axes, in whole units: `850`, `12 k`, `1,2 M`
    /// (`ألف` / `مليون` in Arabic). No currency.
    pub fn compact(&self, whole_units: i64) -> String {
        let n = whole_units.unsigned_abs();
        let sign = if whole_units < 0 { "\u{2212}" } else { "" };
        // Tenths, rounded half up, from integers only.
        let tenths = |div: u64| (n * 10 + div / 2) / div;
        let (num, unit) = if n >= 1_000_000 {
            (tenths(1_000_000), 2)
        } else if n >= 1_000 {
            (tenths(1_000), 1)
        } else {
            (n * 10, 0)
        };
        let int = (num / 10).to_string();
        let frac = num % 10;
        let mut body = format!("{sign}{}", self.group(&int));
        if frac != 0 && num < 100 {
            body.push(self.decimal_sep());
            body.push_str(&frac.to_string());
        }
        let body = isolate_ltr(&self.shape_digits(body));
        let suffix = match (unit, self.lang) {
            (0, _) => "",
            (1, Lang::Ar) => "\u{00A0}ألف",
            (2, Lang::Ar) => "\u{00A0}مليون",
            (1, Lang::Fr) => "\u{00A0}k",
            (2, Lang::Fr) => "\u{00A0}M",
            (1, _) => "K",
            _ => "M",
        };
        format!("{body}{suffix}")
    }

    /// A signed percentage from basis points, one decimal: `+12.4%`,
    /// `−3,1 %` in French. The sign stays with the number in Arabic.
    pub fn percent_signed(&self, bps: i64) -> String {
        let n = bps.unsigned_abs();
        let tenths = (n + 5) / 10;
        let mut s = String::new();
        // The sign follows what is shown: −0.04 % prints as 0.0 %.
        if tenths > 0 && bps < 0 {
            s.push('\u{2212}');
        } else if tenths > 0 {
            s.push('+');
        }
        s.push_str(&(tenths / 10).to_string());
        s.push(self.decimal_sep());
        s.push_str(&(tenths % 10).to_string());
        match self.lang {
            Lang::Fr => s.push_str("\u{00A0}%"),
            Lang::Ar | Lang::En => s.push('%'),
        }
        isolate_ltr(&self.shape_digits(s))
    }

    /// A short month name (1–12), for chart axes. Arabic uses the month
    /// names of Algeria and the Maghreb (جانفي، فيفري…).
    pub fn month_short(&self, month: u32) -> &'static str {
        const AR: [&str; 12] = [
            "جانفي",
            "فيفري",
            "مارس",
            "أفريل",
            "ماي",
            "جوان",
            "جويلية",
            "أوت",
            "سبتمبر",
            "أكتوبر",
            "نوفمبر",
            "ديسمبر",
        ];
        const FR: [&str; 12] = [
            "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.",
            "nov.", "déc.",
        ];
        const EN: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let i = (month.clamp(1, 12) - 1) as usize;
        match self.lang {
            Lang::Ar => AR[i],
            Lang::Fr => FR[i],
            Lang::En => EN[i],
        }
    }

    /// Day and month, for chart axes: `07/11`.
    pub fn day_month(&self, month: u32, day: u32) -> String {
        isolate_ltr(&self.shape_digits(format!("{day:02}/{month:02}")))
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

    #[test]
    fn whole_money_rounds_half_away_from_zero() {
        let f = Format::new(Lang::Fr);
        let m = |c| Money::from_minor(c, Currency::DZD);
        assert_eq!(
            strip_controls(&f.money_whole(m(128_450_050))),
            "1\u{a0}284\u{a0}501\u{a0}DA"
        );
        assert_eq!(strip_controls(&f.money_whole(m(149))), "1\u{a0}DA");
        assert_eq!(strip_controls(&f.money_whole(m(-150))), "\u{2212}2\u{a0}DA");
        let ar = Format::new(Lang::Ar);
        assert_eq!(
            strip_controls(&ar.money_whole(m(100_000))),
            "1\u{a0}000\u{a0}دج"
        );
    }

    #[test]
    fn compact_amounts_for_axes() {
        let fr = Format::new(Lang::Fr);
        let en = Format::new(Lang::En);
        let ar = Format::new(Lang::Ar);
        assert_eq!(strip_controls(&fr.compact(850)), "850");
        assert_eq!(strip_controls(&fr.compact(1_250)), "1,3\u{a0}k");
        assert_eq!(strip_controls(&en.compact(12_400)), "12K");
        assert_eq!(strip_controls(&en.compact(1_250_000)), "1.3M");
        assert_eq!(strip_controls(&ar.compact(250_000)), "250\u{a0}ألف");
        assert_eq!(strip_controls(&ar.compact(0)), "0");
    }

    #[test]
    fn signed_percentages() {
        assert_eq!(
            strip_controls(&Format::new(Lang::En).percent_signed(1_244)),
            "+12.4%"
        );
        assert_eq!(
            strip_controls(&Format::new(Lang::Fr).percent_signed(-310)),
            "\u{2212}3,1\u{a0}%"
        );
        assert_eq!(
            strip_controls(&Format::new(Lang::Ar).percent_signed(0)),
            "0.0%"
        );
        assert_eq!(
            strip_controls(&Format::new(Lang::En).percent_signed(-4)),
            "0.0%"
        );
    }

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

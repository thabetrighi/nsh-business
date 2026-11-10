//! Barcodes (PRD §6.5): GTIN check digits (EAN-13, EAN-8, UPC-A) and the
//! barcodes printed by scales, where the article code and its weight or
//! price are embedded in an EAN-13 (`2 PPPPP VVVVV C`). Every scale brand
//! uses its own layout, so the layout is data ([`ScaleFormat`]).

use nb_core::{Currency, Decimal, Money, Quantity};

use crate::RuleError;

/// The GTIN check digit of `body` (the digits before it).
pub fn gtin_check_digit(body: &str) -> Option<u32> {
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // From the right, digits alternate weights 3 and 1.
    let sum: u32 = body
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    Some((10 - sum % 10) % 10)
}

/// Whether `code` is a GTIN-8/12/13/14 with a correct check digit.
pub fn is_valid_gtin(code: &str) -> bool {
    let n = code.len();
    if !matches!(n, 8 | 12 | 13 | 14) || !code.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let (body, last) = code.split_at(n - 1);
    gtin_check_digit(body) == last.parse().ok()
}

/// A barcode as entered for a product: trimmed, and validated when it looks
/// like a GTIN (internal codes of letters and digits are allowed as they are).
pub fn normalize(code: &str) -> Result<String, RuleError> {
    let c = code.trim();
    if c.is_empty() || c.len() > 48 {
        return Err(RuleError::on("barcode", "barcode.length"));
    }
    if c.chars().any(char::is_whitespace) {
        return Err(RuleError::on("barcode", "barcode.spaces"));
    }
    let numeric = c.bytes().all(|b| b.is_ascii_digit());
    if numeric && matches!(c.len(), 8 | 12 | 13 | 14) && !is_valid_gtin(c) {
        return Err(RuleError::on("barcode", "barcode.check_digit"));
    }
    Ok(c.to_owned())
}

/// What the value digits of a scale barcode carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleValue {
    /// A weight, with `decimals` (3 for grams of a kilogram).
    Weight { decimals: u32 },
    /// A price in minor units with `decimals` relative to the currency's
    /// minor unit (0: the digits are centimes; -2 would be whole dinars).
    Price { major_decimals: u32 },
}

/// One scale label layout, configurable per store (PRD §15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleFormat {
    /// Leading digits that mark a scale label (`"2"`, `"21"`, `"28"`).
    pub prefix: String,
    /// Digits of the article code after the prefix.
    pub item_digits: usize,
    /// Digits of the value after the article code.
    pub value_digits: usize,
    pub value: ScaleValue,
}

/// What a scale barcode says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScaleRead {
    Weight { item: String, quantity: Quantity },
    Price { item: String, amount: Money },
}

impl ScaleFormat {
    /// The common layout `2 IIIII WWWWW C`: 5-digit article, weight in grams.
    pub fn weight_ean13(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            item_digits: 12 - prefix.len() - 5,
            value_digits: 5,
            value: ScaleValue::Weight { decimals: 3 },
        }
    }

    /// Checks the layout fits an EAN-13 (12 digits before the check digit).
    pub fn validate(&self) -> Result<(), RuleError> {
        let used = self.prefix.len() + self.item_digits + self.value_digits;
        if self.prefix.is_empty()
            || !self.prefix.bytes().all(|b| b.is_ascii_digit())
            || self.item_digits == 0
            || self.value_digits == 0
            || used > 12
        {
            return Err(RuleError::on("scale_format", "scale.layout"));
        }
        Ok(())
    }

    /// Reads `code` if it is a valid EAN-13 in this layout.
    pub fn read(&self, code: &str, currency: Currency) -> Option<ScaleRead> {
        if self.validate().is_err() || code.len() != 13 || !is_valid_gtin(code) {
            return None;
        }
        let rest = code.strip_prefix(&self.prefix)?;
        let item = rest.get(..self.item_digits)?.to_owned();
        let digits = rest.get(self.item_digits..self.item_digits + self.value_digits)?;
        let raw: i64 = digits.parse().ok()?;
        Some(match self.value {
            ScaleValue::Weight { decimals } => ScaleRead::Weight {
                item,
                quantity: Quantity::new(Decimal::new(raw, decimals)),
            },
            ScaleValue::Price { major_decimals } => {
                // `raw` has `major_decimals` decimals of the major unit;
                // bring it to minor units.
                let exp = i64::from(currency.exponent()) - i64::from(major_decimals);
                let minor = if exp >= 0 {
                    raw.checked_mul(10i64.checked_pow(u32::try_from(exp).ok()?)?)?
                } else {
                    raw / 10i64.checked_pow(u32::try_from(-exp).ok()?)?
                };
                ScaleRead::Price {
                    item,
                    amount: Money::from_minor(minor, currency),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_digits() {
        assert_eq!(gtin_check_digit("613000012345"), Some(5));
        assert!(is_valid_gtin("6130000123455"));
        assert!(!is_valid_gtin("6130000123454"));
        assert!(is_valid_gtin("96385074")); // EAN-8
        assert!(is_valid_gtin("036000291452")); // UPC-A
        assert!(!is_valid_gtin("abc"));
        assert_eq!(gtin_check_digit(""), None);
    }

    #[test]
    fn product_barcodes() {
        assert_eq!(normalize(" 6130000123455 "), Ok("6130000123455".into()));
        assert_eq!(
            normalize("6130000123454"),
            Err(RuleError::on("barcode", "barcode.check_digit"))
        );
        assert_eq!(normalize("INT-0042"), Ok("INT-0042".into()));
        assert!(normalize("").is_err());
        assert!(normalize("A B").is_err());
    }

    fn with_check(body: &str) -> String {
        format!("{body}{}", gtin_check_digit(body).unwrap_or(0))
    }

    #[test]
    fn weighed_labels() {
        let f = ScaleFormat::weight_ean13("2");
        assert_eq!(f.item_digits, 6);
        assert!(
            f.read(&with_check("200042"), Currency::DZD).is_none(),
            "too short"
        );
        let code = with_check(&format!("2{}{}", "000042", "01250"));
        assert_eq!(
            f.read(&code, Currency::DZD),
            Some(ScaleRead::Weight {
                item: "000042".into(),
                quantity: Quantity::from_milli(1_250),
            })
        );
        // A wrong check digit is not read.
        let bad = format!("{}{}", &code[..12], (code.as_bytes()[12] - b'0' + 1) % 10);
        assert!(f.read(&bad, Currency::DZD).is_none());
        // Another prefix is not this format.
        assert!(f.read(&with_check("390004201250"), Currency::DZD).is_none());
    }

    #[test]
    fn priced_labels() {
        // 22 IIII PPPPPP C: price in dinars with 2 decimals (centimes).
        let f = ScaleFormat {
            prefix: "22".into(),
            item_digits: 4,
            value_digits: 6,
            value: ScaleValue::Price { major_decimals: 2 },
        };
        let code = with_check("221234045075");
        assert_eq!(
            f.read(&code, Currency::DZD),
            Some(ScaleRead::Price {
                item: "1234".into(),
                amount: Money::from_minor(45_075, Currency::DZD),
            })
        );
        // Whole dinars on the label.
        let f0 = ScaleFormat {
            value: ScaleValue::Price { major_decimals: 0 },
            ..f
        };
        assert_eq!(
            f0.read(&code, Currency::DZD),
            Some(ScaleRead::Price {
                item: "1234".into(),
                amount: Money::from_minor(4_507_500, Currency::DZD),
            })
        );
    }

    #[test]
    fn layouts_must_fit() {
        let mut f = ScaleFormat::weight_ean13("2");
        assert!(f.validate().is_ok());
        f.value_digits = 9;
        assert!(f.validate().is_err());
    }
}

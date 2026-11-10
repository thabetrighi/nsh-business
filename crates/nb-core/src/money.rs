//! Money as an integer count of the currency's minor unit.
//!
//! No floating point anywhere: amounts are `i64` minor units (centimes for
//! DZD). Arithmetic is checked; mixing currencies is an error, not a panic.

use std::fmt;

use crate::error::CoreError;

/// An ISO 4217 currency with its minor-unit exponent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Currency {
    code: [u8; 3],
    exponent: u8,
}

impl Currency {
    /// Algerian dinar: 2 decimals (centimes).
    pub const DZD: Currency = Currency {
        code: *b"DZD",
        exponent: 2,
    };
    /// Euro.
    pub const EUR: Currency = Currency {
        code: *b"EUR",
        exponent: 2,
    };
    /// US dollar.
    pub const USD: Currency = Currency {
        code: *b"USD",
        exponent: 2,
    };

    /// A currency from an uppercase ASCII code and its exponent (0..=4).
    pub fn new(code: &str, exponent: u8) -> Result<Self, CoreError> {
        let bytes = code.as_bytes();
        if bytes.len() != 3 || !bytes.iter().all(u8::is_ascii_uppercase) || exponent > 4 {
            return Err(CoreError::InvalidCurrency(code.to_owned()));
        }
        Ok(Self {
            code: [bytes[0], bytes[1], bytes[2]],
            exponent,
        })
    }

    /// The ISO code, e.g. `"DZD"`.
    pub fn code(&self) -> &str {
        // The constructor only accepts ASCII uppercase, so this is valid UTF-8.
        std::str::from_utf8(&self.code).unwrap_or("???")
    }

    /// Digits after the decimal point.
    pub fn exponent(&self) -> u8 {
        self.exponent
    }

    /// Minor units per major unit (100 for DZD).
    pub fn minor_per_major(&self) -> i64 {
        10_i64.pow(u32::from(self.exponent))
    }
}

/// An amount of money in a currency's minor unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Money {
    minor: i64,
    currency: Currency,
}

impl Money {
    /// An amount from minor units (`Money::from_minor(12_050, DZD)` is 120.50 DA).
    pub const fn from_minor(minor: i64, currency: Currency) -> Self {
        Self { minor, currency }
    }

    /// Zero in `currency`.
    pub const fn zero(currency: Currency) -> Self {
        Self { minor: 0, currency }
    }

    /// Parses a decimal string such as `"1234.5"` or `"-0.05"` exactly.
    /// More decimals than the currency allows is an error, never a silent rounding.
    pub fn parse(s: &str, currency: Currency) -> Result<Self, CoreError> {
        let bad = || CoreError::InvalidAmount(s.to_owned());
        let t = s.trim();
        let (neg, digits) = match t.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
        if int.is_empty() && frac.is_empty() {
            return Err(bad());
        }
        if !int.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        let exp = usize::from(currency.exponent);
        if frac.len() > exp {
            return Err(CoreError::TooManyDecimals {
                value: s.to_owned(),
                allowed: currency.exponent,
            });
        }
        let int_v: i64 = if int.is_empty() {
            0
        } else {
            int.parse().map_err(|_| bad())?
        };
        let mut frac_s = frac.to_owned();
        while frac_s.len() < exp {
            frac_s.push('0');
        }
        let frac_v: i64 = if frac_s.is_empty() {
            0
        } else {
            frac_s.parse().map_err(|_| bad())?
        };
        let minor = int_v
            .checked_mul(currency.minor_per_major())
            .and_then(|v| v.checked_add(frac_v))
            .ok_or(CoreError::Overflow)?;
        Ok(Self {
            minor: if neg { -minor } else { minor },
            currency,
        })
    }

    pub const fn minor(&self) -> i64 {
        self.minor
    }

    pub const fn currency(&self) -> Currency {
        self.currency
    }

    pub const fn is_zero(&self) -> bool {
        self.minor == 0
    }

    pub const fn is_negative(&self) -> bool {
        self.minor < 0
    }

    fn same(&self, other: &Money) -> Result<(), CoreError> {
        if self.currency == other.currency {
            Ok(())
        } else {
            Err(CoreError::CurrencyMismatch {
                left: self.currency.code().to_owned(),
                right: other.currency.code().to_owned(),
            })
        }
    }

    pub fn checked_add(self, other: Money) -> Result<Money, CoreError> {
        self.same(&other)?;
        let minor = self
            .minor
            .checked_add(other.minor)
            .ok_or(CoreError::Overflow)?;
        Ok(Money { minor, ..self })
    }

    pub fn checked_sub(self, other: Money) -> Result<Money, CoreError> {
        self.same(&other)?;
        let minor = self
            .minor
            .checked_sub(other.minor)
            .ok_or(CoreError::Overflow)?;
        Ok(Money { minor, ..self })
    }

    pub fn checked_neg(self) -> Result<Money, CoreError> {
        let minor = self.minor.checked_neg().ok_or(CoreError::Overflow)?;
        Ok(Money { minor, ..self })
    }

    /// Multiplies by an integer count.
    pub fn checked_mul(self, n: i64) -> Result<Money, CoreError> {
        let minor = self.minor.checked_mul(n).ok_or(CoreError::Overflow)?;
        Ok(Money { minor, ..self })
    }

    /// Multiplies by `numerator / denominator`, rounding half away from zero.
    /// Used for quantities (`qty_milli / 1000`) and rates (`basis_points / 10_000`).
    pub fn mul_ratio(self, numerator: i64, denominator: i64) -> Result<Money, CoreError> {
        if denominator == 0 {
            return Err(CoreError::DivisionByZero);
        }
        let n = i128::from(self.minor) * i128::from(numerator);
        let d = i128::from(denominator);
        let q = n / d;
        let r = n % d;
        // Round half away from zero.
        let rounded = if 2 * r.abs() >= d.abs() {
            q + n.signum() * d.signum()
        } else {
            q
        };
        let minor = i64::try_from(rounded).map_err(|_| CoreError::Overflow)?;
        Ok(Money { minor, ..self })
    }

    /// Splits the amount into parts proportional to `weights`, so that the
    /// parts always sum to exactly this amount (largest-remainder method).
    /// Used for installments, landed cost and invoice-level discounts.
    pub fn allocate(self, weights: &[u64]) -> Result<Vec<Money>, CoreError> {
        let total: u128 = weights.iter().map(|&w| u128::from(w)).sum();
        if weights.is_empty() || total == 0 {
            return Err(CoreError::EmptyAllocation);
        }
        let sign: i128 = if self.minor < 0 { -1 } else { 1 };
        let amount = i128::from(self.minor).unsigned_abs();
        let mut parts: Vec<u128> = Vec::with_capacity(weights.len());
        let mut remainders: Vec<(u128, usize)> = Vec::with_capacity(weights.len());
        let mut assigned: u128 = 0;
        for (i, &w) in weights.iter().enumerate() {
            let exact = amount * u128::from(w);
            let share = exact / total;
            parts.push(share);
            remainders.push((exact % total, i));
            assigned += share;
        }
        // Hand out the leftover minor units to the largest remainders, earliest first on ties.
        remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let leftover = amount - assigned;
        for &(_, i) in remainders
            .iter()
            .take(usize::try_from(leftover).map_err(|_| CoreError::Overflow)?)
        {
            parts[i] += 1;
        }
        parts
            .into_iter()
            .map(|p| {
                let v = i128::try_from(p).map_err(|_| CoreError::Overflow)? * sign;
                let minor = i64::try_from(v).map_err(|_| CoreError::Overflow)?;
                Ok(Money { minor, ..self })
            })
            .collect()
    }

    /// Splits into `n` near-equal parts (earlier parts take the extra minor units).
    pub fn split_even(self, n: usize) -> Result<Vec<Money>, CoreError> {
        self.allocate(&vec![1; n])
    }

    /// Sums amounts of one currency.
    pub fn sum<'a>(
        currency: Currency,
        items: impl IntoIterator<Item = &'a Money>,
    ) -> Result<Money, CoreError> {
        items
            .into_iter()
            .try_fold(Money::zero(currency), |acc, m| acc.checked_add(*m))
    }

    /// Plain decimal rendering without grouping, e.g. `-1234.50`.
    /// Locale-aware display lives in `nb-i18n`.
    pub fn to_plain_string(&self) -> String {
        let exp = u32::from(self.currency.exponent);
        let per = self.currency.minor_per_major().unsigned_abs();
        let abs = self.minor.unsigned_abs();
        let sign = if self.minor < 0 { "-" } else { "" };
        if exp == 0 {
            format!("{sign}{abs}")
        } else {
            format!(
                "{sign}{}.{:0width$}",
                abs / per,
                abs % per,
                width = exp as usize
            )
        }
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.to_plain_string(), self.currency.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const DZD: Currency = Currency::DZD;

    #[test]
    fn parses_exactly() -> Result<(), CoreError> {
        assert_eq!(Money::parse("1234.5", DZD)?.minor(), 123_450);
        assert_eq!(Money::parse("-0.05", DZD)?.minor(), -5);
        assert_eq!(Money::parse("7", DZD)?.minor(), 700);
        assert_eq!(Money::parse(".5", DZD)?.minor(), 50);
        assert!(matches!(
            Money::parse("1.234", DZD),
            Err(CoreError::TooManyDecimals { .. })
        ));
        assert!(Money::parse("1,5", DZD).is_err());
        assert!(Money::parse("", DZD).is_err());
        assert!(Money::parse("abc", DZD).is_err());
        Ok(())
    }

    #[test]
    fn renders_plain() {
        assert_eq!(
            Money::from_minor(-123_405, DZD).to_plain_string(),
            "-1234.05"
        );
        assert_eq!(Money::from_minor(5, DZD).to_string(), "0.05 DZD");
    }

    #[test]
    fn refuses_mixed_currencies() {
        let r = Money::from_minor(1, DZD).checked_add(Money::from_minor(1, Currency::EUR));
        assert!(matches!(r, Err(CoreError::CurrencyMismatch { .. })));
    }

    #[test]
    fn weighed_line_and_vat() -> Result<(), CoreError> {
        // 1.250 kg at 180.00 DA/kg.
        assert_eq!(
            Money::from_minor(18_000, DZD)
                .mul_ratio(1_250, 1_000)?
                .minor(),
            22_500
        );
        // 19% VAT included in 2251.00: 2251 * 19 / 119 = 359.4033 -> 359.40.
        assert_eq!(
            Money::from_minor(225_100, DZD).mul_ratio(19, 119)?.minor(),
            35_940
        );
        // Half away from zero, both signs.
        assert_eq!(Money::from_minor(5, DZD).mul_ratio(1, 2)?.minor(), 3);
        assert_eq!(Money::from_minor(-5, DZD).mul_ratio(1, 2)?.minor(), -3);
        Ok(())
    }

    #[test]
    fn installments_sum_to_total() -> Result<(), CoreError> {
        let parts = Money::from_minor(100_000, DZD).split_even(3)?;
        assert_eq!(
            parts.iter().map(Money::minor).collect::<Vec<_>>(),
            vec![33_334, 33_333, 33_333]
        );
        Ok(())
    }

    proptest! {
        #[test]
        fn allocation_never_loses_a_centime(
            amount in -10_000_000_000i64..10_000_000_000,
            weights in proptest::collection::vec(0u64..1_000, 1..12),
        ) {
            prop_assume!(weights.iter().any(|&w| w > 0));
            let m = Money::from_minor(amount, DZD);
            let parts = m.allocate(&weights).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(parts.len(), weights.len());
            let total: i64 = parts.iter().map(Money::minor).sum();
            prop_assert_eq!(total, amount);
            for (p, &w) in parts.iter().zip(&weights) {
                if w == 0 { prop_assert_eq!(p.minor(), 0); }
                prop_assert!(p.minor() == 0 || p.minor().signum() == amount.signum());
            }
        }

        #[test]
        fn parse_round_trips(minor in -1_000_000_000_000i64..1_000_000_000_000) {
            let m = Money::from_minor(minor, DZD);
            let back = Money::parse(&m.to_plain_string(), DZD).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(back, m);
        }
    }
}

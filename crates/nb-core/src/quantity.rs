//! Quantities and rates as exact decimals (weights, lengths, unit conversions).

use std::fmt;
use std::str::FromStr;

use rust_decimal::{Decimal, RoundingStrategy};

use crate::error::CoreError;
use crate::money::Money;

/// An exact quantity (pieces, kilograms, litres...). The unit lives with the
/// product; this type only guarantees exact arithmetic and a precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Quantity(Decimal);

impl Quantity {
    pub const ZERO: Quantity = Quantity(Decimal::ZERO);
    pub const ONE: Quantity = Quantity(Decimal::ONE);

    pub fn new(d: Decimal) -> Self {
        Self(d)
    }

    /// Whole units, e.g. 3 pieces.
    pub fn whole(n: i64) -> Self {
        Self(Decimal::from(n))
    }

    /// Thousandths, e.g. `from_milli(1_250)` is 1.250 kg. Stored precisely.
    pub fn from_milli(milli: i64) -> Self {
        Self(Decimal::new(milli, 3))
    }

    pub fn decimal(&self) -> Decimal {
        self.0
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.0.is_sign_negative() && !self.0.is_zero()
    }

    /// Rounds to `dp` decimals, half away from zero (unit precision).
    pub fn round_to(self, dp: u32) -> Self {
        Self(
            self.0
                .round_dp_with_strategy(dp, RoundingStrategy::MidpointAwayFromZero)
                .normalize(),
        )
    }

    /// True when the value has no more than `dp` decimals.
    pub fn fits_precision(&self, dp: u32) -> bool {
        self.0.normalize().scale() <= dp
    }

    pub fn checked_add(self, o: Quantity) -> Result<Quantity, CoreError> {
        self.0
            .checked_add(o.0)
            .map(Quantity)
            .ok_or(CoreError::Overflow)
    }

    pub fn checked_sub(self, o: Quantity) -> Result<Quantity, CoreError> {
        self.0
            .checked_sub(o.0)
            .map(Quantity)
            .ok_or(CoreError::Overflow)
    }

    /// Converts between units with an exact factor (1 carton = 12 boxes -> factor 12).
    pub fn convert(self, factor: Decimal) -> Result<Quantity, CoreError> {
        self.0
            .checked_mul(factor)
            .map(Quantity)
            .ok_or(CoreError::Overflow)
    }

    /// `unit_price × quantity`, rounded to the currency's minor unit (half away from zero).
    pub fn times_price(self, unit_price: Money) -> Result<Money, CoreError> {
        let exact = Decimal::from(unit_price.minor())
            .checked_mul(self.0)
            .ok_or(CoreError::Overflow)?;
        let rounded = exact.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
        let minor: i64 = rounded.try_into().map_err(|_| CoreError::Overflow)?;
        Ok(Money::from_minor(minor, unit_price.currency()))
    }
}

impl FromStr for Quantity {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::from_str_exact(s.trim())
            .map(|d| Quantity(d.normalize()))
            .map_err(|_| CoreError::InvalidQuantity(s.to_owned()))
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.normalize())
    }
}

/// A rate in basis points (1900 = 19.00%). Exact for tax and discount rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Rate {
    bps: i64,
}

impl Rate {
    pub const fn from_bps(bps: i64) -> Self {
        Self { bps }
    }

    /// From a percentage with up to two decimals, e.g. `"19"` or `"2.5"`.
    pub fn parse_percent(s: &str) -> Result<Self, CoreError> {
        let d = Decimal::from_str_exact(s.trim())
            .map_err(|_| CoreError::InvalidQuantity(s.to_owned()))?;
        let bps = d
            .checked_mul(Decimal::from(100))
            .ok_or(CoreError::Overflow)?;
        if bps.fract() != Decimal::ZERO {
            return Err(CoreError::InvalidQuantity(s.to_owned()));
        }
        Ok(Self {
            bps: bps.try_into().map_err(|_| CoreError::Overflow)?,
        })
    }

    pub const fn bps(&self) -> i64 {
        self.bps
    }

    /// Tax added on top of a net amount.
    pub fn of(self, amount: Money) -> Result<Money, CoreError> {
        amount.mul_ratio(self.bps, 10_000)
    }

    /// Tax contained in a gross (tax-inclusive) amount.
    pub fn included_in(self, gross: Money) -> Result<Money, CoreError> {
        gross.mul_ratio(self.bps, 10_000 + self.bps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::Currency;

    #[test]
    fn exact_weights_and_conversions() -> Result<(), CoreError> {
        let kg: Quantity = "1.250".parse()?;
        assert_eq!(kg, Quantity::from_milli(1_250));
        assert_eq!(kg.to_string(), "1.25");
        assert_eq!(
            kg.times_price(Money::from_minor(18_000, Currency::DZD))?
                .minor(),
            22_500
        );
        // 1 carton = 12 boxes = 72 pieces.
        let pieces = Quantity::whole(1)
            .convert(Decimal::from(12))?
            .convert(Decimal::from(6))?;
        assert_eq!(pieces, Quantity::whole(72));
        // 0.1 + 0.2 is exactly 0.3 here.
        let s = "0.1".parse::<Quantity>()?.checked_add("0.2".parse()?)?;
        assert_eq!(s, "0.3".parse()?);
        assert!(Quantity::from_milli(1_234).fits_precision(3));
        assert!(!Quantity::from_milli(1_234).fits_precision(2));
        assert_eq!(Quantity::from_milli(1_235).round_to(2), "1.24".parse()?);
        Ok(())
    }

    #[test]
    fn vat_rates() -> Result<(), CoreError> {
        let tva = Rate::parse_percent("19")?;
        assert_eq!(tva.bps(), 1_900);
        let gross = Money::from_minor(225_100, Currency::DZD);
        assert_eq!(tva.included_in(gross)?.minor(), 35_940);
        assert_eq!(
            tva.of(Money::from_minor(10_000, Currency::DZD))?.minor(),
            1_900
        );
        assert!(Rate::parse_percent("19.005").is_err());
        Ok(())
    }
}

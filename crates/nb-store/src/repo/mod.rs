//! Rows in and out. Repositories map tables to domain values; they hold no
//! business rules (those are in `nb-domain`, called by the use cases).

pub mod access;
pub mod catalog;
pub mod ledger;
pub mod org;
pub mod stock;
pub mod tracking;

use nb_core::{Decimal, Quantity};
use nb_domain::RuleError;

use crate::{Error, Result};

/// A quantity as exact thousandths (storage form; at most 3 decimals).
pub fn to_milli(q: Quantity) -> Result<i64> {
    if !q.fits_precision(3) {
        return Err(Error::from(RuleError::on("quantity", "units.too_precise")));
    }
    let m = q
        .decimal()
        .checked_mul(Decimal::from(1000))
        .ok_or(Error::from(RuleError::new("overflow")))?;
    i64::try_from(m).map_err(|_| Error::from(RuleError::new("overflow")))
}

pub fn from_milli(m: i64) -> Quantity {
    Quantity::from_milli(m)
}

/// Parses a stored id, reporting corrupt rows instead of panicking.
pub fn parse_id<T: std::str::FromStr>(s: &str) -> Result<T> {
    s.parse().map_err(|_| Error::Corrupt(format!("id {s}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milli_round_trip() -> Result<()> {
        assert_eq!(to_milli(Quantity::from_milli(1_250))?, 1_250);
        assert_eq!(from_milli(-4_000), Quantity::whole(-4));
        assert!(to_milli("0.0001".parse().map_err(Error::from)?).is_err());
        Ok(())
    }
}

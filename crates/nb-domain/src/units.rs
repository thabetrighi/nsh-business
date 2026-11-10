//! Units of a product (PRD §6.4): one base unit in which stock is kept, and
//! packaging units with an exact factor to it (1 carton = 6 packs = 72
//! pieces). Each unit has a precision: how many decimals a quantity in it
//! may have (0 for pieces, 3 for kilograms).

use nb_core::{Decimal, Quantity};

use crate::RuleError;

/// One unit a product is counted, bought or sold in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductUnit {
    /// Short code, unique within the product (`pc`, `box`, `ctn`, `kg`).
    pub code: String,
    /// How many base units one of this unit holds (1 for the base unit).
    pub factor: Decimal,
    /// Decimals allowed for quantities in this unit (0–3).
    pub precision: u32,
}

/// The units of one product, the base unit first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitSet {
    units: Vec<ProductUnit>,
}

impl UnitSet {
    /// A set with only its base unit.
    pub fn base(code: &str, precision: u32) -> Result<Self, RuleError> {
        Self::new(vec![ProductUnit {
            code: code.to_owned(),
            factor: Decimal::ONE,
            precision,
        }])
    }

    /// Validates the units: the first is the base (factor 1), factors are
    /// positive, codes are unique and not empty, precision at most 3.
    pub fn new(units: Vec<ProductUnit>) -> Result<Self, RuleError> {
        let Some(base) = units.first() else {
            return Err(RuleError::on("units", "units.empty"));
        };
        if base.factor != Decimal::ONE {
            return Err(RuleError::on("units", "units.base_factor"));
        }
        for (i, u) in units.iter().enumerate() {
            if u.code.trim().is_empty() {
                return Err(RuleError::on("units", "units.code_empty"));
            }
            if u.factor <= Decimal::ZERO {
                return Err(RuleError::on("units", "units.factor_positive"));
            }
            if u.precision > 3 {
                return Err(RuleError::on("units", "units.precision"));
            }
            if units[..i].iter().any(|o| o.code == u.code) {
                return Err(RuleError::on("units", "units.duplicate"));
            }
        }
        Ok(Self { units })
    }

    pub fn base_unit(&self) -> &ProductUnit {
        // `new` guarantees at least one unit.
        &self.units[0]
    }

    pub fn units(&self) -> &[ProductUnit] {
        &self.units
    }

    pub fn get(&self, code: &str) -> Option<&ProductUnit> {
        self.units.iter().find(|u| u.code == code)
    }

    /// A quantity typed in `unit`, checked against its precision, in base units.
    pub fn to_base(&self, qty: Quantity, unit: &str) -> Result<Quantity, RuleError> {
        let u = self
            .get(unit)
            .ok_or(RuleError::on("unit", "units.unknown"))?;
        if !qty.fits_precision(u.precision) {
            return Err(RuleError::on("quantity", "units.too_precise"));
        }
        Ok(qty.convert(u.factor)?)
    }

    /// A base quantity expressed in `unit` (exact when divisible).
    pub fn from_base(&self, base_qty: Quantity, unit: &str) -> Result<Quantity, RuleError> {
        let u = self
            .get(unit)
            .ok_or(RuleError::on("unit", "units.unknown"))?;
        base_qty
            .decimal()
            .checked_div(u.factor)
            .map(Quantity::new)
            .ok_or(RuleError::new("overflow"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap_or_default()
    }

    fn cartons() -> Result<UnitSet, RuleError> {
        UnitSet::new(vec![
            ProductUnit {
                code: "pc".into(),
                factor: d("1"),
                precision: 0,
            },
            ProductUnit {
                code: "pack".into(),
                factor: d("12"),
                precision: 0,
            },
            ProductUnit {
                code: "ctn".into(),
                factor: d("72"),
                precision: 0,
            },
        ])
    }

    #[test]
    fn carton_pack_piece_conversions_are_exact() -> Result<(), RuleError> {
        let u = cartons()?;
        assert_eq!(u.to_base(Quantity::whole(1), "ctn")?, Quantity::whole(72));
        assert_eq!(u.to_base(Quantity::whole(6), "pack")?, Quantity::whole(72));
        assert_eq!(
            u.from_base(Quantity::whole(72), "pack")?,
            Quantity::whole(6)
        );
        assert_eq!(u.from_base(Quantity::whole(36), "ctn")?.to_string(), "0.5");
        Ok(())
    }

    #[test]
    fn weights_respect_precision() -> Result<(), RuleError> {
        let kg = UnitSet::base("kg", 3)?;
        assert_eq!(
            kg.to_base(Quantity::from_milli(1_250), "kg")?,
            Quantity::from_milli(1_250)
        );
        let pc = UnitSet::base("pc", 0)?;
        assert_eq!(
            pc.to_base(Quantity::from_milli(1_500), "pc"),
            Err(RuleError::on("quantity", "units.too_precise"))
        );
        Ok(())
    }

    #[test]
    fn invalid_sets_are_refused() {
        let unit = |c: &str, f: &str| ProductUnit {
            code: c.into(),
            factor: d(f),
            precision: 0,
        };
        assert!(UnitSet::new(vec![]).is_err());
        assert!(UnitSet::new(vec![unit("pc", "2")]).is_err());
        assert!(UnitSet::new(vec![unit("pc", "1"), unit("box", "0")]).is_err());
        assert!(UnitSet::new(vec![unit("pc", "1"), unit("pc", "6")]).is_err());
        assert!(UnitSet::new(vec![unit(" ", "1")]).is_err());
    }
}

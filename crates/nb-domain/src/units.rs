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

impl UnitSet {
    /// Nested packaging given level by level (`[("box", 6), ("ctn", 12)]`:
    /// a box holds 6 base units, a carton 12 boxes = 72 base units).
    pub fn nested(
        base: &str,
        precision: u32,
        levels: &[(&str, Decimal)],
    ) -> Result<Self, RuleError> {
        let mut units = vec![ProductUnit {
            code: base.to_owned(),
            factor: Decimal::ONE,
            precision,
        }];
        let mut factor = Decimal::ONE;
        for (code, per_previous) in levels {
            factor = factor
                .checked_mul(*per_previous)
                .ok_or(RuleError::new("overflow"))?;
            units.push(ProductUnit {
                code: (*code).to_owned(),
                factor,
                precision: 0,
            });
        }
        Self::new(units)
    }

    /// A base quantity shown in whole packages, largest first, with the
    /// rest in the base unit (`75 pc` → `1 ctn, 0 box, 3 pc`, zeros left
    /// out): `[("ctn", 1), ("pc", 3)]`. Negative quantities keep their sign
    /// on every part.
    pub fn breakdown(&self, base_qty: Quantity) -> Vec<(String, Quantity)> {
        let negative = base_qty.decimal().is_sign_negative();
        let mut rest = base_qty.decimal().abs();
        let mut packs: Vec<&ProductUnit> = self.units[1..]
            .iter()
            .filter(|u| u.factor > Decimal::ONE && u.factor.fract().is_zero())
            .collect();
        packs.sort_by_key(|u| std::cmp::Reverse(u.factor));
        let signed = |d: Decimal| Quantity::new(if negative { -d } else { d });
        let mut out = Vec::new();
        for u in packs {
            let n = (rest / u.factor).trunc();
            if !n.is_zero() {
                out.push((u.code.clone(), signed(n)));
                rest -= n * u.factor;
            }
        }
        if !rest.is_zero() || out.is_empty() {
            out.push((self.base_unit().code.clone(), signed(rest)));
        }
        out
    }

    /// Whether changing to `next` would read past movements differently: a
    /// unit kept with another factor, or removed (PRD §6.4: a conversion in
    /// use is never changed; add a new unit instead).
    pub fn reinterprets(&self, next: &UnitSet) -> bool {
        self.base_unit().code != next.base_unit().code
            || self
                .units
                .iter()
                .any(|u| next.get(&u.code).is_none_or(|n| n.factor != u.factor))
    }
}

/// What a unit measures; conversions exist only within one dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    Count,
    Weight,
    Volume,
    Length,
    Area,
}

/// A unit of the catalog every organization starts with (PRD §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitDef {
    /// Stable code, also the message key `unit.<code>`.
    pub code: &'static str,
    pub dimension: Dimension,
    /// Size in the dimension's reference unit (g, ml, cm, m²; pieces for
    /// counts, where packages have no fixed size and this is 1).
    pub reference: &'static str,
    /// Decimals a quantity in this unit may have by default.
    pub precision: u32,
}

/// The standard units: counts, weights, volumes, lengths, areas.
pub const STANDARD_UNITS: &[UnitDef] = &[
    UnitDef {
        code: "pc",
        dimension: Dimension::Count,
        reference: "1",
        precision: 0,
    },
    UnitDef {
        code: "box",
        dimension: Dimension::Count,
        reference: "1",
        precision: 0,
    },
    UnitDef {
        code: "pack",
        dimension: Dimension::Count,
        reference: "1",
        precision: 0,
    },
    UnitDef {
        code: "ctn",
        dimension: Dimension::Count,
        reference: "1",
        precision: 0,
    },
    UnitDef {
        code: "mg",
        dimension: Dimension::Weight,
        reference: "0.001",
        precision: 0,
    },
    UnitDef {
        code: "g",
        dimension: Dimension::Weight,
        reference: "1",
        precision: 0,
    },
    UnitDef {
        code: "kg",
        dimension: Dimension::Weight,
        reference: "1000",
        precision: 3,
    },
    UnitDef {
        code: "t",
        dimension: Dimension::Weight,
        reference: "1000000",
        precision: 3,
    },
    UnitDef {
        code: "ml",
        dimension: Dimension::Volume,
        reference: "1",
        precision: 0,
    },
    UnitDef {
        code: "l",
        dimension: Dimension::Volume,
        reference: "1000",
        precision: 3,
    },
    UnitDef {
        code: "m3",
        dimension: Dimension::Volume,
        reference: "1000000",
        precision: 3,
    },
    UnitDef {
        code: "cm",
        dimension: Dimension::Length,
        reference: "1",
        precision: 1,
    },
    UnitDef {
        code: "m",
        dimension: Dimension::Length,
        reference: "100",
        precision: 2,
    },
    UnitDef {
        code: "m2",
        dimension: Dimension::Area,
        reference: "1",
        precision: 2,
    },
];

pub fn standard_unit(code: &str) -> Option<&'static UnitDef> {
    STANDARD_UNITS.iter().find(|u| u.code == code)
}

/// The fixed factor between two standard units of one measured dimension
/// (`kg` → `g`: 1000). Counts have no fixed factor (a box's size is the
/// product's), nor do units of different dimensions.
pub fn standard_factor(from: &str, to: &str) -> Option<Decimal> {
    let (a, b) = (standard_unit(from)?, standard_unit(to)?);
    if a.dimension != b.dimension || a.dimension == Dimension::Count {
        return None;
    }
    let ra: Decimal = a.reference.parse().ok()?;
    let rb: Decimal = b.reference.parse().ok()?;
    ra.checked_div(rb).map(|d| d.normalize())
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

    /// Acceptance of M5.2: 1 carton = 12 boxes = 72 pieces.
    #[test]
    fn nested_packaging() -> Result<(), RuleError> {
        let u = UnitSet::nested("pc", 0, &[("box", d("6")), ("ctn", d("12"))])?;
        assert_eq!(u.to_base(Quantity::whole(1), "ctn")?, Quantity::whole(72));
        assert_eq!(
            u.from_base(Quantity::whole(72), "box")?,
            Quantity::whole(12)
        );
        assert_eq!(
            u.to_base(Quantity::whole(12), "box")?,
            u.to_base(Quantity::whole(1), "ctn")?
        );
        let parts = u.breakdown(Quantity::whole(75));
        assert_eq!(
            parts,
            vec![
                ("ctn".into(), Quantity::whole(1)),
                ("pc".into(), Quantity::whole(3))
            ]
        );
        let parts = u.breakdown(Quantity::whole(-81));
        assert_eq!(
            parts,
            vec![
                ("ctn".into(), Quantity::whole(-1)),
                ("box".into(), Quantity::whole(-1)),
                ("pc".into(), Quantity::whole(-3))
            ]
        );
        assert_eq!(
            u.breakdown(Quantity::whole(0)),
            vec![("pc".into(), Quantity::whole(0))]
        );
        let kg = UnitSet::base("kg", 3)?;
        assert_eq!(
            kg.breakdown(Quantity::from_milli(1_250)),
            vec![("kg".into(), Quantity::from_milli(1_250))]
        );
        Ok(())
    }

    #[test]
    fn a_conversion_in_use_is_not_changed() -> Result<(), RuleError> {
        let before = cartons()?;
        let mut more = before.units().to_vec();
        more.push(ProductUnit {
            code: "pal".into(),
            factor: d("2880"),
            precision: 0,
        });
        assert!(
            !before.reinterprets(&UnitSet::new(more)?),
            "adding a unit is fine"
        );
        let mut changed = before.units().to_vec();
        changed[2].factor = d("48");
        assert!(before.reinterprets(&UnitSet::new(changed)?));
        assert!(
            before.reinterprets(&UnitSet::new(before.units()[..2].to_vec())?),
            "removing one"
        );
        assert!(before.reinterprets(&UnitSet::base("kg", 3)?));
        Ok(())
    }

    #[test]
    fn standard_conversions() {
        assert_eq!(standard_factor("kg", "g"), Some(d("1000")));
        assert_eq!(standard_factor("g", "kg"), Some(d("0.001")));
        assert_eq!(standard_factor("l", "ml"), Some(d("1000")));
        assert_eq!(standard_factor("m", "cm"), Some(d("100")));
        assert_eq!(standard_factor("kg", "l"), None);
        assert_eq!(standard_factor("ctn", "pc"), None);
        for u in STANDARD_UNITS {
            assert!(u.reference.parse::<Decimal>().is_ok(), "{}", u.code);
            assert!(u.precision <= 3);
        }
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

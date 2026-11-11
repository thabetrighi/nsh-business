//! Activity profiles and feature toggles (M4.3–M4.4, PRD §14). A profile is
//! data (`data/activity_profiles.json`): the features an activity turns on,
//! above the core every organization has. An organization can then switch
//! single features on or off; the result decides which modules and screens
//! appear (progressive disclosure).

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::RuleError;

macro_rules! features {
    ($( $name:ident $code:literal ),* $(,)?) => {
        /// A switchable capability of the application.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Feature { $( $name ),* }

        impl Feature {
            pub const ALL: &'static [Feature] = &[ $( Feature::$name ),* ];
            /// Stable code, also the message key `feature.<code>`.
            pub const fn code(self) -> &'static str {
                match self { $( Feature::$name => $code ),* }
            }
            pub fn from_code(code: &str) -> Option<Feature> {
                match code { $( $code => Some(Feature::$name), )* _ => None }
            }
        }
    };
}

features! {
    // Core
    Pos "pos",
    Sales "sales",
    Catalog "catalog",
    Inventory "inventory",
    Customers "customers",
    Debts "debts",
    Cash "cash",
    Accounting "accounting",
    Reports "reports",
    // Optional
    Purchasing "purchasing",
    Promotions "promotions",
    PriceLists "price_lists",
    UnitConversion "unit_conversion",
    FractionalQty "fractional_qty",
    Packaging "packaging",
    Scale "scale",
    WeighedBarcodes "weighed_barcodes",
    Labels "labels",
    Expiry "expiry",
    Batches "batches",
    Serials "serials",
    Warranty "warranty",
    Variants "variants",
    Seasons "seasons",
    Exchanges "exchanges",
    FastPos "fast_pos",
    Installments "installments",
    CreditLimits "credit_limits",
    MultiWarehouse "multi_warehouse",
    SalesReps "sales_reps",
    Vehicles "vehicles",
    Routes "routes",
    FieldCollection "field_collection",
    Delivery "delivery",
    Tables "tables",
    Kitchen "kitchen",
    Modifiers "modifiers",
    SplitBill "split_bill",
    Compatibility "compatibility",
    OemNumbers "oem_numbers",
    Compliance "compliance",
}

#[derive(Deserialize)]
struct RawProfiles {
    version: u32,
    core: Vec<String>,
    profiles: BTreeMap<String, RawProfile>,
}

#[derive(Deserialize)]
struct RawProfile {
    features: Vec<String>,
}

/// The activity profiles shipped with the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profiles {
    pub version: u32,
    core: BTreeSet<Feature>,
    profiles: BTreeMap<String, BTreeSet<Feature>>,
}

const BUILTIN: &str = include_str!("../data/activity_profiles.json");

fn features_of(codes: &[String]) -> Result<BTreeSet<Feature>, RuleError> {
    codes
        .iter()
        .map(|c| Feature::from_code(c).ok_or(RuleError::new("features.unknown")))
        .collect()
}

impl Profiles {
    /// Reads profiles from JSON; unknown features are refused.
    pub fn parse(json: &str) -> Result<Self, RuleError> {
        let raw: RawProfiles =
            serde_json::from_str(json).map_err(|_| RuleError::new("features.invalid"))?;
        let core = features_of(&raw.core)?;
        let profiles = raw
            .profiles
            .into_iter()
            .map(|(k, p)| features_of(&p.features).map(|f| (k, f)))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            version: raw.version,
            core,
            profiles,
        })
    }

    /// The profiles that come with the application.
    pub fn builtin() -> Self {
        // The embedded file is checked by a test; an empty set is the safe
        // fallback should it ever not parse (only the core would show).
        Self::parse(BUILTIN).unwrap_or(Self {
            version: 0,
            core: BTreeSet::new(),
            profiles: BTreeMap::new(),
        })
    }

    /// Activity codes (also the message keys `activity.<code>`).
    pub fn activities(&self) -> impl Iterator<Item = &str> {
        self.profiles.keys().map(String::as_str)
    }

    pub fn has(&self, activity: &str) -> bool {
        self.profiles.contains_key(activity)
    }

    pub fn is_core(&self, f: Feature) -> bool {
        self.core.contains(&f)
    }

    /// Features of an activity with the organization's switches applied
    /// (`true` turns a feature on, `false` off). An unknown activity gets
    /// the core only.
    pub fn effective(&self, activity: &str, overrides: &[(Feature, bool)]) -> FeatureSet {
        let mut set: BTreeSet<Feature> = self.core.clone();
        if let Some(p) = self.profiles.get(activity) {
            set.extend(p.iter().copied());
        }
        for (f, on) in overrides {
            if *on {
                set.insert(*f);
            } else {
                set.remove(f);
            }
        }
        FeatureSet(set)
    }
}

/// The features an organization has on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureSet(BTreeSet<Feature>);

impl FeatureSet {
    pub fn all() -> Self {
        Self(Feature::ALL.iter().copied().collect())
    }
    pub fn has(&self, f: Feature) -> bool {
        self.0.contains(&f)
    }
    pub fn iter(&self) -> impl Iterator<Item = Feature> + '_ {
        self.0.iter().copied()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_profiles_parse() {
        let p = Profiles::parse(BUILTIN);
        assert!(p.is_ok(), "{p:?}");
        let p = Profiles::builtin();
        assert_eq!(p.version, 1);
        for a in [
            "general",
            "grocery",
            "apparel",
            "electronics",
            "auto_parts",
            "wholesale",
            "restaurant",
            "building",
            "pharmacy",
        ] {
            assert!(p.has(a), "{a}");
        }
        for f in Feature::ALL {
            assert_eq!(Feature::from_code(f.code()), Some(*f));
        }
    }

    #[test]
    fn the_activity_changes_the_features() {
        let p = Profiles::builtin();
        let grocery = p.effective("grocery", &[]);
        let wholesale = p.effective("wholesale", &[]);
        assert!(grocery.has(Feature::Scale) && grocery.has(Feature::WeighedBarcodes));
        assert!(!grocery.has(Feature::Routes));
        assert!(wholesale.has(Feature::Routes) && wholesale.has(Feature::PriceLists));
        assert!(!wholesale.has(Feature::Scale));
        // The core is always there.
        for f in [Feature::Pos, Feature::Inventory, Feature::Accounting] {
            assert!(p.is_core(f));
            assert!(grocery.has(f) && wholesale.has(f) && p.effective("unknown", &[]).has(f));
        }
    }

    #[test]
    fn switches_override_the_profile() {
        let p = Profiles::builtin();
        let f = p.effective(
            "grocery",
            &[(Feature::Scale, false), (Feature::Installments, true)],
        );
        assert!(!f.has(Feature::Scale));
        assert!(f.has(Feature::Installments));
    }

    #[test]
    fn bad_profiles_are_refused() {
        assert_eq!(
            Profiles::parse(
                r#"{"version":1,"core":["pos"],"profiles":{"x":{"features":["teleport"]}}}"#
            ),
            Err(RuleError::new("features.unknown"))
        );
        assert_eq!(
            Profiles::parse("{"),
            Err(RuleError::new("features.invalid"))
        );
    }
}

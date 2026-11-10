//! Taxes (PRD §6.3, §11): VAT included in or added to a price, a per-rate
//! summary for documents, and the stamp duty on cash payments. Rates and
//! thresholds are data (localization packs), never constants in code.

use nb_core::{Money, Rate};

use crate::RuleError;

/// Whether prices are typed with tax included (TTC) or not (HT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxMode {
    Inclusive,
    Exclusive,
}

/// An amount split into net, tax and gross; always `net + tax = gross`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Taxed {
    pub net: Money,
    pub tax: Money,
    pub gross: Money,
}

/// Splits `amount` (typed in `mode`) at `rate`. Tax is rounded once, half
/// away from zero, and the other part derived so nothing is lost.
pub fn split(amount: Money, rate: Rate, mode: TaxMode) -> Result<Taxed, RuleError> {
    if rate.bps() < 0 {
        return Err(RuleError::on("tax_rate", "tax.negative_rate"));
    }
    Ok(match mode {
        TaxMode::Inclusive => {
            let tax = rate.included_in(amount)?;
            Taxed {
                net: amount.checked_sub(tax)?,
                tax,
                gross: amount,
            }
        }
        TaxMode::Exclusive => {
            let tax = rate.of(amount)?;
            Taxed {
                net: amount,
                tax,
                gross: amount.checked_add(tax)?,
            }
        }
    })
}

/// One line of a document's tax summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateTotal {
    pub rate: Rate,
    pub net: Money,
    pub tax: Money,
}

/// Totals per rate, in the order rates first appear.
pub fn summary(lines: &[(Rate, Taxed)]) -> Result<Vec<RateTotal>, RuleError> {
    let mut out: Vec<RateTotal> = Vec::new();
    for (rate, t) in lines {
        match out.iter_mut().find(|r| r.rate == *rate) {
            Some(r) => {
                r.net = r.net.checked_add(t.net)?;
                r.tax = r.tax.checked_add(t.tax)?;
            }
            None => out.push(RateTotal {
                rate: *rate,
                net: t.net,
                tax: t.tax,
            }),
        }
    }
    Ok(out)
}

/// The stamp duty on invoices paid in cash, as a localization rule:
/// `rate` of the amount, rounded up to a whole `step`, between `min` and
/// `max`, only from `threshold`. Values must be checked with an accountant
/// for each country and year (see `docs/NEEDS_OWNER.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StampDuty {
    pub threshold: Money,
    pub rate: Rate,
    pub step: Money,
    pub min: Money,
    pub max: Option<Money>,
}

impl StampDuty {
    pub fn on(&self, amount: Money) -> Result<Money, RuleError> {
        let zero = Money::zero(amount.currency());
        if amount.minor() < self.threshold.minor() {
            return Ok(zero);
        }
        let raw = self.rate.of(amount)?;
        let step = self.step.minor().max(1);
        // Round up to the step.
        let up = raw
            .minor()
            .checked_add(step - 1)
            .ok_or(RuleError::new("overflow"))?
            / step
            * step;
        let mut duty = up.max(self.min.minor());
        if let Some(max) = self.max {
            duty = duty.min(max.minor());
        }
        Ok(Money::from_minor(duty, amount.currency()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nb_core::Currency;

    fn da(minor: i64) -> Money {
        Money::from_minor(minor, Currency::DZD)
    }

    #[test]
    fn inclusive_and_exclusive_vat() -> Result<(), RuleError> {
        let r = Rate::from_bps(1_900);
        let t = split(da(11_900), r, TaxMode::Inclusive)?;
        assert_eq!((t.net, t.tax, t.gross), (da(10_000), da(1_900), da(11_900)));
        let t = split(da(10_000), r, TaxMode::Exclusive)?;
        assert_eq!((t.net, t.tax, t.gross), (da(10_000), da(1_900), da(11_900)));
        // Rounding: 100.00 TTC at 19% -> tax 15.97, net 84.03.
        let t = split(da(10_000), r, TaxMode::Inclusive)?;
        assert_eq!((t.net, t.tax), (da(8_403), da(1_597)));
        assert!(split(da(1), Rate::from_bps(-1), TaxMode::Exclusive).is_err());
        Ok(())
    }

    #[test]
    fn summary_groups_by_rate() -> Result<(), RuleError> {
        let a = Rate::from_bps(1_900);
        let b = Rate::from_bps(900);
        let lines = [
            (a, split(da(11_900), a, TaxMode::Inclusive)?),
            (b, split(da(10_900), b, TaxMode::Inclusive)?),
            (a, split(da(5_950), a, TaxMode::Inclusive)?),
        ];
        let s = summary(&lines)?;
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].net, da(15_000));
        assert_eq!(s[0].tax, da(2_850));
        assert_eq!(s[1].tax, da(900));
        Ok(())
    }

    #[test]
    fn stamp_duty_rule() -> Result<(), RuleError> {
        // Illustrative values only: 1% from 300 DA, by whole dinars, minimum 5 DA, maximum 2 500 DA.
        let rule = StampDuty {
            threshold: da(30_000),
            rate: Rate::from_bps(100),
            step: da(100),
            min: da(500),
            max: Some(da(250_000)),
        };
        assert_eq!(rule.on(da(29_999))?, da(0));
        assert_eq!(rule.on(da(30_000))?, da(500));
        assert_eq!(rule.on(da(123_456))?, da(1_300));
        assert_eq!(rule.on(da(100_000_000))?, da(250_000));
        Ok(())
    }

    proptest::proptest! {
        #[test]
        fn split_never_loses_a_centime(minor in -1_000_000_000i64..1_000_000_000, bps in 0i64..3_000) {
            for mode in [TaxMode::Inclusive, TaxMode::Exclusive] {
                let t = split(da(minor), Rate::from_bps(bps), mode).map_err(|e| proptest::test_runner::TestCaseError::fail(e.to_string()))?;
                proptest::prop_assert_eq!(t.net.minor() + t.tax.minor(), t.gross.minor());
            }
        }
    }
}

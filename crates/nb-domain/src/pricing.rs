//! Price selection (PRD §6.3): a product has prices in several lists
//! (retail, wholesale, special ones), optionally per branch, customer or
//! from a minimum quantity, and a floor below which it may not be sold
//! without permission.

use nb_core::{Money, Quantity};

use crate::{BranchId, CustomerId, RuleError};

/// Which list a price belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriceList {
    Retail,
    Wholesale,
    /// A list of the organization's own (semi-wholesale, distributors…).
    Custom(u32),
}

/// One price of a product, for a base unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceRule {
    pub list: PriceList,
    pub price: Money,
    /// Applies from this quantity (in base units); zero for any quantity.
    pub min_qty: Quantity,
    pub branch: Option<BranchId>,
    pub customer: Option<CustomerId>,
}

/// What the price is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceContext {
    pub list: PriceList,
    pub qty: Quantity,
    pub branch: Option<BranchId>,
    pub customer: Option<CustomerId>,
}

/// The price that applies: among rules of the context's list (and those for
/// the customer, whatever their list) that match its branch and customer
/// and whose minimum quantity is reached, the most specific wins: customer,
/// then branch, then the highest minimum quantity. Falls back to the
/// retail list when the asked list has no rule.
pub fn select(rules: &[PriceRule], ctx: &PriceContext) -> Option<Money> {
    let pick = |list: PriceList| {
        rules
            .iter()
            .filter(|r| {
                let list_ok =
                    r.list == list || (r.customer.is_some() && r.customer == ctx.customer);
                let branch_ok = r.branch.is_none() || r.branch == ctx.branch;
                let customer_ok = r.customer.is_none() || r.customer == ctx.customer;
                let qty_ok = r.min_qty.decimal() <= ctx.qty.decimal();
                list_ok && branch_ok && customer_ok && qty_ok
            })
            .max_by_key(|r| {
                (
                    r.customer.is_some(),
                    r.branch.is_some(),
                    r.min_qty.decimal(),
                )
            })
            .map(|r| r.price)
    };
    pick(ctx.list).or_else(|| {
        if ctx.list == PriceList::Retail {
            None
        } else {
            pick(PriceList::Retail)
        }
    })
}

/// Refuses a price under the floor unless the user may override it.
pub fn check_floor(
    price: Money,
    floor: Option<Money>,
    may_override: bool,
) -> Result<(), RuleError> {
    match floor {
        Some(f) if price.minor() < f.minor() && !may_override => {
            Err(RuleError::on("price", "price.below_floor"))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nb_core::Currency;

    fn da(minor: i64) -> Money {
        Money::from_minor(minor, Currency::DZD)
    }

    fn rule(list: PriceList, price: i64, min: i64) -> PriceRule {
        PriceRule {
            list,
            price: da(price),
            min_qty: Quantity::whole(min),
            branch: None,
            customer: None,
        }
    }

    fn ctx(list: PriceList, qty: i64) -> PriceContext {
        PriceContext {
            list,
            qty: Quantity::whole(qty),
            branch: None,
            customer: None,
        }
    }

    #[test]
    fn quantity_breaks_and_lists() {
        let rules = [
            rule(PriceList::Retail, 12_000, 0),
            rule(PriceList::Retail, 11_500, 12),
            rule(PriceList::Wholesale, 10_000, 0),
        ];
        assert_eq!(select(&rules, &ctx(PriceList::Retail, 1)), Some(da(12_000)));
        assert_eq!(
            select(&rules, &ctx(PriceList::Retail, 12)),
            Some(da(11_500))
        );
        assert_eq!(
            select(&rules, &ctx(PriceList::Wholesale, 1)),
            Some(da(10_000))
        );
        // No rule in a custom list: retail.
        assert_eq!(
            select(&rules, &ctx(PriceList::Custom(1), 1)),
            Some(da(12_000))
        );
        assert_eq!(select(&[], &ctx(PriceList::Retail, 1)), None);
    }

    #[test]
    fn customer_and_branch_prices_win() {
        let b = BranchId::new();
        let c = CustomerId::new();
        let mut branch = rule(PriceList::Retail, 11_800, 0);
        branch.branch = Some(b);
        let mut special = rule(PriceList::Custom(9), 9_900, 0);
        special.customer = Some(c);
        let rules = [rule(PriceList::Retail, 12_000, 0), branch, special];
        let mut x = ctx(PriceList::Retail, 1);
        assert_eq!(select(&rules, &x), Some(da(12_000)));
        x.branch = Some(b);
        assert_eq!(select(&rules, &x), Some(da(11_800)));
        x.customer = Some(c);
        assert_eq!(select(&rules, &x), Some(da(9_900)));
        // Another customer does not get it.
        x.customer = Some(CustomerId::new());
        assert_eq!(select(&rules, &x), Some(da(11_800)));
    }

    #[test]
    fn floor_needs_permission() {
        assert!(check_floor(da(900), Some(da(1_000)), false).is_err());
        assert!(check_floor(da(900), Some(da(1_000)), true).is_ok());
        assert!(check_floor(da(1_000), Some(da(1_000)), false).is_ok());
        assert!(check_floor(da(1), None, false).is_ok());
    }
}

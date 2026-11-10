//! Stock (PRD §6.6): movements are append-only facts in base units; a
//! balance is their projection and can always be rebuilt from them. Cost is
//! the weighted average, updated by receipts only.

use nb_core::{Money, Quantity};

use crate::RuleError;

/// Why stock moved. The kind fixes the direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MovementKind {
    Opening,
    Purchase,
    Sale,
    /// A customer brought goods back.
    SaleReturn,
    /// Goods sent back to a supplier.
    PurchaseReturn,
    AdjustmentIn,
    AdjustmentOut,
    TransferOut,
    TransferIn,
    CountGain,
    CountLoss,
    Waste,
}

impl MovementKind {
    pub const ALL: [MovementKind; 12] = [
        MovementKind::Opening,
        MovementKind::Purchase,
        MovementKind::Sale,
        MovementKind::SaleReturn,
        MovementKind::PurchaseReturn,
        MovementKind::AdjustmentIn,
        MovementKind::AdjustmentOut,
        MovementKind::TransferOut,
        MovementKind::TransferIn,
        MovementKind::CountGain,
        MovementKind::CountLoss,
        MovementKind::Waste,
    ];

    /// Stable code stored in the database.
    pub fn code(self) -> &'static str {
        match self {
            MovementKind::Opening => "opening",
            MovementKind::Purchase => "purchase",
            MovementKind::Sale => "sale",
            MovementKind::SaleReturn => "sale_return",
            MovementKind::PurchaseReturn => "purchase_return",
            MovementKind::AdjustmentIn => "adjust_in",
            MovementKind::AdjustmentOut => "adjust_out",
            MovementKind::TransferOut => "transfer_out",
            MovementKind::TransferIn => "transfer_in",
            MovementKind::CountGain => "count_gain",
            MovementKind::CountLoss => "count_loss",
            MovementKind::Waste => "waste",
        }
    }

    pub fn from_code(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.code() == s)
    }

    /// +1 for stock coming in, -1 for stock going out.
    pub fn sign(self) -> i64 {
        match self {
            MovementKind::Opening
            | MovementKind::Purchase
            | MovementKind::SaleReturn
            | MovementKind::AdjustmentIn
            | MovementKind::TransferIn
            | MovementKind::CountGain => 1,
            MovementKind::Sale
            | MovementKind::PurchaseReturn
            | MovementKind::AdjustmentOut
            | MovementKind::TransferOut
            | MovementKind::CountLoss
            | MovementKind::Waste => -1,
        }
    }

    /// Receipts at a known cost change the average cost.
    pub fn carries_cost(self) -> bool {
        matches!(self, MovementKind::Opening | MovementKind::Purchase)
    }
}

/// A quantity moved, as typed (always positive): the kind gives the sign.
pub fn signed_delta(kind: MovementKind, qty: Quantity) -> Result<Quantity, RuleError> {
    if qty.is_negative() || qty.is_zero() {
        return Err(RuleError::on("quantity", "stock.quantity_positive"));
    }
    if kind.sign() < 0 {
        Ok(Quantity::new(-qty.decimal()))
    } else {
        Ok(qty)
    }
}

/// What may happen when stock would go below zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegativeStock {
    Forbid,
    /// Allowed for users with the permission (selling before receiving).
    AllowWithPermission,
    Allow,
}

/// The balance of one product in one warehouse, and its average unit cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Balance {
    pub on_hand: Quantity,
    /// Average cost of one base unit, in minor units.
    pub avg_cost: Money,
}

impl Balance {
    pub fn empty(avg_cost: Money) -> Self {
        Self {
            on_hand: Quantity::whole(0),
            avg_cost,
        }
    }

    /// The balance after a movement. `unit_cost` is required for receipts
    /// that carry cost; the average is weighted by quantities on hand
    /// (negative stock does not weigh).
    pub fn apply(
        self,
        kind: MovementKind,
        qty: Quantity,
        unit_cost: Option<Money>,
        policy: NegativeStock,
        may_go_negative: bool,
    ) -> Result<Balance, RuleError> {
        let delta = signed_delta(kind, qty)?;
        let on_hand = self.on_hand.checked_add(delta)?;
        if on_hand.is_negative() && delta.is_negative() {
            let allowed = match policy {
                NegativeStock::Forbid => false,
                NegativeStock::AllowWithPermission => may_go_negative,
                NegativeStock::Allow => true,
            };
            if !allowed {
                return Err(RuleError::on("quantity", "stock.insufficient"));
            }
        }
        let avg_cost = match (kind.carries_cost(), unit_cost) {
            (true, Some(cost)) => weighted_average(self.on_hand, self.avg_cost, qty, cost)?,
            (true, None) => return Err(RuleError::on("unit_cost", "stock.cost_required")),
            _ => self.avg_cost,
        };
        Ok(Balance { on_hand, avg_cost })
    }
}

/// `(old_qty × old_cost + in_qty × in_cost) / (old_qty + in_qty)`, rounded
/// half away from zero; stock below zero counts as zero.
pub fn weighted_average(
    old_qty: Quantity,
    old_cost: Money,
    in_qty: Quantity,
    in_cost: Money,
) -> Result<Money, RuleError> {
    let old = if old_qty.is_negative() {
        Quantity::whole(0)
    } else {
        old_qty
    };
    let total_qty = old.checked_add(in_qty)?;
    if total_qty.is_zero() {
        return Ok(in_cost);
    }
    let value = old
        .times_price(old_cost)?
        .checked_add(in_qty.times_price(in_cost)?)?;
    // value / total_qty with exact decimals, rounded to a minor unit.
    let per = nb_core::Decimal::from(value.minor())
        .checked_div(total_qty.decimal())
        .ok_or(RuleError::new("overflow"))?
        .round_dp_with_strategy(0, nb_core::RoundingStrategy::MidpointAwayFromZero);
    let minor: i64 = per.try_into().map_err(|_| RuleError::new("overflow"))?;
    Ok(Money::from_minor(minor, in_cost.currency()))
}

/// Rebuilds a balance from its movements, in order (the projection check).
pub fn replay(
    movements: &[(MovementKind, Quantity, Option<Money>)],
    zero_cost: Money,
) -> Result<Balance, RuleError> {
    movements
        .iter()
        .try_fold(Balance::empty(zero_cost), |b, (k, q, c)| {
            b.apply(*k, *q, *c, NegativeStock::Allow, true)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nb_core::Currency;

    fn da(minor: i64) -> Money {
        Money::from_minor(minor, Currency::DZD)
    }

    #[test]
    fn kinds_have_codes_and_directions() {
        for k in MovementKind::ALL {
            assert_eq!(MovementKind::from_code(k.code()), Some(k));
        }
        assert_eq!(MovementKind::Sale.sign(), -1);
        assert_eq!(MovementKind::SaleReturn.sign(), 1);
        assert!(signed_delta(MovementKind::Sale, Quantity::whole(0)).is_err());
    }

    #[test]
    fn average_cost_follows_receipts() -> Result<(), RuleError> {
        let b = Balance::empty(da(0));
        let b = b.apply(
            MovementKind::Purchase,
            Quantity::whole(10),
            Some(da(10_000)),
            NegativeStock::Forbid,
            false,
        )?;
        assert_eq!(b.avg_cost, da(10_000));
        let b = b.apply(
            MovementKind::Purchase,
            Quantity::whole(30),
            Some(da(12_000)),
            NegativeStock::Forbid,
            false,
        )?;
        assert_eq!(b.avg_cost, da(11_500));
        let b = b.apply(
            MovementKind::Sale,
            Quantity::whole(15),
            None,
            NegativeStock::Forbid,
            false,
        )?;
        assert_eq!(b.on_hand, Quantity::whole(25));
        assert_eq!(b.avg_cost, da(11_500), "issues do not change the average");
        assert!(
            b.apply(
                MovementKind::Purchase,
                Quantity::whole(1),
                None,
                NegativeStock::Forbid,
                false
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn negative_stock_policy() -> Result<(), RuleError> {
        let b = Balance::empty(da(0)).apply(
            MovementKind::Opening,
            Quantity::whole(2),
            Some(da(100)),
            NegativeStock::Forbid,
            false,
        )?;
        let three = Quantity::whole(3);
        assert_eq!(
            b.apply(MovementKind::Sale, three, None, NegativeStock::Forbid, true),
            Err(RuleError::on("quantity", "stock.insufficient"))
        );
        assert!(
            b.apply(
                MovementKind::Sale,
                three,
                None,
                NegativeStock::AllowWithPermission,
                false
            )
            .is_err()
        );
        let neg = b.apply(
            MovementKind::Sale,
            three,
            None,
            NegativeStock::AllowWithPermission,
            true,
        )?;
        assert_eq!(neg.on_hand, Quantity::whole(-1));
        // Receiving into negative stock: the negative part does not weigh.
        let after = neg.apply(
            MovementKind::Purchase,
            Quantity::whole(5),
            Some(da(300)),
            NegativeStock::Forbid,
            false,
        )?;
        assert_eq!(after.on_hand, Quantity::whole(4));
        assert_eq!(after.avg_cost, da(300));
        Ok(())
    }

    #[test]
    fn weighed_goods_average() -> Result<(), RuleError> {
        let avg = weighted_average(
            Quantity::from_milli(1_500),
            da(20_000),
            Quantity::from_milli(500),
            da(24_000),
        )?;
        assert_eq!(avg, da(21_000));
        Ok(())
    }

    #[test]
    fn replay_matches_incremental() -> Result<(), RuleError> {
        let moves = [
            (MovementKind::Opening, Quantity::whole(5), Some(da(1_000))),
            (MovementKind::Sale, Quantity::whole(2), None),
            (MovementKind::Purchase, Quantity::whole(7), Some(da(1_300))),
            (MovementKind::Waste, Quantity::whole(1), None),
        ];
        let b = replay(&moves, da(0))?;
        assert_eq!(b.on_hand, Quantity::whole(9));
        assert_eq!(b.avg_cost, da(1_210));
        Ok(())
    }
}

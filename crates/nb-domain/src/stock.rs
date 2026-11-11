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

    /// Receipts at a known cost change the average cost (a transfer brings
    /// the sending warehouse's cost into the receiving one).
    pub fn carries_cost(self) -> bool {
        matches!(
            self,
            MovementKind::Opening | MovementKind::Purchase | MovementKind::TransferIn
        )
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

/// What a counted quantity means against the book: the movement that
/// brings the book to the count, or `None` when they agree (M6.7).
pub fn count_difference(
    on_hand: Quantity,
    counted: Quantity,
) -> Result<Option<(MovementKind, Quantity)>, RuleError> {
    if counted.is_negative() {
        return Err(RuleError::on("counted", "stock.quantity_positive"));
    }
    let diff = counted.decimal() - on_hand.decimal();
    Ok(if diff.is_zero() {
        None
    } else if diff.is_sign_positive() {
        Some((MovementKind::CountGain, Quantity::new(diff)))
    } else {
        Some((MovementKind::CountLoss, Quantity::new(-diff)))
    })
}

/// What went missing between a transfer's sending and its receiving
/// (M6.5). Receiving more than was sent is refused: it is a new transfer.
pub fn transfer_shortfall(sent: Quantity, received: Quantity) -> Result<Quantity, RuleError> {
    if received.is_negative() {
        return Err(RuleError::on("received", "stock.quantity_positive"));
    }
    if received.decimal() > sent.decimal() {
        return Err(RuleError::on("received", "transfer.received_more"));
    }
    Ok(Quantity::new(sent.decimal() - received.decimal()))
}

/// Quantity free to sell or send: on hand less what is reserved (M6.4).
pub fn available(on_hand: Quantity, reserved: Quantity) -> Quantity {
    Quantity::new(on_hand.decimal() - reserved.decimal())
}

/// A lot's stock in one warehouse, for allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LotStock {
    pub lot: String,
    /// ISO date, or none.
    pub expiry: Option<String>,
    pub qty: Quantity,
}

/// Which lots a quantity leaves from: first expiring first out (M6.8);
/// lots without expiry go last. Expired lots (expiry before `today`) are
/// skipped unless `include_expired` (losses, waste).
pub fn fefo(
    lots: &[LotStock],
    needed: Quantity,
    today: &str,
    include_expired: bool,
) -> Result<Vec<(String, Quantity)>, RuleError> {
    let mut usable: Vec<&LotStock> = lots
        .iter()
        .filter(|l| l.qty.decimal().is_sign_positive() && !l.qty.is_zero())
        .filter(|l| include_expired || l.expiry.as_deref().is_none_or(|e| e >= today))
        .collect();
    usable.sort_by(|a, b| match (&a.expiry, &b.expiry) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let mut left = needed.decimal();
    let mut out = Vec::new();
    for l in usable {
        if left.is_zero() {
            break;
        }
        let take = left.min(l.qty.decimal());
        out.push((l.lot.clone(), Quantity::new(take)));
        left -= take;
    }
    if !left.is_zero() {
        return Err(RuleError::on("lot", "lot.insufficient"));
    }
    Ok(out)
}

/// Checks the serial numbers given for a quantity of a serial-tracked
/// product: one per unit, none twice, none empty.
pub fn check_serials(qty: Quantity, serials: &[String]) -> Result<Vec<String>, RuleError> {
    let clean: Vec<String> = serials
        .iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    let whole = qty
        .fits_precision(0)
        .then(|| qty.decimal().normalize().to_string().parse::<usize>().ok())
        .flatten();
    if whole != Some(clean.len()) {
        return Err(RuleError::on("serials", "serial.count"));
    }
    let mut seen = std::collections::BTreeSet::new();
    if clean.iter().any(|s| !seen.insert(s.to_uppercase())) {
        return Err(RuleError::on("serials", "serial.duplicate"));
    }
    Ok(clean)
}

/// A valid expiry date (`YYYY-MM-DD`).
pub fn check_expiry(date: &str) -> Result<(), RuleError> {
    let b = date.as_bytes();
    let ok = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && date[..4]
            .parse::<u32>()
            .is_ok_and(|y| (2000..=2200).contains(&y))
        && date[5..7]
            .parse::<u32>()
            .is_ok_and(|m| (1..=12).contains(&m))
        && date[8..]
            .parse::<u32>()
            .is_ok_and(|d| (1..=31).contains(&d));
    if ok {
        Ok(())
    } else {
        Err(RuleError::on("expiry", "lot.expiry_format"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_expiring_first_out() -> Result<(), RuleError> {
        let lot = |l: &str, e: Option<&str>, q: i64| LotStock {
            lot: l.into(),
            expiry: e.map(Into::into),
            qty: Quantity::whole(q),
        };
        let lots = [
            lot("C", None, 10),
            lot("B", Some("2026-12-01"), 3),
            lot("A", Some("2026-11-20"), 2),
            lot("X", Some("2026-11-01"), 5),
        ];
        let today = "2026-11-11";
        assert_eq!(
            fefo(&lots, Quantity::whole(4), today, false)?,
            vec![
                ("A".into(), Quantity::whole(2)),
                ("B".into(), Quantity::whole(2))
            ]
        );
        assert_eq!(
            fefo(&lots, Quantity::whole(2), today, true)?,
            vec![("X".into(), Quantity::whole(2))],
            "losses take the expired first"
        );
        assert_eq!(fefo(&lots, Quantity::whole(14), today, false)?.len(), 3);
        assert_eq!(
            fefo(&lots, Quantity::whole(16), today, false),
            Err(RuleError::on("lot", "lot.insufficient"))
        );
        Ok(())
    }

    #[test]
    fn serial_numbers_and_dates() {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            check_serials(Quantity::whole(2), &s(&[" SN1", "SN2 "])),
            Ok(s(&["SN1", "SN2"]))
        );
        assert_eq!(
            check_serials(Quantity::whole(2), &s(&["SN1"])),
            Err(RuleError::on("serials", "serial.count"))
        );
        assert_eq!(
            check_serials(Quantity::whole(2), &s(&["sn1", "SN1"])),
            Err(RuleError::on("serials", "serial.duplicate"))
        );
        assert_eq!(
            check_serials(Quantity::from_milli(1_500), &s(&["A"])),
            Err(RuleError::on("serials", "serial.count"))
        );
        assert!(check_expiry("2027-03-31").is_ok());
        assert!(check_expiry("31/03/2027").is_err());
        assert!(check_expiry("2027-13-01").is_err());
    }

    #[test]
    fn counts_and_transfers() -> Result<(), RuleError> {
        let q = Quantity::whole;
        assert_eq!(
            count_difference(q(10), q(12))?,
            Some((MovementKind::CountGain, q(2)))
        );
        assert_eq!(
            count_difference(q(10), q(7))?,
            Some((MovementKind::CountLoss, q(3)))
        );
        assert_eq!(
            count_difference(q(-2), q(0))?,
            Some((MovementKind::CountGain, q(2)))
        );
        assert_eq!(count_difference(q(5), q(5))?, None);
        assert!(count_difference(q(5), q(-1)).is_err());
        assert_eq!(transfer_shortfall(q(10), q(9))?, q(1));
        assert_eq!(transfer_shortfall(q(10), q(10))?, q(0));
        assert_eq!(
            transfer_shortfall(q(10), q(11)),
            Err(RuleError::on("received", "transfer.received_more"))
        );
        assert_eq!(available(q(10), q(4)), q(6));
        Ok(())
    }
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

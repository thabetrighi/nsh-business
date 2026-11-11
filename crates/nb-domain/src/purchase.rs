//! Purchase orders and matching what arrives against them (M10.1, M10.3).
//!
//! draft → approved → partial → received; an approved order with nothing
//! received may be cancelled, a partial one closed (the rest will not
//! come). Goods are received against an approved order's lines: never more
//! than what is left, never a product it did not order; a cost above the
//! ordered one is a variance that needs the right to approve purchases.

use crate::RuleError;

/// Where a purchase order stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Draft,
    Approved,
    /// Some lines received, some still to come.
    Partial,
    Received,
    /// Partly received; the rest will not come.
    Closed,
    Cancelled,
}

impl OrderStatus {
    pub const ALL: [OrderStatus; 6] = [
        OrderStatus::Draft,
        OrderStatus::Approved,
        OrderStatus::Partial,
        OrderStatus::Received,
        OrderStatus::Closed,
        OrderStatus::Cancelled,
    ];

    pub fn code(self) -> &'static str {
        match self {
            OrderStatus::Draft => "draft",
            OrderStatus::Approved => "approved",
            OrderStatus::Partial => "partial",
            OrderStatus::Received => "received",
            OrderStatus::Closed => "closed",
            OrderStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.code() == code)
    }

    /// Goods may be received against it.
    pub fn receivable(self) -> bool {
        matches!(self, OrderStatus::Approved | OrderStatus::Partial)
    }

    /// Whether the order may move to `to` by a person's action (receiving
    /// moves it on its own, see [`after_receipt`]).
    pub fn may_become(self, to: OrderStatus) -> bool {
        matches!(
            (self, to),
            (OrderStatus::Draft, OrderStatus::Approved)
                | (
                    OrderStatus::Draft | OrderStatus::Approved,
                    OrderStatus::Cancelled
                )
                | (OrderStatus::Partial, OrderStatus::Closed)
        )
    }
}

/// An order line as far as receiving goes, in thousandths of its unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderedLine {
    pub ordered_milli: i64,
    pub received_milli: i64,
}

impl OrderedLine {
    pub fn remaining_milli(self) -> i64 {
        (self.ordered_milli - self.received_milli).max(0)
    }
}

/// Checks `qty_milli` more of a line may be received.
pub fn check_receipt(line: OrderedLine, qty_milli: i64) -> Result<(), RuleError> {
    if qty_milli <= 0 {
        return Err(RuleError::on("qty", "invalid_quantity"));
    }
    if qty_milli > line.remaining_milli() {
        return Err(RuleError::on("qty", "purchase.over_ordered"));
    }
    Ok(())
}

/// The state an approved order reaches once its lines have received what
/// they have.
pub fn after_receipt(lines: &[OrderedLine]) -> OrderStatus {
    if lines.iter().all(|l| l.remaining_milli() == 0) {
        OrderStatus::Received
    } else if lines.iter().any(|l| l.received_milli > 0) {
        OrderStatus::Partial
    } else {
        OrderStatus::Approved
    }
}

/// How far the invoiced cost is above the ordered one, in basis points of
/// the ordered cost (0 when at or below it).
pub fn cost_variance_bps(ordered_minor: i64, invoiced_minor: i64) -> i64 {
    if invoiced_minor <= ordered_minor {
        return 0;
    }
    if ordered_minor <= 0 {
        return i64::MAX;
    }
    let over = i128::from(invoiced_minor - ordered_minor) * 10_000;
    let bps = (over + i128::from(ordered_minor) - 1) / i128::from(ordered_minor);
    i64::try_from(bps).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(o: i64, r: i64) -> OrderedLine {
        OrderedLine {
            ordered_milli: o,
            received_milli: r,
        }
    }

    #[test]
    fn states_move_one_way() {
        use OrderStatus::*;
        assert!(Draft.may_become(Approved));
        assert!(Draft.may_become(Cancelled));
        assert!(Approved.may_become(Cancelled));
        assert!(Partial.may_become(Closed));
        for s in [Partial, Received, Closed, Cancelled] {
            assert!(!s.may_become(Cancelled), "{s:?}");
            assert!(!s.may_become(Approved), "{s:?}");
        }
        assert!(!Approved.may_become(Closed));
        assert!(!Draft.receivable() && Approved.receivable() && Partial.receivable());
        for s in OrderStatus::ALL {
            assert_eq!(OrderStatus::parse(s.code()), Some(s));
        }
    }

    #[test]
    fn never_more_than_what_is_left() {
        assert_eq!(check_receipt(l(10_000, 4_000), 6_000), Ok(()));
        assert_eq!(
            check_receipt(l(10_000, 4_000), 6_001).map_err(|e| e.code),
            Err("purchase.over_ordered")
        );
        assert_eq!(
            check_receipt(l(10_000, 0), 0).map_err(|e| e.code),
            Err("invalid_quantity")
        );
    }

    #[test]
    fn the_order_follows_its_lines() {
        assert_eq!(after_receipt(&[l(5, 0), l(3, 0)]), OrderStatus::Approved);
        assert_eq!(after_receipt(&[l(5, 5), l(3, 0)]), OrderStatus::Partial);
        assert_eq!(after_receipt(&[l(5, 2), l(3, 3)]), OrderStatus::Partial);
        assert_eq!(after_receipt(&[l(5, 5), l(3, 3)]), OrderStatus::Received);
    }

    #[test]
    fn variance_rounds_up() {
        assert_eq!(cost_variance_bps(1_000, 1_000), 0);
        assert_eq!(cost_variance_bps(1_000, 900), 0);
        assert_eq!(cost_variance_bps(1_000, 1_050), 500);
        assert_eq!(cost_variance_bps(3, 4), 3_334);
        assert_eq!(cost_variance_bps(0, 1), i64::MAX);
    }
}

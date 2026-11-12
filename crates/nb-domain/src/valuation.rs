//! Stock valuation (M6.10): the weighted average, or FIFO — what came in
//! first goes out first, at what it cost. FIFO keeps cost layers (what is
//! left of each receipt, at its cost); they are a projection of the
//! movements, rebuilt from them like the balances. In minor units and
//! thousandths of the base unit: no floating point.

use crate::RuleError;

/// How issues (sales, transfers, losses) are costed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Method {
    #[default]
    Average,
    Fifo,
}

impl Method {
    pub fn code(self) -> &'static str {
        match self {
            Method::Average => "average",
            Method::Fifo => "fifo",
        }
    }

    pub fn from_code(code: &str) -> Result<Self, RuleError> {
        match code {
            "average" => Ok(Method::Average),
            "fifo" => Ok(Method::Fifo),
            _ => Err(RuleError::on("valuation", "invalid_value")),
        }
    }
}

/// What is left of a receipt: its order, quantity (milli) and unit cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer {
    pub seq: i64,
    pub qty_milli: i64,
    pub unit_cost: i64,
}

/// `qty × unit` for a quantity in thousandths, rounded half away from zero.
pub fn cost_of(qty_milli: i64, unit_cost: i64) -> i64 {
    let p = i128::from(qty_milli) * i128::from(unit_cost);
    let r = if p >= 0 {
        (p + 500) / 1000
    } else {
        (p - 500) / 1000
    };
    i64::try_from(r).unwrap_or(if r > 0 { i64::MAX } else { i64::MIN })
}

/// The layers after a receipt of `qty_milli` at `unit_cost`. Stock below
/// zero before it (sold before it was received) takes the receipt first:
/// only what brings it above zero becomes a layer.
pub fn receive(
    layers: &mut Vec<Layer>,
    on_hand_before: i64,
    seq: i64,
    qty_milli: i64,
    unit_cost: i64,
) {
    let short = (-on_hand_before).max(0);
    let left = qty_milli - short;
    if left > 0 {
        layers.push(Layer {
            seq,
            qty_milli: left,
            unit_cost,
        });
    }
}

/// Takes `qty_milli` from the oldest layers. Returns the cost of what was
/// taken from layers and the quantity no layer covered (stock going
/// below zero), which the caller costs at the average.
pub fn issue(layers: &mut Vec<Layer>, qty_milli: i64) -> (i64, i64) {
    let mut want = qty_milli.max(0);
    let mut cost = 0;
    let mut gone = 0;
    for l in layers.iter_mut() {
        if want == 0 {
            break;
        }
        let take = want.min(l.qty_milli);
        cost += cost_of(take, l.unit_cost);
        l.qty_milli -= take;
        want -= take;
        if l.qty_milli == 0 {
            gone += 1;
        }
    }
    layers.drain(..gone);
    (cost, want)
}

/// What an issue of `qty_milli` costs under `method`: (unit cost, total).
/// The average: as it stands. FIFO: the layers' cost (the uncovered rest
/// at the average), the unit rounded; the total is exact and is what the
/// books carry.
pub fn issue_cost(method: Method, layers: &[Layer], qty_milli: i64, average: i64) -> (i64, i64) {
    if method == Method::Average || qty_milli <= 0 {
        return (average, cost_of(qty_milli, average));
    }
    let mut copy = layers.to_vec();
    let (cost, uncovered) = issue(&mut copy, qty_milli);
    let total = cost + cost_of(uncovered, average);
    let unit = (i128::from(total) * 1000 + i128::from(qty_milli) / 2) / i128::from(qty_milli);
    (i64::try_from(unit).unwrap_or(average), total)
}

/// What the layers are worth.
pub fn value(layers: &[Layer]) -> i64 {
    layers
        .iter()
        .map(|l| cost_of(l.qty_milli, l.unit_cost))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_in_first_out() {
        let mut l = Vec::new();
        // 10 at 100.00, then 10 at 120.00.
        receive(&mut l, 0, 1, 10_000, 10_000);
        receive(&mut l, 10_000, 2, 10_000, 12_000);
        assert_eq!(value(&l), 220_000);
        // 15 out: 10 × 100 + 5 × 120 = 1 600.00, 106.67 each.
        assert_eq!(
            issue_cost(Method::Fifo, &l, 15_000, 11_000),
            (10_667, 160_000)
        );
        assert_eq!(
            issue_cost(Method::Average, &l, 15_000, 11_000),
            (11_000, 165_000)
        );
        assert_eq!(issue(&mut l, 15_000), (160_000, 0));
        assert_eq!(
            l,
            [Layer {
                seq: 2,
                qty_milli: 5_000,
                unit_cost: 12_000
            }]
        );
        // More than is left: the rest is uncovered (stock goes below zero).
        assert_eq!(issue(&mut l, 7_000), (60_000, 2_000));
        assert!(l.is_empty());
        // Stock at −2: a receipt of 5 first fills the hole, 3 become a layer.
        receive(&mut l, -2_000, 3, 5_000, 13_000);
        assert_eq!(l[0].qty_milli, 3_000);
        // Weights in thousandths: 0.250 kg at 650.00 = 162.50.
        assert_eq!(cost_of(250, 65_000), 16_250);
        assert_eq!(Method::from_code("fifo").map(Method::code), Ok("fifo"));
        assert!(Method::from_code("lifo").is_err());
    }
}

//! The entry matrix (M7.5, M7.6, PRD §11.3): which accounts each business
//! operation moves. Operations name *roles* (cash, sales, output VAT…); an
//! organization maps each role to an account of its chart, so the matrix
//! stays the same across countries and charts.

use std::collections::BTreeMap;

use nb_core::Money;

use crate::RuleError;
use crate::ledger::{Entry, Line};

macro_rules! roles {
    ($($variant:ident $code:literal $dz:literal,)*) => {
        /// What an account is used for in automatic entries.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Role {
            $($variant,)*
        }

        impl Role {
            pub const ALL: &'static [Role] = &[$(Role::$variant,)*];

            /// Stable key (database, translations: `role.<code>`).
            pub fn code(self) -> &'static str {
                match self {
                    $(Role::$variant => $code,)*
                }
            }

            pub fn from_code(code: &str) -> Option<Role> {
                match code {
                    $($code => Some(Role::$variant),)*
                    _ => None,
                }
            }

            /// The account of the Algerian chart (SCF) proposed for this
            /// role; to be reviewed by an accountant (gate G2).
            pub fn dz_account(self) -> &'static str {
                match self {
                    $(Role::$variant => $dz,)*
                }
            }
        }
    };
}

roles! {
    Cash "cash" "531",
    Bank "bank" "5121",
    Receivable "receivable" "411",
    Payable "payable" "401",
    Stock "stock" "30",
    Sales "sales" "700",
    SalesReturns "sales_returns" "709",
    CostOfSales "cost_of_sales" "600",
    VatOut "vat_out" "4457",
    VatIn "vat_in" "4456",
    StampDuty "stamp_duty" "447",
    OpeningEquity "opening_equity" "101",
    StockGain "stock_gain" "758",
    StockLoss "stock_loss" "658",
    CashShort "cash_short" "658",
    CashOver "cash_over" "758",
    InternalTransfer "internal_transfer" "58",
    BadDebt "bad_debt" "654",
}

/// The accounts an organization uses for each role.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccountMap(BTreeMap<Role, String>);

impl AccountMap {
    /// The proposed Algerian mapping.
    pub fn dz() -> Self {
        Self(
            Role::ALL
                .iter()
                .map(|r| (*r, r.dz_account().to_owned()))
                .collect(),
        )
    }

    pub fn set(&mut self, role: Role, account: &str) {
        self.0.insert(role, account.to_owned());
    }

    pub fn get(&self, role: Role) -> Result<&str, RuleError> {
        self.0
            .get(&role)
            .map(String::as_str)
            .ok_or(RuleError::on("role", "posting.role_unmapped"))
    }

    pub fn iter(&self) -> impl Iterator<Item = (Role, &str)> {
        self.0.iter().map(|(r, a)| (*r, a.as_str()))
    }
}

/// How an amount is settled: what goes to the till, the bank, or on credit
/// (the customer's or the supplier's account).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settlement {
    pub cash: Money,
    pub bank: Money,
    pub credit: Money,
}

impl Settlement {
    pub fn cash(amount: Money) -> Self {
        let zero = Money::zero(amount.currency());
        Self {
            cash: amount,
            bank: zero,
            credit: zero,
        }
    }
    pub fn bank(amount: Money) -> Self {
        let zero = Money::zero(amount.currency());
        Self {
            cash: zero,
            bank: amount,
            credit: zero,
        }
    }
    pub fn credit(amount: Money) -> Self {
        let zero = Money::zero(amount.currency());
        Self {
            cash: zero,
            bank: zero,
            credit: amount,
        }
    }
    fn total(&self) -> Result<Money, RuleError> {
        Ok(self.cash.checked_add(self.bank)?.checked_add(self.credit)?)
    }
}

/// A business operation with an accounting effect. Amounts are positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Goods or services sold: net of VAT, the VAT, the stamp duty on cash
    /// payments, settled in cash, by bank, or on the customer's account.
    Sale {
        net: Money,
        vat: Money,
        stamp: Money,
        paid: Settlement,
    },
    /// The cost of the goods that left with a sale.
    CostOfSale { cost: Money },
    /// Money a customer pays on account.
    Collection { amount: Money, to_bank: bool },
    /// Goods bought into stock (perpetual inventory).
    Purchase {
        net: Money,
        vat: Money,
        paid: Settlement,
    },
    /// Money paid to a supplier on account.
    SupplierPayment { amount: Money, from_bank: bool },
    /// A sale returned: refunded, or credited to the customer's account.
    SaleReturn {
        net: Money,
        vat: Money,
        refund: Settlement,
    },
    /// The returned goods back in stock, at their cost.
    ReturnToStock { cost: Money },
    /// Opening stock, against equity.
    StockOpening { value: Money },
    /// Stock found (count gain, adjustment in).
    StockGain { value: Money },
    /// Stock lost (count loss, waste, adjustment out, transfer shortfall).
    StockLoss { value: Money },
    /// The till counted less (short) or more (over) than expected.
    CashDifference { short: Money, over: Money },
    /// Money put in the till (`into_till`) or taken out of it, through
    /// internal transfers (to or from the bank, the safe, another till).
    TillTransfer { amount: Money, into_till: bool },
    /// A customer's debt given up (M9.9).
    WriteOff { amount: Money },
    /// An allowance granted on a customer's debt (M9.9).
    Allowance { amount: Money },
    /// What a customer owed (`owed`) or was owed (`owing`) before the
    /// books started here.
    CustomerOpening { owed: Money, owing: Money },
}

impl Operation {
    /// Stable key, kept as the entry's source.
    pub fn code(&self) -> &'static str {
        match self {
            Operation::Sale { .. } => "sale",
            Operation::CostOfSale { .. } => "sale.cost",
            Operation::Collection { .. } => "collection",
            Operation::Purchase { .. } => "purchase",
            Operation::SupplierPayment { .. } => "supplier.payment",
            Operation::SaleReturn { .. } => "sale.return",
            Operation::ReturnToStock { .. } => "sale.return.stock",
            Operation::StockOpening { .. } => "stock.opening",
            Operation::StockGain { .. } => "stock.gain",
            Operation::StockLoss { .. } => "stock.loss",
            Operation::CashDifference { .. } => "cash.difference",
            Operation::TillTransfer { .. } => "cash.transfer",
            Operation::WriteOff { .. } => "debt.write_off",
            Operation::Allowance { .. } => "debt.allowance",
            Operation::CustomerOpening { .. } => "debt.opening",
        }
    }
}

fn check_positive(amounts: &[Money]) -> Result<(), RuleError> {
    if amounts.iter().any(Money::is_negative) {
        return Err(RuleError::new("ledger.negative"));
    }
    Ok(())
}

/// The entry of `op` with `map`'s accounts; `None` when it moves nothing.
pub fn entry(op: &Operation, map: &AccountMap) -> Result<Option<Entry>, RuleError> {
    combined(std::slice::from_ref(op), map)
}

/// One entry for several operations of one document (a count's gains and
/// losses, a sale and its cost); `None` when they move nothing.
pub fn combined(ops: &[Operation], map: &AccountMap) -> Result<Option<Entry>, RuleError> {
    let mut all = Vec::new();
    for op in ops {
        all.extend(lines(op, map)?);
    }
    let all = merge(all);
    if all.is_empty() {
        return Ok(None);
    }
    Ok(Some(Entry::new(all)?))
}

fn lines(op: &Operation, map: &AccountMap) -> Result<Vec<Line>, RuleError> {
    let a = |r: Role| map.get(r);
    let lines = match *op {
        Operation::Sale {
            net,
            vat,
            stamp,
            paid,
        } => {
            check_positive(&[net, vat, stamp, paid.cash, paid.bank, paid.credit])?;
            let due = net.checked_add(vat)?.checked_add(stamp)?;
            if paid.total()? != due {
                return Err(RuleError::on("paid", "posting.settlement_mismatch"));
            }
            vec![
                Line::debit(a(Role::Cash)?, paid.cash),
                Line::debit(a(Role::Bank)?, paid.bank),
                Line::debit(a(Role::Receivable)?, paid.credit),
                Line::credit(a(Role::Sales)?, net),
                Line::credit(a(Role::VatOut)?, vat),
                Line::credit(a(Role::StampDuty)?, stamp),
            ]
        }
        Operation::CostOfSale { cost } => {
            check_positive(&[cost])?;
            vec![
                Line::debit(a(Role::CostOfSales)?, cost),
                Line::credit(a(Role::Stock)?, cost),
            ]
        }
        Operation::Collection { amount, to_bank } => {
            check_positive(&[amount])?;
            let to = if to_bank { Role::Bank } else { Role::Cash };
            vec![
                Line::debit(a(to)?, amount),
                Line::credit(a(Role::Receivable)?, amount),
            ]
        }
        Operation::Purchase { net, vat, paid } => {
            check_positive(&[net, vat, paid.cash, paid.bank, paid.credit])?;
            if paid.total()? != net.checked_add(vat)? {
                return Err(RuleError::on("paid", "posting.settlement_mismatch"));
            }
            vec![
                Line::debit(a(Role::Stock)?, net),
                Line::debit(a(Role::VatIn)?, vat),
                Line::credit(a(Role::Cash)?, paid.cash),
                Line::credit(a(Role::Bank)?, paid.bank),
                Line::credit(a(Role::Payable)?, paid.credit),
            ]
        }
        Operation::SupplierPayment { amount, from_bank } => {
            check_positive(&[amount])?;
            let from = if from_bank { Role::Bank } else { Role::Cash };
            vec![
                Line::debit(a(Role::Payable)?, amount),
                Line::credit(a(from)?, amount),
            ]
        }
        Operation::SaleReturn { net, vat, refund } => {
            check_positive(&[net, vat, refund.cash, refund.bank, refund.credit])?;
            if refund.total()? != net.checked_add(vat)? {
                return Err(RuleError::on("refund", "posting.settlement_mismatch"));
            }
            vec![
                Line::debit(a(Role::SalesReturns)?, net),
                Line::debit(a(Role::VatOut)?, vat),
                Line::credit(a(Role::Cash)?, refund.cash),
                Line::credit(a(Role::Bank)?, refund.bank),
                Line::credit(a(Role::Receivable)?, refund.credit),
            ]
        }
        Operation::ReturnToStock { cost } => {
            check_positive(&[cost])?;
            vec![
                Line::debit(a(Role::Stock)?, cost),
                Line::credit(a(Role::CostOfSales)?, cost),
            ]
        }
        Operation::StockOpening { value } => {
            check_positive(&[value])?;
            vec![
                Line::debit(a(Role::Stock)?, value),
                Line::credit(a(Role::OpeningEquity)?, value),
            ]
        }
        Operation::StockGain { value } => {
            check_positive(&[value])?;
            vec![
                Line::debit(a(Role::Stock)?, value),
                Line::credit(a(Role::StockGain)?, value),
            ]
        }
        Operation::StockLoss { value } => {
            check_positive(&[value])?;
            vec![
                Line::debit(a(Role::StockLoss)?, value),
                Line::credit(a(Role::Stock)?, value),
            ]
        }
        Operation::CashDifference { short, over } => {
            check_positive(&[short, over])?;
            vec![
                Line::debit(a(Role::CashShort)?, short),
                Line::credit(a(Role::Cash)?, short),
                Line::debit(a(Role::Cash)?, over),
                Line::credit(a(Role::CashOver)?, over),
            ]
        }
        Operation::TillTransfer { amount, into_till } => {
            check_positive(&[amount])?;
            let (debit, credit) = if into_till {
                (Role::Cash, Role::InternalTransfer)
            } else {
                (Role::InternalTransfer, Role::Cash)
            };
            vec![
                Line::debit(a(debit)?, amount),
                Line::credit(a(credit)?, amount),
            ]
        }
        Operation::WriteOff { amount } => {
            check_positive(&[amount])?;
            vec![
                Line::debit(a(Role::BadDebt)?, amount),
                Line::credit(a(Role::Receivable)?, amount),
            ]
        }
        Operation::Allowance { amount } => {
            check_positive(&[amount])?;
            vec![
                Line::debit(a(Role::SalesReturns)?, amount),
                Line::credit(a(Role::Receivable)?, amount),
            ]
        }
        Operation::CustomerOpening { owed, owing } => {
            check_positive(&[owed, owing])?;
            vec![
                Line::debit(a(Role::Receivable)?, owed),
                Line::credit(a(Role::OpeningEquity)?, owed),
                Line::debit(a(Role::OpeningEquity)?, owing),
                Line::credit(a(Role::Receivable)?, owing),
            ]
        }
    };
    Ok(lines)
}

/// One line per account and side (a cash short and over in one entry would
/// otherwise both touch the till).
fn merge(lines: Vec<Line>) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for l in lines {
        if l.debit.is_zero() && l.credit.is_zero() {
            continue;
        }
        let debit_side = !l.debit.is_zero();
        if let Some(prev) = out
            .iter_mut()
            .find(|p| p.account == l.account && !p.debit.is_zero() == debit_side)
        {
            // Same currency: Entry::new checks it on the merged lines too.
            prev.debit = prev.debit.checked_add(l.debit).unwrap_or(prev.debit);
            prev.credit = prev.credit.checked_add(l.credit).unwrap_or(prev.credit);
        } else {
            out.push(l);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nb_core::Currency;

    fn da(minor: i64) -> Money {
        Money::from_minor(minor, Currency::DZD)
    }

    fn lines(op: Operation) -> Vec<(String, i64, i64)> {
        entry(&op, &AccountMap::dz())
            .ok()
            .flatten()
            .map(|e| {
                e.lines()
                    .iter()
                    .map(|l| (l.account.clone(), l.debit.minor(), l.credit.minor()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn l(a: &str, d: i64, c: i64) -> (String, i64, i64) {
        (a.to_owned(), d, c)
    }

    #[test]
    fn the_matrix() {
        // Cash sale with VAT and stamp duty.
        assert_eq!(
            lines(Operation::Sale {
                net: da(10_000),
                vat: da(1_900),
                stamp: da(119),
                paid: Settlement::cash(da(12_019)),
            }),
            [
                l("531", 12_019, 0),
                l("700", 0, 10_000),
                l("4457", 0, 1_900),
                l("447", 0, 119)
            ]
        );
        // Mixed: part paid by card, the rest on credit.
        assert_eq!(
            lines(Operation::Sale {
                net: da(10_000),
                vat: da(1_900),
                stamp: da(0),
                paid: Settlement {
                    cash: da(0),
                    bank: da(5_000),
                    credit: da(6_900)
                },
            }),
            [
                l("5121", 5_000, 0),
                l("411", 6_900, 0),
                l("700", 0, 10_000),
                l("4457", 0, 1_900)
            ]
        );
        assert_eq!(
            lines(Operation::CostOfSale { cost: da(7_000) }),
            [l("600", 7_000, 0), l("30", 0, 7_000)]
        );
        assert_eq!(
            lines(Operation::Collection {
                amount: da(500),
                to_bank: false
            }),
            [l("531", 500, 0), l("411", 0, 500)]
        );
        assert_eq!(
            lines(Operation::Purchase {
                net: da(20_000),
                vat: da(3_800),
                paid: Settlement::credit(da(23_800))
            }),
            [l("30", 20_000, 0), l("4456", 3_800, 0), l("401", 0, 23_800)]
        );
        assert_eq!(
            lines(Operation::SupplierPayment {
                amount: da(23_800),
                from_bank: true
            }),
            [l("401", 23_800, 0), l("5121", 0, 23_800)]
        );
        assert_eq!(
            lines(Operation::SaleReturn {
                net: da(1_000),
                vat: da(190),
                refund: Settlement::cash(da(1_190))
            }),
            [l("709", 1_000, 0), l("4457", 190, 0), l("531", 0, 1_190)]
        );
        assert_eq!(
            lines(Operation::ReturnToStock { cost: da(700) }),
            [l("30", 700, 0), l("600", 0, 700)]
        );
        assert_eq!(
            lines(Operation::StockOpening { value: da(5) }),
            [l("30", 5, 0), l("101", 0, 5)]
        );
        assert_eq!(
            lines(Operation::StockGain { value: da(5) }),
            [l("30", 5, 0), l("758", 0, 5)]
        );
        assert_eq!(
            lines(Operation::StockLoss { value: da(5) }),
            [l("658", 5, 0), l("30", 0, 5)]
        );
        assert_eq!(
            lines(Operation::CashDifference {
                short: da(200),
                over: da(0)
            }),
            [l("658", 200, 0), l("531", 0, 200)]
        );
        assert_eq!(
            lines(Operation::CashDifference {
                short: da(0),
                over: da(50)
            }),
            [l("531", 50, 0), l("758", 0, 50)]
        );
        assert_eq!(
            lines(Operation::WriteOff { amount: da(90) }),
            [l("654", 90, 0), l("411", 0, 90)]
        );
        assert_eq!(
            lines(Operation::Allowance { amount: da(10) }),
            [l("709", 10, 0), l("411", 0, 10)]
        );
        assert_eq!(
            lines(Operation::CustomerOpening {
                owed: da(400),
                owing: da(0)
            }),
            [l("411", 400, 0), l("101", 0, 400)]
        );
    }

    #[test]
    fn till_transfers() {
        assert_eq!(
            lines(Operation::TillTransfer {
                amount: da(5_000),
                into_till: true
            }),
            [l("531", 5_000, 0), l("58", 0, 5_000)]
        );
        assert_eq!(
            lines(Operation::TillTransfer {
                amount: da(5_000),
                into_till: false
            }),
            [l("58", 5_000, 0), l("531", 0, 5_000)]
        );
    }

    #[test]
    fn nothing_to_post_and_wrong_amounts() {
        assert_eq!(
            entry(&Operation::StockLoss { value: da(0) }, &AccountMap::dz()),
            Ok(None)
        );
        let e = entry(
            &Operation::Sale {
                net: da(100),
                vat: da(19),
                stamp: da(0),
                paid: Settlement::cash(da(100)),
            },
            &AccountMap::dz(),
        );
        assert_eq!(e.err().map(|e| e.code), Some("posting.settlement_mismatch"));
        let e = entry(&Operation::StockGain { value: da(-1) }, &AccountMap::dz());
        assert_eq!(e.err().map(|e| e.code), Some("ledger.negative"));
        let e = entry(
            &Operation::StockGain { value: da(1) },
            &AccountMap::default(),
        );
        assert_eq!(e.err().map(|e| e.code), Some("posting.role_unmapped"));
    }

    #[test]
    fn an_organization_remaps_a_role() -> Result<(), RuleError> {
        let mut map = AccountMap::dz();
        map.set(Role::Cash, "532");
        let e = entry(
            &Operation::Collection {
                amount: da(10),
                to_bank: false,
            },
            &map,
        )?
        .ok_or(RuleError::new("empty"))?;
        assert_eq!(e.lines()[0].account, "532");
        for r in Role::ALL {
            assert_eq!(Role::from_code(r.code()), Some(*r));
        }
        Ok(())
    }
}

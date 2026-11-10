//! Double-entry bookkeeping (PRD §11): an entry is balanced, one-sided per
//! line, in one currency, and never edited once posted — it is corrected by
//! its reversal.

use nb_core::Money;

use crate::RuleError;

/// One line of an entry: an account, and a debit or a credit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Account code in the chart of accounts (`512`, `4111`, `70`…).
    pub account: String,
    pub debit: Money,
    pub credit: Money,
    pub memo: Option<String>,
}

impl Line {
    pub fn debit(account: &str, amount: Money) -> Self {
        Self {
            account: account.to_owned(),
            debit: amount,
            credit: Money::zero(amount.currency()),
            memo: None,
        }
    }
    pub fn credit(account: &str, amount: Money) -> Self {
        Self {
            account: account.to_owned(),
            debit: Money::zero(amount.currency()),
            credit: amount,
            memo: None,
        }
    }
}

/// An entry checked by [`Entry::new`]: only balanced entries exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    lines: Vec<Line>,
    total: Money,
}

impl Entry {
    /// Validates `lines`: at least two, each one-sided and not negative,
    /// one currency, debits equal credits, and not all zero. Zero lines are
    /// dropped.
    pub fn new(lines: Vec<Line>) -> Result<Self, RuleError> {
        let lines: Vec<Line> = lines
            .into_iter()
            .filter(|l| !(l.debit.is_zero() && l.credit.is_zero()))
            .collect();
        let Some(first) = lines.first() else {
            return Err(RuleError::new("ledger.empty"));
        };
        let currency = first.debit.currency();
        let mut debit = Money::zero(currency);
        let mut credit = Money::zero(currency);
        for l in &lines {
            if l.account.trim().is_empty() {
                return Err(RuleError::new("ledger.account_missing"));
            }
            if l.debit.is_negative() || l.credit.is_negative() {
                return Err(RuleError::new("ledger.negative"));
            }
            if !l.debit.is_zero() && !l.credit.is_zero() {
                return Err(RuleError::new("ledger.two_sided"));
            }
            debit = debit.checked_add(l.debit)?;
            credit = credit.checked_add(l.credit)?;
        }
        if lines.len() < 2 {
            return Err(RuleError::new("ledger.single_line"));
        }
        if debit != credit {
            return Err(RuleError::new("ledger.unbalanced"));
        }
        Ok(Self {
            lines,
            total: debit,
        })
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Total of debits (= total of credits).
    pub fn total(&self) -> Money {
        self.total
    }

    /// The entry that cancels this one: debits and credits swapped.
    pub fn reversal(&self) -> Entry {
        Entry {
            lines: self
                .lines
                .iter()
                .map(|l| Line {
                    account: l.account.clone(),
                    debit: l.credit,
                    credit: l.debit,
                    memo: l.memo.clone(),
                })
                .collect(),
            total: self.total,
        }
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
    fn a_cash_sale_with_vat() -> Result<(), RuleError> {
        let e = Entry::new(vec![
            Line::debit("53", da(11_900)),
            Line::credit("700", da(10_000)),
            Line::credit("4457", da(1_900)),
        ])?;
        assert_eq!(e.total(), da(11_900));
        let r = e.reversal();
        assert_eq!(r.lines()[0].credit, da(11_900));
        assert_eq!(Entry::new(r.lines().to_vec())?.total(), da(11_900));
        Ok(())
    }

    #[test]
    fn broken_entries_are_refused() {
        let code = |r: Result<Entry, RuleError>| r.err().map(|e| e.code);
        assert_eq!(code(Entry::new(vec![])), Some("ledger.empty"));
        assert_eq!(
            code(Entry::new(vec![
                Line::debit("53", da(1)),
                Line::credit("70", da(2))
            ])),
            Some("ledger.unbalanced")
        );
        assert_eq!(
            code(Entry::new(vec![Line::debit("53", da(1))])),
            Some("ledger.single_line")
        );
        let mut both = Line::debit("53", da(1));
        both.credit = da(1);
        assert_eq!(
            code(Entry::new(vec![both, Line::credit("70", da(0))])),
            Some("ledger.two_sided")
        );
        assert_eq!(
            code(Entry::new(vec![
                Line::debit("53", da(-1)),
                Line::credit("70", da(-1))
            ])),
            Some("ledger.negative")
        );
        assert_eq!(
            code(Entry::new(vec![
                Line::debit(" ", da(1)),
                Line::credit("70", da(1))
            ])),
            Some("ledger.account_missing")
        );
        let usd = Money::from_minor(1, Currency::USD);
        assert_eq!(
            code(Entry::new(vec![
                Line::debit("53", da(1)),
                Line::credit("70", usd)
            ])),
            Some("currency_mismatch")
        );
    }
}

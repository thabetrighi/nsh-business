//! Instalment plans (M9.6–M9.8): what is financed, split into instalments
//! due weekly, monthly or quarterly; what a payment settles; the state of
//! each instalment.
//!
//! Rounding rule (declared to the customer): the financed amount is split
//! into equal instalments to the centime, and the centimes left over go to
//! the first instalments, one each (`Money::allocate`) — so the
//! instalments always add up to the amount financed.

use nb_core::Money;
use serde::{Deserialize, Serialize};

use crate::RuleError;
use crate::fiscal::{Date, days_in_month};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    Weekly,
    Monthly,
    Quarterly,
}

impl Frequency {
    pub fn code(self) -> &'static str {
        match self {
            Frequency::Weekly => "weekly",
            Frequency::Monthly => "monthly",
            Frequency::Quarterly => "quarterly",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        match s {
            "weekly" => Some(Frequency::Weekly),
            "monthly" => Some(Frequency::Monthly),
            "quarterly" => Some(Frequency::Quarterly),
            _ => None,
        }
    }
}

/// The most instalments a plan may have.
pub const MAX_COUNT: u32 = 60;

/// `n` periods after `first`: weeks add days; months keep the day of the
/// month, or the last day of a shorter month (31 January → 28 February →
/// 31 March).
pub fn due_date(first: Date, f: Frequency, n: u32) -> Date {
    match f {
        Frequency::Weekly => add_days(first, i64::from(n) * 7),
        Frequency::Monthly => add_months(first, n),
        Frequency::Quarterly => add_months(first, n * 3),
    }
}

fn add_months(d: Date, months: u32) -> Date {
    let index = d.y * 12 + i32::try_from(d.m - 1).unwrap_or(0) + i32::try_from(months).unwrap_or(0);
    let y = index.div_euclid(12);
    let m = u32::try_from(index.rem_euclid(12)).unwrap_or(0) + 1;
    Date {
        y,
        m,
        d: d.d.min(days_in_month(y, m)),
    }
}

fn days_from_civil(d: Date) -> i64 {
    let y = i64::from(d.y) - i64::from(d.m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(d.m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d.d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> Date {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Date {
        y: i32::try_from(y).unwrap_or(i32::MAX),
        m: u32::try_from(m).unwrap_or(1),
        d: u32::try_from(d).unwrap_or(1),
    }
}

pub fn add_days(d: Date, days: i64) -> Date {
    civil_from_days(days_from_civil(d) + days)
}

/// Days from `a` to `b` (negative when `b` is earlier).
pub fn days_between(a: Date, b: Date) -> i64 {
    days_from_civil(b) - days_from_civil(a)
}

/// One instalment of a schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub date: Date,
    pub amount: Money,
}

/// The instalments of `financed` in `count` parts, the first due on
/// `first`, then every period.
pub fn schedule(
    financed: Money,
    count: u32,
    f: Frequency,
    first: Date,
) -> Result<Vec<Due>, RuleError> {
    if financed.minor() <= 0 {
        return Err(RuleError::on("amount", "plan.nothing_to_finance"));
    }
    if count == 0 || count > MAX_COUNT {
        return Err(RuleError::on("count", "plan.count"));
    }
    let parts = financed.allocate(&vec![1; usize::try_from(count).unwrap_or(1)])?;
    Ok(parts
        .into_iter()
        .enumerate()
        .map(|(i, amount)| Due {
            date: due_date(first, f, u32::try_from(i).unwrap_or(0)),
            amount,
        })
        .collect())
}

/// Where an instalment stands on `today`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Not yet due.
    Upcoming,
    /// Due today.
    Due,
    /// Partly paid, not yet late.
    Partial,
    Paid,
    /// Past its date and not fully paid.
    Late,
}

impl State {
    pub fn code(self) -> &'static str {
        match self {
            State::Upcoming => "upcoming",
            State::Due => "due",
            State::Partial => "partial",
            State::Paid => "paid",
            State::Late => "late",
        }
    }
}

pub fn state(due: Date, amount: i64, paid: i64, today: Date) -> State {
    if paid >= amount {
        State::Paid
    } else if today > due {
        State::Late
    } else if paid > 0 {
        State::Partial
    } else if today == due {
        State::Due
    } else {
        State::Upcoming
    }
}

/// What a payment of `amount` settles, oldest instalment first: (index,
/// part). Each takes at most what it still owes; what is left is returned.
pub fn settle(open: &[i64], amount: i64) -> (Vec<(usize, i64)>, i64) {
    let mut left = amount.max(0);
    let mut out = Vec::new();
    for (i, &owed) in open.iter().enumerate() {
        if left == 0 {
            break;
        }
        if owed <= 0 {
            continue;
        }
        let take = owed.min(left);
        out.push((i, take));
        left -= take;
    }
    (out, left)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nb_core::Currency;

    fn d(s: &str) -> Date {
        Date::parse(s).unwrap_or_else(|e| panic!("{e:?}"))
    }

    #[test]
    fn instalments_add_up_with_the_centimes_first() -> Result<(), RuleError> {
        let s = schedule(
            Money::from_minor(100_000, Currency::DZD),
            3,
            Frequency::Monthly,
            d("2026-01-31"),
        )?;
        assert_eq!(
            s.iter()
                .map(|x| (x.date.iso(), x.amount.minor()))
                .collect::<Vec<_>>(),
            [
                ("2026-01-31".to_owned(), 33_334),
                ("2026-02-28".to_owned(), 33_333),
                ("2026-03-31".to_owned(), 33_333)
            ]
        );
        assert!(
            schedule(
                Money::from_minor(1, Currency::DZD),
                0,
                Frequency::Weekly,
                d("2026-01-01")
            )
            .is_err()
        );
        assert!(
            schedule(
                Money::from_minor(0, Currency::DZD),
                3,
                Frequency::Weekly,
                d("2026-01-01")
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn dates_move_by_weeks_months_and_quarters() {
        let first = d("2026-11-30");
        assert_eq!(due_date(first, Frequency::Weekly, 1).iso(), "2026-12-07");
        assert_eq!(due_date(first, Frequency::Weekly, 5).iso(), "2027-01-04");
        assert_eq!(due_date(first, Frequency::Monthly, 3).iso(), "2027-02-28");
        assert_eq!(due_date(first, Frequency::Quarterly, 1).iso(), "2027-02-28");
        assert_eq!(
            due_date(d("2028-01-31"), Frequency::Monthly, 1).iso(),
            "2028-02-29"
        );
        assert_eq!(days_between(d("2026-12-31"), d("2027-01-02")), 2);
        assert_eq!(add_days(d("2026-03-01"), -1).iso(), "2026-02-28");
    }

    #[test]
    fn states_follow_the_date_and_the_payments() {
        let due = d("2026-11-15");
        assert_eq!(state(due, 100, 0, d("2026-11-01")), State::Upcoming);
        assert_eq!(state(due, 100, 0, due), State::Due);
        assert_eq!(state(due, 100, 40, d("2026-11-01")), State::Partial);
        assert_eq!(state(due, 100, 40, d("2026-11-16")), State::Late);
        assert_eq!(state(due, 100, 100, d("2027-01-01")), State::Paid);
    }

    #[test]
    fn a_payment_settles_the_oldest_first() {
        assert_eq!(settle(&[0, 300, 300], 450), (vec![(1, 300), (2, 150)], 0));
        assert_eq!(settle(&[100], 250), (vec![(0, 100)], 150));
    }
}

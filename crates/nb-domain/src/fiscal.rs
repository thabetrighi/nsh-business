//! Fiscal years and periods (M7.2): a year starts on the first day of the
//! organization's chosen month and has twelve monthly periods. Dates are
//! ISO `YYYY-MM-DD` text, which orders like the dates themselves.

use crate::RuleError;

/// A calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

impl Date {
    pub fn parse(s: &str) -> Result<Date, RuleError> {
        let bad = || RuleError::on("date", "date.format");
        let mut it = s.trim().split('-');
        let (Some(y), Some(m), Some(d), None) = (it.next(), it.next(), it.next(), it.next()) else {
            return Err(bad());
        };
        if y.len() != 4 || m.len() != 2 || d.len() != 2 {
            return Err(bad());
        }
        let date = Date {
            y: y.parse().map_err(|_| bad())?,
            m: m.parse().map_err(|_| bad())?,
            d: d.parse().map_err(|_| bad())?,
        };
        if !(1..=12).contains(&date.m) || date.d == 0 || date.d > days_in_month(date.y, date.m) {
            return Err(bad());
        }
        Ok(date)
    }

    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }

    /// The first day of the month `months` after this date's month.
    fn month_start_after(self, months: u32) -> Date {
        let index = self.y * 12
            + i32::try_from(self.m - 1).unwrap_or(0)
            + i32::try_from(months).unwrap_or(0);
        Date {
            y: index.div_euclid(12),
            m: u32::try_from(index.rem_euclid(12)).unwrap_or(0) + 1,
            d: 1,
        }
    }

    /// The day before (first days of months only need it).
    fn day_before(self) -> Date {
        if self.d > 1 {
            return Date {
                d: self.d - 1,
                ..self
            };
        }
        let (y, m) = if self.m == 1 {
            (self.y - 1, 12)
        } else {
            (self.y, self.m - 1)
        };
        Date {
            y,
            m,
            d: days_in_month(y, m),
        }
    }
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// A fiscal period: its number in the year (1–12) and its first and last
/// days.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Period {
    pub number: u32,
    pub start: Date,
    pub end: Date,
}

/// A fiscal year and its periods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Year {
    /// `2026`, or `2026-2027` when it does not start in January.
    pub name: String,
    pub start: Date,
    pub end: Date,
    pub periods: Vec<Period>,
}

/// The fiscal year holding `date`, for years starting in `start_month`.
pub fn year_of(date: Date, start_month: u32) -> Result<Year, RuleError> {
    if !(1..=12).contains(&start_month) {
        return Err(RuleError::on("fiscal_year_start", "invalid_value"));
    }
    let first_year = if date.m >= start_month {
        date.y
    } else {
        date.y - 1
    };
    let start = Date {
        y: first_year,
        m: start_month,
        d: 1,
    };
    let periods: Vec<Period> = (0..12)
        .map(|i| Period {
            number: i + 1,
            start: start.month_start_after(i),
            end: start.month_start_after(i + 1).day_before(),
        })
        .collect();
    let end = start.month_start_after(12).day_before();
    let name = if start_month == 1 {
        format!("{first_year}")
    } else {
        format!("{first_year}-{}", first_year + 1)
    };
    Ok(Year {
        name,
        start,
        end,
        periods,
    })
}

/// A period may close only after the ones before it, and reopen only
/// before the ones after it: closed periods always form the start of the
/// year. `closed` lists each period's state, in order.
pub fn may_close(closed: &[bool], number: u32) -> Result<(), RuleError> {
    let i = usize::try_from(number).unwrap_or(0).wrapping_sub(1);
    match closed.get(i) {
        None => Err(RuleError::on("period", "fiscal.unknown_period")),
        Some(true) => Err(RuleError::on("period", "fiscal.already_closed")),
        Some(false) if closed[..i].iter().any(|c| !c) => {
            Err(RuleError::on("period", "fiscal.close_in_order"))
        }
        Some(false) => Ok(()),
    }
}

pub fn may_reopen(closed: &[bool], number: u32) -> Result<(), RuleError> {
    let i = usize::try_from(number).unwrap_or(0).wrapping_sub(1);
    match closed.get(i) {
        None => Err(RuleError::on("period", "fiscal.unknown_period")),
        Some(false) => Err(RuleError::on("period", "fiscal.not_closed")),
        Some(true) if closed[i + 1..].iter().any(|c| *c) => {
            Err(RuleError::on("period", "fiscal.reopen_in_order"))
        }
        Some(true) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> Date {
        Date::parse(s).unwrap_or(Date { y: 0, m: 0, d: 0 })
    }

    #[test]
    fn dates() {
        assert_eq!(date("2028-02-29").iso(), "2028-02-29");
        for bad in [
            "2026-02-29",
            "2026-13-01",
            "2026-1-01",
            "26-01-01",
            "2026-01-00",
            "x",
        ] {
            assert_eq!(
                Date::parse(bad).err().map(|e| e.code),
                Some("date.format"),
                "{bad}"
            );
        }
        assert!(date("2026-01-31") < date("2026-02-01"));
    }

    #[test]
    fn calendar_and_shifted_years() -> Result<(), RuleError> {
        let y = year_of(date("2026-11-11"), 1)?;
        assert_eq!(
            (y.name.as_str(), y.start.iso(), y.end.iso()),
            ("2026", "2026-01-01".into(), "2026-12-31".into())
        );
        assert_eq!(y.periods.len(), 12);
        assert_eq!(
            (y.periods[1].start.iso(), y.periods[1].end.iso()),
            ("2026-02-01".into(), "2026-02-28".into())
        );
        // A year from July: March 2027 belongs to 2026-2027.
        let y = year_of(date("2027-03-15"), 7)?;
        assert_eq!(
            (y.name.as_str(), y.start.iso(), y.end.iso()),
            ("2026-2027", "2026-07-01".into(), "2027-06-30".into())
        );
        assert_eq!(y.periods[11].start.iso(), "2027-06-01");
        // Periods follow each other without gaps.
        for w in y.periods.windows(2) {
            assert!(w[0].end < w[1].start);
        }
        assert!(year_of(date("2026-01-01"), 13).is_err());
        Ok(())
    }

    #[test]
    fn periods_close_and_reopen_in_order() {
        let code = |r: Result<(), RuleError>| r.err().map(|e| e.code);
        assert_eq!(may_close(&[false, false], 1), Ok(()));
        assert_eq!(
            code(may_close(&[false, false], 2)),
            Some("fiscal.close_in_order")
        );
        assert_eq!(
            code(may_close(&[true, false], 1)),
            Some("fiscal.already_closed")
        );
        assert_eq!(
            code(may_close(&[true, false], 3)),
            Some("fiscal.unknown_period")
        );
        assert_eq!(may_reopen(&[true, true], 2), Ok(()));
        assert_eq!(
            code(may_reopen(&[true, true], 1)),
            Some("fiscal.reopen_in_order")
        );
        assert_eq!(
            code(may_reopen(&[true, false], 2)),
            Some("fiscal.not_closed")
        );
    }
}

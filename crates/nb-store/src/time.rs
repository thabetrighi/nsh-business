//! Calendar dates from timestamps, without a time library: the business day
//! of a sale depends on the store's UTC offset.

/// (year, month, day) of `ms` since the epoch, shifted by `offset_min`.
pub fn civil_date(ms: i64, offset_min: i32) -> (i32, u32, u32) {
    let local = ms + i64::from(offset_min) * 60_000;
    let days = local.div_euclid(86_400_000);
    civil_from_days(days)
}

/// Howard Hinnant's algorithm: days since 1970-01-01 to a Gregorian date.
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        i32::try_from(y).unwrap_or(i32::MAX),
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// `YYYY-MM-DD`.
pub fn iso_date(ms: i64, offset_min: i32) -> String {
    let (y, m, d) = civil_date(ms, offset_min);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-11-10T22:30:00Z
        let ms = 1_794_349_800_000;
        assert_eq!(civil_date(ms, 0), (2026, 11, 10));
        // Algiers is UTC+1: still the 10th at 23:30.
        assert_eq!(civil_date(ms, 60), (2026, 11, 10));
        // New year's eve 23:30 UTC is already January 1st in Algiers.
        let nye = 1_798_759_800_000; // 2026-12-31T23:30:00Z
        assert_eq!(civil_date(nye, 0), (2026, 12, 31));
        assert_eq!(civil_date(nye, 60), (2027, 1, 1));
        assert_eq!(iso_date(nye, 60), "2027-01-01");
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }
}

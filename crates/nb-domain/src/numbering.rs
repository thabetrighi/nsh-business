//! Document numbers that stay unique without a connection (PRD §8.8, M2.8):
//! each device has a short prefix, and counts per (document kind, branch,
//! year). Two devices selling offline never produce the same number.
//!
//! Format: `{KIND}-{BRANCH}-{YY}-{DEVICE}{SEQ:05}`, e.g. `FV-01-26-A00042`.

use crate::RuleError;

/// A kind of numbered document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocKind {
    Invoice,
    Quote,
    Order,
    SaleReturn,
    Purchase,
    PurchaseOrder,
    PurchaseReturn,
    Transfer,
    StockCount,
    Adjustment,
    Receipt,
    Payment,
    Expense,
    Journal,
}

impl DocKind {
    pub const ALL: [DocKind; 14] = [
        DocKind::Invoice,
        DocKind::Quote,
        DocKind::Order,
        DocKind::SaleReturn,
        DocKind::Purchase,
        DocKind::PurchaseOrder,
        DocKind::PurchaseReturn,
        DocKind::Transfer,
        DocKind::StockCount,
        DocKind::Adjustment,
        DocKind::Receipt,
        DocKind::Payment,
        DocKind::Expense,
        DocKind::Journal,
    ];

    /// The prefix printed on the document (French, as on Algerian forms).
    pub fn code(self) -> &'static str {
        match self {
            DocKind::Invoice => "FV",
            DocKind::Quote => "DV",
            DocKind::Order => "BC",
            DocKind::SaleReturn => "AV",
            DocKind::Purchase => "FA",
            DocKind::PurchaseOrder => "BCF",
            DocKind::PurchaseReturn => "RF",
            DocKind::Transfer => "TR",
            DocKind::StockCount => "INV",
            DocKind::Adjustment => "AJ",
            DocKind::Receipt => "RC",
            DocKind::Payment => "PM",
            DocKind::Expense => "DP",
            DocKind::Journal => "OD",
        }
    }

    pub fn from_code(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.code() == s)
    }
}

/// Checks a device prefix: 1–3 uppercase letters. Letters only, so the
/// digits after it are always the sequence (no `B7`+`123` = `B`+`7123`).
pub fn validate_device_prefix(p: &str) -> Result<(), RuleError> {
    if (1..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(RuleError::on("device_prefix", "numbering.prefix"))
    }
}

/// The number of the `seq`-th document (from 1).
pub fn format(kind: DocKind, branch_code: &str, year: i32, device: &str, seq: u64) -> String {
    format!(
        "{}-{}-{:02}-{}{:05}",
        kind.code(),
        branch_code,
        year.rem_euclid(100),
        device,
        seq
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn numbers_read_well() {
        assert_eq!(
            format(DocKind::Invoice, "01", 2026, "A", 42),
            "FV-01-26-A00042"
        );
        assert_eq!(
            format(DocKind::Purchase, "02", 2031, "BK", 123_456),
            "FA-02-31-BK123456"
        );
        for k in DocKind::ALL {
            assert_eq!(DocKind::from_code(k.code()), Some(k));
        }
    }

    #[test]
    fn devices_never_collide() {
        let mut seen = HashSet::new();
        for device in ["A", "B", "AB"] {
            for seq in 1..=500 {
                assert!(seen.insert(format(DocKind::Invoice, "01", 2026, device, seq)));
            }
        }
    }

    #[test]
    fn prefixes() {
        assert!(validate_device_prefix("A").is_ok());
        assert!(validate_device_prefix("ZQX").is_ok());
        assert!(validate_device_prefix("B7").is_err());
        assert!(validate_device_prefix("").is_err());
        assert!(validate_device_prefix("ab").is_err());
        assert!(validate_device_prefix("ABCD").is_err());
    }
}

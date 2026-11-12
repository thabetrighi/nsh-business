//! Business rules of NSH Business. Pure: no IO, no SQL, no UI, no NSH.
//! Use cases (in `nb-store`) read data, call these rules, and write the
//! results in one transaction (ADR-0003).

pub mod access;
pub mod barcode;
pub mod catalog;
pub mod features;
pub mod fiscal;
pub mod ids;
pub mod import;
pub mod installment;
pub mod ledger;
pub mod numbering;
pub mod posting;
pub mod pricing;
pub mod product;
pub mod purchase;
pub mod sale;
pub mod stock;
pub mod tax;
pub mod units;

pub use ids::*;

/// A rule broken by input: a stable code (the UI translates it as
/// `error.<code>`) and the field it concerns, if any.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}{}", field.map(|f| format!(" ({f})")).unwrap_or_default())]
pub struct RuleError {
    pub code: &'static str,
    pub field: Option<&'static str>,
}

impl RuleError {
    pub const fn new(code: &'static str) -> Self {
        Self { code, field: None }
    }
    pub const fn on(field: &'static str, code: &'static str) -> Self {
        Self {
            code,
            field: Some(field),
        }
    }
}

impl From<nb_core::CoreError> for RuleError {
    fn from(e: nb_core::CoreError) -> Self {
        match e {
            nb_core::CoreError::Overflow => RuleError::new("overflow"),
            nb_core::CoreError::CurrencyMismatch { .. } => RuleError::new("currency_mismatch"),
            nb_core::CoreError::DivisionByZero => RuleError::new("division_by_zero"),
            _ => RuleError::new("invalid_value"),
        }
    }
}

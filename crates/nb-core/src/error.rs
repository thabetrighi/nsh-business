//! Errors of the core value types. Messages are developer-facing; the UI
//! maps each variant to a translated message through `nb-i18n`.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoreError {
    #[error("invalid currency code or exponent: {0}")]
    InvalidCurrency(String),
    #[error("invalid amount: {0}")]
    InvalidAmount(String),
    #[error("{value} has more than {allowed} decimals")]
    TooManyDecimals { value: String, allowed: u8 },
    #[error("currency mismatch: {left} and {right}")]
    CurrencyMismatch { left: String, right: String },
    #[error("arithmetic overflow")]
    Overflow,
    #[error("division by zero")]
    DivisionByZero,
    #[error("allocation needs at least one positive weight")]
    EmptyAllocation,
    #[error("invalid quantity: {0}")]
    InvalidQuantity(String),
    #[error("invalid identifier: {0}")]
    InvalidId(String),
}

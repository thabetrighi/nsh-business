//! Core value types for NSH Business. No IO, no UI, no platform code.
//!
//! - [`Money`]: integer minor units, checked arithmetic, exact allocation.
//! - [`Quantity`] / [`Rate`]: exact decimals for weights, conversions and taxes.
//! - Typed UUIDv7 identifiers ([`typed_id!`]).
//! - [`Clock`] and [`Hlc`] for ordering events across devices.

pub mod clock;
pub mod error;
pub mod id;
pub mod money;
pub mod quantity;

pub use clock::{Clock, FixedClock, Hlc, HlcTimestamp, SystemClock};
pub use error::CoreError;
pub use id::{DeviceId, OperationId, OrganizationId, random_bytes};
pub use money::{Currency, Money};
pub use quantity::{Quantity, Rate};
pub use rust_decimal::{Decimal, RoundingStrategy};

#[doc(hidden)]
pub use serde;
#[doc(hidden)]
pub use uuid;

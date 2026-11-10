//! The operations the app runs. Each takes an [`crate::OpMeta`] (who, which
//! operation id) and runs in one transaction; business rules come from
//! `nb-domain`.

pub mod ledger;
pub mod products;
pub mod setup;
pub mod stock;

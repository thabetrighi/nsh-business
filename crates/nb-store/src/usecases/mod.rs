//! The operations the app runs. Each takes an [`crate::OpMeta`] (who, which
//! operation id) and runs in one transaction; business rules come from
//! `nb-domain`.

pub mod access;
pub mod catalog;
pub mod inventory;
pub mod ledger;
pub mod org;
pub mod products;
pub mod setup;
pub mod stock;
pub mod tracked;

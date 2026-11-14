//! The operations the app runs. Each takes an [`crate::OpMeta`] (who, which
//! operation id) and runs in one transaction; business rules come from
//! `nb-domain`.

pub mod access;
pub mod accounting;
#[cfg(not(target_arch = "wasm32"))]
pub mod backups;
pub mod catalog;
pub mod customers;
pub mod debts;
pub mod distribution;
pub mod documents;
pub mod expenses;
pub mod import;
pub mod installments;
pub mod inventory;
pub mod ledger;
pub mod org;
pub mod printing;
pub mod products;
pub mod purchase_orders;
pub mod purchasing;
pub mod reports;
pub mod returns;
pub mod reviews;
pub mod sales;
pub mod setup;
pub mod stock;
pub mod templates;
pub mod tracked;

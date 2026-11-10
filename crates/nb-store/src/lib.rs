//! The local storage engine (ADR-0002, ADR-0003): SQLite on every platform.
//!
//! - [`Store::open`]: pragmas, numbered migrations with a backup first.
//! - [`Store::write`]: the unit of work. One transaction per business
//!   operation, with its outbox events and audit records, idempotent by
//!   [`nb_core::OperationId`].
//! - [`repo`]: reading and writing rows; [`usecases`]: the operations the
//!   app runs (they call the rules of `nb-domain`).

pub mod error;
pub mod migrate;
pub mod repo;
pub mod store;
pub mod time;
pub mod uow;
pub mod usecases;

pub use error::{Error, Result};
pub use store::{Device, Store, StoreOptions};
pub use uow::{OpMeta, Reader, Writer};

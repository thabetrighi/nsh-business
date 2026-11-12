//! Screens. Each screen is a function of the app state.

pub mod accounting;
pub mod approval;
#[cfg(not(target_arch = "wasm32"))]
pub mod backup;
pub mod catalog;
pub mod common;
pub mod customers;
pub mod design;
pub mod expenses;
pub mod forms;
pub mod home;
pub mod inventory;
pub mod lock;
pub mod picker;
pub mod pos;
pub mod product_editor;
pub mod products;
pub mod purchases;
pub mod reports;
pub mod sales;
pub mod settings;
pub mod settings_catalog;
pub mod users;

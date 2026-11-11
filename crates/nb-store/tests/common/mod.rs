#![allow(dead_code)]

use std::sync::Arc;

use nb_core::FixedClock;
use nb_store::usecases::products::ProductInput;
use nb_store::usecases::setup::{SetupInput, SetupResult, setup_organization};
use nb_store::{OpMeta, Store, StoreOptions};

/// 2026-11-10T10:00:00Z
pub const T0: i64 = 1_794_304_800_000;

pub fn options(prefix: &str) -> StoreOptions {
    StoreOptions {
        clock: Arc::new(FixedClock::new(T0)),
        device_prefix: prefix.into(),
        device_name: "test".into(),
        utc_offset_min: 60,
    }
}

pub fn store() -> (Store, SetupResult) {
    let mut s = Store::open_in_memory(options("A")).unwrap_or_else(|e| panic!("open: {e}"));
    let setup = setup_organization(&mut s, setup_input()).unwrap_or_else(|e| panic!("setup: {e}"));
    (s, setup)
}

pub fn setup_input() -> SetupInput {
    SetupInput {
        org_name: "Magasin En-Nour".into(),
        activity: "grocery".into(),
        country: "DZ".into(),
        currency: "DZD".into(),
        currency_exp: 2,
        branch_name: "El Oued".into(),
        owner_name: "Ahmed".into(),
        language: "fr".into(),
        fiscal_year_start: 1,
    }
}

pub fn meta(s: &SetupResult, kind: &'static str) -> OpMeta {
    OpMeta::new(kind, Some(s.org), Some(s.owner))
}

pub fn product(name: &str, sku: &str, price: i64) -> ProductInput {
    ProductInput {
        name: name.into(),
        sku: sku.into(),
        kind: "goods".into(),
        category_id: None,
        base_unit: "pc".into(),
        base_precision: 0,
        extra_units: vec![],
        barcodes: vec![],
        tax_bps: 1_900,
        price_minor: price,
        min_price_minor: None,
        min_stock: None,
        opening: None,
    }
}

pub fn count(s: &Store, sql: &str) -> i64 {
    s.connection()
        .query_row(sql, [], |r| r.get(0))
        .unwrap_or(-1)
}

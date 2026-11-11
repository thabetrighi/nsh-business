//! Catalog use cases (M5): create, update, archive and list products, with
//! units, barcodes, prices, opening stock and the price history.

use nb_core::{Decimal, Money, Quantity, Rate};
use nb_domain::ledger::{Entry, Line};
use nb_domain::product::{ProductDraft, ProductKind, ValidProduct};
use nb_domain::stock::MovementKind;
use nb_domain::units::{ProductUnit, UnitSet};
use nb_domain::{ProductId, RuleError, WarehouseId};
use serde::{Deserialize, Serialize};

use crate::repo::catalog::{self, ProductPage, ProductQuery, ProductRow};
use crate::repo::{ledger, org, parse_id, stock};
use crate::uow::{OpMeta, Writer};
use crate::{Error, Result, Store};
use nb_domain::access::{Permission, Target};

/// Opening stock entered with a new product.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opening {
    pub warehouse_id: String,
    /// Quantity in the base unit, as exact decimal text.
    pub qty: String,
    pub unit_cost_minor: i64,
}

/// A product as a form or an import sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductInput {
    pub name: String,
    /// Empty: the next free code (`P-00042`).
    pub sku: String,
    pub kind: String,
    pub category_id: Option<String>,
    pub base_unit: String,
    pub base_precision: u32,
    /// Packaging units: (code, factor to the base unit as decimal text).
    pub extra_units: Vec<(String, String)>,
    pub barcodes: Vec<String>,
    pub tax_bps: i64,
    pub price_minor: i64,
    pub min_price_minor: Option<i64>,
    /// Alert level in base units, as decimal text.
    pub min_stock: Option<String>,
    pub opening: Option<Opening>,
}

fn quantity(field: &'static str, s: &str) -> std::result::Result<Quantity, RuleError> {
    s.trim()
        .replace(',', ".")
        .parse::<Quantity>()
        .map_err(|_| RuleError::on(field, "invalid_quantity"))
}

fn draft(
    store_currency: nb_core::Currency,
    input: &ProductInput,
    sku: String,
) -> std::result::Result<ProductDraft, Vec<RuleError>> {
    let mut errors = Vec::new();
    let kind = ProductKind::from_code(&input.kind).unwrap_or_else(|| {
        errors.push(RuleError::on("kind", "invalid_value"));
        ProductKind::Goods
    });
    let mut units = vec![ProductUnit {
        code: input.base_unit.trim().to_owned(),
        factor: Decimal::ONE,
        precision: input.base_precision,
    }];
    for (code, factor) in &input.extra_units {
        match factor.trim().replace(',', ".").parse::<Decimal>() {
            Ok(f) => units.push(ProductUnit {
                code: code.trim().to_owned(),
                factor: f,
                precision: 0,
            }),
            Err(_) => errors.push(RuleError::on("units", "units.factor_positive")),
        }
    }
    let units = match UnitSet::new(units) {
        Ok(u) => Some(u),
        Err(e) => {
            errors.push(e);
            None
        }
    };
    let min_stock = match input.min_stock.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(s) => match quantity("min_stock", s) {
            Ok(q) => Some(q),
            Err(e) => {
                errors.push(e);
                None
            }
        },
    };
    let Some(units) = units else {
        return Err(errors);
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(ProductDraft {
        name: input.name.clone(),
        sku,
        kind,
        units,
        barcodes: input.barcodes.clone(),
        tax_rate: Rate::from_bps(input.tax_bps),
        price: Money::from_minor(input.price_minor, store_currency),
        min_price: input
            .min_price_minor
            .map(|m| Money::from_minor(m, store_currency)),
        min_stock,
    })
}

/// The next free product code: `P-` and one more than the highest number.
pub fn next_sku(store: &Store) -> Result<String> {
    store.read(|r| {
        let max: Option<i64> = r.conn().query_row(
            "SELECT MAX(CAST(substr(sku, 3) AS INTEGER)) FROM products WHERE sku GLOB 'P-[0-9]*'",
            [],
            |row| row.get(0),
        )?;
        Ok(format!("P-{:05}", max.unwrap_or(0) + 1))
    })
}

fn check_unique(w: &Writer<'_>, p: &ValidProduct, except: Option<ProductId>) -> Result<()> {
    let org = w.org()?;
    let mut errors = Vec::new();
    if catalog::sku_taken(w.tx(), org, &p.0.sku, except)? {
        errors.push(RuleError::on("sku", "product.sku_taken"));
    }
    if !catalog::barcodes_taken(w.tx(), org, &p.0.barcodes, except)?.is_empty() {
        errors.push(RuleError::on("barcodes", "barcode.taken"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::Rules(errors))
    }
}

/// Creates a product, with its opening stock if given.
pub fn create_product(store: &mut Store, meta: OpMeta, input: ProductInput) -> Result<ProductRow> {
    let org_row = store
        .read(|r| org::the_org(r.conn()))?
        .ok_or(Error::NotFound("organization"))?;
    let currency = org_row.currency()?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    let sku = if input.sku.trim().is_empty() {
        next_sku(store)?
    } else {
        input.sku.clone()
    };
    let valid = draft(currency, &input, sku)?.validate()?;
    let opening = match &input.opening {
        Some(o) => {
            let q = quantity("opening_qty", &o.qty)?;
            if q.is_negative() {
                return Err(RuleError::on("opening_qty", "stock.quantity_positive").into());
            }
            if o.unit_cost_minor < 0 {
                return Err(RuleError::on("unit_cost", "product.price_negative").into());
            }
            if q.is_zero() {
                None
            } else {
                Some((
                    parse_id::<WarehouseId>(&o.warehouse_id)?,
                    q,
                    o.unit_cost_minor,
                ))
            }
        }
        None => None,
    };
    let id = ProductId::new();
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        check_unique(w, &valid, None)?;
        let org = w.org()?;
        catalog::insert(w, org, id, &valid, input.category_id.as_deref())?;
        catalog::record_price_change(w, id, "retail", None, valid.0.price.minor())?;
        if let Some((wh, qty, cost)) = opening {
            if valid.0.kind == ProductKind::Service {
                return Err(RuleError::on("opening_qty", "product.service_stock").into());
            }
            let warehouse = org::warehouse(w.tx(), wh)?;
            let branch = org::branch(w.tx(), parse_id(&warehouse.branch_id)?)?;
            let number = w.next_number(nb_domain::numbering::DocKind::Adjustment, &branch.id, &branch.code)?;
            let unit_cost = Money::from_minor(cost, currency);
            let before = stock::balance(w.tx(), id, wh, currency)?;
            let after = before.apply(MovementKind::Opening, qty, Some(unit_cost), warehouse.policy(), false)?;
            stock::append(w, id, wh, MovementKind::Opening, qty, Some(unit_cost), Some(&number), None, after)?;
            // Opening stock enters the books against opening equity
            // (provisional mapping 30 / 101, gate G2).
            let value = qty.times_price(unit_cost)?;
            if !value.is_zero() {
                let entry = Entry::new(vec![Line::debit("30", value), Line::credit("101", value)])?;
                let jnumber = w.next_number(nb_domain::numbering::DocKind::Journal, &branch.id, &branch.code)?;
                let date = crate::time::iso_date(w.now(), offset);
                let memo = format!("{number} {}", valid.0.name);
                ledger::post(w, w.org()?, ledger::Header { number: &jnumber, date: &date, memo: &memo, source: "stock.opening", reverses: None }, &entry)?;
            }
            w.event("stock.moved", "product", &id.to_string(), &serde_json::json!({
                "kind": "opening", "warehouse": wh.to_string(), "qty": qty.to_string(), "unit_cost": cost, "number": number
            }))?;
        }
        let row = catalog::get(w.tx(), id)?;
        w.event("product.created", "product", &row.id, &row)?;
        w.audit("create", "product", &row.id, &row)?;
        Ok(row)
    })
}

/// Updates a product read at `version`.
pub fn update_product(
    store: &mut Store,
    meta: OpMeta,
    id: ProductId,
    version: i64,
    input: ProductInput,
) -> Result<ProductRow> {
    let org_row = store
        .read(|r| org::the_org(r.conn()))?
        .ok_or(Error::NotFound("organization"))?;
    let currency = org_row.currency()?;
    if input.opening.is_some() {
        return Err(RuleError::on("opening_qty", "product.opening_on_update").into());
    }
    let valid = draft(currency, &input, input.sku.clone())?.validate()?;
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        let before = catalog::get(w.tx(), id)?;
        if before.price_minor != valid.0.price.minor() {
            w.require(Permission::ProductsPrice, &Target::org())?;
        }
        check_unique(w, &valid, Some(id))?;
        let org = w.org()?;
        catalog::update(w, org, id, version, &valid, input.category_id.as_deref())?;
        if before.price_minor != valid.0.price.minor() {
            catalog::record_price_change(
                w,
                id,
                "retail",
                Some(before.price_minor),
                valid.0.price.minor(),
            )?;
        }
        let after = catalog::get(w.tx(), id)?;
        w.event("product.updated", "product", &after.id, &after)?;
        w.audit(
            "update",
            "product",
            &after.id,
            &serde_json::json!({ "before": before, "after": after }),
        )?;
        Ok(after)
    })
}

/// Archives products (they stay in past documents).
pub fn archive_products(store: &mut Store, meta: OpMeta, ids: &[ProductId]) -> Result<u32> {
    store.write(meta, |w| {
        w.require(Permission::ProductsArchive, &Target::org())?;
        for id in ids {
            catalog::archive(w, *id)?;
            w.event(
                "product.archived",
                "product",
                &id.to_string(),
                &serde_json::json!({}),
            )?;
            w.audit(
                "archive",
                "product",
                &id.to_string(),
                &serde_json::json!({}),
            )?;
        }
        Ok(u32::try_from(ids.len()).unwrap_or(u32::MAX))
    })
}

/// The products screen's page.
pub fn list_products(store: &Store, q: &ProductQuery) -> Result<ProductPage> {
    store.read(|r| {
        let org = org::the_org(r.conn())?.ok_or(Error::NotFound("organization"))?;
        catalog::query(r.conn(), org.org_id()?, q)
    })
}

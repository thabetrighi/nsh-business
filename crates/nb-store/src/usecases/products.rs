//! Catalog use cases (M5): create, update, archive and list products, with
//! units, barcodes, prices, opening stock and the price history.

use nb_core::{Decimal, Money, Quantity, Rate};
use nb_domain::product::{ProductDraft, ProductKind, ValidProduct};
use nb_domain::stock::MovementKind;
use nb_domain::units::{ProductUnit, UnitSet};
use nb_domain::{ProductId, RuleError, WarehouseId};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::repo::catalog::{self, ProductPage, ProductQuery, ProductRow};
use crate::repo::{org, parse_id, stock};
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
    /// Lot (with expiry) or serial numbers of the opening stock.
    #[serde(default)]
    pub tracked: crate::usecases::tracked::Tracked,
}

/// A product as a form or an import sends it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Barcodes of packaging units: (code, unit code).
    #[serde(default)]
    pub pack_barcodes: Vec<(String, String)>,
    /// A tax class; when given, its rate replaces `tax_bps`.
    #[serde(default)]
    pub tax_class_id: Option<String>,
    #[serde(default)]
    pub brand_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub alt_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Usual purchase cost of one base unit.
    #[serde(default)]
    pub cost_minor: Option<i64>,
    /// Article code in scale labels (weighed goods), digits only.
    #[serde(default)]
    pub plu: Option<String>,
    /// `none` (default), `lot` or `serial`.
    #[serde(default)]
    pub tracking: Option<String>,
}

impl ProductInput {
    fn extra(&self, parent: Option<(String, String)>) -> catalog::ProductExtra {
        let text = |s: &Option<String>| {
            s.as_deref()
                .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|v| !v.is_empty())
        };
        let (parent_id, variant_attrs) = parent.unzip();
        catalog::ProductExtra {
            category_id: self.category_id.clone().filter(|c| !c.is_empty()),
            brand_id: self.brand_id.clone().filter(|c| !c.is_empty()),
            tax_class_id: self.tax_class_id.clone().filter(|c| !c.is_empty()),
            alt_name: text(&self.alt_name),
            description: self
                .description
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(Into::into),
            plu: self
                .plu
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(Into::into),
            parent_id,
            variant_attrs,
            tracking: Some(
                self.tracking
                    .clone()
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "none".into()),
            ),
        }
    }
}

/// Checks what refers to other rows (category, brand, tax class exist) and
/// takes the tax class's rate.
fn resolve_refs(w: &Writer<'_>, input: &ProductInput, valid: &mut ValidProduct) -> Result<()> {
    let org = w.org()?.to_string();
    let exists = |table: &str, id: &Option<String>| -> Result<bool> {
        match id.as_deref().filter(|i| !i.is_empty()) {
            None => Ok(true),
            Some(id) => Ok(w.tx().query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL"),
                rusqlite::params![id, org],
                |r| r.get::<_, i64>(0),
            )? == 1),
        }
    };
    let mut errors = Vec::new();
    if !exists("categories", &input.category_id)? {
        errors.push(RuleError::on("category", "not_found"));
    }
    if !exists("brands", &input.brand_id)? {
        errors.push(RuleError::on("brand", "not_found"));
    }
    match input.tax_class_id.as_deref().filter(|i| !i.is_empty()) {
        None => {}
        Some(id) => {
            let rate: Option<i64> = rusqlite::OptionalExtension::optional(w.tx().query_row(
                "SELECT rate_bps FROM tax_classes WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
                rusqlite::params![id, org],
                |r| r.get(0),
            ))?;
            match rate {
                Some(bps) => valid.0.tax_rate = Rate::from_bps(bps),
                None => errors.push(RuleError::on("tax_class", "not_found")),
            }
        }
    }
    if input
        .tracking
        .as_deref()
        .is_some_and(|t| !t.is_empty() && crate::repo::tracking::Tracking::from_code(t).is_none())
    {
        errors.push(RuleError::on("tracking", "invalid_value"));
    }
    if let Some(plu) = input
        .plu
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        && (plu.len() > 6 || !plu.bytes().all(|b| b.is_ascii_digit()))
    {
        errors.push(RuleError::on("plu", "product.plu_format"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::Rules(errors))
    }
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
        pack_barcodes: input.pack_barcodes.clone(),
        tax_rate: Rate::from_bps(input.tax_bps),
        price: Money::from_minor(input.price_minor, store_currency),
        min_price: input
            .min_price_minor
            .map(|m| Money::from_minor(m, store_currency)),
        cost: input
            .cost_minor
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
/// A product checked and ready to write.
pub(crate) struct Prepared {
    pub input: ProductInput,
    pub valid: ValidProduct,
    /// Opening stock: warehouse, base quantity, unit cost.
    pub opening: Option<(WarehouseId, Quantity, i64)>,
    pub opening_tracked: crate::usecases::tracked::Tracked,
}

/// Checks a product input with the rules that need no database.
pub(crate) fn prepare(
    input: ProductInput,
    sku: String,
    currency: nb_core::Currency,
) -> Result<Prepared> {
    let valid = draft(currency, &input, sku)?.validate()?;
    let opening = match &input.opening {
        Some(o) => {
            let q = quantity("opening_qty", &o.qty)?;
            if q.is_negative() {
                return Err(RuleError::on("opening_qty", "stock.quantity_positive").into());
            }
            if !q.fits_precision(valid.0.units.base_unit().precision) {
                return Err(RuleError::on("opening_qty", "units.too_precise").into());
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
    let opening_tracked = input
        .opening
        .as_ref()
        .map(|o| o.tracked.clone())
        .unwrap_or_default();
    Ok(Prepared {
        input,
        valid,
        opening,
        opening_tracked,
    })
}

/// Writes a prepared product (and its opening stock) in the running
/// operation. With `journal`, the opening stock's entry is posted here;
/// otherwise its value is returned for one entry over many products.
pub(crate) fn write_new(
    w: &mut Writer<'_>,
    id: ProductId,
    p: &mut Prepared,
    currency: nb_core::Currency,
    offset: i32,
    parent: Option<(String, String)>,
    journal: bool,
) -> Result<(ProductRow, Money)> {
    check_unique(w, &p.valid, None)?;
    resolve_refs(w, &p.input, &mut p.valid)?;
    let org = w.org()?;
    catalog::insert(w, org, id, &p.valid, &p.input.extra(parent))?;
    catalog::set_tags(w, org, id, &p.input.tags)?;
    catalog::record_price_change(w, id, "retail", None, p.valid.0.price.minor())?;
    let mut value = Money::zero(currency);
    if let Some((wh, qty, cost)) = p.opening {
        if p.valid.0.kind == ProductKind::Service {
            return Err(RuleError::on("opening_qty", "product.service_stock").into());
        }
        let warehouse = org::warehouse(w.tx(), wh)?;
        let branch = org::branch(w.tx(), parse_id(&warehouse.branch_id)?)?;
        let number = w.next_number(
            nb_domain::numbering::DocKind::Adjustment,
            &branch.id,
            &branch.code,
        )?;
        let unit_cost = Money::from_minor(cost, currency);
        let before = stock::balance(w.tx(), id, wh, currency)?;
        let after = before.apply(
            MovementKind::Opening,
            qty,
            Some(unit_cost),
            warehouse.policy(),
            false,
        )?;
        let movement = stock::append(
            w,
            id,
            wh,
            MovementKind::Opening,
            qty,
            Some(unit_cost),
            Some(&number),
            None,
            after,
        )?;
        crate::usecases::tracked::record(
            w,
            movement,
            id,
            wh,
            MovementKind::Opening,
            qty,
            &p.opening_tracked,
            false,
        )?;
        value = qty.times_price(unit_cost)?;
        // Opening stock enters the books against opening equity
        // (provisional mapping 30 / 101, gate G2).
        if journal && !value.is_zero() {
            let memo = format!("{number} {}", p.valid.0.name);
            post_opening(w, &branch, offset, value, &memo)?;
        }
        w.event("stock.moved", "product", &id.to_string(), &serde_json::json!({
            "kind": "opening", "warehouse": wh.to_string(), "qty": qty.to_string(), "unit_cost": cost, "number": number
        }))?;
    }
    let row = catalog::get(w.tx(), id)?;
    w.event("product.created", "product", &row.id, &row)?;
    w.audit("create", "product", &row.id, &row)?;
    Ok((row, value))
}

/// The opening stock entry (stock / opening equity).
pub(crate) fn post_opening(
    w: &mut Writer<'_>,
    branch: &org::Branch,
    offset: i32,
    value: Money,
    memo: &str,
) -> Result<()> {
    crate::usecases::accounting::post_operations(
        w,
        branch,
        offset,
        &[nb_domain::posting::Operation::StockOpening { value }],
        memo,
        "stock.opening",
    )?;
    Ok(())
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
    let mut p = prepare(input, sku, currency)?;
    let id = ProductId::new();
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        Ok(write_new(w, id, &mut p, currency, offset, None, true)?.0)
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
    let currency = store
        .read(|r| org::the_org(r.conn()))?
        .ok_or(Error::NotFound("organization"))?
        .currency()?;
    let mut p = prepare_update(input, currency)?;
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        write_update(w, id, version, &mut p)
    })
}

/// Checks an edited product with the rules that need no database.
pub(crate) fn prepare_update(input: ProductInput, currency: nb_core::Currency) -> Result<Prepared> {
    if input.opening.is_some() {
        return Err(RuleError::on("opening_qty", "product.opening_on_update").into());
    }
    let sku = input.sku.clone();
    prepare(input, sku, currency)
}

/// Writes an edited product read at `version` in the running operation.
pub(crate) fn write_update(
    w: &mut Writer<'_>,
    id: ProductId,
    version: i64,
    p: &mut Prepared,
) -> Result<ProductRow> {
    let before = catalog::get(w.tx(), id)?;
    if before.price_minor != p.valid.0.price.minor() {
        w.require(Permission::ProductsPrice, &Target::org())?;
    }
    check_unique(w, &p.valid, Some(id))?;
    resolve_refs(w, &p.input, &mut p.valid)?;
    // A conversion stock already moved with keeps its meaning (PRD §6.4).
    if before.unit_set()?.reinterprets(&p.valid.0.units) && catalog::has_movements(w.tx(), id)? {
        return Err(RuleError::on("units", "units.in_use").into());
    }
    let tracking = p
        .input
        .tracking
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| before.tracking.clone());
    if tracking != before.tracking && catalog::has_movements(w.tx(), id)? {
        return Err(RuleError::on("tracking", "product.tracking_in_use").into());
    }
    let org = w.org()?;
    let parent = before
        .parent_id
        .clone()
        .map(|pid| (pid, before.variant_attrs.clone().unwrap_or_default()));
    let mut extra = p.input.extra(parent);
    extra.tracking = Some(tracking);
    if extra.parent_id.is_none() {
        extra.variant_attrs = before.variant_attrs.clone();
    }
    catalog::update(w, org, id, version, &p.valid, &extra)?;
    catalog::set_tags(w, org, id, &p.input.tags)?;
    if before.price_minor != p.valid.0.price.minor() {
        catalog::record_price_change(
            w,
            id,
            "retail",
            Some(before.price_minor),
            p.valid.0.price.minor(),
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

/// Moves products to a category (`None`: out of any), several at once.
pub fn set_products_category(
    store: &mut Store,
    meta: OpMeta,
    ids: &[ProductId],
    category_id: Option<&str>,
) -> Result<u32> {
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        let org = w.org()?.to_string();
        if let Some(c) = category_id {
            let known: Option<i64> = w
                .tx()
                .query_row(
                    "SELECT 1 FROM categories WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
                    rusqlite::params![c, org],
                    |r| r.get(0),
                )
                .optional()?;
            if known.is_none() {
                return Err(Error::NotFound("category"));
            }
        }
        for id in ids {
            let n = w.tx().execute(
                "UPDATE products SET category_id = ?1, updated_at = ?2, version = version + 1
                 WHERE id = ?3 AND org_id = ?4 AND deleted_at IS NULL",
                rusqlite::params![category_id, w.now(), id.to_string(), org],
            )?;
            if n == 0 {
                return Err(Error::NotFound("product"));
            }
            catalog::reindex(w.tx(), *id)?;
            let what = serde_json::json!({ "category": category_id });
            w.event("product.updated", "product", &id.to_string(), &what)?;
            w.audit("set_category", "product", &id.to_string(), &what)?;
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

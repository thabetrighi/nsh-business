//! The catalog around products (M5): category trees, brands, tax classes,
//! price lists and list prices, scale label layouts and scanning, variants,
//! pictures, and the CSV import. Changes need `products.edit` (prices:
//! `products.price`).

use std::collections::BTreeMap;

use nb_core::{Currency, Decimal, Money, OrganizationId, Quantity};
use nb_domain::access::{Permission, Target};
use nb_domain::barcode::{ScaleFormat, ScaleRead, ScaleValue};
use nb_domain::catalog::{self as rules, Attribute};
use nb_domain::pricing::{self, PriceContext, PriceList, PriceRule};
use nb_domain::{BranchId, CategoryId, CustomerId, ProductId, RuleError};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::repo::catalog::{self, ProductRow};
use crate::repo::{from_milli, org, parse_id, to_milli};
use crate::uow::{OpMeta, Writer};
use crate::usecases::products::{
    Opening, ProductInput, next_sku, post_opening, prepare, write_new,
};
use crate::{Error, Result, Store};

fn the_org(conn: &Connection) -> Result<(OrganizationId, Currency)> {
    let o = org::the_org(conn)?.ok_or(Error::NotFound("organization"))?;
    Ok((o.org_id()?, o.currency()?))
}

fn clean(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn constraint<T>(r: &rusqlite::Result<T>) -> bool {
    matches!(r, Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation)
}

/// What a new organization's catalog starts with: the retail and wholesale
/// lists, the country's tax classes, the common scale label layout.
pub(crate) fn seed_defaults(
    w: &Writer<'_>,
    org: OrganizationId,
    country: &str,
    language: &str,
) -> Result<()> {
    let device = w.device().id.to_string();
    for code in ["retail", "wholesale"] {
        w.tx().execute(
            "INSERT INTO price_lists (id, org_id, code, name, created_at, updated_at, device_id) VALUES (?1, ?2, ?3, ?3, ?4, ?4, ?5)",
            params![nb_core::OperationId::new().to_string(), org.to_string(), code, w.now(), device],
        )?;
    }
    if country == "DZ" {
        // Named in the organization's language; renamed freely later.
        let names = match language {
            "ar" => [
                "الرسم على القيمة المضافة 19 %",
                "الرسم على القيمة المضافة 9 %",
                "معفى",
            ],
            "en" => ["VAT 19 %", "VAT 9 %", "Exempt"],
            _ => ["TVA 19 %", "TVA 9 %", "Exonéré"],
        };
        for (name, bps, default) in [
            (names[0], 1_900, true),
            (names[1], 900, false),
            (names[2], 0, false),
        ] {
            w.tx().execute(
                "INSERT INTO tax_classes (id, org_id, name, rate_bps, is_default, created_at, updated_at, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
                params![nb_core::OperationId::new().to_string(), org.to_string(), name, bps, default, w.now(), device],
            )?;
        }
    }
    w.tx().execute(
        "INSERT INTO scale_formats (id, org_id, position, prefix, item_digits, value_digits, value_kind, decimals, created_at)
         VALUES (?1, ?2, 0, '21', 5, 5, 'weight', 3, ?3)",
        params![nb_core::OperationId::new().to_string(), org.to_string(), w.now()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Categories (M5.1)

/// Category → parent, and category → name.
type Parents = BTreeMap<String, Option<String>>;
type Names = BTreeMap<String, String>;
/// (list, old price, new price, when).
pub type PriceChange = (String, Option<i64>, i64, i64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryRow {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    /// Names from the top, this one last.
    pub path: Vec<String>,
    pub depth: u32,
    /// Products directly in it.
    pub products: u32,
}

fn tree(conn: &Connection, org: OrganizationId) -> Result<(Parents, Names)> {
    let mut s = conn.prepare(
        "SELECT id, parent_id, name FROM categories WHERE org_id = ?1 AND deleted_at IS NULL",
    )?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut parents = BTreeMap::new();
    let mut names = BTreeMap::new();
    for row in rows {
        let (id, parent, name) = row?;
        parents.insert(id.clone(), parent);
        names.insert(id, name);
    }
    Ok((parents, names))
}

/// The categories in tree order (each followed by its children).
pub fn list_categories(store: &Store) -> Result<Vec<CategoryRow>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let (parents, names) = tree(r.conn(), org)?;
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        let mut s = r.conn().prepare(
            "SELECT category_id, COUNT(*) FROM products WHERE org_id = ?1 AND deleted_at IS NULL AND category_id IS NOT NULL GROUP BY category_id",
        )?;
        for row in s.query_map([org.to_string()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))? {
            let (id, n) = row?;
            counts.insert(id, n);
        }
        let mut rows: Vec<CategoryRow> = names
            .keys()
            .map(|id| {
                let path = rules::path(&parents, &names, id);
                CategoryRow {
                    id: id.clone(),
                    name: names.get(id).cloned().unwrap_or_default(),
                    parent_id: parents.get(id).cloned().flatten(),
                    depth: u32::try_from(path.len()).unwrap_or(1),
                    products: counts.get(id).copied().unwrap_or(0),
                    path,
                }
            })
            .collect();
        rows.sort_by_key(|c| c.path.iter().map(|p| p.to_lowercase()).collect::<Vec<_>>());
        Ok(rows)
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryInput {
    pub id: Option<String>,
    pub name: String,
    pub parent_id: Option<String>,
}

pub fn save_category(store: &mut Store, meta: OpMeta, input: CategoryInput) -> Result<CategoryRow> {
    let name = clean(&input.name);
    if name.is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| CategoryId::new().to_string());
    let parent = input.parent_id.clone().filter(|p| !p.is_empty());
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        let org = w.org()?;
        let (parents, names) = tree(w.tx(), org)?;
        if let Some(p) = &parent
            && !names.contains_key(p)
        {
            return Err(RuleError::on("parent", "not_found").into());
        }
        if rules::would_cycle(&parents, &id, parent.as_deref()) {
            return Err(RuleError::on("parent", "category.cycle").into());
        }
        if rules::depth_with(&parents, parent.as_deref()) > rules::MAX_DEPTH {
            return Err(RuleError::on("parent", "category.too_deep").into());
        }
        let same_name = names
            .iter()
            .any(|(other, n)| *other != id && n.to_lowercase() == name.to_lowercase() && parents.get(other).cloned().flatten() == parent);
        if same_name {
            return Err(RuleError::on("name", "category.name_taken").into());
        }
        if input.id.is_some() {
            if !names.contains_key(&id) {
                return Err(Error::NotFound("category"));
            }
            w.tx().execute(
                "UPDATE categories SET name = ?2, parent_id = ?3, updated_at = ?4, version = version + 1 WHERE id = ?1",
                params![id, name, parent, w.now()],
            )?;
        } else {
            w.tx().execute(
                "INSERT INTO categories (id, org_id, parent_id, name, created_at, updated_at, created_by, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
                params![id, org.to_string(), parent, name, w.now(), w.actor_text(), w.device().id.to_string()],
            )?;
        }
        let (parents, names) = tree(w.tx(), org)?;
        let path = rules::path(&parents, &names, &id);
        let row = CategoryRow {
            id: id.clone(),
            name,
            parent_id: parent,
            depth: u32::try_from(path.len()).unwrap_or(1),
            path,
            products: 0,
        };
        w.event("category.saved", "category", &id, &row)?;
        w.audit("save", "category", &id, &row)?;
        Ok(row)
    })
}

/// Archives an empty category (no products, no sub-categories).
pub fn archive_category(store: &mut Store, meta: OpMeta, id: &str) -> Result<()> {
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        let busy: i64 = w.tx().query_row(
            "SELECT (SELECT COUNT(*) FROM products WHERE category_id = ?1 AND deleted_at IS NULL)
                  + (SELECT COUNT(*) FROM categories WHERE parent_id = ?1 AND deleted_at IS NULL)",
            [id],
            |r| r.get(0),
        )?;
        if busy > 0 {
            return Err(RuleError::on("category", "category.not_empty").into());
        }
        let n = w.tx().execute(
            "UPDATE categories SET deleted_at = ?2, updated_at = ?2, version = version + 1 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, w.now()],
        )?;
        if n == 0 {
            return Err(Error::NotFound("category"));
        }
        w.event("category.archived", "category", id, &id)?;
        w.audit("archive", "category", id, &id)?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Brands (M5.1)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Named {
    pub id: String,
    pub name: String,
}

pub fn list_brands(store: &Store) -> Result<Vec<Named>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT id, name FROM brands WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY name COLLATE NOCASE",
        )?;
        let rows = s.query_map([org.to_string()], |r| Ok(Named { id: r.get(0)?, name: r.get(1)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

pub fn save_brand(
    store: &mut Store,
    meta: OpMeta,
    id: Option<String>,
    name: &str,
) -> Result<Named> {
    let name = clean(name);
    if name.is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        let org = w.org()?;
        let row = Named { id: id.clone().unwrap_or_else(|| nb_core::OperationId::new().to_string()), name };
        let r = match &id {
            Some(id) => w.tx().execute(
                "UPDATE brands SET name = ?3, updated_at = ?4, version = version + 1 WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
                params![id, org.to_string(), row.name, w.now()],
            ),
            None => w.tx().execute(
                "INSERT INTO brands (id, org_id, name, created_at, updated_at, created_by, device_id) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6)",
                params![row.id, org.to_string(), row.name, w.now(), w.actor_text(), w.device().id.to_string()],
            ),
        };
        if constraint(&r) {
            return Err(RuleError::on("name", "catalog.name_taken").into());
        }
        if r? == 0 {
            return Err(Error::NotFound("brand"));
        }
        w.event("brand.saved", "brand", &row.id, &row)?;
        w.audit("save", "brand", &row.id, &row)?;
        Ok(row)
    })
}

/// Tags in use, for suggestions.
pub fn list_tags(store: &Store) -> Result<Vec<String>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT name FROM tags WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY name COLLATE NOCASE",
        )?;
        let rows = s.query_map([org.to_string()], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

// ---------------------------------------------------------------------------
// Tax classes (M5.8)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxClass {
    pub id: String,
    pub name: String,
    pub rate_bps: i64,
    pub is_default: bool,
}

pub fn list_tax_classes(store: &Store) -> Result<Vec<TaxClass>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT id, name, rate_bps, is_default FROM tax_classes WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY rate_bps DESC, name",
        )?;
        let rows = s.query_map([org.to_string()], |r| {
            Ok(TaxClass { id: r.get(0)?, name: r.get(1)?, rate_bps: r.get(2)?, is_default: r.get(3)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

/// Saves a tax class. Changing a rate re-rates its products from now on
/// (documents already issued keep their own rate).
pub fn save_tax_class(store: &mut Store, meta: OpMeta, c: TaxClass) -> Result<TaxClass> {
    let name = clean(&c.name);
    if name.is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    if !(0..=10_000).contains(&c.rate_bps) {
        return Err(RuleError::on("rate", "tax.rate_range").into());
    }
    store.write(meta, |w| {
        w.require(Permission::SettingsEdit, &Target::org())?;
        let org = w.org()?;
        let id = if c.id.is_empty() { nb_core::OperationId::new().to_string() } else { c.id.clone() };
        if c.is_default {
            w.tx().execute("UPDATE tax_classes SET is_default = 0 WHERE org_id = ?1", [org.to_string()])?;
        }
        let n = w.tx().execute(
            "UPDATE tax_classes SET name = ?3, rate_bps = ?4, is_default = ?5, updated_at = ?6, version = version + 1
             WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
            params![id, org.to_string(), name, c.rate_bps, c.is_default, w.now()],
        )?;
        if n == 0 {
            w.tx().execute(
                "INSERT INTO tax_classes (id, org_id, name, rate_bps, is_default, created_at, updated_at, created_by, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
                params![id, org.to_string(), name, c.rate_bps, c.is_default, w.now(), w.actor_text(), w.device().id.to_string()],
            )?;
        }
        let moved = w.tx().execute(
            "UPDATE products SET tax_bps = ?2, updated_at = ?3, version = version + 1 WHERE tax_class_id = ?1 AND tax_bps != ?2",
            params![id, c.rate_bps, w.now()],
        )?;
        let row = TaxClass { id, name, rate_bps: c.rate_bps, is_default: c.is_default };
        w.event("tax_class.saved", "tax_class", &row.id, &row)?;
        w.audit("save", "tax_class", &row.id, &serde_json::json!({ "class": row, "products_rerated": moved }))?;
        Ok(row)
    })
}

// ---------------------------------------------------------------------------
// Price lists and prices (M5.7)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceListRow {
    pub id: String,
    /// `retail`, `wholesale`, or `c<n>` for the organization's own.
    pub code: String,
    /// Empty for the two standard lists (named by `price_list.<code>`).
    pub name: String,
}

pub fn list_price_lists(store: &Store) -> Result<Vec<PriceListRow>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT id, code, CASE WHEN code IN ('retail', 'wholesale') THEN '' ELSE name END FROM price_lists
             WHERE org_id = ?1 AND deleted_at IS NULL
             ORDER BY CASE code WHEN 'retail' THEN 0 WHEN 'wholesale' THEN 1 ELSE 2 END, name COLLATE NOCASE",
        )?;
        let rows = s.query_map([org.to_string()], |r| Ok(PriceListRow { id: r.get(0)?, code: r.get(1)?, name: r.get(2)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

/// Adds a list of the organization's own (semi-wholesale, distributors…).
pub fn create_price_list(store: &mut Store, meta: OpMeta, name: &str) -> Result<PriceListRow> {
    let name = clean(name);
    if name.is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    store.write(meta, |w| {
        w.require(Permission::ProductsPrice, &Target::org())?;
        let org = w.org()?;
        let taken: bool = w.tx().query_row(
            "SELECT EXISTS (SELECT 1 FROM price_lists WHERE org_id = ?1 AND lower(name) = lower(?2) AND deleted_at IS NULL)",
            params![org.to_string(), name],
            |r| r.get(0),
        )?;
        if taken {
            return Err(RuleError::on("name", "catalog.name_taken").into());
        }
        let n: i64 = w.tx().query_row("SELECT COUNT(*) FROM price_lists WHERE org_id = ?1", [org.to_string()], |r| r.get(0))?;
        let row = PriceListRow { id: nb_core::OperationId::new().to_string(), code: format!("c{}", n - 1), name };
        w.tx().execute(
            "INSERT INTO price_lists (id, org_id, code, name, created_at, updated_at, device_id) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
            params![row.id, org.to_string(), row.code, row.name, w.now(), w.device().id.to_string()],
        )?;
        w.event("price_list.created", "price_list", &row.id, &row)?;
        w.audit("create", "price_list", &row.id, &row)?;
        Ok(row)
    })
}

/// One price of a product in a list, as stored and edited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceInput {
    pub list: String,
    pub price_minor: i64,
    /// From this quantity in base units (decimal text); empty for any.
    pub min_qty: String,
    pub branch_id: Option<String>,
    pub customer_id: Option<String>,
}

fn list_of(code: &str) -> Option<PriceList> {
    match code {
        "retail" => Some(PriceList::Retail),
        "wholesale" => Some(PriceList::Wholesale),
        c => c
            .strip_prefix('c')
            .and_then(|n| n.parse().ok())
            .map(PriceList::Custom),
    }
}

/// The list prices of a product (besides its retail base price).
pub fn product_prices(store: &Store, product: ProductId) -> Result<Vec<PriceInput>> {
    store.read(|r| prices_of(r.conn(), product))
}

fn prices_of(conn: &Connection, product: ProductId) -> Result<Vec<PriceInput>> {
    let mut s = conn.prepare(
        "SELECT list, price_minor, min_qty_milli, branch_id, customer_id FROM product_prices
         WHERE product_id = ?1 ORDER BY list, min_qty_milli",
    )?;
    let rows = s.query_map([product.to_string()], |r| {
        Ok(PriceInput {
            list: r.get(0)?,
            price_minor: r.get(1)?,
            min_qty: from_milli(r.get(2)?).to_string(),
            branch_id: r.get(3)?,
            customer_id: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Replaces a product's list prices. Prices under the product's floor are
/// refused; every change is kept in the price history.
pub fn set_product_prices(
    store: &mut Store,
    meta: OpMeta,
    product: ProductId,
    prices: Vec<PriceInput>,
) -> Result<Vec<PriceInput>> {
    let mut parsed = Vec::new();
    for p in &prices {
        if list_of(&p.list).is_none() {
            return Err(RuleError::on("list", "invalid_value").into());
        }
        if p.price_minor < 0 {
            return Err(RuleError::on("price", "product.price_negative").into());
        }
        let q = if p.min_qty.trim().is_empty() {
            Quantity::whole(0)
        } else {
            p.min_qty
                .trim()
                .replace(',', ".")
                .parse::<Quantity>()
                .map_err(|_| RuleError::on("min_qty", "invalid_quantity"))?
        };
        if q.is_negative() {
            return Err(RuleError::on("min_qty", "invalid_quantity").into());
        }
        parsed.push((p, to_milli(q)?));
    }
    let mut keys: Vec<_> = parsed
        .iter()
        .map(|(p, q)| (&p.list, *q, &p.branch_id, &p.customer_id))
        .collect();
    keys.sort();
    if keys.windows(2).any(|k| k[0] == k[1]) {
        return Err(RuleError::on("prices", "price.duplicate").into());
    }
    store.write(meta, |w| {
        w.require(Permission::ProductsPrice, &Target::org())?;
        let org = w.org()?;
        let p = catalog::get(w.tx(), product)?;
        for (pi, _) in &parsed {
            let listed: bool = w.tx().query_row(
                "SELECT EXISTS (SELECT 1 FROM price_lists WHERE org_id = ?1 AND code = ?2 AND deleted_at IS NULL)",
                params![org.to_string(), pi.list],
                |r| r.get(0),
            )?;
            if !listed {
                return Err(RuleError::on("list", "not_found").into());
            }
            if p.min_price_minor.is_some_and(|f| pi.price_minor < f) {
                return Err(RuleError::on("price", "price.below_floor").into());
            }
        }
        let before = prices_of(w.tx(), product)?;
        w.tx().execute("DELETE FROM product_prices WHERE product_id = ?1", [product.to_string()])?;
        for (pi, q) in &parsed {
            w.tx().execute(
                "INSERT INTO product_prices (id, product_id, list, price_minor, min_qty_milli, branch_id, customer_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![nb_core::OperationId::new().to_string(), product.to_string(), pi.list, pi.price_minor, q, pi.branch_id, pi.customer_id],
            )?;
            let old = before
                .iter()
                .find(|b| b.list == pi.list && b.branch_id == pi.branch_id && b.customer_id == pi.customer_id && to_milli(b.min_qty.parse().unwrap_or_default()).ok() == Some(*q))
                .map(|b| b.price_minor);
            if old != Some(pi.price_minor) {
                catalog::record_price_change(w, product, &pi.list, old, pi.price_minor)?;
            }
        }
        let after = prices_of(w.tx(), product)?;
        w.event("product.prices", "product", &product.to_string(), &after)?;
        w.audit("prices", "product", &product.to_string(), &serde_json::json!({ "before": before, "after": after }))?;
        Ok(after)
    })
}

/// What one base unit of a product costs in a context (list, quantity,
/// branch, customer). The product's own price is its retail price for
/// any quantity.
pub fn resolve_price(
    store: &Store,
    product: ProductId,
    list: &str,
    qty: Quantity,
    branch: Option<BranchId>,
    customer: Option<CustomerId>,
) -> Result<Money> {
    store.read(|r| {
        let (_, currency) = the_org(r.conn())?;
        let p = catalog::get(r.conn(), product)?;
        let mut rules = vec![PriceRule {
            list: PriceList::Retail,
            price: p.price(currency),
            min_qty: Quantity::whole(0),
            branch: None,
            customer: None,
        }];
        for pi in prices_of(r.conn(), product)? {
            let Some(l) = list_of(&pi.list) else { continue };
            rules.push(PriceRule {
                list: l,
                price: Money::from_minor(pi.price_minor, currency),
                min_qty: pi.min_qty.parse().unwrap_or_default(),
                branch: pi.branch_id.as_deref().map(parse_id).transpose()?,
                customer: pi.customer_id.as_deref().map(parse_id).transpose()?,
            });
        }
        let ctx = PriceContext {
            list: list_of(list).unwrap_or(PriceList::Retail),
            qty,
            branch,
            customer,
        };
        Ok(pricing::select(&rules, &ctx).unwrap_or(p.price(currency)))
    })
}

/// Price history of a product, newest first: (list, old, new, when).
pub fn price_history(store: &Store, product: ProductId) -> Result<Vec<PriceChange>> {
    store.read(|r| {
        let mut s = r.conn().prepare(
            "SELECT list, old_minor, new_minor, changed_at FROM price_history WHERE product_id = ?1 ORDER BY seq DESC",
        )?;
        let rows = s.query_map([product.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

// ---------------------------------------------------------------------------
// Scale labels and scanning (M5.5, M5.6)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaleFormatRow {
    pub prefix: String,
    pub item_digits: u32,
    pub value_digits: u32,
    /// `weight` or `price`.
    pub value_kind: String,
    pub decimals: u32,
}

impl ScaleFormatRow {
    fn rule(&self) -> Result<ScaleFormat> {
        let value = match self.value_kind.as_str() {
            "weight" => ScaleValue::Weight {
                decimals: self.decimals,
            },
            "price" => ScaleValue::Price {
                major_decimals: self.decimals,
            },
            _ => return Err(RuleError::on("value_kind", "invalid_value").into()),
        };
        let f = ScaleFormat {
            prefix: self.prefix.trim().to_owned(),
            item_digits: self.item_digits as usize,
            value_digits: self.value_digits as usize,
            value,
        };
        f.validate()?;
        Ok(f)
    }
}

fn formats_of(conn: &Connection, org: OrganizationId) -> Result<Vec<ScaleFormatRow>> {
    let mut s = conn.prepare(
        "SELECT prefix, item_digits, value_digits, value_kind, decimals FROM scale_formats
         WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY position",
    )?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok(ScaleFormatRow {
            prefix: r.get(0)?,
            item_digits: r.get(1)?,
            value_digits: r.get(2)?,
            value_kind: r.get(3)?,
            decimals: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn list_scale_formats(store: &Store) -> Result<Vec<ScaleFormatRow>> {
    store.read(|r| formats_of(r.conn(), the_org(r.conn())?.0))
}

/// Replaces the scale label layouts (tried in order on a scan).
pub fn set_scale_formats(
    store: &mut Store,
    meta: OpMeta,
    formats: Vec<ScaleFormatRow>,
) -> Result<Vec<ScaleFormatRow>> {
    for f in &formats {
        f.rule()?;
    }
    let mut prefixes: Vec<&str> = formats.iter().map(|f| f.prefix.trim()).collect();
    prefixes.sort_unstable();
    if prefixes.windows(2).any(|p| p[0] == p[1]) {
        return Err(RuleError::on("prefix", "scale.prefix_duplicate").into());
    }
    store.write(meta, |w| {
        w.require(Permission::SettingsEdit, &Target::org())?;
        let org = w.org()?;
        w.tx().execute("UPDATE scale_formats SET deleted_at = ?2 WHERE org_id = ?1 AND deleted_at IS NULL", params![org.to_string(), w.now()])?;
        for (i, f) in formats.iter().enumerate() {
            w.tx().execute(
                "INSERT INTO scale_formats (id, org_id, position, prefix, item_digits, value_digits, value_kind, decimals, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![nb_core::OperationId::new().to_string(), org.to_string(), i64::try_from(i).unwrap_or(0), f.prefix.trim(), f.item_digits, f.value_digits, f.value_kind, f.decimals, w.now()],
            )?;
        }
        w.event("scale.formats", "organization", &org.to_string(), &formats)?;
        w.audit("scale_formats", "organization", &org.to_string(), &formats)?;
        Ok(formats.clone())
    })
}

/// What a scan found (PRD §6.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scan {
    /// A product's barcode, in the unit the barcode belongs to.
    Item { product: ProductRow, unit: String },
    /// A scale label with the weight (base units of the product).
    Weighed { product: ProductRow, qty: String },
    /// A scale label with the amount to pay; `qty` is derived from the
    /// price (rounded to the unit's precision).
    Priced {
        product: ProductRow,
        amount_minor: i64,
        qty: String,
    },
    /// Nothing known: the caller may offer to link it to a product.
    Unknown { code: String },
}

/// Reads a scanned or typed code: a known barcode first, then the scale
/// layouts (article code = the product's `plu`).
pub fn scan(store: &Store, code: &str) -> Result<Scan> {
    let code = code.trim().to_owned();
    store.read(|r| {
        let (org, currency) = the_org(r.conn())?;
        if let Some((product, unit)) = catalog::by_barcode(r.conn(), org, &code)? {
            return Ok(Scan::Item { product, unit });
        }
        for f in formats_of(r.conn(), org)? {
            let Ok(rule) = f.rule() else { continue };
            let Some(read) = rule.read(&code, currency) else { continue };
            let item = match &read {
                ScaleRead::Weight { item, .. } | ScaleRead::Price { item, .. } => item.clone(),
            };
            // Article codes are compared as numbers: `00042` is `42`.
            let plu = item.trim_start_matches('0');
            let id: Option<String> = r
                .conn()
                .query_row(
                    "SELECT id FROM products WHERE org_id = ?1 AND deleted_at IS NULL AND ltrim(plu, '0') = ?2",
                    params![org.to_string(), plu],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(id) = id else { continue };
            let product = catalog::get(r.conn(), parse_id(&id)?)?;
            return Ok(match read {
                ScaleRead::Weight { quantity, .. } => Scan::Weighed { product, qty: quantity.to_string() },
                ScaleRead::Price { amount, .. } => {
                    let precision = product.units.first().map_or(3, |u| u.precision);
                    let qty = if product.price_minor > 0 {
                        Decimal::from(amount.minor())
                            .checked_div(Decimal::from(product.price_minor))
                            .map(|d| d.round_dp(precision).normalize())
                            .unwrap_or_default()
                    } else {
                        Decimal::ZERO
                    };
                    Scan::Priced { product, amount_minor: amount.minor(), qty: qty.to_string() }
                }
            });
        }
        Ok(Scan::Unknown { code })
    })
}

/// Links an unknown barcode to a product (in a unit of it).
pub fn link_barcode(
    store: &mut Store,
    meta: OpMeta,
    product: ProductId,
    code: &str,
    unit: &str,
) -> Result<ProductRow> {
    let code = nb_domain::barcode::normalize(code)?;
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        let org = w.org()?;
        let p = catalog::get(w.tx(), product)?;
        if p.unit_set()?.get(unit).is_none() {
            return Err(RuleError::on("unit", "units.unknown").into());
        }
        if !catalog::barcodes_taken(w.tx(), org, std::slice::from_ref(&code), None)?.is_empty() {
            return Err(RuleError::on("barcode", "barcode.taken").into());
        }
        w.tx().execute(
            "INSERT INTO product_barcodes (org_id, code, product_id, unit_code) VALUES (?1, ?2, ?3, ?4)",
            params![org.to_string(), code, product.to_string(), unit],
        )?;
        catalog::reindex(w.tx(), product)?;
        let row = catalog::get(w.tx(), product)?;
        w.event("product.barcode_linked", "product", &row.id, &(code.as_str(), unit))?;
        w.audit("link_barcode", "product", &row.id, &(code.as_str(), unit))?;
        Ok(row)
    })
}

// ---------------------------------------------------------------------------
// Variants (M5.4)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeInput {
    pub name: String,
    pub values: Vec<String>,
}

/// Generates a product's variants: one product per combination of the
/// attributes' values, with its own code (parent code + values), the
/// parent's units, price, tax and category. Existing combinations are kept.
pub fn create_variants(
    store: &mut Store,
    meta: OpMeta,
    parent: ProductId,
    attrs: Vec<AttributeInput>,
) -> Result<Vec<ProductRow>> {
    let attrs: Vec<Attribute> = attrs
        .into_iter()
        .map(|a| Attribute {
            name: a.name,
            values: a.values,
        })
        .collect();
    let combos = rules::combinations(&attrs)?;
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    let p = store.read(|r| catalog::get(r.conn(), parent))?;
    if p.parent_id.is_some() {
        return Err(RuleError::on("product", "variants.of_variant").into());
    }
    let existing: Vec<String> = store.read(|r| {
        let mut s = r.conn().prepare(
            "SELECT variant_attrs FROM products WHERE parent_id = ?1 AND deleted_at IS NULL",
        )?;
        let rows = s.query_map([parent.to_string()], |r| r.get::<_, Option<String>>(0))?;
        Ok(rows.filter_map(|r| r.ok().flatten()).collect())
    })?;
    let defs = serde_json::to_string(
        &attrs
            .iter()
            .map(|a| (&a.name, &a.values))
            .collect::<Vec<_>>(),
    )?;
    let mut todo = Vec::new();
    for c in &combos {
        let json = serde_json::to_string(c)?;
        if existing.contains(&json) {
            continue;
        }
        let input = ProductInput {
            name: rules::variant_name(&p.name, c),
            sku: rules::variant_sku(&p.sku, c, &attrs),
            kind: p.kind.clone(),
            category_id: p.category_id.clone(),
            base_unit: p.units.first().map(|u| u.code.clone()).unwrap_or_default(),
            base_precision: p.units.first().map_or(0, |u| u.precision),
            extra_units: p
                .units
                .iter()
                .skip(1)
                .map(|u| (u.code.clone(), u.factor.clone()))
                .collect(),
            tax_bps: p.tax_bps,
            tax_class_id: p.tax_class_id.clone(),
            brand_id: p.brand_id.clone(),
            price_minor: p.price_minor,
            min_price_minor: p.min_price_minor,
            cost_minor: p.cost_minor,
            min_stock: p.min_stock_milli.map(|m| from_milli(m).to_string()),
            tags: p.tags.clone(),
            ..Default::default()
        };
        let sku = input.sku.clone();
        todo.push((prepare(input, sku, currency)?, json));
    }
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        w.tx().execute(
            "UPDATE products SET variant_attrs = ?2, updated_at = ?3, version = version + 1 WHERE id = ?1",
            params![parent.to_string(), defs, w.now()],
        )?;
        let mut out = Vec::new();
        for (prep, json) in &mut todo {
            let (row, _) = write_new(w, nb_domain::ProductId::new(), prep, currency, offset, Some((parent.to_string(), json.clone())), true)?;
            out.push(row);
        }
        w.audit("variants", "product", &parent.to_string(), &defs)?;
        if out.is_empty() {
            w.event("product.variants", "product", &parent.to_string(), &defs)?;
        }
        Ok(out)
    })
}

/// A parent's variants.
pub fn variants_of(store: &Store, parent: ProductId) -> Result<Vec<ProductRow>> {
    store.read(|r| {
        let mut s = r.conn().prepare(
            "SELECT id FROM products WHERE parent_id = ?1 AND deleted_at IS NULL ORDER BY rowid",
        )?;
        let ids: Vec<String> = s
            .query_map([parent.to_string()], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        ids.iter()
            .map(|id| catalog::get(r.conn(), parse_id(id)?))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Pictures (M5.9)

/// Largest picture accepted (pictures are resized by the app before).
pub const IMAGE_MAX_BYTES: usize = 1_500_000;
/// Most pictures per product.
pub const IMAGES_MAX: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRow {
    pub id: String,
    pub mime: String,
    pub size: i64,
    pub sha256: String,
}

/// The type of a picture from its first bytes (the name may lie).
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

pub fn add_image(
    store: &mut Store,
    meta: OpMeta,
    product: ProductId,
    bytes: Vec<u8>,
) -> Result<ImageRow> {
    let mime = sniff_image(&bytes).ok_or(Error::from(RuleError::on("image", "image.format")))?;
    if bytes.len() > IMAGE_MAX_BYTES {
        return Err(RuleError::on("image", "image.too_large").into());
    }
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        catalog::get(w.tx(), product)?;
        let n: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM product_images WHERE product_id = ?1 AND deleted_at IS NULL",
            [product.to_string()],
            |r| r.get(0),
        )?;
        if n as usize >= IMAGES_MAX {
            return Err(RuleError::on("image", "image.too_many").into());
        }
        let row = ImageRow { id: nb_core::OperationId::new().to_string(), mime: mime.into(), size: i64::try_from(bytes.len()).unwrap_or(i64::MAX), sha256: sha };
        w.tx().execute(
            "INSERT INTO product_images (id, product_id, position, mime, bytes, sha256, created_at, created_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![row.id, product.to_string(), n, row.mime, bytes, row.sha256, w.now(), w.actor_text()],
        )?;
        // The bytes travel with sync separately; the event says what changed.
        w.event("product.image_added", "product", &product.to_string(), &row)?;
        w.audit("image_add", "product", &product.to_string(), &row)?;
        Ok(row)
    })
}

pub fn list_images(store: &Store, product: ProductId) -> Result<Vec<ImageRow>> {
    store.read(|r| {
        let mut s = r.conn().prepare(
            "SELECT id, mime, length(bytes), sha256 FROM product_images WHERE product_id = ?1 AND deleted_at IS NULL ORDER BY position",
        )?;
        let rows = s.query_map([product.to_string()], |r| Ok(ImageRow { id: r.get(0)?, mime: r.get(1)?, size: r.get(2)?, sha256: r.get(3)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

pub fn image_bytes(store: &Store, id: &str) -> Result<Vec<u8>> {
    store.read(|r| {
        r.conn()
            .query_row(
                "SELECT bytes FROM product_images WHERE id = ?1 AND deleted_at IS NULL",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound("image"))
    })
}

pub fn remove_image(store: &mut Store, meta: OpMeta, id: &str) -> Result<()> {
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        let n = w.tx().execute(
            "UPDATE product_images SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, w.now()],
        )?;
        if n == 0 {
            return Err(Error::NotFound("image"));
        }
        w.event("product.image_removed", "image", id, &id)?;
        w.audit("image_remove", "image", id, &id)?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// CSV import (M5.10)

/// A rule broken by one line of an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportError {
    pub line: usize,
    pub field: Option<String>,
    pub code: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub created: u32,
    pub categories_created: u32,
    /// When not empty, nothing was written: fix the file and import again.
    pub errors: Vec<ImportError>,
}

fn money_text(s: &str, currency: Currency) -> std::result::Result<i64, RuleError> {
    let t: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{A0}')
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    Money::parse(&t, currency)
        .map(|m| m.minor())
        .map_err(|_| RuleError::new("price.invalid"))
}

/// Imports products from CSV text: all lines or none. Categories are
/// matched by name (created when new); stock goes to `warehouse` at the
/// line's cost, in one opening entry.
pub fn import_products(
    store: &mut Store,
    meta: OpMeta,
    text: &str,
    warehouse: nb_domain::WarehouseId,
) -> Result<ImportReport> {
    let rows = rules::parse_csv(text)?;
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    let first_sku = next_sku(store)?;
    let mut next: i64 = first_sku.trim_start_matches("P-").parse().unwrap_or(1);
    let mut report = ImportReport::default();
    let mut prepared = Vec::new();
    let mut seen_skus = std::collections::BTreeSet::new();
    let mut seen_codes = std::collections::BTreeSet::new();
    for row in &rows {
        let mut err = |field: Option<&str>, code: &str| {
            report.errors.push(ImportError {
                line: row.line,
                field: field.map(Into::into),
                code: code.into(),
            });
        };
        let price = match money_text(row.get("price"), currency) {
            Ok(p) => p,
            Err(_) => {
                err(Some("price"), "price.invalid");
                continue;
            }
        };
        let cost = match row.get("cost") {
            "" => None,
            c => match money_text(c, currency) {
                Ok(v) => Some(v),
                Err(_) => {
                    err(Some("cost"), "price.invalid");
                    continue;
                }
            },
        };
        let unit = match row.get("unit") {
            "" => "pc".to_owned(),
            u => u.to_lowercase(),
        };
        let precision = nb_domain::units::standard_unit(&unit).map_or(0, |u| u.precision);
        let tax_bps = match row
            .get("tax")
            .trim_end_matches('%')
            .trim()
            .replace(',', ".")
        {
            t if t.is_empty() => 1_900,
            t => match t.parse::<Decimal>() {
                Ok(d) => (d * Decimal::from(100)).round().try_into().unwrap_or(-1),
                Err(_) => -1,
            },
        };
        let sku = match row.get("sku") {
            "" => {
                let s = format!("P-{next:05}");
                next += 1;
                s
            }
            s => s.to_owned(),
        };
        if !seen_skus.insert(sku.trim().to_uppercase()) {
            err(Some("sku"), "product.sku_taken");
            continue;
        }
        let barcodes: Vec<String> = row
            .get("barcode")
            .split([' ', '|'])
            .filter(|b| !b.is_empty())
            .map(Into::into)
            .collect();
        if barcodes.iter().any(|b| !seen_codes.insert(b.clone())) {
            err(Some("barcode"), "barcode.taken");
            continue;
        }
        let stock = row.get("stock").replace(',', ".");
        let input = ProductInput {
            name: row.get("name").into(),
            sku: sku.clone(),
            kind: "goods".into(),
            base_unit: unit,
            base_precision: precision,
            barcodes,
            tax_bps,
            price_minor: price,
            cost_minor: cost,
            opening: (!stock.is_empty()).then(|| Opening {
                warehouse_id: warehouse.to_string(),
                qty: stock,
                unit_cost_minor: cost.unwrap_or(0),
            }),
            ..Default::default()
        };
        match prepare(input, sku, currency) {
            Ok(p) => prepared.push((row.line, row.get("category").to_owned(), p)),
            Err(e) => {
                for (f, c) in e.rule_codes() {
                    err(f, c);
                }
            }
        }
    }
    // Codes already in the catalog, line by line (the operation checks again).
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        for (line, _, p) in &prepared {
            if catalog::sku_taken(r.conn(), org, &p.valid.0.sku, None)? {
                report.errors.push(ImportError {
                    line: *line,
                    field: Some("sku".into()),
                    code: "product.sku_taken".into(),
                });
            }
            if !catalog::barcodes_taken(r.conn(), org, &p.valid.0.barcodes, None)?.is_empty() {
                report.errors.push(ImportError {
                    line: *line,
                    field: Some("barcode".into()),
                    code: "barcode.taken".into(),
                });
            }
        }
        Ok(())
    })?;
    if !report.errors.is_empty() || prepared.is_empty() {
        return Ok(report);
    }
    store.write(meta, |w| {
        w.require(Permission::ProductsEdit, &Target::org())?;
        let org = w.org()?;
        let wh = org::warehouse(w.tx(), warehouse)?;
        let branch = org::branch(w.tx(), parse_id(&wh.branch_id)?)?;
        let (_, mut names) = tree(w.tx(), org)?;
        let mut by_name: BTreeMap<String, String> = names.iter().map(|(id, n)| (n.to_lowercase(), id.clone())).collect();
        let mut out = ImportReport::default();
        let mut value = Money::zero(currency);
        for (line, category, p) in &mut prepared {
            if !category.is_empty() {
                let key = clean(category).to_lowercase();
                let id = match by_name.get(&key) {
                    Some(id) => id.clone(),
                    None => {
                        let id = CategoryId::new().to_string();
                        w.tx().execute(
                            "INSERT INTO categories (id, org_id, name, created_at, updated_at, created_by, device_id) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6)",
                            params![id, org.to_string(), clean(category), w.now(), w.actor_text(), w.device().id.to_string()],
                        )?;
                        by_name.insert(key, id.clone());
                        names.insert(id.clone(), clean(category));
                        out.categories_created += 1;
                        id
                    }
                };
                p.input.category_id = Some(id);
            }
            let _ = line;
            let (_, v) = write_new(w, nb_domain::ProductId::new(), p, currency, offset, None, false)?;
            value = value.checked_add(v)?;
            out.created += 1;
        }
        if !value.is_zero() {
            post_opening(w, &branch, offset, value, "import")?;
        }
        w.audit("import", "organization", &org.to_string(), &(out.created, out.categories_created))?;
        Ok(out)
    })
}

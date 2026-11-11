//! Suppliers (M9.10) and buying from them (M10): the purchase invoice that
//! brings goods in at their landed cost (the charges of bringing them in
//! spread on them, the average cost updated), returns to the supplier,
//! payments to it settling its invoices, and what is still owed.
//!
//! What a supplier is owed is the sum of the payable lines tagged with it
//! (`journal_lines.party_id`); its balance is their projection, kept in the
//! same transaction ([`supplier_drift`]).

use nb_core::{Money, Quantity, Rate};
use nb_domain::access::{Permission, Target};
use nb_domain::numbering::DocKind;
use nb_domain::posting::{Operation, Settlement};
use nb_domain::sale::{Cart, CartLine, Discount};
use nb_domain::stock::MovementKind;
use nb_domain::tax::TaxMode;
use nb_domain::{ProductId, RegisterId, RuleError, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::repo::{catalog, org, parse_id, stock, to_milli};
use crate::uow::{OpMeta, Writer};
use crate::usecases::accounting::post_operations_for;
use crate::usecases::sales::{shift_of, the_org, till};
use crate::{Error, Result, Store};

// ---------------------------------------------------------------------------
// Suppliers (M9.10)

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SupplierInput {
    #[serde(default)]
    pub id: Option<String>,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub nif: String,
    #[serde(default)]
    pub nis: String,
    #[serde(default)]
    pub rc: String,
    #[serde(default)]
    pub ai: String,
    #[serde(default)]
    pub payment_days: i64,
    #[serde(default = "active")]
    pub status: String,
    #[serde(default)]
    pub note: String,
}

fn active() -> String {
    "active".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierRow {
    pub id: String,
    pub code: String,
    pub kind: String,
    pub name: String,
    pub phone: String,
    pub email: String,
    pub address: String,
    pub city: String,
    pub nif: String,
    pub nis: String,
    pub rc: String,
    pub ai: String,
    pub payment_days: i64,
    pub status: String,
    pub note: String,
    /// What is owed to it (negative: an advance paid).
    pub balance_minor: i64,
    pub version: i64,
}

const SUPPLIER_SELECT: &str =
    "SELECT id, code, kind, name, phone, email, address, city, nif, nis, rc, ai,
        payment_days, status, note, balance_minor, version FROM suppliers";

fn supplier_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<SupplierRow> {
    Ok(SupplierRow {
        id: r.get(0)?,
        code: r.get(1)?,
        kind: r.get(2)?,
        name: r.get(3)?,
        phone: r.get(4)?,
        email: r.get(5)?,
        address: r.get(6)?,
        city: r.get(7)?,
        nif: r.get(8)?,
        nis: r.get(9)?,
        rc: r.get(10)?,
        ai: r.get(11)?,
        payment_days: r.get(12)?,
        status: r.get(13)?,
        note: r.get(14)?,
        balance_minor: r.get(15)?,
        version: r.get(16)?,
    })
}

pub(crate) fn load_supplier(conn: &Connection, id: &str) -> Result<SupplierRow> {
    conn.query_row(
        &format!("{SUPPLIER_SELECT} WHERE id = ?1"),
        [id],
        supplier_of,
    )
    .optional()?
    .ok_or(Error::Rules(vec![RuleError::on(
        "supplier",
        "supplier.unknown",
    )]))
}

pub fn get_supplier(store: &Store, id: &str) -> Result<SupplierRow> {
    store.read(|r| load_supplier(r.conn(), id))
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SupplierPage {
    pub rows: Vec<SupplierRow>,
    pub total: u32,
    /// What is owed to the suppliers the filter keeps.
    pub owed_minor: i64,
}

/// Suppliers, those owed something first; `owed` keeps only them.
pub fn list_suppliers(
    store: &Store,
    search: &str,
    owed: bool,
    limit: u32,
    offset: u32,
) -> Result<SupplierPage> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let search = search.trim().to_lowercase();
        let filter = " WHERE org_id = ?1 AND (?2 = '' OR lower(name) LIKE '%' || ?2 || '%'
               OR lower(code) LIKE '%' || ?2 || '%' OR phone LIKE '%' || ?2 || '%'
               OR lower(city) LIKE '%' || ?2 || '%') AND (?3 = 0 OR balance_minor > 0)";
        let (total, sum): (u32, i64) = r.conn().query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(MAX(balance_minor, 0)), 0) FROM suppliers{filter}"
            ),
            params![org.to_string(), search, owed],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut s = r.conn().prepare(&format!(
            "{SUPPLIER_SELECT}{filter} ORDER BY balance_minor > 0 DESC, name COLLATE NOCASE LIMIT ?4 OFFSET ?5"
        ))?;
        let rows = s
            .query_map(
                params![org.to_string(), search, owed, limit.clamp(1, 500), offset],
                supplier_of,
            )?
            .collect::<std::result::Result<_, _>>()?;
        Ok(SupplierPage {
            rows,
            total,
            owed_minor: sum,
        })
    })
}

/// Creates a supplier (numbered F-00001…) or changes one.
pub fn save_supplier(store: &mut Store, meta: OpMeta, input: SupplierInput) -> Result<SupplierRow> {
    let mut errors = Vec::new();
    if input.name.trim().is_empty() {
        errors.push(RuleError::on("name", "required"));
    }
    if !matches!(input.kind.as_str(), "person" | "company") {
        errors.push(RuleError::on("kind", "invalid_value"));
    }
    if !matches!(input.status.as_str(), "active" | "blocked") {
        errors.push(RuleError::on("status", "invalid_value"));
    }
    if !(0..=365).contains(&input.payment_days) {
        errors.push(RuleError::on("payment_days", "invalid_value"));
    }
    let email = input.email.trim();
    if !email.is_empty() && !(email.contains('@') && email.contains('.')) {
        errors.push(RuleError::on("email", "invalid_email"));
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }
    let id = store.write(meta, |w| {
        w.require(Permission::SuppliersEdit, &Target::org())?;
        let org = w.org()?.to_string();
        let t = |s: &str| s.trim().to_owned();
        let id = match input.id.as_deref().filter(|i| !i.is_empty()) {
            Some(id) => {
                load_supplier(w.tx(), id)?;
                w.tx().execute(
                    "UPDATE suppliers SET kind = ?2, name = ?3, phone = ?4, email = ?5, address = ?6, city = ?7,
                            nif = ?8, nis = ?9, rc = ?10, ai = ?11, payment_days = ?12, status = ?13, note = ?14,
                            updated_at = ?15, version = version + 1
                     WHERE id = ?1",
                    params![
                        id,
                        input.kind,
                        t(&input.name),
                        t(&input.phone),
                        t(&input.email),
                        t(&input.address),
                        t(&input.city),
                        t(&input.nif),
                        t(&input.nis),
                        t(&input.rc),
                        t(&input.ai),
                        input.payment_days,
                        input.status,
                        t(&input.note),
                        w.now()
                    ],
                )?;
                w.audit("update", "supplier", id, &input)?;
                id.to_owned()
            }
            None => {
                w.tx().execute(
                    "INSERT INTO supplier_counters (org_id, last) VALUES (?1, 1)
                     ON CONFLICT (org_id) DO UPDATE SET last = last + 1",
                    [&org],
                )?;
                let n: i64 = w.tx().query_row(
                    "SELECT last FROM supplier_counters WHERE org_id = ?1",
                    [&org],
                    |r| r.get(0),
                )?;
                let id = nb_core::uuid::Uuid::now_v7().to_string();
                w.tx().execute(
                    "INSERT INTO suppliers (id, org_id, code, kind, name, phone, email, address, city, nif, nis, rc, ai,
                            payment_days, status, note, created_at, created_by, updated_at, device_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?17, ?19)",
                    params![
                        id,
                        org,
                        format!("F-{n:05}"),
                        input.kind,
                        t(&input.name),
                        t(&input.phone),
                        t(&input.email),
                        t(&input.address),
                        t(&input.city),
                        t(&input.nif),
                        t(&input.nis),
                        t(&input.rc),
                        t(&input.ai),
                        input.payment_days,
                        input.status,
                        t(&input.note),
                        w.now(),
                        w.actor_text(),
                        w.device().id.to_string()
                    ],
                )?;
                w.audit("create", "supplier", &id, &input)?;
                id
            }
        };
        let row = load_supplier(w.tx(), &id)?;
        w.event("supplier.saved", "supplier", &id, &row)?;
        Ok(id)
    })?;
    get_supplier(store, &id)
}

/// Suppliers whose balance differs from their payable lines.
pub fn supplier_drift(conn: &Connection) -> Result<Vec<(String, i64, i64)>> {
    let mut s = conn.prepare(
        "SELECT s.id, s.balance_minor,
                COALESCE((SELECT SUM(l.credit_minor - l.debit_minor) FROM journal_lines l WHERE l.party_id = s.id), 0)
         FROM suppliers s",
    )?;
    let rows = s
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<Vec<(String, i64, i64)>, _>>()?;
    Ok(rows.into_iter().filter(|(_, a, b)| a != b).collect())
}

// ---------------------------------------------------------------------------
// Purchase invoices (M10.2–M10.5)

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PurchaseLineInput {
    pub product_id: String,
    /// In `unit` (decimal text); empty unit: the base unit.
    pub qty: String,
    #[serde(default)]
    pub unit: String,
    /// Before tax, per unit of the line.
    pub unit_cost_minor: i64,
    /// Empty: the product's rate.
    #[serde(default)]
    pub tax_bps: Option<i64>,
    /// The line of the order (`PurchaseInput::order_id`) it receives.
    #[serde(default)]
    pub order_position: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PurchaseInput {
    pub supplier_id: String,
    pub warehouse_id: String,
    #[serde(default)]
    pub supplier_ref: String,
    pub lines: Vec<PurchaseLineInput>,
    /// Costs of bringing the goods in (before tax), spread on them.
    #[serde(default)]
    pub charges_minor: i64,
    #[serde(default)]
    pub charges_vat_minor: i64,
    /// `value` (default) or `qty`.
    #[serde(default)]
    pub charges_by: String,
    /// Paid now: in cash (from the open till of `register_id`) or by bank;
    /// the rest is owed to the supplier.
    #[serde(default)]
    pub cash_minor: i64,
    #[serde(default)]
    pub bank_minor: i64,
    #[serde(default)]
    pub register_id: Option<String>,
    #[serde(default)]
    pub note: String,
    /// The approved order these goods are received against (M10.3).
    #[serde(default)]
    pub order_id: Option<String>,
    /// A supervisor's approval of a cost above the one ordered.
    #[serde(default)]
    pub approval_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseResult {
    pub id: String,
    pub number: String,
    pub goods_minor: i64,
    pub charges_minor: i64,
    pub net_minor: i64,
    pub vat_minor: i64,
    pub total_minor: i64,
    pub credit_minor: i64,
    /// Per line: the landed cost of one base unit.
    pub landed: Vec<i64>,
}

/// A line to buy, resolved: the product, its unit, the quantity in it and
/// in the base unit, the VAT rate.
pub(crate) type BuyLine = (catalog::ProductRow, String, Quantity, Quantity, i64);

/// Resolves lines to buy and prices them before tax.
pub(crate) fn parse_buy_lines(
    conn: &Connection,
    lines: &[PurchaseLineInput],
    currency: nb_core::Currency,
) -> Result<(Vec<BuyLine>, Vec<CartLine>)> {
    let mut products = Vec::with_capacity(lines.len());
    let mut cart_lines = Vec::with_capacity(lines.len());
    for l in lines {
        let qty: Quantity = l
            .qty
            .trim()
            .replace(',', ".")
            .parse()
            .map_err(|_| RuleError::on("qty", "invalid_quantity"))?;
        if to_milli(qty)? <= 0 {
            return Err(RuleError::on("qty", "invalid_quantity").into());
        }
        if l.unit_cost_minor < 0 {
            return Err(RuleError::on("unit_cost", "ledger.negative").into());
        }
        let p = catalog::get(conn, parse_id::<ProductId>(&l.product_id)?)?;
        let units = p.unit_set()?;
        let unit = if l.unit.is_empty() {
            p.units[0].code.clone()
        } else {
            l.unit.clone()
        };
        let base_qty = units.to_base(qty, &unit)?;
        let tax_bps = l.tax_bps.unwrap_or(p.tax_bps);
        if !(0..=10_000).contains(&tax_bps) {
            return Err(RuleError::on("tax_rate", "invalid_value").into());
        }
        cart_lines.push(CartLine {
            qty,
            unit_price: Money::from_minor(l.unit_cost_minor, currency),
            min_price: None,
            tax_rate: Rate::from_bps(tax_bps),
            discount: Discount::None,
        });
        products.push((p, unit, qty, base_qty, tax_bps));
    }
    Ok((products, cart_lines))
}

/// The open till of a register, if one is asked (cash needs it).
fn open_shift_of(w: &Writer<'_>, register: Option<&str>) -> Result<Option<(String, String)>> {
    let Some(reg) = register.filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    let (_, _, account) = till(w, parse_id::<RegisterId>(reg)?)?;
    let shift: Option<String> = w
        .tx()
        .query_row(
            "SELECT id FROM shifts WHERE register_id = ?1 AND status = 'open'",
            [reg],
            |r| r.get(0),
        )
        .optional()?;
    Ok(shift.map(|s| (s, account)))
}

/// Receives goods on a supplier's invoice: the stock comes in at its landed
/// cost (the average cost follows), the entry posts the goods, the VAT and
/// what is paid or owed — at once and once per operation.
pub fn record_purchase(
    store: &mut Store,
    meta: OpMeta,
    input: PurchaseInput,
) -> Result<PurchaseResult> {
    if input.lines.is_empty() {
        return Err(RuleError::on("lines", "sale.empty").into());
    }
    if input.charges_minor < 0
        || input.charges_vat_minor < 0
        || input.cash_minor < 0
        || input.bank_minor < 0
    {
        return Err(RuleError::on("amount", "ledger.negative").into());
    }
    let by_qty = match input.charges_by.as_str() {
        "" | "value" => false,
        "qty" => true,
        _ => return Err(RuleError::on("charges_by", "invalid_value").into()),
    };
    let warehouse_id: WarehouseId = parse_id(&input.warehouse_id)?;
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.utc_offset_min;
    store.write(meta, |w| {
        let supplier = load_supplier(w.tx(), &input.supplier_id)?;
        if supplier.status == "blocked" {
            return Err(RuleError::on("supplier", "supplier.blocked").into());
        }
        let warehouse = org::warehouse(w.tx(), warehouse_id)?;
        let branch = org::branch(w.tx(), parse_id(&warehouse.branch_id)?)?;
        let at = Target::warehouse(parse_id(&branch.id)?, warehouse_id);
        w.require(Permission::PurchasesReceive, &at)?;

        let (products, cart_lines) = parse_buy_lines(w.tx(), &input.lines, currency)?;
        let totals = Cart {
            mode: TaxMode::Exclusive,
            lines: cart_lines.clone(),
            discount: Discount::None,
        }
        .totals()?;
        // The charges spread on the lines, by value or by quantity.
        let weights: Vec<u64> = if by_qty {
            products
                .iter()
                .map(|(_, _, _, b, _)| u64::try_from(to_milli(*b).unwrap_or(0)).unwrap_or(0))
                .collect()
        } else {
            totals
                .lines
                .iter()
                .map(|t| u64::try_from(t.taxed.net.minor()).unwrap_or(0))
                .collect()
        };
        let charges = Money::from_minor(input.charges_minor, currency);
        let shares = if input.charges_minor > 0 && weights.iter().any(|w| *w > 0) {
            charges.allocate(&weights)?
        } else {
            vec![Money::zero(currency); products.len()]
        };
        let goods = totals.net.minor();
        let net = goods + input.charges_minor;
        let vat = totals.tax.minor() + input.charges_vat_minor;
        let total = net + vat;
        let credit = total - input.cash_minor - input.bank_minor;
        if credit < 0 {
            return Err(RuleError::on("payments", "sale.overpaid").into());
        }
        // Cash comes out of an open till, never more than it holds.
        let till_of = open_shift_of(w, input.register_id.as_deref())?;
        if input.cash_minor > 0 {
            let Some((shift, _)) = &till_of else {
                return Err(RuleError::on("register", "shift.none_open").into());
            };
            if input.cash_minor > shift_of(w.tx(), shift)?.expected_minor {
                return Err(RuleError::on("payments", "shift.not_enough_cash").into());
            }
            w.require(Permission::SuppliersPay, &at)?;
        }
        if input.bank_minor > 0 {
            w.require(Permission::SuppliersPay, &at)?;
        }

        // Against an order: matched line by line, its lines then received.
        let ordered_costs = match input.order_id.as_deref().filter(|o| !o.is_empty()) {
            Some(order) => crate::usecases::purchase_orders::receive(
                w,
                order,
                &supplier.id,
                &warehouse.id,
                &input.lines,
                &products,
                input.approval_id.as_deref(),
            )?
            .into_iter()
            .map(Some)
            .collect(),
            None => {
                if input.lines.iter().any(|l| l.order_position.is_some()) {
                    return Err(RuleError::on("order", "purchase.order_unknown").into());
                }
                vec![None; products.len()]
            }
        };
        let number = w.next_number(DocKind::Purchase, &branch.id, &branch.code)?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        let mut landed = Vec::with_capacity(products.len());
        let mut moves = Vec::with_capacity(products.len());
        for (i, (p, _, _, base_qty, _)) in products.iter().enumerate() {
            let line_value = totals.lines[i].taxed.net.checked_add(shares[i])?;
            // One base unit's landed cost, to the centime.
            let per = nb_core::Decimal::from(line_value.minor())
                .checked_div(base_qty.decimal())
                .ok_or(RuleError::new("overflow"))?
                .round_dp_with_strategy(0, nb_core::RoundingStrategy::MidpointAwayFromZero);
            let unit_cost: i64 = per.try_into().map_err(|_| RuleError::new("overflow"))?;
            let product_id: ProductId = parse_id(&p.id)?;
            let movement = if catalog::kind(p) == nb_domain::product::ProductKind::Service {
                None
            } else {
                let cost = Money::from_minor(unit_cost, currency);
                let before = stock::balance(w.tx(), product_id, warehouse_id, currency)?;
                let after = before.apply(MovementKind::Purchase, *base_qty, Some(cost), warehouse.policy(), true)?;
                Some(
                    stock::append(
                        w,
                        product_id,
                        warehouse_id,
                        MovementKind::Purchase,
                        *base_qty,
                        Some(cost),
                        Some(&number),
                        None,
                        after,
                    )?
                    .to_string(),
                )
            };
            landed.push(unit_cost);
            moves.push(movement);
        }
        let shift_id = till_of.as_ref().filter(|_| input.cash_minor > 0).map(|t| t.0.clone());
        w.tx().execute(
            "INSERT INTO purchases (id, org_id, branch_id, warehouse_id, supplier_id, number, supplier_ref, goods_minor,
                    charges_minor, charges_by, net_minor, vat_minor, total_minor, cash_minor, bank_minor, credit_minor,
                    register_id, shift_id, note, operation_id, created_at, created_by, device_id, order_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                id,
                w.org()?.to_string(),
                branch.id,
                warehouse.id,
                supplier.id,
                number,
                input.supplier_ref.trim(),
                goods,
                input.charges_minor,
                if by_qty { "qty" } else { "value" },
                net,
                vat,
                total,
                input.cash_minor,
                input.bank_minor,
                credit,
                shift_id.as_ref().and(input.register_id.clone()),
                shift_id,
                input.note.trim(),
                w.meta().id.to_string(),
                w.now(),
                w.actor_text(),
                w.device().id.to_string(),
                input.order_id.as_deref().filter(|o| !o.is_empty())
            ],
        )?;
        for (i, (p, unit, qty, base_qty, tax_bps)) in products.iter().enumerate() {
            w.tx().execute(
                "INSERT INTO purchase_lines (purchase_id, position, product_id, name, unit, qty_milli, base_qty_milli,
                        unit_cost_minor, tax_bps, net_minor, tax_minor, charges_minor, landed_cost_minor, movement_id,
                        order_position, ordered_cost_minor)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    id,
                    i64::try_from(i).unwrap_or(0),
                    p.id,
                    p.name,
                    unit,
                    to_milli(*qty)?,
                    to_milli(*base_qty)?,
                    cart_lines[i].unit_price.minor(),
                    tax_bps,
                    totals.lines[i].taxed.net.minor(),
                    totals.lines[i].taxed.tax.minor(),
                    shares[i].minor(),
                    landed[i],
                    moves[i],
                    input.lines[i].order_position,
                    ordered_costs[i]
                ],
            )?;
        }
        let m = |v: i64| Money::from_minor(v, currency);
        post_operations_for(
            w,
            &branch,
            offset,
            &[Operation::Purchase {
                net: m(net),
                vat: m(vat),
                paid: Settlement {
                    cash: m(input.cash_minor),
                    bank: m(input.bank_minor),
                    credit: m(credit),
                },
            }],
            &format!("{number} — {}", supplier.name),
            "purchase",
            till_of.as_ref().map(|t| t.1.as_str()),
            Some(&supplier.id),
        )?;
        let out = PurchaseResult {
            id: id.clone(),
            number,
            goods_minor: goods,
            charges_minor: input.charges_minor,
            net_minor: net,
            vat_minor: vat,
            total_minor: total,
            credit_minor: credit,
            landed,
        };
        w.event("purchase.recorded", "purchase", &id, &out)?;
        w.audit("record", "purchase", &id, &input)?;
        Ok(out)
    })
}

/// A purchase invoice in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseRow {
    pub id: String,
    pub number: String,
    pub supplier_id: String,
    pub supplier: String,
    pub supplier_ref: String,
    pub created_at: i64,
    pub lines: u32,
    pub total_minor: i64,
    /// Still owed on it.
    pub open_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PurchasePage {
    pub rows: Vec<PurchaseRow>,
    pub total: u32,
    pub total_minor: i64,
}

const OPEN_SQL: &str = "p.credit_minor
    - COALESCE((SELECT SUM(a.amount_minor) FROM supplier_allocations a WHERE a.purchase_id = p.id), 0)
    - COALESCE((SELECT SUM(x.total_minor) FROM purchase_returns x WHERE x.purchase_id = p.id AND x.refund_method = 'credit'), 0)";

/// Purchases, the latest first; `supplier` keeps one supplier's.
pub fn list_purchases(
    store: &Store,
    search: &str,
    supplier: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<PurchasePage> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let search = search.trim().to_lowercase();
        let filter = " FROM purchases p JOIN suppliers s ON s.id = p.supplier_id
             WHERE p.org_id = ?1 AND (?2 = '' OR lower(p.number) LIKE '%' || ?2 || '%'
                   OR lower(s.name) LIKE '%' || ?2 || '%' OR lower(p.supplier_ref) LIKE '%' || ?2 || '%')
               AND (?3 IS NULL OR p.supplier_id = ?3)";
        let (total, sum): (u32, i64) = r.conn().query_row(
            &format!("SELECT COUNT(*), COALESCE(SUM(p.total_minor), 0){filter}"),
            params![org.to_string(), search, supplier],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut s = r.conn().prepare(&format!(
            "SELECT p.id, p.number, p.supplier_id, s.name, p.supplier_ref, p.created_at,
                    (SELECT COUNT(*) FROM purchase_lines l WHERE l.purchase_id = p.id), p.total_minor, {OPEN_SQL}
             {filter} ORDER BY p.created_at DESC, p.number DESC LIMIT ?4 OFFSET ?5"
        ))?;
        let rows = s
            .query_map(
                params![org.to_string(), search, supplier, limit.clamp(1, 500), offset],
                |r| {
                    Ok(PurchaseRow {
                        id: r.get(0)?,
                        number: r.get(1)?,
                        supplier_id: r.get(2)?,
                        supplier: r.get(3)?,
                        supplier_ref: r.get(4)?,
                        created_at: r.get(5)?,
                        lines: r.get(6)?,
                        total_minor: r.get(7)?,
                        open_minor: r.get(8)?,
                    })
                },
            )?
            .collect::<std::result::Result<_, _>>()?;
        Ok(PurchasePage {
            rows,
            total,
            total_minor: sum,
        })
    })
}

/// A supplier's invoices still owed, the oldest first.
pub fn open_purchases(store: &Store, supplier: &str) -> Result<Vec<PurchaseRow>> {
    store.read(|r| open_purchases_of(r.conn(), supplier))
}

fn open_purchases_of(conn: &Connection, supplier: &str) -> Result<Vec<PurchaseRow>> {
    let mut s = conn.prepare(&format!(
        "SELECT p.id, p.number, p.supplier_id, s.name, p.supplier_ref, p.created_at,
                (SELECT COUNT(*) FROM purchase_lines l WHERE l.purchase_id = p.id), p.total_minor, {OPEN_SQL}
         FROM purchases p JOIN suppliers s ON s.id = p.supplier_id
         WHERE p.supplier_id = ?1 AND p.credit_minor > 0 ORDER BY p.created_at, p.number"
    ))?;
    let rows = s
        .query_map([supplier], |r| {
            Ok(PurchaseRow {
                id: r.get(0)?,
                number: r.get(1)?,
                supplier_id: r.get(2)?,
                supplier: r.get(3)?,
                supplier_ref: r.get(4)?,
                created_at: r.get(5)?,
                lines: r.get(6)?,
                total_minor: r.get(7)?,
                open_minor: r.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().filter(|p| p.open_minor > 0).collect())
}

/// A line of a purchase, with what can still be sent back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoughtLine {
    pub position: u32,
    pub product_id: String,
    pub name: String,
    pub unit: String,
    pub qty_milli: i64,
    pub returned_milli: i64,
    pub unit_cost_minor: i64,
    pub tax_bps: i64,
}

pub fn purchase_lines(store: &Store, purchase: &str) -> Result<Vec<BoughtLine>> {
    store.read(|r| {
        let mut s = r.conn().prepare(
            "SELECT l.position, l.product_id, l.name, l.unit, l.qty_milli,
                    COALESCE((SELECT SUM(x.qty_milli) FROM purchase_return_lines x
                              WHERE x.purchase_id = l.purchase_id AND x.position = l.position), 0),
                    l.unit_cost_minor, l.tax_bps
             FROM purchase_lines l WHERE l.purchase_id = ?1 ORDER BY l.position",
        )?;
        let rows = s
            .query_map([purchase], |r| {
                Ok(BoughtLine {
                    position: r.get(0)?,
                    product_id: r.get(1)?,
                    name: r.get(2)?,
                    unit: r.get(3)?,
                    qty_milli: r.get(4)?,
                    returned_milli: r.get(5)?,
                    unit_cost_minor: r.get(6)?,
                    tax_bps: r.get(7)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    })
}

// ---------------------------------------------------------------------------
// Returns to the supplier (M10.6)

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PurchaseReturnInput {
    pub purchase_id: String,
    /// (line position, quantity in the line's unit).
    pub lines: Vec<(u32, String)>,
    /// `credit` (off what is owed), `cash` (into the open till) or `bank`.
    pub refund: String,
    #[serde(default)]
    pub register_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseReturnResult {
    pub id: String,
    pub number: String,
    pub net_minor: i64,
    pub tax_minor: i64,
    pub total_minor: i64,
    pub stock_value_minor: i64,
}

/// Sends goods back to the supplier: never more than bought; the stock
/// leaves at its average cost; the supplier refunds the price paid.
pub fn return_purchase(
    store: &mut Store,
    meta: OpMeta,
    input: PurchaseReturnInput,
) -> Result<PurchaseReturnResult> {
    if input.reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    if input.lines.is_empty() {
        return Err(RuleError::on("lines", "return.empty").into());
    }
    let method = match input.refund.as_str() {
        m @ ("cash" | "bank" | "credit") => m.to_owned(),
        _ => return Err(RuleError::on("refund", "invalid_value").into()),
    };
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.utc_offset_min;
    let bought = purchase_lines(store, &input.purchase_id)?;
    store.write(meta, |w| {
        let (supplier_id, warehouse_id, branch_id): (String, String, String) = w
            .tx()
            .query_row(
                "SELECT supplier_id, warehouse_id, branch_id FROM purchases WHERE id = ?1",
                [&input.purchase_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or(Error::Rules(vec![RuleError::on("purchase", "purchase.unknown")]))?;
        let supplier = load_supplier(w.tx(), &supplier_id)?;
        let wh: WarehouseId = parse_id(&warehouse_id)?;
        let warehouse = org::warehouse(w.tx(), wh)?;
        let branch = org::branch(w.tx(), parse_id(&branch_id)?)?;
        let at = Target::warehouse(parse_id(&branch.id)?, wh);
        w.require(Permission::PurchasesReceive, &at)?;
        let till_of = open_shift_of(w, input.register_id.as_deref())?;
        if method == "cash" && till_of.is_none() {
            return Err(RuleError::on("register", "shift.none_open").into());
        }
        let number = w.next_number(DocKind::PurchaseReturn, &branch.id, &branch.code)?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        let may_go_negative = w.allowed(Permission::StockNegative, &at)?;
        let (mut net, mut tax, mut value) = (0i64, 0i64, 0i64);
        let mut rows = Vec::new();
        for (position, qty_text) in &input.lines {
            let b = bought
                .iter()
                .find(|b| b.position == *position)
                .ok_or(Error::Rules(vec![RuleError::on("lines", "return.line_unknown")]))?;
            let qty: Quantity = qty_text
                .trim()
                .replace(',', ".")
                .parse()
                .map_err(|_| RuleError::on("qty", "invalid_quantity"))?;
            let q = to_milli(qty)?;
            if q <= 0 {
                return Err(RuleError::on("qty", "sale.qty_positive").into());
            }
            if q > b.qty_milli - b.returned_milli {
                return Err(RuleError::on("qty", "return.too_much").into());
            }
            let line = Cart {
                mode: TaxMode::Exclusive,
                lines: vec![CartLine {
                    qty,
                    unit_price: Money::from_minor(b.unit_cost_minor, currency),
                    min_price: None,
                    tax_rate: Rate::from_bps(b.tax_bps),
                    discount: Discount::None,
                }],
                discount: Discount::None,
            }
            .totals()?;
            // The stock leaves at its average cost, in base units.
            let product_id: ProductId = parse_id(&b.product_id)?;
            let p = catalog::get(w.tx(), product_id)?;
            let base_qty = p.unit_set()?.to_base(qty, &b.unit)?;
            let before = stock::balance(w.tx(), product_id, wh, currency)?;
            let after = before.apply(
                MovementKind::PurchaseReturn,
                base_qty,
                None,
                warehouse.policy(),
                may_go_negative,
            )?;
            let movement = stock::append(
                w,
                product_id,
                wh,
                MovementKind::PurchaseReturn,
                base_qty,
                Some(before.avg_cost),
                Some(&number),
                Some(input.reason.trim()),
                after,
            )?;
            let line_value = base_qty.times_price(before.avg_cost)?.minor();
            net += line.net.minor();
            tax += line.tax.minor();
            value += line_value;
            rows.push((*position, q, line.net.minor(), line.tax.minor(), line_value, movement.to_string()));
        }
        let total = net + tax;
        if method == "credit" && total > supplier.balance_minor {
            return Err(RuleError::on("refund", "purchase.return_more_than_owed").into());
        }
        let shift = till_of.as_ref().filter(|_| method == "cash").map(|t| t.0.clone());
        w.tx().execute(
            "INSERT INTO purchase_returns (id, org_id, purchase_id, number, refund_method, reason, net_minor, tax_minor,
                    total_minor, stock_value_minor, shift_id, operation_id, created_at, created_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                w.org()?.to_string(),
                input.purchase_id,
                number,
                method,
                input.reason.trim(),
                net,
                tax,
                total,
                value,
                shift,
                w.meta().id.to_string(),
                w.now(),
                w.actor_text(),
                w.device().id.to_string()
            ],
        )?;
        for (position, q, n, t, v, movement) in &rows {
            w.tx().execute(
                "INSERT INTO purchase_return_lines (return_id, purchase_id, position, qty_milli, net_minor, tax_minor,
                        stock_value_minor, movement_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, input.purchase_id, position, q, n, t, v, movement],
            )?;
        }
        let m = |v: i64| Money::from_minor(v, currency);
        let zero = m(0);
        let refund = match method.as_str() {
            "cash" => Settlement { cash: m(total), bank: zero, credit: zero },
            "bank" => Settlement { cash: zero, bank: m(total), credit: zero },
            _ => Settlement { cash: zero, bank: zero, credit: m(total) },
        };
        post_operations_for(
            w,
            &branch,
            offset,
            &[Operation::PurchaseReturn {
                net: m(net),
                vat: m(tax),
                refund,
                stock_value: m(value),
            }],
            &format!("{number} — {}", supplier.name),
            "purchase.return",
            till_of.as_ref().map(|t| t.1.as_str()),
            Some(&supplier.id),
        )?;
        let out = PurchaseReturnResult {
            id: id.clone(),
            number,
            net_minor: net,
            tax_minor: tax,
            total_minor: total,
            stock_value_minor: value,
        };
        w.event("purchase.returned", "purchase_return", &id, &out)?;
        w.audit("return", "purchase", &input.purchase_id, &input)?;
        Ok(out)
    })
}

// ---------------------------------------------------------------------------
// Paying suppliers (M10.7)

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SupplierPaymentInput {
    pub supplier_id: String,
    pub amount_minor: i64,
    /// `cash` (from the open till of `register_id`) or `bank`.
    pub method: String,
    #[serde(default)]
    pub register_id: Option<String>,
    /// (purchase id, amount); empty: the oldest invoices first.
    #[serde(default)]
    pub allocations: Vec<(String, i64)>,
    #[serde(default)]
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierPaymentResult {
    pub id: String,
    pub number: String,
    pub settled: Vec<(String, i64)>,
    pub unallocated_minor: i64,
    pub balance_minor: i64,
}

/// Pays a supplier: from the till (never more than it holds) or the bank,
/// settling its invoices; what is left is an advance.
pub fn pay_supplier(
    store: &mut Store,
    meta: OpMeta,
    input: SupplierPaymentInput,
) -> Result<SupplierPaymentResult> {
    if input.amount_minor <= 0 {
        return Err(RuleError::on("amount", "receipt.amount_positive").into());
    }
    let from_bank = match input.method.as_str() {
        "cash" => false,
        "bank" => true,
        _ => return Err(RuleError::on("method", "invalid_value").into()),
    };
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.utc_offset_min;
    store.write(meta, |w| {
        let supplier = load_supplier(w.tx(), &input.supplier_id)?;
        let branch = org::branches(w.tx(), w.org()?)?
            .into_iter()
            .next()
            .ok_or(Error::NotFound("branch"))?;
        w.require(
            Permission::SuppliersPay,
            &Target {
                branch: Some(parse_id(&branch.id)?),
                ..Target::org()
            },
        )?;
        let till_of = open_shift_of(w, input.register_id.as_deref())?;
        if !from_bank {
            let Some((shift, _)) = &till_of else {
                return Err(RuleError::on("register", "shift.none_open").into());
            };
            if input.amount_minor > shift_of(w.tx(), shift)?.expected_minor {
                return Err(RuleError::on("amount", "shift.not_enough_cash").into());
            }
        }
        let open = open_purchases_of(w.tx(), &supplier.id)?;
        let mut left = input.amount_minor;
        let mut settled: Vec<(String, String, i64)> = Vec::new();
        if input.allocations.is_empty() {
            for p in &open {
                if left == 0 {
                    break;
                }
                let take = p.open_minor.min(left);
                left -= take;
                settled.push((p.id.clone(), p.number.clone(), take));
            }
        } else {
            for (pid, amount) in &input.allocations {
                let p = open
                    .iter()
                    .find(|p| &p.id == pid)
                    .ok_or(Error::Rules(vec![RuleError::on("allocations", "receipt.not_open")]))?;
                if *amount <= 0 || *amount > p.open_minor {
                    return Err(RuleError::on("allocations", "receipt.more_than_open").into());
                }
                if *amount > left {
                    return Err(RuleError::on("allocations", "receipt.more_than_paid").into());
                }
                left -= amount;
                settled.push((p.id.clone(), p.number.clone(), *amount));
            }
        }
        let number = w.next_number(DocKind::Payment, &branch.id, &branch.code)?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        let (register, shift) = match (&till_of, from_bank) {
            (Some((s, _)), false) => (input.register_id.clone(), Some(s.clone())),
            _ => (None, None),
        };
        w.tx().execute(
            "INSERT INTO supplier_payments (id, org_id, branch_id, supplier_id, number, method, register_id, shift_id,
                    amount_minor, unallocated_minor, reference, operation_id, created_at, created_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                w.org()?.to_string(),
                branch.id,
                supplier.id,
                number,
                input.method,
                register,
                shift,
                input.amount_minor,
                left,
                input.reference.trim(),
                w.meta().id.to_string(),
                w.now(),
                w.actor_text(),
                w.device().id.to_string()
            ],
        )?;
        for (pid, _, amount) in &settled {
            w.tx().execute(
                "INSERT INTO supplier_allocations (payment_id, purchase_id, amount_minor) VALUES (?1, ?2, ?3)",
                params![id, pid, amount],
            )?;
        }
        post_operations_for(
            w,
            &branch,
            offset,
            &[Operation::SupplierPayment {
                amount: Money::from_minor(input.amount_minor, currency),
                from_bank,
            }],
            &format!("{number} — {}", supplier.name),
            "supplier.payment",
            if from_bank {
                None
            } else {
                till_of.as_ref().map(|t| t.1.as_str())
            },
            Some(&supplier.id),
        )?;
        let balance: i64 = w.tx().query_row(
            "SELECT balance_minor FROM suppliers WHERE id = ?1",
            [&supplier.id],
            |r| r.get(0),
        )?;
        let out = SupplierPaymentResult {
            id: id.clone(),
            number,
            settled: settled.into_iter().map(|(_, n, a)| (n, a)).collect(),
            unallocated_minor: left,
            balance_minor: balance,
        };
        w.event("supplier.paid", "supplier_payment", &id, &out)?;
        w.audit("pay", "supplier", &supplier.id, &input)?;
        Ok(out)
    })
}

/// A line of a supplier's statement (what is owed to it, running).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierStatementLine {
    pub date: String,
    pub entry_number: String,
    pub source: String,
    pub memo: String,
    /// What it raised (an invoice) and lowered (a payment, a return).
    pub owed_minor: i64,
    pub paid_minor: i64,
    pub balance_minor: i64,
}

pub fn supplier_statement(store: &Store, supplier: &str) -> Result<Vec<SupplierStatementLine>> {
    store.read(|r| {
        load_supplier(r.conn(), supplier)?;
        let mut s = r.conn().prepare(
            "SELECT e.date, e.number, e.source, e.memo, l.credit_minor, l.debit_minor
             FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
             WHERE l.party_id = ?1 ORDER BY e.date, e.created_at, e.number, l.position",
        )?;
        let mut balance = 0;
        let mut out = Vec::new();
        for row in s.query_map([supplier], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })? {
            let (date, entry_number, source, memo, owed, paid) = row?;
            balance += owed - paid;
            out.push(SupplierStatementLine {
                date,
                entry_number,
                source,
                memo,
                owed_minor: owed,
                paid_minor: paid,
                balance_minor: balance,
            });
        }
        Ok(out)
    })
}

/// What a supplier was owed before the books started here.
pub fn supplier_opening(
    store: &mut Store,
    meta: OpMeta,
    supplier: &str,
    owed_minor: i64,
    reason: &str,
) -> Result<i64> {
    if owed_minor <= 0 {
        return Err(RuleError::on("amount", "receipt.amount_positive").into());
    }
    if reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.utc_offset_min;
    store.write(meta, |w| {
        w.require(Permission::DebtsAdjust, &Target::org())?;
        let s = load_supplier(w.tx(), supplier)?;
        let branch = org::branches(w.tx(), w.org()?)?
            .into_iter()
            .next()
            .ok_or(Error::NotFound("branch"))?;
        post_operations_for(
            w,
            &branch,
            offset,
            &[Operation::SupplierOpening {
                owed: Money::from_minor(owed_minor, currency),
            }],
            &format!("{} — {}", s.name, reason.trim()),
            "supplier.opening",
            None,
            Some(&s.id),
        )?;
        let balance: i64 = w.tx().query_row(
            "SELECT balance_minor FROM suppliers WHERE id = ?1",
            [supplier],
            |r| r.get(0),
        )?;
        w.event("supplier.opening", "supplier", supplier, &balance)?;
        w.audit("opening", "supplier", supplier, &owed_minor)?;
        Ok(balance)
    })
}

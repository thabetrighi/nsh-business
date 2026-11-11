//! Point of sale (M8): till sessions with their cash movements, and the
//! completed sale — invoice, lines, payments, stock, entry, event and audit
//! in one transaction, once per operation (a double tap sells once).

use nb_core::{Money, OrganizationId, Quantity, Rate};
use nb_domain::access::{Permission, Target};
use nb_domain::numbering::DocKind;
use nb_domain::posting::{Operation, Settlement as Paid};
use nb_domain::sale::{Cart, CartLine, Discount, Method, settle};
use nb_domain::stock::MovementKind;
use nb_domain::tax::TaxMode;
use nb_domain::{ProductId, RegisterId, RuleError, WarehouseId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::repo::{catalog, ledger, org, parse_id, stock, to_milli};
use crate::uow::{OpMeta, Writer};
use crate::usecases::accounting::post_operations_at;
use crate::usecases::tracked::{self, Tracked};
use crate::{Error, Result, Store};

/// Line discounts above this share need `sales.discount_exceptional`.
pub const ORDINARY_DISCOUNT_BPS: i64 = 1_000;

pub(crate) fn the_org(conn: &Connection) -> Result<(OrganizationId, nb_core::Currency)> {
    let o = org::the_org(conn)?.ok_or(Error::NotFound("organization"))?;
    Ok((o.org_id()?, o.currency()?))
}

pub(crate) fn money_field(field: &'static str, minor: i64) -> std::result::Result<i64, RuleError> {
    if minor < 0 {
        Err(RuleError::on(field, "ledger.negative"))
    } else {
        Ok(minor)
    }
}

// ---------------------------------------------------------------------------
// Till sessions (M8.6)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShiftRow {
    pub id: String,
    pub number: String,
    pub register_id: String,
    pub register: String,
    pub status: String,
    pub opening_minor: i64,
    pub opened_at: i64,
    pub closed_at: Option<i64>,
    /// Cash taken by sales (after change).
    pub cash_sales_minor: i64,
    pub card_minor: i64,
    pub transfer_minor: i64,
    pub credit_minor: i64,
    pub cash_in_minor: i64,
    pub cash_out_minor: i64,
    /// Cash given back for returns.
    pub refunds_minor: i64,
    pub sales: u32,
    /// What the till should hold: opening + cash sales + in − out − refunds.
    pub expected_minor: i64,
    pub counted_minor: Option<i64>,
    pub difference_minor: Option<i64>,
}

pub(crate) fn shift_of(conn: &Connection, id: &str) -> Result<ShiftRow> {
    let mut row = conn
        .query_row(
            "SELECT s.id, s.number, s.register_id, r.name, s.status, s.opening_minor, s.opened_at, s.closed_at,
                    s.counted_minor, s.difference_minor
             FROM shifts s JOIN cash_registers r ON r.id = s.register_id WHERE s.id = ?1",
            [id],
            |r| {
                Ok(ShiftRow {
                    id: r.get(0)?,
                    number: r.get(1)?,
                    register_id: r.get(2)?,
                    register: r.get(3)?,
                    status: r.get(4)?,
                    opening_minor: r.get(5)?,
                    opened_at: r.get(6)?,
                    closed_at: r.get(7)?,
                    counted_minor: r.get(8)?,
                    difference_minor: r.get(9)?,
                    cash_sales_minor: 0,
                    card_minor: 0,
                    transfer_minor: 0,
                    credit_minor: 0,
                    cash_in_minor: 0,
                    cash_out_minor: 0,
                    refunds_minor: 0,
                    sales: 0,
                    expected_minor: 0,
                })
            },
        )
        .optional()?
        .ok_or(Error::NotFound("shift"))?;
    let mut s = conn.prepare(
        "SELECT method, COALESCE(SUM(amount_minor), 0) FROM payments WHERE shift_id = ?1 GROUP BY method",
    )?;
    for r in s.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (method, amount) = r?;
        match method.as_str() {
            "cash" => row.cash_sales_minor = amount,
            "card" => row.card_minor = amount,
            "transfer" => row.transfer_minor = amount,
            _ => row.credit_minor = amount,
        }
    }
    let (cash_in, cash_out): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(CASE WHEN direction = 'in' THEN amount_minor END), 0),
                COALESCE(SUM(CASE WHEN direction = 'out' THEN amount_minor END), 0)
         FROM cash_movements WHERE shift_id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    row.cash_in_minor = cash_in;
    row.cash_out_minor = cash_out;
    row.sales = conn.query_row(
        "SELECT COUNT(*) FROM sales WHERE shift_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    row.refunds_minor = conn.query_row(
        "SELECT COALESCE(SUM(total_minor), 0) FROM sale_returns WHERE shift_id = ?1 AND refund_method = 'cash'",
        [id],
        |r| r.get(0),
    )?;
    row.expected_minor =
        row.opening_minor + row.cash_sales_minor + cash_in - cash_out - row.refunds_minor;
    Ok(row)
}

/// The open session of a till, if any.
pub fn current_shift(store: &Store, register_id: &str) -> Result<Option<ShiftRow>> {
    store.read(|r| {
        let id: Option<String> = r
            .conn()
            .query_row(
                "SELECT id FROM shifts WHERE register_id = ?1 AND status = 'open'",
                [register_id],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| shift_of(r.conn(), &id)).transpose()
    })
}

pub fn shift_summary(store: &Store, shift_id: &str) -> Result<ShiftRow> {
    store.read(|r| shift_of(r.conn(), shift_id))
}

/// The till, its branch, and the account of its cash.
pub(crate) fn till(w: &Writer<'_>, register: RegisterId) -> Result<(org::Branch, String, String)> {
    let (branch_id, code, account): (String, String, Option<String>) = w
        .tx()
        .query_row(
            "SELECT branch_id, code, money_account_code FROM cash_registers WHERE id = ?1 AND deleted_at IS NULL",
            [register.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or(Error::NotFound("register"))?;
    let branch = org::branch(w.tx(), parse_id(&branch_id)?)?;
    let account = match account {
        Some(a) => a,
        None => ledger::account_map(w.tx(), w.org()?)?
            .get(nb_domain::posting::Role::Cash)?
            .to_owned(),
    };
    Ok((branch, code, account))
}

/// Opens a till session with the cash already in the drawer.
pub fn open_shift(
    store: &mut Store,
    meta: OpMeta,
    register_id: &str,
    opening_minor: i64,
) -> Result<ShiftRow> {
    let register: RegisterId = parse_id(register_id)?;
    money_field("opening", opening_minor)?;
    let id = store.write(meta, |w| {
        let (branch, code, account) = till(w, register)?;
        w.require(
            Permission::CashOpenShift,
            &Target::register(parse_id(&branch.id)?, register),
        )?;
        let open: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM shifts WHERE register_id = ?1 AND status = 'open'",
            [register_id],
            |r| r.get(0),
        )?;
        if open > 0 {
            return Err(RuleError::on("register", "shift.already_open").into());
        }
        let n: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM shifts WHERE register_id = ?1",
            [register_id],
            |r| r.get(0),
        )?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        let number = format!("{}-{}-{:04}", branch.code, code, n + 1);
        w.tx().execute(
            "INSERT INTO shifts (id, org_id, branch_id, register_id, account_code, number, opening_minor, opened_at, opened_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![id, w.org()?.to_string(), branch.id, register_id, account, number, opening_minor, w.now(), w.actor_text(), w.device().id.to_string()],
        )?;
        let detail = serde_json::json!({ "number": number, "opening": opening_minor });
        w.event("shift.opened", "shift", &id, &detail)?;
        w.audit("open", "shift", &id, &detail)?;
        Ok(id)
    })?;
    shift_summary(store, &id)
}

/// Money put into (`in`) or taken out of (`out`) the drawer during a
/// session, through internal transfers, with its reason.
pub fn cash_movement(
    store: &mut Store,
    meta: OpMeta,
    shift_id: &str,
    into_till: bool,
    amount_minor: i64,
    reason: &str,
) -> Result<ShiftRow> {
    if amount_minor <= 0 {
        return Err(RuleError::on("amount", "sale.amount_positive").into());
    }
    if reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    store.write(meta, |w| {
        let s = shift_of(w.tx(), shift_id)?;
        if s.status != "open" {
            return Err(RuleError::on("shift", "shift.closed").into());
        }
        let register: RegisterId = parse_id(&s.register_id)?;
        let (branch, _, account) = till(w, register)?;
        w.require(
            Permission::CashMovement,
            &Target::register(parse_id(&branch.id)?, register),
        )?;
        if !into_till && amount_minor > s.expected_minor {
            return Err(RuleError::on("amount", "shift.not_enough_cash").into());
        }
        let (_, currency) = the_org(w.tx())?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        w.tx().execute(
            "INSERT INTO cash_movements (id, org_id, shift_id, direction, amount_minor, reason, created_at, created_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, w.org()?.to_string(), shift_id, if into_till { "in" } else { "out" }, amount_minor, reason.trim(), w.now(), w.actor_text(), w.device().id.to_string()],
        )?;
        post_operations_at(
            w,
            &branch,
            offset,
            &[Operation::TillTransfer {
                amount: Money::from_minor(amount_minor, currency),
                into_till,
            }],
            &format!("{} — {}", s.number, reason.trim()),
            "cash.transfer",
            Some(&account),
        )?;
        let detail = serde_json::json!({ "in": into_till, "amount": amount_minor, "reason": reason.trim() });
        w.event("shift.cash_moved", "shift", shift_id, &detail)?;
        w.audit("cash_movement", "shift", shift_id, &detail)?;
        Ok(())
    })?;
    shift_summary(store, shift_id)
}

/// Closes a session with the cash counted in the drawer: a difference
/// with what it should hold is booked as a shortage or a surplus.
pub fn close_shift(
    store: &mut Store,
    meta: OpMeta,
    shift_id: &str,
    counted_minor: i64,
    note: &str,
) -> Result<ShiftRow> {
    money_field("counted", counted_minor)?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    store.write(meta, |w| {
        let s = shift_of(w.tx(), shift_id)?;
        if s.status != "open" {
            return Err(RuleError::on("shift", "shift.closed").into());
        }
        let register: RegisterId = parse_id(&s.register_id)?;
        let (branch, _, account) = till(w, register)?;
        w.require(
            Permission::CashCloseShift,
            &Target::register(parse_id(&branch.id)?, register),
        )?;
        let (_, currency) = the_org(w.tx())?;
        let difference = counted_minor - s.expected_minor;
        let m = |v: i64| Money::from_minor(v, currency);
        post_operations_at(
            w,
            &branch,
            offset,
            &[Operation::CashDifference {
                short: m((-difference).max(0)),
                over: m(difference.max(0)),
            }],
            &s.number,
            "cash.difference",
            Some(&account),
        )?;
        w.tx().execute(
            "UPDATE shifts SET status = 'closed', closed_at = ?2, closed_by = ?3, expected_minor = ?4,
                    counted_minor = ?5, difference_minor = ?6, note = ?7 WHERE id = ?1",
            params![shift_id, w.now(), w.actor_text(), s.expected_minor, counted_minor, difference, note.trim()],
        )?;
        let detail = serde_json::json!({
            "expected": s.expected_minor, "counted": counted_minor, "difference": difference
        });
        w.event("shift.closed", "shift", shift_id, &detail)?;
        w.audit("close", "shift", shift_id, &detail)?;
        Ok(())
    })?;
    shift_summary(store, shift_id)
}

// ---------------------------------------------------------------------------
// Completed sale (M8.4, M8.5)

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SaleLineInput {
    /// Empty: an item not in the catalog, sold under `name` at
    /// `unit_price_minor` with `tax_bps` (no stock moves).
    pub product_id: String,
    /// In `unit` (decimal text).
    pub qty: String,
    /// Empty: the base unit.
    #[serde(default)]
    pub unit: String,
    /// Another price per unit than the product's (`sales.price_override`).
    #[serde(default)]
    pub unit_price_minor: Option<i64>,
    #[serde(default)]
    pub discount_bps: Option<i64>,
    #[serde(default)]
    pub discount_minor: Option<i64>,
    #[serde(default)]
    pub tracked: Tracked,
    /// The name printed on the line, if not the product's.
    #[serde(default)]
    pub name: Option<String>,
    /// The VAT rate of an item not in the catalog.
    #[serde(default)]
    pub tax_bps: Option<i64>,
}

/// What a line sells: a product of the catalog, or a typed item.
pub(crate) struct Priced {
    pub(crate) product: Option<catalog::ProductRow>,
    pub(crate) name: String,
    pub(crate) unit: String,
    pub(crate) base_qty: Quantity,
    pub(crate) tax_bps: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentInput {
    /// `cash`, `card`, `transfer` or `credit`.
    pub method: String,
    /// For cash: what the customer gave (change is worked out).
    pub amount_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SaleInput {
    pub register_id: String,
    pub warehouse_id: String,
    pub lines: Vec<SaleLineInput>,
    #[serde(default)]
    pub discount_bps: Option<i64>,
    #[serde(default)]
    pub discount_minor: Option<i64>,
    pub payments: Vec<PaymentInput>,
    /// Needed to leave anything on credit.
    #[serde(default)]
    pub customer_id: Option<String>,
    #[serde(default)]
    pub note: String,
    /// A supervisor's approval for a discount, price or floor beyond the
    /// seller's rights.
    #[serde(default)]
    pub approval: Option<String>,
    /// The approved quote or order this sale invoices.
    #[serde(default)]
    pub document_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaleResult {
    pub id: String,
    pub number: String,
    pub net_minor: i64,
    pub tax_minor: i64,
    pub total_minor: i64,
    pub cash_minor: i64,
    pub card_minor: i64,
    pub transfer_minor: i64,
    pub credit_minor: i64,
    pub change_minor: i64,
    pub cost_minor: i64,
}

/// The lines' products and quantities, as typed.
pub(crate) fn parse_lines(lines: &[SaleLineInput]) -> Result<Vec<(Option<ProductId>, Quantity)>> {
    let mut parsed = Vec::with_capacity(lines.len());
    for l in lines {
        let qty: Quantity = l
            .qty
            .trim()
            .replace(',', ".")
            .parse()
            .map_err(|_| RuleError::on("qty", "invalid_quantity"))?;
        let product = if l.product_id.trim().is_empty() {
            None
        } else {
            Some(parse_id::<ProductId>(&l.product_id)?)
        };
        parsed.push((product, qty));
    }
    Ok(parsed)
}

/// Prices lines from the catalog (or as typed for items not in it): a
/// price other than the catalog's needs its right (or an approval).
pub(crate) fn price_lines(
    w: &mut Writer<'_>,
    parsed: &[(Option<ProductId>, Quantity)],
    input: &[SaleLineInput],
    currency: nb_core::Currency,
    at_till: &Target,
    approval: Option<nb_domain::ApprovalId>,
) -> Result<(Vec<Priced>, Vec<CartLine>)> {
    let mut products: Vec<Priced> = Vec::with_capacity(parsed.len());
    let mut lines = Vec::with_capacity(parsed.len());
    for ((product_id, qty), l) in parsed.iter().zip(input) {
        let name = l
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
        let Some(product_id) = product_id else {
            // An item not in the catalog: its name, price and rate as typed.
            let name = name.ok_or(Error::from(RuleError::on("name", "sale.free_name")))?;
            let price = l
                .unit_price_minor
                .ok_or(Error::from(RuleError::on("price", "sale.free_price")))?;
            let tax_bps = l.tax_bps.unwrap_or(0);
            if !(0..=10_000).contains(&tax_bps) {
                return Err(RuleError::on("tax_rate", "invalid_value").into());
            }
            lines.push(CartLine {
                qty: *qty,
                unit_price: Money::from_minor(money_field("price", price)?, currency),
                min_price: None,
                tax_rate: Rate::from_bps(tax_bps),
                discount: discount(l.discount_bps, l.discount_minor, currency),
            });
            products.push(Priced {
                product: None,
                name,
                unit: if l.unit.is_empty() {
                    "pc".into()
                } else {
                    l.unit.clone()
                },
                base_qty: *qty,
                tax_bps,
            });
            continue;
        };
        let p = catalog::get(w.tx(), *product_id)?;
        if !p.active {
            return Err(RuleError::on("product", "sale.product_inactive").into());
        }
        let units = p.unit_set()?;
        let unit = if l.unit.is_empty() {
            p.units[0].code.clone()
        } else {
            l.unit.clone()
        };
        let base_qty = units.to_base(*qty, &unit)?;
        let per_unit = units.to_base(Quantity::whole(1), &unit)?;
        let catalog_price = per_unit.times_price(Money::from_minor(p.price_minor, currency))?;
        let unit_price = match l.unit_price_minor {
            Some(price) if price != catalog_price.minor() => {
                w.require_approved(Permission::SalesPriceOverride, at_till, approval)?;
                Money::from_minor(money_field("price", price)?, currency)
            }
            _ => catalog_price,
        };
        let min_price = p
            .min_price_minor
            .map(|m| per_unit.times_price(Money::from_minor(m, currency)))
            .transpose()?;
        lines.push(CartLine {
            qty: *qty,
            unit_price,
            min_price,
            tax_rate: Rate::from_bps(p.tax_bps),
            discount: discount(l.discount_bps, l.discount_minor, currency),
        });
        products.push(Priced {
            name: name.unwrap_or_else(|| p.name.clone()),
            tax_bps: p.tax_bps,
            product: Some(p),
            unit,
            base_qty,
        });
    }
    Ok((products, lines))
}

pub(crate) fn discount(bps: Option<i64>, minor: Option<i64>, c: nb_core::Currency) -> Discount {
    match (bps, minor) {
        (Some(b), _) if b != 0 => Discount::Percent(b),
        (_, Some(m)) if m != 0 => Discount::Amount(Money::from_minor(m, c)),
        _ => Discount::None,
    }
}

/// Completes a sale in one transaction: the invoice and its lines, the
/// payments, the stock leaving (with its lots or serial numbers), the
/// entry (sale and its cost), the event and the audit. The same operation
/// sent again returns the same sale.
pub fn complete_sale(store: &mut Store, meta: OpMeta, input: SaleInput) -> Result<SaleResult> {
    let register: RegisterId = parse_id(&input.register_id)?;
    let warehouse_id: WarehouseId = parse_id(&input.warehouse_id)?;
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    let parsed = parse_lines(&input.lines)?;
    let mut given = Vec::with_capacity(input.payments.len());
    for p in &input.payments {
        let method =
            Method::from_code(&p.method).ok_or(RuleError::on("payments", "invalid_value"))?;
        given.push((
            method,
            Money::from_minor(money_field("payments", p.amount_minor)?, currency),
        ));
    }
    store.write(meta, |w| {
        let (branch, _, till_account) = till(w, register)?;
        let warehouse = org::warehouse(w.tx(), warehouse_id)?;
        if warehouse.branch_id != branch.id {
            return Err(RuleError::on("warehouse", "sale.other_branch").into());
        }
        let branch_id = parse_id(&branch.id)?;
        let at_till = Target::register(branch_id, register);
        let at_store = Target::warehouse(branch_id, warehouse_id);
        w.require(Permission::SalesSell, &at_till)?;
        let shift: String = w
            .tx()
            .query_row(
                "SELECT id FROM shifts WHERE register_id = ?1 AND status = 'open'",
                [register.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::from(RuleError::on("register", "shift.none_open")))?;
        let approval = input.approval.as_deref().map(parse_id).transpose()?;

        // The cart, priced from the catalog (or as typed for items not in it).
        let (products, lines) = price_lines(w, &parsed, &input.lines, currency, &at_till, approval)?;
        let cart = Cart {
            mode: TaxMode::Inclusive,
            lines,
            discount: discount(input.discount_bps, input.discount_minor, currency),
        };
        let totals = cart.totals()?;
        // What the seller may give, else a supervisor's approval.
        if totals.discounts.minor() > 0 {
            w.require(Permission::SalesDiscountLine, &at_till)?;
        }
        if totals.lines.iter().any(|l| l.discount_bps > ORDINARY_DISCOUNT_BPS) {
            w.require_approved(Permission::SalesDiscountExceptional, &at_till, approval)?;
        }
        if totals.lines.iter().any(|l| l.below_min) {
            w.require_approved(Permission::SalesBelowMinPrice, &at_till, approval)?;
        }
        let customer = input.customer_id.as_deref().filter(|c| !c.is_empty());
        let paid = settle(totals.total, &given, customer.is_some())?;
        // The customer must exist; credit past its limit needs a supervisor.
        if let Some(c) = customer {
            crate::usecases::customers::check_credit(w, c, paid.credit.minor(), &at_till, approval)?;
        }

        let number = w.next_number(DocKind::Invoice, &branch.id, &branch.code)?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();

        // Stock leaves at its average cost.
        let may_go_negative = w.allowed(Permission::StockNegative, &at_store)?;
        let mut cost = Money::zero(currency);
        let mut costs = Vec::with_capacity(products.len());
        let mut movements = Vec::with_capacity(products.len());
        for (item, l) in products.iter().zip(&input.lines) {
            let base_qty = &item.base_qty;
            let Some(p) = item
                .product
                .as_ref()
                .filter(|p| catalog::kind(p) != nb_domain::product::ProductKind::Service)
            else {
                // Services and items not in the catalog move no stock.
                costs.push(Money::zero(currency));
                movements.push(None);
                continue;
            };
            let product_id: ProductId = parse_id(&p.id)?;
            let before = stock::balance(w.tx(), product_id, warehouse_id, currency)?;
            let after = before.apply(
                MovementKind::Sale,
                *base_qty,
                None,
                warehouse.policy(),
                may_go_negative,
            )?;
            let movement = stock::append(
                w,
                product_id,
                warehouse_id,
                MovementKind::Sale,
                *base_qty,
                Some(before.avg_cost),
                Some(&number),
                None,
                after,
            )?;
            tracked::record(w, movement, product_id, warehouse_id, MovementKind::Sale, *base_qty, &l.tracked, false)?;
            let line_cost = base_qty.times_price(before.avg_cost)?;
            cost = cost.checked_add(line_cost)?;
            costs.push(before.avg_cost);
            movements.push(Some(movement.to_string()));
        }

        w.tx().execute(
            "INSERT INTO sales (id, org_id, branch_id, warehouse_id, register_id, shift_id, number, customer_id, tax_mode,
                    gross_before_minor, discount_minor, net_minor, tax_minor, total_minor, cash_minor, card_minor,
                    transfer_minor, credit_minor, change_minor, cost_minor, note, operation_id, created_at, created_by, device_id, document_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'inclusive', ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
            params![
                id, w.org()?.to_string(), branch.id, warehouse.id, register.to_string(), shift, number,
                customer, totals.gross_before.minor(), totals.discounts.minor(), totals.net.minor(),
                totals.tax.minor(), totals.total.minor(), paid.cash.minor(), paid.card.minor(), paid.transfer.minor(),
                paid.credit.minor(), paid.change.minor(), cost.minor(), input.note.trim(), w.meta().id.to_string(),
                w.now(), w.actor_text(), w.device().id.to_string(), input.document_id
            ],
        )?;
        if let Some(doc) = input.document_id.as_deref() {
            crate::usecases::documents::invoice_from(w, doc, &id)?;
        }
        for (i, (item, t)) in products.iter().zip(&totals.lines).enumerate() {
            w.tx().execute(
                "INSERT INTO sale_lines (sale_id, position, product_id, name, unit, qty_milli, base_qty_milli, unit_price_minor,
                        line_discount_minor, sale_discount_minor, tax_bps, net_minor, tax_minor, gross_minor, unit_cost_minor, movement_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    id, i64::try_from(i).unwrap_or(0), item.product.as_ref().map(|p| p.id.clone()), item.name, item.unit,
                    to_milli(cart.lines[i].qty)?, to_milli(item.base_qty)?,
                    cart.lines[i].unit_price.minor(), t.line_discount.minor(), t.sale_discount.minor(), item.tax_bps,
                    t.taxed.net.minor(), t.taxed.tax.minor(), t.taxed.gross.minor(), costs[i].minor(), movements[i]
                ],
            )?;
        }
        for (method, amount, account) in [
            ("cash", paid.cash, Some(till_account.clone())),
            ("card", paid.card, None),
            ("transfer", paid.transfer, None),
            ("credit", paid.credit, None),
        ] {
            if amount.is_zero() {
                continue;
            }
            w.tx().execute(
                "INSERT INTO payments (id, org_id, sale_id, shift_id, method, amount_minor, account_code, created_at, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![nb_core::uuid::Uuid::now_v7().to_string(), w.org()?.to_string(), id, shift, method, amount.minor(), account, w.now(), w.device().id.to_string()],
            )?;
        }
        crate::usecases::accounting::post_operations_for(
            w,
            &branch,
            offset,
            &[
                Operation::Sale {
                    net: totals.net,
                    vat: totals.tax,
                    stamp: Money::zero(currency),
                    paid: Paid {
                        cash: paid.cash,
                        bank: paid.card.checked_add(paid.transfer)?,
                        credit: paid.credit,
                    },
                },
                Operation::CostOfSale { cost },
            ],
            &number,
            "sale",
            Some(&till_account),
            customer,
        )?;
        let out = SaleResult {
            id: id.clone(),
            number,
            net_minor: totals.net.minor(),
            tax_minor: totals.tax.minor(),
            total_minor: totals.total.minor(),
            cash_minor: paid.cash.minor(),
            card_minor: paid.card.minor(),
            transfer_minor: paid.transfer.minor(),
            credit_minor: paid.credit.minor(),
            change_minor: paid.change.minor(),
            cost_minor: cost.minor(),
        };
        w.event("sale.completed", "sale", &id, &out)?;
        w.audit("complete", "sale", &id, &input)?;
        Ok(out)
    })
}

/// A sale in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaleRow {
    pub id: String,
    pub number: String,
    pub created_at: i64,
    pub total_minor: i64,
    pub lines: u32,
    pub methods: Vec<String>,
    /// What came back from it.
    pub returned_minor: i64,
    /// The quote or order it invoices.
    pub document: Option<String>,
    pub customer_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SalesQuery {
    /// In the number (or the document's).
    #[serde(default)]
    pub search: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SalesPage {
    pub rows: Vec<SaleRow>,
    pub total: u32,
    /// Of all the sales the filter keeps.
    pub total_minor: i64,
}

pub fn list_sales(store: &Store, q: &SalesQuery) -> Result<SalesPage> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let search = q.search.trim().to_uppercase();
        let filter = " FROM sales s LEFT JOIN sales_documents d ON d.id = s.document_id
             WHERE s.org_id = ?1 AND (?2 = '' OR upper(s.number) LIKE '%' || ?2 || '%'
                   OR upper(d.number) LIKE '%' || ?2 || '%')";
        let (total, sum): (u32, i64) = r.conn().query_row(
            &format!("SELECT COUNT(*), COALESCE(SUM(s.total_minor), 0){filter}"),
            params![org.to_string(), search],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut st = r.conn().prepare(&format!(
            "SELECT s.id, s.number, s.created_at, s.total_minor,
                    (SELECT COUNT(*) FROM sale_lines l WHERE l.sale_id = s.id),
                    s.cash_minor, s.card_minor, s.transfer_minor, s.credit_minor,
                    (SELECT COALESCE(SUM(x.total_minor), 0) FROM sale_returns x WHERE x.sale_id = s.id),
                    d.number, s.customer_id
             {filter} ORDER BY s.created_at DESC, s.number DESC LIMIT ?3 OFFSET ?4"
        ))?;
        let rows = st.query_map(
            params![org.to_string(), search, q.limit.clamp(1, 500), q.offset],
            |r| {
                let mut methods = Vec::new();
                for (i, m) in ["cash", "card", "transfer", "credit"].iter().enumerate() {
                    if r.get::<_, i64>(5 + i)? > 0 {
                        methods.push((*m).to_owned());
                    }
                }
                Ok(SaleRow {
                    id: r.get(0)?,
                    number: r.get(1)?,
                    created_at: r.get(2)?,
                    total_minor: r.get(3)?,
                    lines: r.get(4)?,
                    methods,
                    returned_minor: r.get(9)?,
                    document: r.get(10)?,
                    customer_id: r.get(11)?,
                })
            },
        )?;
        Ok(SalesPage {
            rows: rows.collect::<std::result::Result<_, _>>()?,
            total,
            total_minor: sum,
        })
    })
}

pub fn recent_sales(store: &Store, limit: u32) -> Result<Vec<SaleRow>> {
    Ok(list_sales(
        store,
        &SalesQuery {
            limit,
            ..Default::default()
        },
    )?
    .rows)
}

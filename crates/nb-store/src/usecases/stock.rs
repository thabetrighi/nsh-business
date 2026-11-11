//! Stock use cases (M6): adjustments, waste and count differences, each
//! with its document number, movement, balance, accounting entry, event and
//! audit — in one transaction.

use nb_core::{Money, Quantity};
use nb_domain::ledger::{Entry, Line};
use nb_domain::numbering::DocKind;
use nb_domain::stock::MovementKind;
use nb_domain::{ProductId, RuleError, WarehouseId};
use serde::{Deserialize, Serialize};

use crate::repo::{catalog, ledger, org, parse_id, stock};
use crate::time::iso_date;
use crate::uow::OpMeta;
use crate::{Error, Result, Store};
use nb_domain::access::{Permission, Target};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjustInput {
    pub product_id: String,
    pub warehouse_id: String,
    /// `adjust_in`, `adjust_out`, `waste`, `count_gain` or `count_loss`.
    pub kind: String,
    /// Quantity in `unit`, as decimal text (positive).
    pub qty: String,
    pub unit: String,
    /// Cost of one base unit for stock coming in; default: the average cost.
    pub unit_cost_minor: Option<i64>,
    pub reason: String,
    /// The user confirms taking stock below zero; it happens only where the
    /// warehouse allows it, or with `stock.negative`.
    pub may_go_negative: bool,
    /// A supervisor's approval (`stock.adjust`) for a user who lacks it.
    #[serde(default)]
    pub approval: Option<String>,
    /// Lot (in, or a chosen lot out) or serial numbers.
    #[serde(default)]
    pub tracked: crate::usecases::tracked::Tracked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjustResult {
    pub number: String,
    pub on_hand_milli: i64,
    pub value_minor: i64,
}

/// The accounts an adjustment moves (provisional mapping, gate G2).
fn accounts(kind: MovementKind) -> Option<(&'static str, &'static str)> {
    match kind {
        MovementKind::AdjustmentIn | MovementKind::CountGain => Some(("30", "758")),
        MovementKind::AdjustmentOut | MovementKind::CountLoss | MovementKind::Waste => {
            Some(("658", "30"))
        }
        _ => None,
    }
}

pub fn adjust_stock(store: &mut Store, meta: OpMeta, input: AdjustInput) -> Result<AdjustResult> {
    let kind = MovementKind::from_code(&input.kind)
        .filter(|k| accounts(*k).is_some())
        .ok_or(RuleError::on("kind", "invalid_value"))?;
    if input.reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    let typed: Quantity = input
        .qty
        .trim()
        .replace(',', ".")
        .parse()
        .map_err(|_| RuleError::on("qty", "invalid_quantity"))?;
    let product_id: ProductId = parse_id(&input.product_id)?;
    let warehouse_id: WarehouseId = parse_id(&input.warehouse_id)?;
    let org_row = store
        .read(|r| org::the_org(r.conn()))?
        .ok_or(Error::NotFound("organization"))?;
    let currency = org_row.currency()?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    store.write(meta, |w| {
        let product = catalog::get(w.tx(), product_id)?;
        if catalog::kind(&product) == nb_domain::product::ProductKind::Service {
            return Err(RuleError::on("product", "product.service_stock").into());
        }
        let qty = product.unit_set()?.to_base(typed, &input.unit)?;
        let warehouse = org::warehouse(w.tx(), warehouse_id)?;
        let branch = org::branch(w.tx(), parse_id(&warehouse.branch_id)?)?;
        let target = Target::warehouse(parse_id(&branch.id)?, warehouse_id);
        let approval = input.approval.as_deref().map(parse_id).transpose()?;
        w.require_approved(Permission::StockAdjust, &target, approval)?;
        let may_go_negative =
            input.may_go_negative && w.allowed(Permission::StockNegative, &target)?;
        let before = stock::balance(w.tx(), product_id, warehouse_id, currency)?;
        let cost = Money::from_minor(
            input.unit_cost_minor.unwrap_or(before.avg_cost.minor()),
            currency,
        );
        // Adjustments in carry a cost but do not change the average unless priced.
        let after = before.apply(kind, qty, None, warehouse.policy(), may_go_negative)?;
        let number = w.next_number(DocKind::Adjustment, &branch.id, &branch.code)?;
        let movement = stock::append(
            w,
            product_id,
            warehouse_id,
            kind,
            qty,
            Some(cost),
            Some(&number),
            Some(input.reason.trim()),
            after,
        )?;
        crate::usecases::tracked::record(
            w,
            movement,
            product_id,
            warehouse_id,
            kind,
            qty,
            &input.tracked,
            true,
        )?;
        let value = qty.times_price(cost)?;
        if !value.is_zero() {
            let (debit, credit) = accounts(kind).ok_or(RuleError::new("invalid_value"))?;
            let entry = Entry::new(vec![Line::debit(debit, value), Line::credit(credit, value)])?;
            let org = w.org()?;
            let jnumber = w.next_number(DocKind::Journal, &branch.id, &branch.code)?;
            let memo = format!("{number} {} — {}", product.name, input.reason.trim());
            let date = iso_date(w.now(), offset);
            ledger::post(
                w,
                org,
                ledger::Header {
                    number: &jnumber,
                    date: &date,
                    memo: &memo,
                    source: "stock.adjust",
                    reverses: None,
                },
                &entry,
            )?;
        }
        let result = AdjustResult {
            number: number.clone(),
            on_hand_milli: crate::repo::to_milli(after.on_hand)?,
            value_minor: value.minor(),
        };
        w.event("stock.moved", "product", &product.id, &serde_json::json!({
            "kind": kind.code(), "warehouse": warehouse.id, "qty": qty.to_string(), "number": number
        }))?;
        w.audit(kind.code(), "product", &product.id, &input)?;
        Ok(result)
    })
}

/// Checks the stock projection against the movements; fixes and reports.
pub fn rebuild_balances(store: &Store) -> Result<Vec<(String, String)>> {
    store.read(|r| {
        let org = org::the_org(r.conn())?.ok_or(Error::NotFound("organization"))?;
        stock::rebuild_balances(r.conn(), org.currency()?)
    })
}

//! What running another device's operations left to look at (M11/M2):
//! goods sold on two devices that were not there (`stock_shortfall`), the
//! same goods taken back twice (`return_excess`), cash paid from a drawer
//! that held less once both devices' payments met (`drawer_short`).
//! Nothing was refused (it
//! happened at a till); someone checks the shelf or the drawer and marks
//! it seen. Each copy finds its own; marking one seen marks it everywhere.

use nb_domain::RuleError;
use nb_domain::access::{Permission, Target};
use serde::{Deserialize, Serialize};

use crate::uow::OpMeta;
use crate::{Result, Store};

/// An item to look at, with what it is about in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub kind: String,
    pub entity: String,
    pub entity_id: String,
    /// The product's name or the sale's number.
    pub subject: String,
    /// Stock now (thousandths) for a shortfall; the quantity taken back
    /// beyond the sale for a return.
    pub qty_milli: i64,
    /// The operation that found it (with `kind` and `entity_id`, the same
    /// item on every copy).
    pub operation_id: String,
    pub created_at: i64,
}

pub fn review_items(store: &Store) -> Result<Vec<ReviewItem>> {
    store.read(|r| {
        let mut st = r.conn().prepare(
            "SELECT v.kind, v.entity, v.entity_id, v.detail, COALESCE(v.operation_id, ''), v.created_at,
                    COALESCE(p.name, s.number, cr.name, v.entity_id)
             FROM reviews v
             LEFT JOIN products p ON v.entity = 'product' AND p.id = v.entity_id
             LEFT JOIN sales s ON v.entity = 'sale' AND s.id = v.entity_id
             LEFT JOIN shifts sh ON v.entity = 'shift' AND sh.id = v.entity_id
             LEFT JOIN cash_registers cr ON cr.id = sh.register_id
             WHERE v.resolved_at IS NULL
             ORDER BY v.created_at DESC, v.rowid DESC LIMIT 200",
        )?;
        let rows = st.query_map([], |row| {
            let detail: String = row.get(3)?;
            let detail: serde_json::Value =
                serde_json::from_str(&detail).unwrap_or(serde_json::Value::Null);
            let kind: String = row.get(0)?;
            let qty_milli = match kind.as_str() {
                "stock_shortfall" => detail["on_hand_milli"].as_i64().unwrap_or(0),
                "return_excess" => {
                    detail["qty_milli"].as_i64().unwrap_or(0)
                        - detail["left_milli"].as_i64().unwrap_or(0).max(0)
                }
                _ => 0,
            };
            Ok(ReviewItem {
                kind,
                entity: row.get(1)?,
                entity_id: row.get(2)?,
                subject: row.get(6)?,
                qty_milli,
                operation_id: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

/// Which item was looked at: its kind and what it is about. Each copy
/// notices a shortfall at the operation that reached it last, so the same
/// shelf may carry different operations on different copies; all of them
/// are the same item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewKey {
    pub kind: String,
    pub entity_id: String,
}

/// Marks an item seen, on this copy and (as an operation) on the others.
pub fn resolve_review(store: &mut Store, meta: OpMeta, key: ReviewKey) -> Result<()> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::ReviewsResolveReview { key: key.clone() },
    )?;
    if key.kind.trim().is_empty() || key.entity_id.trim().is_empty() {
        return Err(RuleError::on("kind", "required").into());
    }
    store.write(meta, |w| {
        w.require(Permission::AuditView, &Target::org())?;
        let at = w.now();
        let by = meta.actor.map(|a| a.to_string());
        w.tx().execute(
            "UPDATE reviews SET resolved_at = ?3, resolved_by = ?4
             WHERE kind = ?1 AND entity_id = ?2 AND resolved_at IS NULL",
            rusqlite::params![key.kind, key.entity_id, at, by],
        )?;
        w.event("review.resolved", "review", &key.entity_id, &key)?;
        Ok(())
    })
}

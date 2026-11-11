//! Debts and their collection (M9.4, M9.5): the receipt that settles
//! invoices on account (one, several, or none yet), what is still open on
//! each invoice, the customer's statement, and the ageing of what is owed.
//!
//! An invoice's open amount is what it left on account, less what receipts
//! gave it and what returns refunded on account. A receipt is posted
//! (cash or bank against the receivable, tagged with the customer) in the
//! same transaction as its allocations, its event and its audit, once per
//! operation.

use nb_core::Money;
use nb_domain::access::{Permission, Target};
use nb_domain::numbering::DocKind;
use nb_domain::posting::Operation;
use nb_domain::{RegisterId, RuleError};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::repo::{org, parse_id};
use crate::uow::{OpMeta, Writer};
use crate::usecases::accounting::post_operations_for;
use crate::usecases::sales::{the_org, till};
use crate::{Error, Result, Store};

/// An invoice that still owes something.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenInvoice {
    pub sale_id: String,
    pub number: String,
    pub created_at: i64,
    /// What it left on account.
    pub credit_minor: i64,
    /// What is still owed on it.
    pub open_minor: i64,
    /// Days since the invoice (at the time asked).
    pub age_days: i64,
    /// Past the customer's days to pay.
    pub overdue: bool,
}

const DAY_MS: i64 = 86_400_000;

/// The customer's invoices still owing something, the oldest first.
pub(crate) fn open_invoices_of(
    conn: &Connection,
    customer: &str,
    now: i64,
) -> Result<Vec<OpenInvoice>> {
    let days: i64 = conn
        .query_row(
            "SELECT payment_days FROM customers WHERE id = ?1",
            [customer],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::Rules(vec![RuleError::on(
            "customer",
            "customer.unknown",
        )]))?;
    let mut s = conn.prepare(
        "SELECT s.id, s.number, s.created_at, s.credit_minor,
                s.credit_minor
                  - COALESCE((SELECT SUM(a.amount_minor) FROM payment_allocations a WHERE a.sale_id = s.id), 0)
                  - COALESCE((SELECT SUM(x.total_minor) FROM sale_returns x
                              WHERE x.sale_id = s.id AND x.refund_method = 'credit'), 0)
         FROM sales s WHERE s.customer_id = ?1 AND s.credit_minor > 0
         ORDER BY s.created_at, s.number",
    )?;
    let rows = s
        .query_map([customer], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|r| r.4 > 0)
        .map(|(sale_id, number, created_at, credit, open)| {
            let age_days = ((now - created_at).max(0)) / DAY_MS;
            OpenInvoice {
                sale_id,
                number,
                created_at,
                credit_minor: credit,
                open_minor: open,
                age_days,
                overdue: age_days > days,
            }
        })
        .collect())
}

pub fn open_invoices(store: &Store, customer: &str) -> Result<Vec<OpenInvoice>> {
    let now = store.clock.now_ms();
    store.read(|r| open_invoices_of(r.conn(), customer, now))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationInput {
    pub sale_id: String,
    pub amount_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReceiptInput {
    pub customer_id: String,
    pub amount_minor: i64,
    /// `cash` (into the open till of `register_id`), `card` or `transfer`.
    pub method: String,
    #[serde(default)]
    pub register_id: Option<String>,
    /// Which invoices it settles; empty: the oldest first.
    #[serde(default)]
    pub allocations: Vec<AllocationInput>,
    /// Keep it all as an advance, settling nothing now.
    #[serde(default)]
    pub unallocated: bool,
    #[serde(default)]
    pub reference: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptResult {
    pub id: String,
    pub number: String,
    pub amount_minor: i64,
    /// (invoice number, amount) settled.
    pub settled: Vec<(String, i64)>,
    pub unallocated_minor: i64,
    /// What the customer owes after it.
    pub balance_minor: i64,
}

/// Records money received from a customer: the receipt, how it settles its
/// invoices, the entry (cash or bank against the receivable) — at once and
/// once per operation.
pub fn collect(store: &mut Store, meta: OpMeta, input: ReceiptInput) -> Result<ReceiptResult> {
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.read(|r| Ok(r.utc_offset_min))?;
    store.write(meta, |w| collect_in(w, &input, currency, offset))
}

/// [`collect`] inside a transaction already open (an instalment payment).
pub(crate) fn collect_in(
    w: &mut Writer<'_>,
    input: &ReceiptInput,
    currency: nb_core::Currency,
    offset: i32,
) -> Result<ReceiptResult> {
    if input.amount_minor <= 0 {
        return Err(RuleError::on("amount", "receipt.amount_positive").into());
    }
    let to_bank = match input.method.as_str() {
        "cash" => false,
        "card" | "transfer" => true,
        _ => return Err(RuleError::on("method", "invalid_value").into()),
    };
    {
        let customer = crate::usecases::customers::load(w.tx(), &input.customer_id)?;
        // Cash goes into an open till; bank money is the branch's.
        let (branch, till_account, register, shift) =
            match input.register_id.as_deref().filter(|r| !r.is_empty()) {
                Some(reg) => {
                    let register: RegisterId = parse_id(reg)?;
                    let (branch, _, account) = till(w, register)?;
                    let shift: Option<String> = w
                        .tx()
                        .query_row(
                            "SELECT id FROM shifts WHERE register_id = ?1 AND status = 'open'",
                            [reg],
                            |r| r.get(0),
                        )
                        .optional()?;
                    (branch, Some(account), Some(reg.to_owned()), shift)
                }
                None => {
                    let org_id = w.org()?;
                    let branch = org::branches(w.tx(), org_id)?
                        .into_iter()
                        .next()
                        .ok_or(Error::NotFound("branch"))?;
                    (branch, None, None, None)
                }
            };
        if !to_bank && shift.is_none() {
            return Err(RuleError::on("register", "shift.none_open").into());
        }
        let target = match register.as_deref() {
            Some(r) => Target::register(parse_id(&branch.id)?, parse_id(r)?),
            None => Target {
                branch: Some(parse_id(&branch.id)?),
                ..Target::org()
            },
        };
        w.require(Permission::DebtsCollect, &target)?;

        // How it settles the invoices.
        let open = open_invoices_of(w.tx(), &customer.id, w.now())?;
        let mut settled: Vec<(String, String, i64)> = Vec::new();
        let mut left = input.amount_minor;
        if !input.allocations.is_empty() {
            for a in &input.allocations {
                if a.amount_minor <= 0 {
                    return Err(RuleError::on("allocations", "receipt.amount_positive").into());
                }
                let Some(inv) = open.iter().find(|o| o.sale_id == a.sale_id) else {
                    return Err(RuleError::on("allocations", "receipt.not_open").into());
                };
                if settled.iter().any(|s| s.0 == a.sale_id) {
                    return Err(RuleError::on("allocations", "receipt.twice").into());
                }
                if a.amount_minor > inv.open_minor {
                    return Err(RuleError::on("allocations", "receipt.more_than_open").into());
                }
                if a.amount_minor > left {
                    return Err(RuleError::on("allocations", "receipt.more_than_paid").into());
                }
                left -= a.amount_minor;
                settled.push((inv.sale_id.clone(), inv.number.clone(), a.amount_minor));
            }
        } else if !input.unallocated {
            for inv in &open {
                if left == 0 {
                    break;
                }
                let take = inv.open_minor.min(left);
                left -= take;
                settled.push((inv.sale_id.clone(), inv.number.clone(), take));
            }
        }

        let number = w.next_number(DocKind::Receipt, &branch.id, &branch.code)?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        w.tx().execute(
            "INSERT INTO receipts (id, org_id, branch_id, customer_id, number, method, register_id, shift_id,
                    amount_minor, unallocated_minor, reference, note, operation_id, created_at, created_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                id,
                w.org()?.to_string(),
                branch.id,
                customer.id,
                number,
                input.method,
                if to_bank { None } else { register.clone() },
                if to_bank { None } else { shift.clone() },
                input.amount_minor,
                left,
                input.reference.trim(),
                input.note.trim(),
                w.meta().id.to_string(),
                w.now(),
                w.actor_text(),
                w.device().id.to_string()
            ],
        )?;
        for (sale, _, amount) in &settled {
            w.tx().execute(
                "INSERT INTO payment_allocations (receipt_id, sale_id, amount_minor) VALUES (?1, ?2, ?3)",
                params![id, sale, amount],
            )?;
            // An invoice paid in instalments: its instalments, oldest first.
            crate::usecases::installments::settle_plan(w, &id, sale, *amount)?;
        }
        post_operations_for(
            w,
            &branch,
            offset,
            &[Operation::Collection {
                amount: Money::from_minor(input.amount_minor, currency),
                to_bank,
            }],
            &format!("{number} — {}", customer.name),
            "collection",
            if to_bank {
                None
            } else {
                till_account.as_deref()
            },
            Some(&customer.id),
        )?;
        let balance: i64 = w.tx().query_row(
            "SELECT balance_minor FROM customers WHERE id = ?1",
            [&customer.id],
            |r| r.get(0),
        )?;
        let out = ReceiptResult {
            id: id.clone(),
            number,
            amount_minor: input.amount_minor,
            settled: settled.into_iter().map(|(_, n, a)| (n, a)).collect(),
            unallocated_minor: left,
            balance_minor: balance,
        };
        w.event("receipt.recorded", "receipt", &id, &out)?;
        w.audit("collect", "customer", &customer.id, input)?;
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Statement (M9.5)

/// A line of a customer's statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementLine {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub entry_number: String,
    /// What produced it (`sale`, `sale.return`, `collection`, `reversal`…).
    pub source: String,
    pub memo: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    /// Owed after this line.
    pub balance_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CustomerStatement {
    /// Owed before `from`.
    pub opening_minor: i64,
    pub lines: Vec<StatementLine>,
    pub debit_minor: i64,
    pub credit_minor: i64,
    /// Owed at the end.
    pub closing_minor: i64,
}

/// What moved a customer's debt between two dates (`YYYY-MM-DD`, either
/// open), from its receivable lines.
pub fn customer_statement(
    store: &Store,
    customer: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<CustomerStatement> {
    store.read(|r| {
        crate::usecases::customers::load(r.conn(), customer)?;
        let opening: i64 = match from {
            Some(f) => r.conn().query_row(
                "SELECT COALESCE(SUM(l.debit_minor - l.credit_minor), 0)
                 FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
                 WHERE l.party_id = ?1 AND e.date < ?2",
                params![customer, f],
                |r| r.get(0),
            )?,
            None => 0,
        };
        let mut s = r.conn().prepare(
            "SELECT e.date, e.number, e.source, e.memo, l.debit_minor, l.credit_minor
             FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
             WHERE l.party_id = ?1 AND (?2 IS NULL OR e.date >= ?2) AND (?3 IS NULL OR e.date <= ?3)
             ORDER BY e.date, e.created_at, e.number, l.position",
        )?;
        let mut balance = opening;
        let (mut debit, mut credit) = (0, 0);
        let mut lines = Vec::new();
        for row in s.query_map(params![customer, from, to], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })? {
            let (date, entry_number, source, memo, d, c) = row?;
            balance += d - c;
            debit += d;
            credit += c;
            lines.push(StatementLine {
                date,
                entry_number,
                source,
                memo,
                debit_minor: d,
                credit_minor: c,
                balance_minor: balance,
            });
        }
        Ok(CustomerStatement {
            opening_minor: opening,
            lines,
            debit_minor: debit,
            credit_minor: credit,
            closing_minor: balance,
        })
    })
}

// ---------------------------------------------------------------------------
// Ageing (M9.5)

/// What a customer owes by the age of its invoices.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgeingRow {
    pub customer_id: String,
    pub code: String,
    pub name: String,
    pub phone: String,
    /// 0–30, 31–60, 61–90 and over 90 days.
    pub buckets: [i64; 4],
    /// Receipts not yet given to an invoice (an advance).
    pub unallocated_minor: i64,
    /// Owed on invoices past the days to pay.
    pub overdue_minor: i64,
    /// What it owes in all (its balance).
    pub balance_minor: i64,
    pub oldest_days: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Ageing {
    pub rows: Vec<AgeingRow>,
    pub totals: [i64; 4],
    pub overdue_minor: i64,
    pub balance_minor: i64,
}

fn bucket(days: i64) -> usize {
    match days {
        ..=30 => 0,
        31..=60 => 1,
        61..=90 => 2,
        _ => 3,
    }
}

/// Every customer that owes something, its open invoices by age, the most
/// overdue first.
pub fn ageing(store: &Store) -> Result<Ageing> {
    let now = store.clock.now_ms();
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT id, code, name, phone, balance_minor,
                    COALESCE((SELECT SUM(unallocated_minor) FROM receipts x WHERE x.customer_id = c.id), 0)
             FROM customers c WHERE org_id = ?1 AND balance_minor <> 0",
        )?;
        let customers = s
            .query_map([org.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = Ageing::default();
        for (id, code, name, phone, balance, unallocated) in customers {
            let mut row = AgeingRow {
                customer_id: id.clone(),
                code,
                name,
                phone,
                balance_minor: balance,
                unallocated_minor: unallocated,
                ..Default::default()
            };
            for inv in open_invoices_of(r.conn(), &id, now)? {
                row.buckets[bucket(inv.age_days)] += inv.open_minor;
                if inv.overdue {
                    row.overdue_minor += inv.open_minor;
                }
                row.oldest_days = row.oldest_days.max(inv.age_days);
            }
            for (t, b) in out.totals.iter_mut().zip(row.buckets) {
                *t += b;
            }
            out.overdue_minor += row.overdue_minor;
            out.balance_minor += row.balance_minor;
            out.rows.push(row);
        }
        out.rows.sort_by(|a, b| {
            b.overdue_minor
                .cmp(&a.overdue_minor)
                .then(b.balance_minor.cmp(&a.balance_minor))
        });
        Ok(out)
    })
}

/// A receipt in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptRow {
    pub id: String,
    pub number: String,
    pub created_at: i64,
    pub method: String,
    pub amount_minor: i64,
    pub unallocated_minor: i64,
    pub reference: String,
}

/// A customer's receipts, the latest first.
pub fn receipts_of(store: &Store, customer: &str) -> Result<Vec<ReceiptRow>> {
    store.read(|r| {
        let mut s = r.conn().prepare(
            "SELECT id, number, created_at, method, amount_minor, unallocated_minor, reference
             FROM receipts WHERE customer_id = ?1 ORDER BY created_at DESC, number DESC",
        )?;
        let rows = s
            .query_map([customer], |r| {
                Ok(ReceiptRow {
                    id: r.get(0)?,
                    number: r.get(1)?,
                    created_at: r.get(2)?,
                    method: r.get(3)?,
                    amount_minor: r.get(4)?,
                    unallocated_minor: r.get(5)?,
                    reference: r.get(6)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    })
}

#[cfg(test)]
mod tests {
    use super::bucket;

    #[test]
    fn ages_fall_in_their_bucket() {
        assert_eq!(
            [0, 30, 31, 60, 61, 90, 91, 400].map(bucket),
            [0, 0, 1, 1, 2, 2, 3, 3]
        );
    }
}

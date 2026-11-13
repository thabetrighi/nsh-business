//! Expenses and other income (M9.11): rent, power, wages… paid from the
//! till or the bank, and income besides sales, each on the account of its
//! category and posted in the same transaction. Cash comes out of (or into)
//! the open till and counts in what it should hold.

use nb_core::Money;
use nb_domain::access::{Permission, Target};
use nb_domain::ledger::{Entry, Line};
use nb_domain::numbering::DocKind;
use nb_domain::posting::Role;
use nb_domain::{RegisterId, RuleError};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::repo::{ledger, org, parse_id};
use crate::uow::OpMeta;
use crate::usecases::sales::{the_org, till};
use crate::{Error, Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpenseCategory {
    pub id: String,
    /// `expense` or `income`.
    pub kind: String,
    pub name: String,
    pub account_code: String,
    pub account_name: String,
}

pub fn expense_categories(store: &Store) -> Result<Vec<ExpenseCategory>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT c.id, c.kind, c.name, c.account_code, COALESCE(a.name, '')
             FROM expense_categories c LEFT JOIN accounts a ON a.org_id = c.org_id AND a.code = c.account_code
             WHERE c.org_id = ?1 AND c.active = 1 ORDER BY c.kind, c.name COLLATE NOCASE",
        )?;
        let rows = s
            .query_map([org.to_string()], |r| {
                Ok(ExpenseCategory {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    name: r.get(2)?,
                    account_code: r.get(3)?,
                    account_name: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    })
}

/// A new category on an account of its kind (a charge for an expense, a
/// product for an income) that takes entries.
pub fn create_category(
    store: &mut Store,
    meta: OpMeta,
    kind: &str,
    name: &str,
    account_code: &str,
) -> Result<ExpenseCategory> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    if !matches!(kind, "expense" | "income") {
        return Err(RuleError::on("kind", "invalid_value").into());
    }
    store.write(meta, |w| {
        w.require(Permission::CashExpense, &Target::org())?;
        let org = w.org()?.to_string();
        let account: Option<(String, String, bool)> = w
            .tx()
            .query_row(
                "SELECT name, kind, is_group FROM accounts WHERE org_id = ?1 AND code = ?2 AND active = 1",
                params![org, account_code],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((account_name, account_kind, group)) = account else {
            return Err(RuleError::on("account", "ledger.unknown_account").into());
        };
        if group || account_kind != kind {
            return Err(RuleError::on("account", "expense.wrong_account").into());
        }
        let taken: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM expense_categories WHERE org_id = ?1 AND kind = ?2 AND lower(name) = lower(?3)",
            params![org, kind, name],
            |r| r.get(0),
        )?;
        if taken > 0 {
            return Err(Error::Duplicate("name"));
        }
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        w.tx().execute(
            "INSERT INTO expense_categories (id, org_id, kind, name, account_code, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, org, kind, name, account_code, w.now()],
        )?;
        let c = ExpenseCategory {
            id: id.clone(),
            kind: kind.into(),
            name,
            account_code: account_code.into(),
            account_name,
        };
        w.event("expense_category.created", "expense_category", &id, &c)?;
        w.audit("create", "expense_category", &id, &c)?;
        Ok(c)
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExpenseInput {
    pub category_id: String,
    /// What was paid (or received), tax included.
    pub total_minor: i64,
    /// Of which deductible VAT (expenses only).
    #[serde(default)]
    pub vat_minor: i64,
    /// `cash` (from the open till of `register_id`) or `bank`.
    pub method: String,
    #[serde(default)]
    pub register_id: Option<String>,
    #[serde(default)]
    pub payee: String,
    #[serde(default)]
    pub reference: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpenseRow {
    pub id: String,
    pub number: String,
    pub kind: String,
    pub category: String,
    pub account_code: String,
    pub method: String,
    pub net_minor: i64,
    pub vat_minor: i64,
    pub total_minor: i64,
    pub payee: String,
    pub reference: String,
    pub note: String,
    pub entry_number: Option<String>,
    pub created_at: i64,
}

/// Records an expense or an income: the document, its entry, its event and
/// its audit, once per operation.
pub fn record_expense(store: &mut Store, meta: OpMeta, input: ExpenseInput) -> Result<ExpenseRow> {
    if input.total_minor <= 0 {
        return Err(RuleError::on("amount", "receipt.amount_positive").into());
    }
    if input.vat_minor < 0 || input.vat_minor >= input.total_minor {
        return Err(RuleError::on("vat", "expense.vat").into());
    }
    let from_bank = match input.method.as_str() {
        "cash" => false,
        "bank" => true,
        _ => return Err(RuleError::on("method", "invalid_value").into()),
    };
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let id = store.write(meta, |w| {
        let org_id = w.org()?;
        let (kind, account): (String, String) = w
            .tx()
            .query_row(
                "SELECT kind, account_code FROM expense_categories WHERE id = ?1 AND org_id = ?2 AND active = 1",
                params![input.category_id, org_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::Rules(vec![RuleError::on(
                "category",
                "expense.unknown_category",
            )]))?;
        if kind == "income" && input.vat_minor > 0 {
            return Err(RuleError::on("vat", "expense.vat").into());
        }
        // Cash: the open till of the register; bank: the branch's bank.
        let register = input.register_id.as_deref().filter(|r| !r.is_empty());
        let (branch, till_account, shift) = match register {
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
                (branch, Some(account), shift)
            }
            None => {
                let branch = org::branches(w.tx(), org_id)?
                    .into_iter()
                    .next()
                    .ok_or(Error::NotFound("branch"))?;
                (branch, None, None)
            }
        };
        if !from_bank && shift.is_none() {
            return Err(RuleError::on("register", "shift.none_open").into());
        }
        let target = match register {
            Some(r) => Target::register(parse_id(&branch.id)?, parse_id(r)?),
            None => Target {
                branch: Some(parse_id(&branch.id)?),
                ..Target::org()
            },
        };
        w.require(Permission::CashExpense, &target)?;
        // Cash paid out cannot be more than the drawer holds.
        if let (false, Some(s), "expense") = (from_bank, shift.as_deref(), kind.as_str())
            && input.total_minor > crate::usecases::sales::shift_of(w.tx(), s)?.expected_minor
        {
            return Err(RuleError::on("amount", "shift.not_enough_cash").into());
        }

        let map = ledger::account_map(w.tx(), org_id)?;
        let money = match (&till_account, from_bank) {
            (Some(t), false) => t.clone(),
            (_, true) => map.get(Role::Bank)?.to_owned(),
            (None, false) => map.get(Role::Cash)?.to_owned(),
        };
        let m = |v: i64| Money::from_minor(v, currency);
        let net = input.total_minor - input.vat_minor;
        let lines = if kind == "expense" {
            vec![
                Line::debit(&account, m(net)),
                Line::debit(map.get(Role::VatIn)?, m(input.vat_minor)),
                Line::credit(&money, m(input.total_minor)),
            ]
        } else {
            vec![
                Line::debit(&money, m(input.total_minor)),
                Line::credit(&account, m(input.total_minor)),
            ]
        };
        let entry = Entry::new(lines)?;
        let number = w.next_number(DocKind::Expense, &branch.id, &branch.code)?;
        let journal = w.next_number(DocKind::Journal, &branch.id, &branch.code)?;
        let (y, mo, d) = w.date();
        let date = format!("{y:04}-{mo:02}-{d:02}");
        let memo = if input.payee.trim().is_empty() {
            number.clone()
        } else {
            format!("{number} — {}", input.payee.trim())
        };
        ledger::post(
            w,
            org_id,
            ledger::Header {
                number: &journal,
                date: &date,
                memo: &memo,
                source: if kind == "expense" { "expense" } else { "income" },
                reverses: None,
                party: None,
            },
            &entry,
        )?;
        let id = nb_core::uuid::Uuid::now_v7().to_string();
        w.tx().execute(
            "INSERT INTO expenses (id, org_id, branch_id, category_id, kind, number, method, register_id, shift_id,
                    net_minor, vat_minor, total_minor, payee, reference, note, entry_number, operation_id,
                    created_at, created_by, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                id,
                org_id.to_string(),
                branch.id,
                input.category_id,
                kind,
                number,
                input.method,
                if from_bank { None } else { register },
                if from_bank { None } else { shift.as_deref() },
                net,
                input.vat_minor,
                input.total_minor,
                input.payee.trim(),
                input.reference.trim(),
                input.note.trim(),
                journal,
                w.meta().id.to_string(),
                w.now(),
                w.actor_text(),
                w.device().id.to_string()
            ],
        )?;
        w.event("expense.recorded", "expense", &id, &input)?;
        w.audit("record", "expense", &id, &input)?;
        Ok(id)
    })?;
    store.read(|r| {
        let mut rows = list(r.conn(), "WHERE e.id = ?1", &[&id])?;
        rows.pop().ok_or(Error::NotFound("expense"))
    })
}

const SELECT: &str =
    "SELECT e.id, e.number, e.kind, c.name, c.account_code, e.method, e.net_minor, e.vat_minor,
        e.total_minor, e.payee, e.reference, e.note, e.entry_number, e.created_at
     FROM expenses e JOIN expense_categories c ON c.id = e.category_id ";

fn list(
    conn: &rusqlite::Connection,
    filter: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<ExpenseRow>> {
    let mut s = conn.prepare(&format!("{SELECT}{filter}"))?;
    let rows = s
        .query_map(args, |r| {
            Ok(ExpenseRow {
                id: r.get(0)?,
                number: r.get(1)?,
                kind: r.get(2)?,
                category: r.get(3)?,
                account_code: r.get(4)?,
                method: r.get(5)?,
                net_minor: r.get(6)?,
                vat_minor: r.get(7)?,
                total_minor: r.get(8)?,
                payee: r.get(9)?,
                reference: r.get(10)?,
                note: r.get(11)?,
                entry_number: r.get(12)?,
                created_at: r.get(13)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExpenseQuery {
    /// ``, `expense` or `income`.
    #[serde(default)]
    pub kind: String,
    /// Times in ms (from included, to excluded); 0: open.
    #[serde(default)]
    pub from_ms: i64,
    #[serde(default)]
    pub to_ms: i64,
    #[serde(default)]
    pub search: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExpensePage {
    pub rows: Vec<ExpenseRow>,
    pub total: u32,
    pub expenses_minor: i64,
    pub income_minor: i64,
    /// (category, kind, total) over the filter.
    pub by_category: Vec<(String, String, i64)>,
}

pub fn list_expenses(store: &Store, q: &ExpenseQuery) -> Result<ExpensePage> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let to = if q.to_ms > 0 { q.to_ms } else { i64::MAX };
        let search = q.search.trim().to_lowercase();
        let filter = "WHERE e.org_id = ?1 AND (?2 = '' OR e.kind = ?2) AND e.created_at >= ?3 AND e.created_at < ?4
               AND (?5 = '' OR lower(e.payee) LIKE '%' || ?5 || '%' OR lower(c.name) LIKE '%' || ?5 || '%'
                    OR lower(e.number) LIKE '%' || ?5 || '%' OR lower(e.note) LIKE '%' || ?5 || '%')";
        let o = org.to_string();
        let base: [&dyn rusqlite::ToSql; 5] = [&o, &q.kind, &q.from_ms, &to, &search];
        let (total, expenses, income): (u32, i64, i64) = r.conn().query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(CASE WHEN e.kind = 'expense' THEN e.total_minor END), 0),
                        COALESCE(SUM(CASE WHEN e.kind = 'income' THEN e.total_minor END), 0)
                 FROM expenses e JOIN expense_categories c ON c.id = e.category_id {filter}"
            ),
            &base[..],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut s = r.conn().prepare(&format!(
            "SELECT c.name, c.kind, SUM(e.total_minor) FROM expenses e JOIN expense_categories c ON c.id = e.category_id
             {filter} GROUP BY c.id ORDER BY SUM(e.total_minor) DESC"
        ))?;
        let by_category = s
            .query_map(&base[..], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let limit = q.limit.clamp(1, 100_000);
        let args: [&dyn rusqlite::ToSql; 7] =
            [&o, &q.kind, &q.from_ms, &to, &search, &limit, &q.offset];
        let rows = list(
            r.conn(),
            &format!("{filter} ORDER BY e.created_at DESC, e.number DESC LIMIT ?6 OFFSET ?7"),
            &args,
        )?;
        Ok(ExpensePage {
            rows,
            total,
            expenses_minor: expenses,
            income_minor: income,
            by_category,
        })
    })
}

//! Basic accounting (M7): the chart of accounts, the accounts each role of
//! the entry matrix uses, tills and bank accounts, fiscal periods, and the
//! reports that read the journal (journal, trial balance, statement).
//! Automatic entries of other use cases go through [`post_operations`].

use std::collections::{BTreeMap, HashMap};

use nb_core::OrganizationId;
use nb_domain::RuleError;
use nb_domain::access::{Permission, Target};
use nb_domain::fiscal::{Date, may_close, may_reopen};
use nb_domain::numbering::DocKind;
use nb_domain::posting::{self, Operation, Role};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::repo::fiscal::{self, YearRow};
use crate::repo::ledger::{self, Account};
use crate::repo::org::{self, Branch};
use crate::time::iso_date;
use crate::uow::{OpMeta, Writer};
use crate::{Error, Result, Store};

fn the_org(conn: &Connection) -> Result<OrganizationId> {
    org::the_org(conn)?
        .ok_or(Error::NotFound("organization"))?
        .org_id()
}

// ---------------------------------------------------------------------------
// Automatic entries (M7.5)

/// Posts the entry of `ops` (one document) with the organization's account
/// map; returns its number, or `None` when the operations move nothing.
pub(crate) fn post_operations(
    w: &mut Writer<'_>,
    branch: &Branch,
    offset: i32,
    ops: &[Operation],
    memo: &str,
    source: &str,
) -> Result<Option<String>> {
    post_operations_at(w, branch, offset, ops, memo, source, None)
}

/// Like [`post_operations`], with the money of the till in `till` (a
/// till's own account) instead of the cash role's.
pub(crate) fn post_operations_at(
    w: &mut Writer<'_>,
    branch: &Branch,
    offset: i32,
    ops: &[Operation],
    memo: &str,
    source: &str,
    till: Option<&str>,
) -> Result<Option<String>> {
    post_operations_for(w, branch, offset, ops, memo, source, till, None)
}

/// Like [`post_operations_at`], the receivable lines being `party`'s debt.
#[allow(clippy::too_many_arguments)]
pub(crate) fn post_operations_for(
    w: &mut Writer<'_>,
    branch: &Branch,
    offset: i32,
    ops: &[Operation],
    memo: &str,
    source: &str,
    till: Option<&str>,
    party: Option<&str>,
) -> Result<Option<String>> {
    let org = w.org()?;
    let mut map = ledger::account_map(w.tx(), org)?;
    if let Some(code) = till {
        map.set(Role::Cash, code);
    }
    let Some(entry) = posting::combined(ops, &map)? else {
        return Ok(None);
    };
    let number = w.next_number(DocKind::Journal, &branch.id, &branch.code)?;
    let date = iso_date(w.now(), offset);
    ledger::post(
        w,
        org,
        ledger::Header {
            number: &number,
            date: &date,
            memo,
            source,
            reverses: None,
            party,
        },
        &entry,
    )?;
    Ok(Some(number))
}

// ---------------------------------------------------------------------------
// Chart of accounts (M7.1)

/// An account of the chart with its totals (its sub-accounts' for a group).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartRow {
    pub code: String,
    pub name: String,
    pub parent_code: Option<String>,
    pub kind: String,
    pub is_group: bool,
    pub active: bool,
    /// 0 for a class.
    pub depth: u32,
    pub debit_minor: i64,
    pub credit_minor: i64,
    /// Roles of the entry matrix it serves.
    pub roles: Vec<String>,
}

impl ChartRow {
    /// Debit balance (negative: credit balance).
    pub fn balance_minor(&self) -> i64 {
        self.debit_minor - self.credit_minor
    }
}

/// The accounts in tree order (each group before its accounts), with depth.
fn tree(accounts: &[Account]) -> Vec<(&Account, u32)> {
    let codes: std::collections::HashSet<&str> = accounts.iter().map(|a| a.code.as_str()).collect();
    let mut children: BTreeMap<Option<&str>, Vec<&Account>> = BTreeMap::new();
    for a in accounts {
        let parent = a
            .parent_code
            .as_deref()
            .filter(|p| codes.contains(p) && *p != a.code);
        children.entry(parent).or_default().push(a);
    }
    let mut out = Vec::with_capacity(accounts.len());
    let mut stack: Vec<(&Account, u32)> = children
        .get(&None)
        .map(|v| v.iter().rev().map(|a| (*a, 0)).collect())
        .unwrap_or_default();
    while let Some((a, depth)) = stack.pop() {
        out.push((a, depth));
        if out.len() > accounts.len() {
            break; // a cycle in a hand-edited chart
        }
        if let Some(kids) = children.get(&Some(a.code.as_str())) {
            stack.extend(kids.iter().rev().map(|k| (*k, depth + 1)));
        }
    }
    out
}

/// Totals per account code for entries dated in `[from, to]` (or before
/// `from` with `before`), rolled up to the groups.
fn totals(
    conn: &Connection,
    org: OrganizationId,
    accounts: &[Account],
    range: (&str, &str),
) -> Result<HashMap<String, (i64, i64)>> {
    let mut s = conn.prepare(
        "SELECT l.account_code, SUM(l.debit_minor), SUM(l.credit_minor)
         FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
         WHERE e.org_id = ?1 AND e.date >= ?2 AND e.date <= ?3
         GROUP BY l.account_code",
    )?;
    let rows = s.query_map(params![org.to_string(), range.0, range.1], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let parents: HashMap<&str, &str> = accounts
        .iter()
        .filter_map(|a| Some((a.code.as_str(), a.parent_code.as_deref()?)))
        .collect();
    let mut out: HashMap<String, (i64, i64)> = HashMap::new();
    for row in rows {
        let (code, d, c) = row?;
        let mut at = Some(code.as_str());
        let mut hops = 0;
        while let Some(code) = at {
            let t = out.entry(code.to_owned()).or_default();
            t.0 += d;
            t.1 += c;
            at = parents.get(code).copied();
            hops += 1;
            if hops > 16 {
                break;
            }
        }
    }
    Ok(out)
}

const ALL_TIME: (&str, &str) = ("0000-01-01", "9999-12-31");

pub fn chart(store: &Store) -> Result<Vec<ChartRow>> {
    store.read(|r| {
        let org = the_org(r.conn())?;
        let accounts = ledger::accounts(r.conn(), org)?;
        let sums = totals(r.conn(), org, &accounts, ALL_TIME)?;
        let map = ledger::account_map(r.conn(), org)?;
        Ok(tree(&accounts)
            .into_iter()
            .map(|(a, depth)| {
                let (d, c) = sums.get(&a.code).copied().unwrap_or_default();
                ChartRow {
                    code: a.code.clone(),
                    name: a.name.clone(),
                    parent_code: a.parent_code.clone(),
                    kind: a.kind.clone(),
                    is_group: a.is_group,
                    active: a.active,
                    depth,
                    debit_minor: d,
                    credit_minor: c,
                    roles: map
                        .iter()
                        .filter(|(_, code)| *code == a.code)
                        .map(|(r, _)| r.code().to_owned())
                        .collect(),
                }
            })
            .collect())
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AccountInput {
    pub code: String,
    pub name: String,
    pub parent_code: Option<String>,
    /// `asset`, `liability`, `equity`, `income` or `expense`; default: the
    /// parent's.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub is_group: bool,
}

const KINDS: [&str; 5] = ["asset", "liability", "equity", "income", "expense"];

/// Adds an account (or a group) to the chart, under a group whose code
/// starts its own (SCF numbering).
pub fn create_account(store: &mut Store, meta: OpMeta, input: AccountInput) -> Result<Account> {
    let code = input.code.trim().to_owned();
    let mut errors = Vec::new();
    if code.is_empty() || code.len() > 10 || !code.bytes().all(|b| b.is_ascii_digit()) {
        errors.push(RuleError::on("code", "account.code_format"));
    }
    if input.name.trim().is_empty() {
        errors.push(RuleError::on("name", "required"));
    }
    if !errors.is_empty() {
        return Err(Error::Rules(errors));
    }
    store.write(meta, |w| {
        w.require(Permission::AccountingChart, &Target::org())?;
        let org = w.org()?;
        if ledger::account(w.tx(), org, &code)?.is_some() {
            return Err(RuleError::on("code", "account.duplicate").into());
        }
        let parent = match input
            .parent_code
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            None => None,
            Some(p) => {
                let parent = ledger::account(w.tx(), org, p)?
                    .ok_or(Error::from(RuleError::on("parent_code", "account.unknown")))?;
                if !parent.is_group {
                    return Err(RuleError::on("parent_code", "account.parent_not_group").into());
                }
                if !code.starts_with(&parent.code) || code == parent.code {
                    return Err(RuleError::on("code", "account.code_prefix").into());
                }
                Some(parent)
            }
        };
        let kind = match (input.kind.as_deref(), &parent) {
            (Some(k), _) if KINDS.contains(&k) => k.to_owned(),
            (None, Some(p)) => p.kind.clone(),
            _ => return Err(RuleError::on("kind", "invalid_value").into()),
        };
        let account = Account {
            code: code.clone(),
            name: input.name.trim().to_owned(),
            parent_code: parent.map(|p| p.code),
            kind,
            is_group: input.is_group,
            active: true,
        };
        ledger::insert_account(w, org, &account)?;
        w.event("account.created", "account", &code, &account)?;
        w.audit("create", "account", &code, &account)?;
        Ok(account)
    })
}

/// Renames an account, or (de)activates it: an account in use by a role or
/// a till, or with a balance, stays active.
pub fn update_account(
    store: &mut Store,
    meta: OpMeta,
    code: &str,
    name: &str,
    active: bool,
) -> Result<Account> {
    if name.trim().is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    let rows = chart(store)?;
    store.write(meta, |w| {
        w.require(Permission::AccountingChart, &Target::org())?;
        let org = w.org()?;
        let before = ledger::account(w.tx(), org, code)?.ok_or(Error::NotFound("account"))?;
        if before.active && !active {
            let row = rows.iter().find(|r| r.code == code);
            if row.is_some_and(|r| !r.roles.is_empty()) {
                return Err(RuleError::on("active", "account.role_in_use").into());
            }
            if row.is_some_and(|r| r.balance_minor() != 0) {
                return Err(RuleError::on("active", "account.balance_not_zero").into());
            }
            let tills: i64 = w.tx().query_row(
                "SELECT COUNT(*) FROM money_accounts WHERE org_id = ?1 AND account_code = ?2 AND active = 1",
                params![org.to_string(), code],
                |r| r.get(0),
            )?;
            let kids: i64 = w.tx().query_row(
                "SELECT COUNT(*) FROM accounts WHERE org_id = ?1 AND parent_code = ?2 AND active = 1",
                params![org.to_string(), code],
                |r| r.get(0),
            )?;
            if tills > 0 {
                return Err(RuleError::on("active", "account.till_in_use").into());
            }
            if kids > 0 {
                return Err(RuleError::on("active", "account.has_active_children").into());
            }
        }
        w.tx().execute(
            "UPDATE accounts SET name = ?3, active = ?4 WHERE org_id = ?1 AND code = ?2",
            params![org.to_string(), code, name.trim(), active],
        )?;
        let after = Account {
            name: name.trim().to_owned(),
            active,
            ..before.clone()
        };
        w.event("account.updated", "account", code, &after)?;
        w.audit(
            "update",
            "account",
            code,
            &serde_json::json!({ "before": before, "after": after }),
        )?;
        Ok(after)
    })
}

/// A role of the entry matrix and its account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRow {
    pub role: String,
    pub account_code: Option<String>,
    pub account_name: Option<String>,
}

pub fn account_roles(store: &Store) -> Result<Vec<RoleRow>> {
    store.read(|r| {
        let org = the_org(r.conn())?;
        let map = ledger::account_map(r.conn(), org)?;
        Role::ALL
            .iter()
            .map(|role| {
                let code = map.get(*role).ok().map(str::to_owned);
                let name = match &code {
                    Some(c) => ledger::account(r.conn(), org, c)?.map(|a| a.name),
                    None => None,
                };
                Ok(RoleRow {
                    role: role.code().to_owned(),
                    account_code: code,
                    account_name: name,
                })
            })
            .collect()
    })
}

/// Points a role of the entry matrix at another account (active, not a
/// group). Entries already posted keep their accounts.
pub fn set_account_role(store: &mut Store, meta: OpMeta, role: &str, code: &str) -> Result<()> {
    let role = Role::from_code(role).ok_or(RuleError::on("role", "invalid_value"))?;
    store.write(meta, |w| {
        w.require(Permission::AccountingChart, &Target::org())?;
        let org = w.org()?;
        match ledger::account(w.tx(), org, code.trim())? {
            None => return Err(RuleError::on("account", "account.unknown").into()),
            Some(a) if a.is_group => {
                return Err(RuleError::on("account", "ledger.group_account").into());
            }
            Some(a) if !a.active => {
                return Err(RuleError::on("account", "ledger.inactive_account").into());
            }
            Some(_) => {}
        }
        ledger::set_role(w, org, role, code.trim())?;
        let detail = serde_json::json!({ "role": role.code(), "account": code.trim() });
        w.event(
            "account.role_set",
            "organization",
            &org.to_string(),
            &detail,
        )?;
        w.audit("set_role", "organization", &org.to_string(), &detail)?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Tills and bank accounts (M7.7)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoneyAccountRow {
    pub id: String,
    /// `cash` or `bank`.
    pub kind: String,
    pub name: String,
    pub account_code: String,
    pub branch_id: Option<String>,
    pub active: bool,
    pub balance_minor: i64,
}

pub(crate) fn insert_money_account(
    w: &Writer<'_>,
    org: OrganizationId,
    branch: Option<&str>,
    kind: &str,
    name: &str,
    code: &str,
) -> Result<String> {
    let id = nb_core::uuid::Uuid::now_v7().to_string();
    w.tx().execute(
        "INSERT INTO money_accounts (id, org_id, branch_id, kind, name, account_code, created_at, updated_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9)",
        params![id, org.to_string(), branch, kind, name, code, w.now(), w.actor_text(), w.device().id.to_string()],
    )?;
    Ok(id)
}

pub fn money_accounts(store: &Store) -> Result<Vec<MoneyAccountRow>> {
    store.read(|r| {
        let org = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT m.id, m.kind, m.name, m.account_code, m.branch_id, m.active,
                    COALESCE((SELECT SUM(l.debit_minor - l.credit_minor) FROM journal_lines l
                              JOIN journal_entries e ON e.id = l.entry_id
                              WHERE e.org_id = m.org_id AND l.account_code = m.account_code), 0)
             FROM money_accounts m WHERE m.org_id = ?1 ORDER BY m.kind DESC, m.account_code",
        )?;
        let rows = s.query_map([org.to_string()], |r| {
            Ok(MoneyAccountRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                name: r.get(2)?,
                account_code: r.get(3)?,
                branch_id: r.get(4)?,
                active: r.get(5)?,
                balance_minor: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    })
}

/// A new till or bank account, with its own account under the chart's
/// group for tills (53) or banks (512).
pub fn create_money_account(
    store: &mut Store,
    meta: OpMeta,
    kind: &str,
    name: &str,
    branch_id: Option<String>,
) -> Result<MoneyAccountRow> {
    let group = match kind {
        "cash" => "53",
        "bank" => "512",
        _ => return Err(RuleError::on("kind", "invalid_value").into()),
    };
    if name.trim().is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    store.write(meta, |w| {
        w.require(Permission::AccountingChart, &Target::org())?;
        let org = w.org()?;
        let parent = ledger::account(w.tx(), org, group)?
            .filter(|a| a.is_group)
            .ok_or(Error::from(RuleError::on("kind", "money.no_group")))?;
        if let Some(b) = &branch_id {
            org::branch(w.tx(), crate::repo::parse_id(b)?)?;
        }
        let mut n = 1;
        let code = loop {
            let c = format!("{group}{n}");
            if ledger::account(w.tx(), org, &c)?.is_none() {
                break c;
            }
            n += 1;
        };
        ledger::insert_account(
            w,
            org,
            &Account {
                code: code.clone(),
                name: name.trim().to_owned(),
                parent_code: Some(parent.code),
                kind: "asset".into(),
                is_group: false,
                active: true,
            },
        )?;
        let id = insert_money_account(w, org, branch_id.as_deref(), kind, name.trim(), &code)?;
        let row = MoneyAccountRow {
            id: id.clone(),
            kind: kind.to_owned(),
            name: name.trim().to_owned(),
            account_code: code,
            branch_id,
            active: true,
            balance_minor: 0,
        };
        w.event("money_account.created", "money_account", &id, &row)?;
        w.audit("create", "money_account", &id, &row)?;
        Ok(row)
    })
}

// ---------------------------------------------------------------------------
// Fiscal periods (M7.2)

pub fn fiscal_years(store: &Store) -> Result<Vec<YearRow>> {
    store.read(|r| fiscal::years(r.conn(), the_org(r.conn())?))
}

/// Closes a period (after the ones before it): no entry is posted in it
/// any more, from any device.
pub fn close_period(store: &mut Store, meta: OpMeta, period: &str) -> Result<()> {
    store.write(meta, |w| {
        w.require(Permission::AccountingClosePeriod, &Target::org())?;
        let (_, number, states) =
            fiscal::year_states(w.tx(), period)?.ok_or(Error::NotFound("fiscal period"))?;
        may_close(&states, number)?;
        fiscal::set_closed(w, period, true)?;
        w.event("period.closed", "fiscal_period", period, &number)?;
        w.audit("close", "fiscal_period", period, &number)?;
        Ok(())
    })
}

/// Reopens the last closed period, with a reason kept in the audit trail.
pub fn reopen_period(store: &mut Store, meta: OpMeta, period: &str, reason: &str) -> Result<()> {
    if reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    store.write(meta, |w| {
        w.require(Permission::AccountingClosePeriod, &Target::org())?;
        let (_, number, states) =
            fiscal::year_states(w.tx(), period)?.ok_or(Error::NotFound("fiscal period"))?;
        may_reopen(&states, number)?;
        fiscal::set_closed(w, period, false)?;
        let detail = serde_json::json!({ "period": number, "reason": reason.trim() });
        w.event("period.reopened", "fiscal_period", period, &detail)?;
        w.audit("reopen", "fiscal_period", period, &detail)?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Reports (M7.8)

fn range(from: Option<&str>, to: Option<&str>) -> Result<(String, String)> {
    let from = match from.map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => Date::parse(f)
            .map_err(|_| RuleError::on("from", "date.format"))?
            .iso(),
        None => ALL_TIME.0.to_owned(),
    };
    let to = match to.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => Date::parse(t)
            .map_err(|_| RuleError::on("to", "date.format"))?
            .iso(),
        None => ALL_TIME.1.to_owned(),
    };
    if from > to {
        return Err(RuleError::on("to", "date.range").into());
    }
    Ok((from, to))
}

/// The day before an ISO date, for balances brought forward.
fn day_before(date: &str) -> String {
    if date == ALL_TIME.0 {
        return date.to_owned();
    }
    let Ok(d) = Date::parse(date) else {
        return date.to_owned();
    };
    if d.d > 1 {
        return Date { d: d.d - 1, ..d }.iso();
    }
    let (y, m) = if d.m == 1 {
        (d.y - 1, 12)
    } else {
        (d.y, d.m - 1)
    };
    Date {
        y,
        m,
        d: nb_domain::fiscal::days_in_month(y, m),
    }
    .iso()
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct JournalQuery {
    /// Number or memo.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    /// Entries touching this account.
    #[serde(default)]
    pub account: Option<String>,
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryLineRow {
    pub account: String,
    pub account_name: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRow {
    pub id: String,
    pub number: String,
    pub date: String,
    pub memo: String,
    pub source: String,
    pub total_minor: i64,
    /// The number of the entry this one reverses.
    pub reverses: Option<String>,
    /// The number of the entry that reversed this one.
    pub reversed_by: Option<String>,
    pub lines: Vec<EntryLineRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct JournalPage {
    pub rows: Vec<EntryRow>,
    pub total: u32,
}

/// The journal, latest first.
pub fn journal(store: &Store, q: &JournalQuery) -> Result<JournalPage> {
    let (from, to) = range(q.from.as_deref(), q.to.as_deref())?;
    store.read(|r| {
        let org = the_org(r.conn())?;
        let text = format!("%{}%", q.text.trim().to_lowercase());
        let filter = "FROM journal_entries e WHERE e.org_id = ?1 AND e.date >= ?2 AND e.date <= ?3
             AND (lower(e.number) LIKE ?4 OR lower(e.memo) LIKE ?4)
             AND (?5 IS NULL OR e.source = ?5)
             AND (?6 IS NULL OR EXISTS (SELECT 1 FROM journal_lines l WHERE l.entry_id = e.id AND l.account_code = ?6))";
        let source = q.source.as_deref().filter(|s| !s.is_empty());
        let account = q.account.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let args = params![org.to_string(), from, to, text, source, account];
        let total: u32 = r
            .conn()
            .query_row(&format!("SELECT COUNT(*) {filter}"), args, |r| r.get(0))?;
        let mut s = r.conn().prepare(&format!(
            "SELECT e.id, e.number, e.date, e.memo, e.source, e.total_minor,
                    (SELECT o.number FROM journal_entries o WHERE o.id = e.reverses_id),
                    (SELECT x.number FROM journal_entries x WHERE x.reverses_id = e.id)
             {filter} ORDER BY e.date DESC, e.created_at DESC LIMIT ?7 OFFSET ?8"
        ))?;
        let mut rows: Vec<EntryRow> = s
            .query_map(
                params![org.to_string(), from, to, text, source, account, q.limit.max(1), q.offset],
                |r| {
                    Ok(EntryRow {
                        id: r.get(0)?,
                        number: r.get(1)?,
                        date: r.get(2)?,
                        memo: r.get(3)?,
                        source: r.get(4)?,
                        total_minor: r.get(5)?,
                        reverses: r.get(6)?,
                        reversed_by: r.get(7)?,
                        lines: vec![],
                    })
                },
            )?
            .collect::<std::result::Result<_, _>>()?;
        let mut l = r.conn().prepare(
            "SELECT l.account_code, COALESCE(a.name, ''), l.debit_minor, l.credit_minor
             FROM journal_lines l LEFT JOIN accounts a ON a.org_id = ?2 AND a.code = l.account_code
             WHERE l.entry_id = ?1 ORDER BY l.position",
        )?;
        for e in &mut rows {
            e.lines = l
                .query_map(params![e.id, org.to_string()], |r| {
                    Ok(EntryLineRow {
                        account: r.get(0)?,
                        account_name: r.get(1)?,
                        debit_minor: r.get(2)?,
                        credit_minor: r.get(3)?,
                    })
                })?
                .collect::<std::result::Result<_, _>>()?;
        }
        Ok(JournalPage { rows, total })
    })
}

/// One account in the trial balance: brought forward, movements of the
/// period, and the balance at its end (debit positive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceRow {
    pub code: String,
    pub name: String,
    pub depth: u32,
    pub is_group: bool,
    pub opening_minor: i64,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub closing_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrialBalance {
    /// Accounts with a balance or movements, groups included.
    pub rows: Vec<BalanceRow>,
    /// Totals of the accounts (groups not counted twice).
    pub debit_minor: i64,
    pub credit_minor: i64,
    /// Sum of debit balances and of credit balances at the end.
    pub closing_debit_minor: i64,
    pub closing_credit_minor: i64,
}

pub fn trial_balance(store: &Store, from: Option<&str>, to: Option<&str>) -> Result<TrialBalance> {
    let (from, to) = range(from, to)?;
    store.read(|r| {
        let org = the_org(r.conn())?;
        let accounts = ledger::accounts(r.conn(), org)?;
        let before = totals(r.conn(), org, &accounts, (ALL_TIME.0, &day_before(&from)))?;
        let before = if from == ALL_TIME.0 {
            HashMap::new()
        } else {
            before
        };
        let during = totals(r.conn(), org, &accounts, (&from, &to))?;
        let mut out = TrialBalance::default();
        for (a, depth) in tree(&accounts) {
            let (bd, bc) = before.get(&a.code).copied().unwrap_or_default();
            let (d, c) = during.get(&a.code).copied().unwrap_or_default();
            if bd == 0 && bc == 0 && d == 0 && c == 0 {
                continue;
            }
            let row = BalanceRow {
                code: a.code.clone(),
                name: a.name.clone(),
                depth,
                is_group: a.is_group,
                opening_minor: bd - bc,
                debit_minor: d,
                credit_minor: c,
                closing_minor: bd - bc + d - c,
            };
            if !a.is_group {
                out.debit_minor += d;
                out.credit_minor += c;
                if row.closing_minor > 0 {
                    out.closing_debit_minor += row.closing_minor;
                } else {
                    out.closing_credit_minor -= row.closing_minor;
                }
            }
            out.rows.push(row);
        }
        Ok(out)
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementLine {
    pub entry_id: String,
    pub date: String,
    pub number: String,
    pub memo: String,
    pub source: String,
    pub account: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    /// Running balance after this line (debit positive).
    pub balance_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Statement {
    pub code: String,
    pub name: String,
    pub opening_minor: i64,
    pub lines: Vec<StatementLine>,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub closing_minor: i64,
}

/// An account's ledger (M7.8): the balance brought forward, then each line
/// of the period with the running balance; a group shows its accounts'.
pub fn statement(
    store: &Store,
    code: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Statement> {
    let (from, to) = range(from, to)?;
    store.read(|r| {
        let org = the_org(r.conn())?;
        let account = ledger::account(r.conn(), org, code.trim())?.ok_or(Error::NotFound("account"))?;
        let accounts = ledger::accounts(r.conn(), org)?;
        // The account and everything under it.
        let mut codes = vec![account.code.clone()];
        let mut i = 0;
        while i < codes.len() {
            let parent = codes[i].clone();
            codes.extend(
                accounts
                    .iter()
                    .filter(|a| a.parent_code.as_deref() == Some(parent.as_str()) && a.code != parent)
                    .map(|a| a.code.clone()),
            );
            i += 1;
            if i > accounts.len() {
                break;
            }
        }
        let list = codes
            .iter()
            .map(|c| format!("'{}'", c.replace('\'', "")))
            .collect::<Vec<_>>()
            .join(",");
        let opening: i64 = if from == ALL_TIME.0 {
            0
        } else {
            r.conn().query_row(
                &format!(
                    "SELECT COALESCE(SUM(l.debit_minor - l.credit_minor), 0)
                     FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
                     WHERE e.org_id = ?1 AND e.date < ?2 AND l.account_code IN ({list})"
                ),
                params![org.to_string(), from],
                |r| r.get(0),
            )?
        };
        let mut s = r.conn().prepare(&format!(
            "SELECT e.id, e.date, e.number, e.memo, e.source, l.account_code, l.debit_minor, l.credit_minor
             FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
             WHERE e.org_id = ?1 AND e.date >= ?2 AND e.date <= ?3 AND l.account_code IN ({list})
             ORDER BY e.date, e.created_at, l.position"
        ))?;
        let mut balance = opening;
        let (mut debit, mut credit) = (0, 0);
        let lines = s
            .query_map(params![org.to_string(), from, to], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })?
            .map(|row| {
                let (entry_id, date, number, memo, source, account, d, c) = row?;
                balance += d - c;
                debit += d;
                credit += c;
                Ok(StatementLine {
                    entry_id,
                    date,
                    number,
                    memo,
                    source,
                    account,
                    debit_minor: d,
                    credit_minor: c,
                    balance_minor: balance,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Statement {
            code: account.code,
            name: account.name,
            opening_minor: opening,
            lines,
            debit_minor: debit,
            credit_minor: credit,
            closing_minor: balance,
        })
    })
}

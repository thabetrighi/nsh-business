//! Chart of accounts and journal (append-only once posted).

use nb_core::{Currency, Money, OrganizationId};
use nb_domain::EntryId;
use nb_domain::ledger::{Entry, Line};
use nb_domain::posting::{AccountMap, Role};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::uow::Writer;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub code: String,
    pub name: String,
    pub parent_code: Option<String>,
    pub kind: String,
    /// A heading of the chart: it totals its accounts and takes no entry.
    #[serde(default)]
    pub is_group: bool,
    #[serde(default = "yes")]
    pub active: bool,
}

fn yes() -> bool {
    true
}

pub fn insert_account(w: &Writer<'_>, org: OrganizationId, a: &Account) -> Result<()> {
    w.tx().execute(
        "INSERT INTO accounts (org_id, code, name, parent_code, kind, is_group, active) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![org.to_string(), a.code, a.name, a.parent_code, a.kind, a.is_group, a.active],
    )?;
    Ok(())
}

/// One account of the chart.
pub fn account(conn: &Connection, org: OrganizationId, code: &str) -> Result<Option<Account>> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row(
            "SELECT code, name, parent_code, kind, is_group, active FROM accounts WHERE org_id = ?1 AND code = ?2",
            params![org.to_string(), code],
            |r| {
                Ok(Account {
                    code: r.get(0)?,
                    name: r.get(1)?,
                    parent_code: r.get(2)?,
                    kind: r.get(3)?,
                    is_group: r.get(4)?,
                    active: r.get(5)?,
                })
            },
        )
        .optional()?)
}

/// The whole chart, by code.
pub fn accounts(conn: &Connection, org: OrganizationId) -> Result<Vec<Account>> {
    let mut s = conn.prepare(
        "SELECT code, name, parent_code, kind, is_group, active FROM accounts WHERE org_id = ?1 ORDER BY code",
    )?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok(Account {
            code: r.get(0)?,
            name: r.get(1)?,
            parent_code: r.get(2)?,
            kind: r.get(3)?,
            is_group: r.get(4)?,
            active: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// The accounts the entry matrix posts to, by role.
pub fn account_map(conn: &Connection, org: OrganizationId) -> Result<AccountMap> {
    let mut s = conn.prepare("SELECT role, account_code FROM account_roles WHERE org_id = ?1")?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut map = AccountMap::default();
    for row in rows {
        let (role, code) = row?;
        if let Some(role) = Role::from_code(&role) {
            map.set(role, &code);
        }
    }
    Ok(map)
}

pub fn set_role(w: &Writer<'_>, org: OrganizationId, role: Role, code: &str) -> Result<()> {
    w.tx().execute(
        "INSERT INTO account_roles (org_id, role, account_code) VALUES (?1, ?2, ?3)
         ON CONFLICT (org_id, role) DO UPDATE SET account_code = ?3",
        params![org.to_string(), role.code(), code],
    )?;
    Ok(())
}

pub fn account_exists(conn: &Connection, org: OrganizationId, code: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM accounts WHERE org_id = ?1 AND code = ?2",
        params![org.to_string(), code],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// What identifies an entry besides its lines.
#[derive(Debug, Clone, Copy)]
pub struct Header<'a> {
    pub number: &'a str,
    /// `YYYY-MM-DD`, the business date.
    pub date: &'a str,
    pub memo: &'a str,
    /// What produced it (`manual`, `stock.adjust`, `sale`…).
    pub source: &'a str,
    pub reverses: Option<EntryId>,
    /// The customer whose debt the receivable lines move (M9.2).
    pub party: Option<&'a str>,
}

/// Posts a balanced entry (its accounts must exist).
pub fn post(w: &Writer<'_>, org: OrganizationId, h: Header<'_>, entry: &Entry) -> Result<EntryId> {
    let Header {
        number,
        date,
        memo,
        source,
        reverses,
        party,
    } = h;
    for l in entry.lines() {
        let code = match account(w.tx(), org, &l.account)? {
            None => Some("ledger.unknown_account"),
            Some(a) if a.is_group => Some("ledger.group_account"),
            Some(a) if !a.active => Some("ledger.inactive_account"),
            Some(_) => None,
        };
        if let Some(code) = code {
            return Err(Error::Rules(vec![nb_domain::RuleError::on(
                "account", code,
            )]));
        }
    }
    // The period must be open; its year is opened on first use.
    if crate::repo::fiscal::ensure_period(w, org, date)?.closed {
        return Err(Error::Rules(vec![nb_domain::RuleError::on(
            "date",
            "fiscal.period_closed",
        )]));
    }
    let id = EntryId::new();
    w.tx().execute(
        "INSERT INTO journal_entries (id, org_id, number, date, memo, source, reverses_id, total_minor, operation_id, created_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id.to_string(),
            org.to_string(),
            number,
            date,
            memo,
            source,
            reverses.map(|r| r.to_string()),
            entry.total().minor(),
            w.meta().id.to_string(),
            w.now(),
            w.actor_text(),
            w.device().id.to_string()
        ],
    )?;
    // The receivable lines are the customer's, the payable lines the
    // supplier's: their sub-ledgers.
    let (receivable, payable) = match party {
        Some(_) => {
            let map = account_map(w.tx(), org)?;
            (
                map.get(Role::Receivable).ok().map(str::to_owned),
                map.get(Role::Payable).ok().map(str::to_owned),
            )
        }
        None => (None, None),
    };
    let mut owed = 0i64;
    let mut owing = 0i64;
    for (i, l) in entry.lines().iter().enumerate() {
        let on_receivable = receivable.as_deref() == Some(l.account.as_str());
        let on_payable = payable.as_deref() == Some(l.account.as_str());
        let tagged = on_receivable || on_payable;
        if on_receivable {
            owed += l.debit.minor() - l.credit.minor();
        }
        if on_payable {
            owing += l.credit.minor() - l.debit.minor();
        }
        w.tx().execute(
            "INSERT INTO journal_lines (entry_id, position, account_code, debit_minor, credit_minor, memo, party_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id.to_string(),
                i64::try_from(i).unwrap_or(i64::MAX),
                l.account,
                l.debit.minor(),
                l.credit.minor(),
                l.memo,
                party.filter(|_| tagged)
            ],
        )?;
    }
    if let (Some(p), true) = (party, owed != 0) {
        let n = w.tx().execute(
            "UPDATE customers SET balance_minor = balance_minor + ?2 WHERE id = ?1",
            params![p, owed],
        )?;
        if n == 0 {
            return Err(Error::NotFound("customer"));
        }
    }
    if let (Some(p), true) = (party, owing != 0) {
        let n = w.tx().execute(
            "UPDATE suppliers SET balance_minor = balance_minor + ?2 WHERE id = ?1",
            params![p, owing],
        )?;
        if n == 0 {
            return Err(Error::NotFound("supplier"));
        }
    }
    Ok(id)
}

/// The lines of an entry, as a domain entry (to reverse it).
pub fn load(conn: &Connection, id: EntryId, currency: Currency) -> Result<Entry> {
    let mut s = conn.prepare(
        "SELECT account_code, debit_minor, credit_minor, memo FROM journal_lines WHERE entry_id = ?1 ORDER BY position",
    )?;
    let lines = s
        .query_map([id.to_string()], |r| {
            Ok(Line {
                account: r.get(0)?,
                debit: Money::from_minor(r.get(1)?, currency),
                credit: Money::from_minor(r.get(2)?, currency),
                memo: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if lines.is_empty() {
        return Err(Error::NotFound("journal entry"));
    }
    Ok(Entry::new(lines)?)
}

/// The customer an entry's receivable lines belong to, if any.
pub fn party_of(conn: &Connection, id: EntryId) -> Result<Option<String>> {
    Ok(conn.query_row(
        "SELECT MAX(party_id) FROM journal_lines WHERE entry_id = ?1",
        [id.to_string()],
        |r| r.get(0),
    )?)
}

pub fn is_reversed(conn: &Connection, id: EntryId) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM journal_entries WHERE reverses_id = ?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// One row of the trial balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrialRow {
    pub code: String,
    pub name: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
}

/// Totals per account (M7.8); the sum of debits equals the sum of credits.
pub fn trial_balance(conn: &Connection, org: OrganizationId) -> Result<Vec<TrialRow>> {
    let mut s = conn.prepare(
        "SELECT a.code, a.name, COALESCE(SUM(l.debit_minor), 0), COALESCE(SUM(l.credit_minor), 0)
         FROM accounts a
         JOIN journal_lines l ON l.account_code = a.code
         JOIN journal_entries e ON e.id = l.entry_id AND e.org_id = a.org_id
         WHERE a.org_id = ?1
         GROUP BY a.code, a.name ORDER BY a.code",
    )?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok(TrialRow {
            code: r.get(0)?,
            name: r.get(1)?,
            debit_minor: r.get(2)?,
            credit_minor: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// The invariant checked in CI (M7.9): every posted entry balances.
pub fn unbalanced_entries(conn: &Connection) -> Result<Vec<String>> {
    let mut s = conn.prepare(
        "SELECT e.number FROM journal_entries e JOIN journal_lines l ON l.entry_id = e.id
         GROUP BY e.id HAVING SUM(l.debit_minor) != SUM(l.credit_minor) OR SUM(l.debit_minor) != e.total_minor",
    )?;
    let rows = s.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

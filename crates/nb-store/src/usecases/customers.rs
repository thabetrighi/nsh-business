//! Customers (M9.1), their sub-ledger (M9.2) and the credit check at the
//! till (M9.3).
//!
//! What a customer owes is the sum of the receivable lines tagged with it
//! (`journal_lines.party_id`, set when an entry is posted); the balance on
//! the customer is the projection of those lines, kept in the same
//! transaction and rebuilt from them ([`customer_drift`]). Credit beyond
//! the limit, or for a blocked customer, needs a supervisor.

use nb_domain::RuleError;
use nb_domain::access::{Permission, Target};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::uow::{OpMeta, Writer};
use crate::usecases::sales::the_org;
use crate::{Error, Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CustomerInput {
    /// The customer to change; empty for a new one.
    #[serde(default)]
    pub id: Option<String>,
    /// `person` or `company`.
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub group_id: Option<String>,
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
    pub credit_limit_minor: i64,
    #[serde(default)]
    pub payment_days: i64,
    /// `active` or `blocked`.
    #[serde(default = "active")]
    pub status: String,
    #[serde(default)]
    pub note: String,
}

fn active() -> String {
    "active".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerRow {
    pub id: String,
    pub code: String,
    pub kind: String,
    pub name: String,
    pub group_id: Option<String>,
    pub group_name: Option<String>,
    pub phone: String,
    pub email: String,
    pub address: String,
    pub city: String,
    pub nif: String,
    pub nis: String,
    pub rc: String,
    pub ai: String,
    pub credit_limit_minor: i64,
    pub payment_days: i64,
    pub status: String,
    pub note: String,
    /// What the customer owes (negative: what is owed to it).
    pub balance_minor: i64,
    pub version: i64,
}

impl CustomerRow {
    /// Credit still available before a supervisor is needed.
    pub fn credit_left_minor(&self) -> i64 {
        (self.credit_limit_minor - self.balance_minor).max(0)
    }
    pub fn over_limit(&self) -> bool {
        self.balance_minor > self.credit_limit_minor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CustomerQuery {
    /// In the name, code, phone or city.
    #[serde(default)]
    pub search: String,
    /// ``, `debtors`, `over_limit` or `blocked`.
    #[serde(default)]
    pub filter: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CustomerPage {
    pub rows: Vec<CustomerRow>,
    pub total: u32,
    /// What the customers the filter keeps owe, together.
    pub owed_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerGroup {
    pub id: String,
    pub name: String,
    pub customers: u32,
}

const SELECT: &str = "SELECT c.id, c.code, c.kind, c.name, c.group_id, g.name, c.phone, c.email, c.address, c.city,
        c.nif, c.nis, c.rc, c.ai, c.credit_limit_minor, c.payment_days, c.status, c.note, c.balance_minor, c.version
     FROM customers c LEFT JOIN customer_groups g ON g.id = c.group_id";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<CustomerRow> {
    Ok(CustomerRow {
        id: r.get(0)?,
        code: r.get(1)?,
        kind: r.get(2)?,
        name: r.get(3)?,
        group_id: r.get(4)?,
        group_name: r.get(5)?,
        phone: r.get(6)?,
        email: r.get(7)?,
        address: r.get(8)?,
        city: r.get(9)?,
        nif: r.get(10)?,
        nis: r.get(11)?,
        rc: r.get(12)?,
        ai: r.get(13)?,
        credit_limit_minor: r.get(14)?,
        payment_days: r.get(15)?,
        status: r.get(16)?,
        note: r.get(17)?,
        balance_minor: r.get(18)?,
        version: r.get(19)?,
    })
}

pub(crate) fn load(conn: &Connection, id: &str) -> Result<CustomerRow> {
    conn.query_row(&format!("{SELECT} WHERE c.id = ?1"), [id], row_of)
        .optional()?
        .ok_or(Error::Rules(vec![RuleError::on(
            "customer",
            "customer.unknown",
        )]))
}

pub fn get_customer(store: &Store, id: &str) -> Result<CustomerRow> {
    store.read(|r| load(r.conn(), id))
}

pub fn list_customers(store: &Store, q: &CustomerQuery) -> Result<CustomerPage> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let search = q.search.trim().to_lowercase();
        let filter = " WHERE c.org_id = ?1
               AND (?2 = '' OR lower(c.name) LIKE '%' || ?2 || '%' OR lower(c.code) LIKE '%' || ?2 || '%'
                    OR c.phone LIKE '%' || ?2 || '%' OR lower(c.city) LIKE '%' || ?2 || '%')
               AND (?3 = '' OR (?3 = 'debtors' AND c.balance_minor > 0)
                    OR (?3 = 'over_limit' AND c.balance_minor > c.credit_limit_minor)
                    OR (?3 = 'blocked' AND c.status = 'blocked'))";
        let (total, owed): (u32, i64) = r.conn().query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(MAX(c.balance_minor, 0)), 0) FROM customers c{filter}"
            ),
            params![org.to_string(), search, q.filter],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut s = r.conn().prepare(&format!(
            "{SELECT}{filter} ORDER BY c.balance_minor > 0 DESC, c.name COLLATE NOCASE LIMIT ?4 OFFSET ?5"
        ))?;
        let rows = s
            .query_map(
                params![
                    org.to_string(),
                    search,
                    q.filter,
                    q.limit.clamp(1, 500),
                    q.offset
                ],
                row_of,
            )?
            .collect::<std::result::Result<_, _>>()?;
        Ok(CustomerPage {
            rows,
            total,
            owed_minor: owed,
        })
    })
}

pub fn customer_groups(store: &Store) -> Result<Vec<CustomerGroup>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT g.id, g.name, (SELECT COUNT(*) FROM customers c WHERE c.group_id = g.id)
             FROM customer_groups g WHERE g.org_id = ?1 ORDER BY g.name COLLATE NOCASE",
        )?;
        let rows = s
            .query_map([org.to_string()], |r| {
                Ok(CustomerGroup {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    customers: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    })
}

pub fn create_group(store: &mut Store, meta: OpMeta, name: &str) -> Result<CustomerGroup> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::CustomersCreateGroup {
            name: name.to_owned(),
        },
    )?;
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    store.write(meta, |w| {
        w.require(Permission::CustomersEdit, &Target::org())?;
        let id = nb_core::id::idgen::uuid().to_string();
        let taken: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM customer_groups WHERE org_id = ?1 AND lower(name) = lower(?2)",
            params![w.org()?.to_string(), name],
            |r| r.get(0),
        )?;
        if taken > 0 {
            return Err(Error::Duplicate("name"));
        }
        w.tx().execute(
            "INSERT INTO customer_groups (id, org_id, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, w.org()?.to_string(), name, w.now()],
        )?;
        let g = CustomerGroup {
            id: id.clone(),
            name,
            customers: 0,
        };
        w.event("customer_group.created", "customer_group", &id, &g)?;
        w.audit("create", "customer_group", &id, &g)?;
        Ok(g)
    })
}

pub(crate) fn check(input: &CustomerInput) -> Result<()> {
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
    if input.credit_limit_minor < 0 {
        errors.push(RuleError::on("credit_limit", "ledger.negative"));
    }
    if !(0..=365).contains(&input.payment_days) {
        errors.push(RuleError::on("payment_days", "invalid_value"));
    }
    let email = input.email.trim();
    if !email.is_empty() && !(email.contains('@') && email.contains('.')) {
        errors.push(RuleError::on("email", "invalid_email"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}

/// Writes a customer in the caller's transaction (checked by the caller).
pub(crate) fn save_customer_in(w: &mut Writer<'_>, input: &CustomerInput) -> Result<String> {
    w.require(Permission::CustomersEdit, &Target::org())?;
    let org = w.org()?.to_string();
    if let Some(g) = input.group_id.as_deref().filter(|g| !g.is_empty()) {
        let n: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM customer_groups WHERE id = ?1 AND org_id = ?2",
            params![g, org],
            |r| r.get(0),
        )?;
        if n == 0 {
            return Err(RuleError::on("group", "invalid_value").into());
        }
    }
    let before = match input.id.as_deref().filter(|i| !i.is_empty()) {
        Some(id) => Some(load(w.tx(), id)?),
        None => None,
    };
    let raises = input.credit_limit_minor > before.as_ref().map_or(0, |b| b.credit_limit_minor);
    if raises {
        w.require(Permission::CreditLimits, &Target::org())?;
    }
    let group = input.group_id.clone().filter(|g| !g.is_empty());
    let t = |s: &str| s.trim().to_owned();
    let id = match before {
        Some(b) => {
            w.tx().execute(
                "UPDATE customers SET kind = ?2, name = ?3, group_id = ?4, phone = ?5, email = ?6, address = ?7,
                        city = ?8, nif = ?9, nis = ?10, rc = ?11, ai = ?12, credit_limit_minor = ?13,
                        payment_days = ?14, status = ?15, note = ?16, updated_at = ?17, version = version + 1
                 WHERE id = ?1",
                params![
                    b.id,
                    input.kind,
                    t(&input.name),
                    group,
                    t(&input.phone),
                    t(&input.email),
                    t(&input.address),
                    t(&input.city),
                    t(&input.nif),
                    t(&input.nis),
                    t(&input.rc),
                    t(&input.ai),
                    input.credit_limit_minor,
                    input.payment_days,
                    input.status,
                    t(&input.note),
                    w.now()
                ],
            )?;
            w.audit("update", "customer", &b.id, &input)?;
            b.id
        }
        None => {
            let code = w.next_code("customer", "C")?;
            let id = nb_core::id::idgen::uuid().to_string();
            w.tx().execute(
                "INSERT INTO customers (id, org_id, code, kind, name, group_id, phone, email, address, city, nif, nis,
                        rc, ai, credit_limit_minor, payment_days, status, note, created_at, created_by, updated_at,
                        device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                         ?19, ?21)",
                params![
                    id,
                    org,
                    code,
                    input.kind,
                    t(&input.name),
                    group,
                    t(&input.phone),
                    t(&input.email),
                    t(&input.address),
                    t(&input.city),
                    t(&input.nif),
                    t(&input.nis),
                    t(&input.rc),
                    t(&input.ai),
                    input.credit_limit_minor,
                    input.payment_days,
                    input.status,
                    t(&input.note),
                    w.now(),
                    w.actor_text(),
                    w.device().id.to_string()
                ],
            )?;
            w.audit("create", "customer", &id, &input)?;
            id
        }
    };
    let row = load(w.tx(), &id)?;
    w.event("customer.saved", "customer", &id, &row)?;
    Ok(id)
}

/// Creates a customer (numbered C-00001…) or changes one. Setting or
/// raising a credit limit needs `credit.limits`.
pub fn save_customer(store: &mut Store, meta: OpMeta, input: CustomerInput) -> Result<CustomerRow> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::CustomersSaveCustomer {
            input: input.clone(),
        },
    )?;
    check(&input)?;
    let id = store.write(meta, |w| save_customer_in(w, &input))?;
    get_customer(store, &id)
}

/// Before selling to `customer` with `credit_minor` left on its account:
/// it must exist, a blocked customer gets no credit, and what goes past
/// its limit needs a supervisor (`credit.exceed`, approvable).
pub(crate) fn check_credit(
    w: &mut Writer<'_>,
    customer: &str,
    credit_minor: i64,
    at: &Target,
    approval: Option<nb_domain::ApprovalId>,
) -> Result<()> {
    let c = load(w.tx(), customer)?;
    if credit_minor <= 0 {
        return Ok(());
    }
    if c.status == "blocked" {
        return Err(RuleError::on("customer", "customer.blocked").into());
    }
    if c.balance_minor + credit_minor > c.credit_limit_minor {
        w.require_approved(Permission::CreditExceed, at, approval)?;
    }
    Ok(())
}

/// Customers whose balance differs from their receivable lines (the
/// projection rebuilt and compared): (customer, kept, from the lines).
pub fn customer_drift(conn: &Connection) -> Result<Vec<(String, i64, i64)>> {
    let mut s = conn.prepare(
        "SELECT c.id, c.balance_minor,
                COALESCE((SELECT SUM(l.debit_minor - l.credit_minor) FROM journal_lines l WHERE l.party_id = c.id), 0)
         FROM customers c",
    )?;
    let rows = s
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<Vec<(String, i64, i64)>, _>>()?;
    Ok(rows.into_iter().filter(|(_, a, b)| a != b).collect())
}

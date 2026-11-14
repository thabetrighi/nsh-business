//! The shops, their devices and their accounts, as the platform's
//! operator sees them; and what the operator may do to them.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{Admin, Confirm, DAY, auth::fresh, log_action};
use crate::accounts::plain_code;
use crate::{ApiError, ApiResult, Server, hash, lock, now_ms};

const PAGE: i64 = 50;

#[derive(Deserialize, Default)]
pub(crate) struct ListQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    filter: String,
    #[serde(default)]
    page: i64,
}

fn file_size(path: &std::path::Path) -> i64 {
    ["", "-wal"]
        .iter()
        .filter_map(|ext| std::fs::metadata(format!("{}{ext}", path.display())).ok())
        .map(|m| i64::try_from(m.len()).unwrap_or(i64::MAX))
        .sum()
}

pub(crate) async fn overview(
    State(server): State<Arc<Server>>,
    _admin: Admin,
) -> ApiResult<axum::Json<serde_json::Value>> {
    let billing = server.billing;
    server
        .control(move |c| {
            let now = now_ms();
            let n = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> ApiResult<i64> {
                Ok(c.query_row(sql, p, |r| r.get(0))?)
            };
            let mut kinds = serde_json::Map::new();
            {
                let mut st = c.prepare(
                    "SELECT kind, COUNT(*) FROM security_events WHERE at > ?1
                     GROUP BY kind ORDER BY 2 DESC",
                )?;
                let rows = st.query_map([now - DAY], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?;
                for row in rows {
                    let (k, v) = row?;
                    kinds.insert(k, v.into());
                }
            }
            Ok(axum::Json(serde_json::json!({
                "billing": billing,
                "orgs": n("SELECT COUNT(*) FROM orgs", &[])?,
                "accounts": n("SELECT COUNT(*) FROM accounts", &[])?,
                "devices": n("SELECT COUNT(*) FROM devices WHERE revoked_at IS NULL", &[])?,
                "devices_today": n(
                    "SELECT COUNT(*) FROM devices WHERE revoked_at IS NULL AND last_seen_at > ?1",
                    &[&(now - DAY)],
                )?,
                "suspended": n("SELECT COUNT(*) FROM orgs WHERE suspended_at IS NOT NULL", &[])?,
                "trial": n("SELECT COUNT(*) FROM subscriptions WHERE status = 'trial'", &[])?,
                "ending": n(
                    "SELECT COUNT(*) FROM subscriptions WHERE ends_at BETWEEN ?1 AND ?2",
                    &[&now, &(now + 7 * DAY)],
                )?,
                "ended": n(
                    "SELECT COUNT(*) FROM subscriptions WHERE ends_at < ?1 OR status = 'cancelled'",
                    &[&now],
                )?,
                "paid_month": n(
                    "SELECT COALESCE(SUM(amount_minor), 0) FROM payments WHERE at > ?1",
                    &[&(now - 30 * DAY)],
                )?,
                "events_today": kinds,
                "blocked": n("SELECT COUNT(*) FROM blocked_ips", &[])?,
            })))
        })
        .await
}

#[derive(Serialize)]
struct OrgRow {
    id: String,
    name: String,
    created_at: i64,
    suspended: bool,
    devices: i64,
    last_seen: Option<i64>,
    owners: String,
    plan: Option<String>,
    status: Option<String>,
    ends_at: Option<i64>,
    size: i64,
}

pub(crate) async fn orgs(
    State(server): State<Arc<Server>>,
    _admin: Admin,
    Query(q): Query<ListQuery>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    let srv = server.clone();
    server
        .control(move |c| {
            let now = now_ms();
            // Fixed words only: nothing of the request goes into the text.
            let filter = match q.filter.as_str() {
                "suspended" => "o.suspended_at IS NOT NULL",
                "trial" => "s.status = 'trial'",
                "ending" => "s.ends_at BETWEEN ?4 AND ?4 + 604800000",
                "ended" => "(s.ends_at < ?4 OR s.status = 'cancelled')",
                _ => "?4 = ?4",
            };
            let search = "(?1 = '' OR o.name LIKE '%' || ?1 || '%' OR o.id = ?1
                 OR EXISTS (SELECT 1 FROM memberships m JOIN accounts a ON a.id = m.account_id
                            WHERE m.org_id = o.id AND (a.phone LIKE '%' || ?1 || '%' OR a.name LIKE '%' || ?1 || '%')))";
            let total: i64 = c.query_row(
                &format!(
                    "SELECT COUNT(*) FROM orgs o LEFT JOIN subscriptions s ON s.org_id = o.id
                     WHERE {search} AND {filter} AND ?2 = ?2 AND ?3 = ?3"
                ),
                params![q.q.trim(), PAGE, 0, now],
                |r| r.get(0),
            )?;
            let mut st = c.prepare(&format!(
                "SELECT o.id, o.name, o.created_at, o.suspended_at IS NOT NULL,
                    (SELECT COUNT(*) FROM devices d WHERE d.org_id = o.id AND d.revoked_at IS NULL),
                    (SELECT MAX(d.last_seen_at) FROM devices d WHERE d.org_id = o.id),
                    COALESCE((SELECT group_concat(a.phone, ', ') FROM memberships m
                              JOIN accounts a ON a.id = m.account_id
                              WHERE m.org_id = o.id AND m.role = 'owner'), ''),
                    s.plan, s.status, s.ends_at
                 FROM orgs o LEFT JOIN subscriptions s ON s.org_id = o.id
                 WHERE {search} AND {filter}
                 ORDER BY o.created_at DESC LIMIT ?2 OFFSET ?3"
            ))?;
            let rows = st.query_map(
                params![q.q.trim(), PAGE, q.page.max(0) * PAGE, now],
                |r| {
                    Ok(OrgRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        created_at: r.get(2)?,
                        suspended: r.get(3)?,
                        devices: r.get(4)?,
                        last_seen: r.get(5)?,
                        owners: r.get(6)?,
                        plan: r.get(7)?,
                        status: r.get(8)?,
                        ends_at: r.get(9)?,
                        size: 0,
                    })
                },
            )?;
            let mut list: Vec<OrgRow> = rows.collect::<Result<_, _>>()?;
            for o in &mut list {
                o.size = file_size(&srv.tenant_path(&o.id));
            }
            Ok(axum::Json(serde_json::json!({
                "total": total,
                "page_size": PAGE,
                "rows": list,
            })))
        })
        .await
}

pub(super) fn known_org(c: &Connection, id: &str) -> ApiResult<String> {
    c.query_row("SELECT name FROM orgs WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or(ApiError::NotFound)
}

pub(crate) async fn org(
    State(server): State<Arc<Server>>,
    _admin: Admin,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    let srv = server.clone();
    server
        .control(move |c| {
            let (name, created, suspended_at, reason, epoch): (String, i64, Option<i64>, String, i64) = c
                .query_row(
                    "SELECT name, created_at, suspended_at, suspend_reason, epoch FROM orgs WHERE id = ?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?
                .ok_or(ApiError::NotFound)?;
            let mut st = c.prepare(
                "SELECT id, prefix, name, created_at, last_seen_at, revoked_at
                 FROM devices WHERE org_id = ?1 ORDER BY prefix",
            )?;
            let devices: Vec<serde_json::Value> = st
                .query_map([&id], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, String>(0)?,
                        "prefix": r.get::<_, String>(1)?,
                        "name": r.get::<_, String>(2)?,
                        "created_at": r.get::<_, i64>(3)?,
                        "last_seen": r.get::<_, Option<i64>>(4)?,
                        "revoked_at": r.get::<_, Option<i64>>(5)?,
                    }))
                })?
                .collect::<Result<_, _>>()?;
            let mut st = c.prepare(
                "SELECT a.id, a.phone, a.name, m.role FROM memberships m
                 JOIN accounts a ON a.id = m.account_id WHERE m.org_id = ?1 ORDER BY m.created_at",
            )?;
            let members: Vec<serde_json::Value> = st
                .query_map([&id], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, String>(0)?,
                        "phone": r.get::<_, String>(1)?,
                        "name": r.get::<_, String>(2)?,
                        "role": r.get::<_, String>(3)?,
                    }))
                })?
                .collect::<Result<_, _>>()?;
            let subscription: Option<serde_json::Value> = c
                .query_row(
                    "SELECT plan, status, ends_at, note, updated_at FROM subscriptions WHERE org_id = ?1",
                    [&id],
                    |r| {
                        Ok(serde_json::json!({
                            "plan": r.get::<_, Option<String>>(0)?,
                            "status": r.get::<_, String>(1)?,
                            "ends_at": r.get::<_, Option<i64>>(2)?,
                            "note": r.get::<_, String>(3)?,
                            "updated_at": r.get::<_, i64>(4)?,
                        }))
                    },
                )
                .optional()?;
            let payments = super::billing::payments_of(c, Some(&id))?;
            Ok(axum::Json(serde_json::json!({
                "id": id,
                "name": name,
                "created_at": created,
                "suspended_at": suspended_at,
                "suspend_reason": reason,
                "epoch": epoch,
                "size": file_size(&srv.tenant_path(&id)),
                "devices": devices,
                "members": members,
                "subscription": subscription,
                "payments": payments,
            })))
        })
        .await
}

#[derive(Deserialize)]
pub(crate) struct Reason {
    #[serde(default)]
    reason: String,
}

pub(crate) async fn suspend(
    State(server): State<Arc<Server>>,
    admin: Admin,
    Path(id): Path<String>,
    axum::Json(req): axum::Json<Reason>,
) -> ApiResult<StatusCode> {
    server
        .control(move |c| {
            known_org(c, &id)?;
            let reason: String = req.reason.trim().chars().take(300).collect();
            c.execute(
                "UPDATE orgs SET suspended_at = ?2, suspend_reason = ?3 WHERE id = ?1",
                params![id, now_ms(), reason],
            )?;
            log_action(c, &admin.login, &admin.ip, "org.suspend", &id, &reason)?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

pub(crate) async fn resume(
    State(server): State<Arc<Server>>,
    admin: Admin,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    server
        .control(move |c| {
            known_org(c, &id)?;
            c.execute(
                "UPDATE orgs SET suspended_at = NULL, suspend_reason = '' WHERE id = ?1",
                [&id],
            )?;
            log_action(c, &admin.login, &admin.ip, "org.resume", &id, "")?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

#[derive(Deserialize)]
pub(crate) struct DeleteRequest {
    name: String,
    code: String,
}

/// A shop leaves the platform (its owner asked): its file goes to
/// `deleted/` (kept for whoever must still answer for it), its devices and
/// accounts' links go. Takes its exact name and a fresh code.
pub(crate) async fn delete(
    State(server): State<Arc<Server>>,
    admin: Admin,
    Path(id): Path<String>,
    axum::Json(req): axum::Json<DeleteRequest>,
) -> ApiResult<StatusCode> {
    let srv = server.clone();
    crate::blocking(move || {
        let tenant = srv.tenant_path(&id);
        {
            let c = lock(&srv.control)?;
            let name = known_org(&c, &id)?;
            fresh(&c, &admin, &req.code)?;
            if req.name.trim() != name.trim() {
                return Err(ApiError::BadRequest("admin.name_mismatch".into()));
            }
            // Closed first: nobody writes into a file being moved.
            if let Some(t) = lock(&srv.tenants)?.remove(&id) {
                *lock(&t)? = None;
            }
            let gone = srv.dir.join("deleted");
            std::fs::create_dir_all(&gone).map_err(|e| ApiError::Internal(e.to_string()))?;
            if tenant.exists() {
                std::fs::rename(&tenant, gone.join(format!("{id}.{}.sqlite", now_ms())))
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
            }
            for ext in ["-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", tenant.display()));
            }
            for sql in [
                "DELETE FROM devices WHERE org_id = ?1",
                "DELETE FROM invites WHERE org_id = ?1",
                "DELETE FROM memberships WHERE org_id = ?1",
                "DELETE FROM subscriptions WHERE org_id = ?1",
                "DELETE FROM orgs WHERE id = ?1",
            ] {
                c.execute(sql, [&id])?;
            }
            log_action(&c, &admin.login, &admin.ip, "org.delete", &id, &name)?;
        }
        if let Ok(mut w) = srv.waves.lock() {
            w.remove(&id);
        }
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

pub(crate) async fn revoke_device(
    State(server): State<Arc<Server>>,
    admin: Admin,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    server
        .control(move |c| {
            let org: String = c
                .query_row("SELECT org_id FROM devices WHERE id = ?1", [&id], |r| {
                    r.get(0)
                })
                .optional()?
                .ok_or(ApiError::NotFound)?;
            c.execute(
                "UPDATE devices SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id, now_ms()],
            )?;
            log_action(c, &admin.login, &admin.ip, "device.revoke", &id, &org)?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

#[derive(Deserialize)]
pub(crate) struct DataQuery {
    #[serde(default)]
    part: String,
    #[serde(default)]
    q: String,
}

/// What is inside a shop, read only.
pub(crate) async fn data(
    State(server): State<Arc<Server>>,
    _admin: Admin,
    Path(id): Path<String>,
    Query(q): Query<DataQuery>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    let srv = server.clone();
    crate::blocking(move || {
        known_org(&*lock(&srv.control)?, &id)?;
        if !srv.tenant_path(&id).exists() {
            return Err(ApiError::NotFound);
        }
        let since = now_ms() - 30 * DAY;
        let search = q.q.trim().to_owned();
        srv.with_tenant(&id, |store| {
            let out = store.read(|r| {
                let c = r.conn();
                let (currency, exp): (String, i64) = c.query_row(
                    "SELECT currency, currency_exp FROM organizations LIMIT 1",
                    [],
                    |x| Ok((x.get(0)?, x.get(1)?)),
                )?;
                let rows = |sql: &str, p: &[&dyn rusqlite::ToSql], cols: &[&str]| {
                    let mut st = c.prepare(sql)?;
                    let n = cols.len();
                    let list = st
                        .query_map(p, |x| {
                            let mut m = serde_json::Map::new();
                            for (i, col) in cols.iter().enumerate().take(n) {
                                let v: rusqlite::types::Value = x.get(i)?;
                                let v = match v {
                                    rusqlite::types::Value::Integer(i) => serde_json::Value::from(i),
                                    rusqlite::types::Value::Text(t) => serde_json::Value::from(t),
                                    rusqlite::types::Value::Null => serde_json::Value::Null,
                                    rusqlite::types::Value::Real(_)
                                    | rusqlite::types::Value::Blob(_) => serde_json::Value::Null,
                                };
                                m.insert((*col).to_owned(), v);
                            }
                            Ok(serde_json::Value::Object(m))
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok::<_, nb_store::Error>(list)
                };
                let count = |sql: &str| -> nb_store::Result<i64> { Ok(c.query_row(sql, [], |x| x.get(0))?) };
                let body = match q.part.as_str() {
                    "customers" => serde_json::json!({ "rows": rows(
                        "SELECT code, name, phone, city, balance_minor, status FROM customers
                         WHERE (?1 = '' OR name LIKE '%' || ?1 || '%' OR phone LIKE '%' || ?1 || '%' OR code = ?1)
                         ORDER BY name LIMIT 200",
                        &[&search],
                        &["code", "name", "phone", "city", "balance_minor", "status"],
                    )? }),
                    "sales" => serde_json::json!({ "rows": rows(
                        "SELECT s.number, s.created_at, s.total_minor, s.credit_minor, COALESCE(c.name, '') AS customer
                         FROM sales s LEFT JOIN customers c ON c.id = s.customer_id
                         WHERE (?1 = '' OR s.number LIKE '%' || ?1 || '%' OR c.name LIKE '%' || ?1 || '%')
                         ORDER BY s.created_at DESC LIMIT 100",
                        &[&search],
                        &["number", "created_at", "total_minor", "credit_minor", "customer"],
                    )? }),
                    "products" => serde_json::json!({ "rows": rows(
                        "SELECT sku, name, price_minor, active FROM products
                         WHERE (?1 = '' OR name LIKE '%' || ?1 || '%' OR sku = ?1)
                         ORDER BY name LIMIT 200",
                        &[&search],
                        &["sku", "name", "price_minor", "active"],
                    )? }),
                    _ => {
                        let sales_30: (i64, i64) = c.query_row(
                            "SELECT COUNT(*), COALESCE(SUM(total_minor), 0) FROM sales WHERE created_at > ?1",
                            [since],
                            |x| Ok((x.get(0)?, x.get(1)?)),
                        )?;
                        serde_json::json!({
                            "customers": count("SELECT COUNT(*) FROM customers")?,
                            "products": count("SELECT COUNT(*) FROM products")?,
                            "sales": count("SELECT COUNT(*) FROM sales")?,
                            "branches": count("SELECT COUNT(*) FROM branches")?,
                            "users": count("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL")?,
                            "sales_30_count": sales_30.0,
                            "sales_30_total": sales_30.1,
                            "debts_total": count("SELECT COALESCE(SUM(balance_minor), 0) FROM customers WHERE balance_minor > 0")?,
                        })
                    }
                };
                Ok(serde_json::json!({ "currency": currency, "exp": exp, "data": body }))
            })?;
            Ok(axum::Json(out))
        })
    })
    .await
}

#[derive(Serialize)]
struct AccountRow {
    id: String,
    phone: String,
    name: String,
    created_at: i64,
    shops: i64,
    locked_until: Option<i64>,
    disabled: bool,
}

pub(crate) async fn accounts(
    State(server): State<Arc<Server>>,
    _admin: Admin,
    Query(q): Query<ListQuery>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    server
        .control(move |c| {
            let total: i64 = c.query_row(
                "SELECT COUNT(*) FROM accounts
                 WHERE ?1 = '' OR phone LIKE '%' || ?1 || '%' OR name LIKE '%' || ?1 || '%'",
                [q.q.trim()],
                |r| r.get(0),
            )?;
            let mut st = c.prepare(
                "SELECT a.id, a.phone, a.name, a.created_at,
                        (SELECT COUNT(*) FROM memberships m WHERE m.account_id = a.id),
                        a.locked_until, a.disabled_at IS NOT NULL
                 FROM accounts a
                 WHERE ?1 = '' OR a.phone LIKE '%' || ?1 || '%' OR a.name LIKE '%' || ?1 || '%'
                 ORDER BY a.created_at DESC LIMIT ?2 OFFSET ?3",
            )?;
            let rows: Vec<AccountRow> = st
                .query_map(params![q.q.trim(), PAGE, q.page.max(0) * PAGE], |r| {
                    Ok(AccountRow {
                        id: r.get(0)?,
                        phone: r.get(1)?,
                        name: r.get(2)?,
                        created_at: r.get(3)?,
                        shops: r.get(4)?,
                        locked_until: r.get(5)?,
                        disabled: r.get(6)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            Ok(axum::Json(
                serde_json::json!({ "total": total, "page_size": PAGE, "rows": rows }),
            ))
        })
        .await
}

pub(crate) async fn account(
    State(server): State<Arc<Server>>,
    _admin: Admin,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    server
        .control(move |c| {
            let now = now_ms();
            let (phone, name, created, locked, disabled, reset_until): (
                String,
                String,
                i64,
                Option<i64>,
                Option<i64>,
                Option<i64>,
            ) = c
                .query_row(
                    "SELECT phone, name, created_at, locked_until, disabled_at, reset_until
                     FROM accounts WHERE id = ?1",
                    [&id],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                        ))
                    },
                )
                .optional()?
                .ok_or(ApiError::NotFound)?;
            let mut st = c.prepare(
                "SELECT o.id, o.name, m.role FROM memberships m JOIN orgs o ON o.id = m.org_id
                 WHERE m.account_id = ?1 ORDER BY o.name",
            )?;
            let shops: Vec<serde_json::Value> = st
                .query_map([&id], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, String>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "role": r.get::<_, String>(2)?,
                    }))
                })?
                .collect::<Result<_, _>>()?;
            let sessions: i64 = c.query_row(
                "SELECT COUNT(*) FROM sessions WHERE account_id = ?1 AND expires_at > ?2",
                params![id, now],
                |r| r.get(0),
            )?;
            Ok(axum::Json(serde_json::json!({
                "id": id, "phone": phone, "name": name, "created_at": created,
                "locked_until": locked, "disabled_at": disabled,
                "reset_until": reset_until.filter(|t| *t > now),
                "sessions": sessions, "shops": shops,
            })))
        })
        .await
}

/// `disable`, `enable`, `unlock`, `end_sessions`, `reset` (a one-time code
/// the support reads to the owner on the phone: a day, once, in place of a
/// recovery code). Disabling and resetting take a fresh code.
pub(crate) async fn account_action(
    State(server): State<Arc<Server>>,
    admin: Admin,
    Path((id, action)): Path<(String, String)>,
    axum::Json(req): axum::Json<Confirm>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    server
        .control(move |c| {
            let phone: String = c
                .query_row("SELECT phone FROM accounts WHERE id = ?1", [&id], |r| r.get(0))
                .optional()?
                .ok_or(ApiError::NotFound)?;
            let mut out = serde_json::json!({});
            match action.as_str() {
                "disable" => {
                    fresh(c, &admin, &req.code)?;
                    c.execute(
                        "UPDATE accounts SET disabled_at = ?2 WHERE id = ?1",
                        params![id, now_ms()],
                    )?;
                    c.execute("DELETE FROM sessions WHERE account_id = ?1", [&id])?;
                }
                "enable" => {
                    c.execute("UPDATE accounts SET disabled_at = NULL WHERE id = ?1", [&id])?;
                }
                "unlock" => {
                    c.execute(
                        "UPDATE accounts SET failed = 0, locked_until = NULL WHERE id = ?1",
                        [&id],
                    )?;
                }
                "end_sessions" => {
                    c.execute("DELETE FROM sessions WHERE account_id = ?1", [&id])?;
                }
                "reset" => {
                    fresh(c, &admin, &req.code)?;
                    let code = crate::new_code()?;
                    let shown = format!("{}-{}", &code[..4], &code[4..]);
                    c.execute(
                        "UPDATE accounts SET reset_hash = ?2, reset_until = ?3, failed = 0, locked_until = NULL
                         WHERE id = ?1",
                        params![id, hash(&plain_code(&shown)), now_ms() + DAY],
                    )?;
                    out = serde_json::json!({ "code": shown });
                }
                _ => return Err(ApiError::NotFound),
            }
            log_action(c, &admin.login, &admin.ip, &format!("account.{action}"), &id, &phone)?;
            Ok(axum::Json(out))
        })
        .await
}

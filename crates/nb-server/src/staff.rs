//! Employees who sign in from their own phone (ADR-0009): the owner gives
//! a user of the shop an account — a phone and a password the owner sets —
//! from a device of the shop. The employee then signs in like the owner,
//! sees the shop, and works on it as that user, with that user's
//! permissions, and nothing more:
//! - the account belongs to the shop as `staff`, tied to one user;
//! - the user must still be active in the shop when a device is added;
//! - a staff account sees and replaces only the devices it added itself,
//!   and cannot repair (reseed) the shop;
//! - taking the access back ends the membership and stops the devices the
//!   account added.
//!
//! - `GET    /v1/staff`            device token → who has access
//! - `PUT    /v1/staff`            device token; `{user_id, phone, name, password}`
//! - `DELETE /v1/staff/{user_id}`  device token

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::accounts::{check_password, hash_password, normalize_phone};
use crate::{ApiError, ApiResult, Server, blocking, lock, now_ms};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaffAccess {
    pub user_id: String,
    pub phone: String,
    pub name: String,
    pub since: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaffGrant {
    pub user_id: String,
    pub phone: String,
    pub name: String,
    /// The account's password, when the account is new (or only this
    /// shop's employee: the owner sets it again).
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaffGranted {
    pub phone: String,
    /// The phone had an account already, used by the person elsewhere: it
    /// keeps its own password.
    pub existing: bool,
}

impl Server {
    /// Whether `user` is an active, non-owner user of `org`'s shop.
    pub(crate) fn staff_user_ok(&self, org: &str, user: &str) -> ApiResult<bool> {
        let user = user.to_owned();
        self.with_tenant(org, move |store| {
            Ok(store.read(|r| {
                Ok(r.conn()
                    .query_row(
                        "SELECT active = 1 AND deleted_at IS NULL AND role != 'owner'
                         FROM users WHERE id = ?1",
                        [&user],
                        |row| row.get::<_, bool>(0),
                    )
                    .optional()?
                    .unwrap_or(false))
            })?)
        })
    }
}

/// The devices an account added to `org` stop.
fn stop_devices_of(c: &Connection, org: &str, account: &str) -> ApiResult<()> {
    c.execute(
        "UPDATE devices SET revoked_at = ?3
         WHERE org_id = ?1 AND added_by = ?2 AND revoked_at IS NULL",
        params![org, account, now_ms()],
    )?;
    Ok(())
}

pub(crate) async fn list(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Vec<StaffAccess>>> {
    let device = server.managing_device(&headers)?;
    blocking(move || {
        let c = lock(&server.control)?;
        let mut st = c.prepare(
            "SELECT m.user_id, a.phone, a.name, m.created_at FROM memberships m
             JOIN accounts a ON a.id = m.account_id
             WHERE m.org_id = ?1 AND m.role = 'staff' AND m.user_id IS NOT NULL
             ORDER BY a.name",
        )?;
        let rows = st.query_map([&device.org], |r| {
            Ok(StaffAccess {
                user_id: r.get(0)?,
                phone: r.get(1)?,
                name: r.get(2)?,
                since: r.get(3)?,
            })
        })?;
        Ok(axum::Json(rows.collect::<Result<_, _>>()?))
    })
    .await
}

pub(crate) async fn grant(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<StaffGrant>,
) -> ApiResult<axum::Json<StaffGranted>> {
    let device = server.managing_device(&headers)?;
    let phone = normalize_phone(&req.phone).ok_or(ApiError::BadRequest("phone.invalid".into()))?;
    let name = req.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::BadRequest("name.required".into()));
    }
    let id_ok = req.user_id.len() == 36
        && req
            .user_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-');
    if !id_ok {
        return Err(ApiError::BadRequest("staff.user".into()));
    }
    blocking(move || {
        let org = device.org.clone();
        if !server.staff_user_ok(&org, &req.user_id)? {
            return Err(ApiError::BadRequest("staff.user".into()));
        }
        let c = lock(&server.control)?;
        let account: Option<(String, i64)> = c
            .query_row(
                "SELECT a.id, (SELECT COUNT(*) FROM memberships m
                               WHERE m.account_id = a.id AND NOT (m.org_id = ?2 AND m.role = 'staff'))
                 FROM accounts a WHERE a.phone = ?1",
                params![phone, org],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (account, existing) = match account {
            Some((id, elsewhere)) => {
                let role: Option<String> = c
                    .query_row(
                        "SELECT role FROM memberships WHERE account_id = ?1 AND org_id = ?2",
                        params![id, org],
                        |r| r.get(0),
                    )
                    .optional()?;
                if role.as_deref() == Some("owner") {
                    return Err(ApiError::Conflict("staff.is_owner".into()));
                }
                // Only this shop's employee: the owner may set its password
                // again (a forgotten one). An account used elsewhere keeps
                // its own.
                if elsewhere == 0 && !req.password.is_empty() {
                    check_password(&req.password)?;
                    c.execute(
                        "UPDATE accounts SET password_hash = ?2, failed = 0, locked_until = NULL
                         WHERE id = ?1",
                        params![id, hash_password(&req.password)?],
                    )?;
                    c.execute("DELETE FROM sessions WHERE account_id = ?1", [&id])?;
                }
                (id, elsewhere > 0)
            }
            None => {
                check_password(&req.password)?;
                let id = nb_core::OperationId::new().to_string();
                c.execute(
                    "INSERT INTO accounts (id, phone, name, password_hash, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, phone, name, hash_password(&req.password)?, now_ms()],
                )?;
                (id, false)
            }
        };
        // The user's access moves to this phone: another account that had
        // it loses it, and its devices stop.
        let others: Vec<String> = {
            let mut st = c.prepare(
                "SELECT account_id FROM memberships
                 WHERE org_id = ?1 AND role = 'staff' AND user_id = ?2 AND account_id != ?3",
            )?;
            st.query_map(params![org, req.user_id, account], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        for other in others {
            c.execute(
                "DELETE FROM memberships WHERE account_id = ?1 AND org_id = ?2",
                params![other, org],
            )?;
            stop_devices_of(&c, &org, &other)?;
        }
        c.execute(
            "INSERT INTO memberships (account_id, org_id, role, user_id, created_at)
             VALUES (?1, ?2, 'staff', ?3, ?4)
             ON CONFLICT (account_id, org_id) DO UPDATE SET user_id = ?3",
            params![account, org, req.user_id, now_ms()],
        )?;
        Ok(axum::Json(StaffGranted { phone, existing }))
    })
    .await
}

pub(crate) async fn revoke(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    let device = server.managing_device(&headers)?;
    blocking(move || {
        let c = lock(&server.control)?;
        let accounts: Vec<String> = {
            let mut st = c.prepare(
                "SELECT account_id FROM memberships
                 WHERE org_id = ?1 AND role = 'staff' AND user_id = ?2",
            )?;
            st.query_map(params![device.org, user_id], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        for a in accounts {
            c.execute(
                "DELETE FROM memberships WHERE account_id = ?1 AND org_id = ?2",
                params![a, device.org],
            )?;
            stop_devices_of(&c, &device.org, &a)?;
        }
        Ok(axum::http::StatusCode::NO_CONTENT)
    })
    .await
}

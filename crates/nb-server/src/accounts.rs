//! Accounts (ADR-0007): a person — the owner, an accountant, a partner —
//! known by a phone number and a password, with ten recovery codes instead
//! of SMS for now. An account links a device's shop to the server and adds
//! devices to the shops it belongs to; devices never hold the password,
//! they get their own token as before.
//!
//! - `POST /v1/signup`  `{phone, password, name, key?}` → a session and the
//!   recovery codes (shown once). On a private server (no open signup) the
//!   server's key is asked.
//! - `POST /v1/login`   `{phone, password}` → a session (30 days)
//! - `POST /v1/recover` `{phone, code, password}` → a new password, every
//!   other session ended, a session
//! - `GET  /v1/orgs`    account session → its organizations
//! - `POST /v1/orgs/{id}/devices` account session; `{name, device_id?}` →
//!   this device joins that organization (no invitation code)
//! - `POST /v1/link`    also with an account session (the shop becomes the
//!   account's)

use std::sync::Arc;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{
    ApiError, ApiResult, ClientIp, Joined, Server, bearer, blocking, hash, lock, new_token, now_ms,
};
pub use nb_store::sync::{OrgInfo, Session};

/// How long an account session lasts.
const SESSION_MS: i64 = 30 * 24 * 3600 * 1000;
/// Wrong passwords before the account waits.
const TRIES: i64 = 8;
/// How long it waits.
const LOCK_MS: i64 = 15 * 60 * 1000;

pub(crate) fn schema(control: &Connection) -> rusqlite::Result<()> {
    control.execute_batch(
        "CREATE TABLE IF NOT EXISTS accounts (
             id             TEXT PRIMARY KEY,
             phone          TEXT NOT NULL UNIQUE,
             name           TEXT NOT NULL,
             password_hash  TEXT NOT NULL,
             recovery       TEXT NOT NULL DEFAULT '[]',
             failed         INTEGER NOT NULL DEFAULT 0,
             locked_until   INTEGER,
             created_at     INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS sessions (
             token_hash  TEXT PRIMARY KEY,
             account_id  TEXT NOT NULL REFERENCES accounts(id),
             created_at  INTEGER NOT NULL,
             expires_at  INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS memberships (
             account_id  TEXT NOT NULL REFERENCES accounts(id),
             org_id      TEXT NOT NULL REFERENCES orgs(id),
             role        TEXT NOT NULL,
             created_at  INTEGER NOT NULL,
             PRIMARY KEY (account_id, org_id)
         );",
    )
}

/// `0550 12 34 56`, `+213 550123456`, `00213550123456` → `+213550123456`.
/// Other countries keep their `+` and digits. `None`: not a phone number.
pub fn normalize_phone(raw: &str) -> Option<String> {
    let plus = raw.trim_start().starts_with('+') || raw.trim_start().starts_with("00");
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    let digits = if raw.trim_start().starts_with("00") {
        digits[2..].to_owned()
    } else {
        digits
    };
    let full = if plus {
        digits
    } else if digits.len() == 10 && digits.starts_with('0') {
        // An Algerian number as people write it.
        format!("213{}", &digits[1..])
    } else {
        return None;
    };
    if full.starts_with("213") && full.len() != 12 {
        return None;
    }
    (8..=15).contains(&full.len()).then(|| format!("+{full}"))
}

pub(crate) fn hash_password(password: &str) -> ApiResult<String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| ApiError::Internal(e.to_string()))?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| ApiError::Internal(e.to_string()))?;
    argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub(crate) fn password_is(stored: &str, password: &str) -> bool {
    PasswordHash::new(stored)
        .map(|h| {
            argon2::Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

fn check_password(password: &str) -> ApiResult<()> {
    if password.chars().count() < 8 {
        return Err(ApiError::BadRequest("password.short".into()));
    }
    Ok(())
}

/// Ten codes like `K7PM-2QXA`: each opens the account once.
fn recovery_codes() -> ApiResult<Vec<String>> {
    (0..10)
        .map(|_| {
            let c = crate::new_code()?;
            Ok(format!("{}-{}", &c[..4], &c[4..]))
        })
        .collect()
}

pub(crate) fn plain_code(code: &str) -> String {
    code.trim().to_uppercase().replace([' ', '-'], "")
}

/// Who an account session is.
#[derive(Debug, Clone)]
pub(crate) struct AccountRow {
    pub id: String,
}

impl Server {
    pub(crate) fn account(&self, headers: &HeaderMap) -> ApiResult<AccountRow> {
        let token = bearer(headers).ok_or(ApiError::Unauthorized)?;
        self.account_of(&token)?.ok_or(ApiError::Unauthorized)
    }

    pub(crate) fn account_of(&self, token: &str) -> ApiResult<Option<AccountRow>> {
        Ok(lock(&self.control)?
            .query_row(
                "SELECT s.account_id FROM sessions s JOIN accounts a ON a.id = s.account_id
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND a.disabled_at IS NULL",
                params![hash(token), now_ms()],
                |r| Ok(AccountRow { id: r.get(0)? }),
            )
            .optional()?)
    }

    fn session_for(&self, control: &Connection, account: &str) -> ApiResult<String> {
        let token = new_token()?;
        let now = now_ms();
        control.execute(
            "INSERT INTO sessions (token_hash, account_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![hash(&token), account, now, now + SESSION_MS],
        )?;
        Ok(token)
    }

    pub(crate) fn member(&self, account: &str, org: &str) -> ApiResult<bool> {
        Ok(lock(&self.control)?.query_row(
            "SELECT EXISTS (SELECT 1 FROM memberships WHERE account_id = ?1 AND org_id = ?2)",
            params![account, org],
            |r| r.get(0),
        )?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignupRequest {
    pub phone: String,
    pub password: String,
    pub name: String,
    /// The server's key, on a private server.
    #[serde(default)]
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub phone: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverRequest {
    pub phone: String,
    pub code: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewDevice {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub device_id: Option<String>,
    /// This (reinstalled or new) device takes the place of that device of
    /// the shop: its letter and identity, and its till. The old one's token
    /// stops at once (what it never sent is lost with it).
    #[serde(default)]
    pub replaces: Option<String>,
}

pub(crate) async fn signup(
    State(server): State<Arc<Server>>,
    axum::Json(req): axum::Json<SignupRequest>,
) -> ApiResult<axum::Json<Session>> {
    if !server.open_signup && hash(req.key.trim()) != hash(&server.enrollment_key) {
        return Err(ApiError::Unauthorized);
    }
    let phone = normalize_phone(&req.phone).ok_or(ApiError::BadRequest("phone.invalid".into()))?;
    check_password(&req.password)?;
    let name = req.name.trim().to_owned();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name.required".into()));
    }
    blocking(move || {
        let password_hash = hash_password(&req.password)?;
        let codes = recovery_codes()?;
        let stored: Vec<String> = codes.iter().map(|c| hash(&plain_code(c))).collect();
        let control = lock(&server.control)?;
        let taken: bool = control.query_row(
            "SELECT EXISTS (SELECT 1 FROM accounts WHERE phone = ?1)",
            [&phone],
            |r| r.get(0),
        )?;
        if taken {
            return Err(ApiError::Conflict("phone.taken".into()));
        }
        let id = nb_core::OperationId::new().to_string();
        control.execute(
            "INSERT INTO accounts (id, phone, name, password_hash, recovery, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                phone,
                name,
                password_hash,
                serde_json::to_string(&stored).map_err(|e| ApiError::Internal(e.to_string()))?,
                now_ms()
            ],
        )?;
        let token = server.session_for(&control, &id)?;
        Ok(axum::Json(Session {
            token,
            name,
            phone,
            recovery_codes: codes,
        }))
    })
    .await
}

pub(crate) async fn login(
    State(server): State<Arc<Server>>,
    axum::Extension(ClientIp(ip)): axum::Extension<ClientIp>,
    axum::Json(req): axum::Json<LoginRequest>,
) -> ApiResult<axum::Json<Session>> {
    let phone = normalize_phone(&req.phone).ok_or(ApiError::Unauthorized)?;
    blocking(move || {
        let control = lock(&server.control)?;
        type Row = (String, String, String, i64, Option<i64>, Option<i64>);
        let row: Option<Row> = control
            .query_row(
                "SELECT id, name, password_hash, failed, locked_until, disabled_at
                 FROM accounts WHERE phone = ?1",
                [&phone],
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
            .optional()?;
        let Some((id, name, stored, failed, locked_until, None)) = row else {
            // As long as a wrong password, so a phone's presence (or a
            // stopped account) is not told.
            let _ = hash_password(&req.password);
            crate::admin::event_on(&control, "account.login_failed", &ip, &phone, "");
            return Err(ApiError::Unauthorized);
        };
        if locked_until.is_some_and(|t| t > now_ms()) {
            return Err(ApiError::Conflict("account.locked".into()));
        }
        if !password_is(&stored, &req.password) {
            let failed = failed + 1;
            let lock_until = (failed >= TRIES).then(|| now_ms() + LOCK_MS);
            control.execute(
                "UPDATE accounts SET failed = ?2, locked_until = ?3 WHERE id = ?1",
                params![
                    id,
                    if lock_until.is_some() { 0 } else { failed },
                    lock_until
                ],
            )?;
            crate::admin::event_on(&control, "account.login_failed", &ip, &phone, "");
            if lock_until.is_some() {
                crate::admin::event_on(&control, "account.locked", &ip, &phone, "");
            }
            return Err(ApiError::Unauthorized);
        }
        control.execute(
            "UPDATE accounts SET failed = 0, locked_until = NULL WHERE id = ?1",
            [&id],
        )?;
        let token = server.session_for(&control, &id)?;
        Ok(axum::Json(Session {
            token,
            name,
            phone,
            recovery_codes: Vec::new(),
        }))
    })
    .await
}

pub(crate) async fn recover(
    State(server): State<Arc<Server>>,
    axum::Extension(ClientIp(ip)): axum::Extension<ClientIp>,
    axum::Json(req): axum::Json<RecoverRequest>,
) -> ApiResult<axum::Json<Session>> {
    let phone = normalize_phone(&req.phone).ok_or(ApiError::Unauthorized)?;
    check_password(&req.password)?;
    blocking(move || {
        let control = lock(&server.control)?;
        type Row = (
            String,
            String,
            String,
            Option<i64>,
            Option<String>,
            Option<i64>,
        );
        let row: Option<Row> = control
            .query_row(
                "SELECT id, name, recovery, locked_until, reset_hash, reset_until
                 FROM accounts WHERE phone = ?1 AND disabled_at IS NULL",
                [&phone],
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
            .optional()?;
        let Some((id, name, recovery, locked_until, reset_hash, reset_until)) = row else {
            crate::admin::event_on(&control, "account.recover_failed", &ip, &phone, "");
            return Err(ApiError::Unauthorized);
        };
        if locked_until.is_some_and(|t| t > now_ms()) {
            return Err(ApiError::Conflict("account.locked".into()));
        }
        let mut codes: Vec<String> = serde_json::from_str(&recovery).unwrap_or_default();
        let wanted = hash(&plain_code(&req.code));
        // A code the platform's support gave (a day, once), or one of the ten.
        let by_support = reset_hash.as_deref() == Some(wanted.as_str())
            && reset_until.is_some_and(|t| t > now_ms());
        let found = codes.iter().position(|c| *c == wanted);
        if !by_support && found.is_none() {
            control.execute(
                "UPDATE accounts SET failed = failed + 1,
                     locked_until = CASE WHEN failed + 1 >= ?2 THEN ?3 END
                 WHERE id = ?1",
                params![id, TRIES, now_ms() + LOCK_MS],
            )?;
            crate::admin::event_on(&control, "account.recover_failed", &ip, &phone, "");
            return Err(ApiError::Unauthorized);
        }
        if let Some(at) = found
            && !by_support
        {
            codes.remove(at);
        }
        control.execute(
            "UPDATE accounts SET reset_hash = NULL, reset_until = NULL WHERE id = ?1",
            [&id],
        )?;
        control.execute(
            "UPDATE accounts SET password_hash = ?2, recovery = ?3, failed = 0, locked_until = NULL
             WHERE id = ?1",
            params![
                id,
                hash_password(&req.password)?,
                serde_json::to_string(&codes).map_err(|e| ApiError::Internal(e.to_string()))?
            ],
        )?;
        // Whoever had the old password is out.
        control.execute("DELETE FROM sessions WHERE account_id = ?1", [&id])?;
        let token = server.session_for(&control, &id)?;
        Ok(axum::Json(Session {
            token,
            name,
            phone,
            recovery_codes: Vec::new(),
        }))
    })
    .await
}

pub(crate) async fn orgs(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Vec<OrgInfo>>> {
    let account = server.account(&headers)?;
    let control = lock(&server.control)?;
    let mut st = control.prepare(
        "SELECT o.id, o.name, m.role,
                (SELECT COUNT(*) FROM devices d WHERE d.org_id = o.id AND d.revoked_at IS NULL)
         FROM memberships m JOIN orgs o ON o.id = m.org_id
         WHERE m.account_id = ?1 ORDER BY o.name, o.id",
    )?;
    let rows = st.query_map([&account.id], |r| {
        Ok(OrgInfo {
            org_id: r.get(0)?,
            name: r.get(1)?,
            role: r.get(2)?,
            devices: r.get(3)?,
        })
    })?;
    Ok(axum::Json(rows.collect::<Result<_, _>>()?))
}

/// A device of the account's organization, without an invitation code.
pub(crate) async fn add_device(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(org): Path<String>,
    axum::Json(req): axum::Json<NewDevice>,
) -> ApiResult<axum::Json<Joined>> {
    let account = server.account(&headers)?;
    if !server.member(&account.id, &org)? {
        // Another account's shop is not even said to exist.
        return Err(ApiError::Unauthorized);
    }
    match req.replaces {
        Some(old) => blocking(move || server.replace_device(&org, &old, &req.name)).await,
        None => {
            blocking(move || server.new_device(&org, &req.name, req.device_id.as_deref())).await
        }
    }
}

/// The account's shop's devices (to pick the one a reinstall replaces).
pub(crate) async fn org_devices(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(org): Path<String>,
) -> ApiResult<axum::Json<Vec<crate::DeviceInfo>>> {
    let account = server.account(&headers)?;
    if !server.member(&account.id, &org)? {
        return Err(ApiError::Unauthorized);
    }
    Ok(axum::Json(server.devices_of(&org, "")?))
}

/// Makes `org` the account's (it linked it).
pub(crate) fn own(control: &Connection, account: &str, org: &str) -> ApiResult<()> {
    control.execute(
        "INSERT OR IGNORE INTO memberships (account_id, org_id, role, created_at) VALUES (?1, ?2, 'owner', ?3)",
        params![account, org, now_ms()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalize_phone;

    #[test]
    fn phones_as_people_write_them() {
        for raw in [
            "0550 12 34 56",
            "0550123456",
            "+213 550 12 34 56",
            "00213550123456",
        ] {
            assert_eq!(
                normalize_phone(raw).as_deref(),
                Some("+213550123456"),
                "{raw}"
            );
        }
        assert_eq!(
            normalize_phone("+33 6 12 34 56 78").as_deref(),
            Some("+33612345678")
        );
        assert_eq!(normalize_phone("550123456"), None);
        assert_eq!(normalize_phone("+213 55012"), None);
        assert_eq!(normalize_phone("hello"), None);
    }
}

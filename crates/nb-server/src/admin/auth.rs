//! The platform's administrators: made on the server's command line,
//! signing in with a password and an authenticator app's code.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{Admin, Confirm, event_on, log_action, totp};
use crate::accounts::{hash_password, password_is};
use crate::{ApiError, ApiResult, ClientIp, Server, hash, lock, new_token, now_ms};

/// The session cookie.
pub(crate) const COOKIE: &str = "nb_admin";
/// A session ends after this long without a request…
pub(crate) const IDLE_MS: i64 = 30 * 60_000;
/// …and after this long in any case.
pub(crate) const MAX_MS: i64 = 12 * 3_600_000;
/// Wrong tries (password or code) before the administrator waits.
const TRIES: i64 = 5;
/// How long.
const LOCK_MS: i64 = 15 * 60_000;
/// Sign-in attempts per address and minute.
const PER_MINUTE: u32 = 10;
/// The shortest password.
const MIN_PASSWORD: usize = 12;
/// The issuer an authenticator app shows.
const ISSUER: &str = "NSH Business";

/// What an authenticator app needs, shown once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAdmin {
    pub login: String,
    pub secret: String,
    pub uri: String,
    /// The same as a picture (`data:image/svg+xml;base64,…`).
    pub qr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminInfo {
    pub id: String,
    pub login: String,
    pub name: String,
    pub created_at: i64,
    pub disabled: bool,
    pub locked_until: Option<i64>,
}

fn check_login(login: &str) -> ApiResult<String> {
    let login = login.trim().to_ascii_lowercase();
    let ok = (3..=32).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b));
    if ok {
        Ok(login)
    } else {
        Err(ApiError::BadRequest("admin.login_invalid".into()))
    }
}

fn check_password(password: &str) -> ApiResult<()> {
    if password.chars().count() < MIN_PASSWORD || password.len() > 256 {
        return Err(ApiError::BadRequest("admin.password_short".into()));
    }
    Ok(())
}

fn second_factor(login: &str, secret: String) -> NewAdmin {
    let uri = totp::uri(ISSUER, login, &secret);
    let qr = format!(
        "data:image/svg+xml;base64,{}",
        data_encoding::BASE64.encode(totp::qr_svg(&uri).as_bytes())
    );
    NewAdmin {
        login: login.to_owned(),
        secret,
        uri,
        qr,
    }
}

/// Makes an administrator; what the app needs comes back once.
pub fn add_admin(c: &Connection, login: &str, name: &str, password: &str) -> ApiResult<NewAdmin> {
    let login = check_login(login)?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::BadRequest("admin.name_invalid".into()));
    }
    check_password(password)?;
    let taken: bool = c.query_row(
        "SELECT EXISTS (SELECT 1 FROM admins WHERE login = ?1)",
        [&login],
        |r| r.get(0),
    )?;
    if taken {
        return Err(ApiError::Conflict("admin.login_taken".into()));
    }
    let secret = totp::new_secret().map_err(ApiError::Internal)?;
    c.execute(
        "INSERT INTO admins (id, login, name, password_hash, totp_secret, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            new_token()?,
            login,
            name,
            hash_password(password)?,
            secret,
            now_ms()
        ],
    )?;
    Ok(second_factor(&login, secret))
}

fn admin_id(c: &Connection, login: &str) -> ApiResult<String> {
    c.query_row(
        "SELECT id FROM admins WHERE login = ?1",
        [login.trim().to_ascii_lowercase()],
        |r| r.get(0),
    )
    .optional()?
    .ok_or(ApiError::NotFound)
}

/// Every session of an administrator ends.
fn end_sessions(c: &Connection, id: &str) -> ApiResult<()> {
    c.execute("DELETE FROM admin_sessions WHERE admin_id = ?1", [id])?;
    Ok(())
}

pub(crate) fn list_admins(c: &Connection) -> ApiResult<Vec<AdminInfo>> {
    let mut st = c.prepare(
        "SELECT id, login, name, created_at, disabled_at IS NOT NULL, locked_until
         FROM admins ORDER BY login",
    )?;
    let rows = st.query_map([], |r| {
        Ok(AdminInfo {
            id: r.get(0)?,
            login: r.get(1)?,
            name: r.get(2)?,
            created_at: r.get(3)?,
            disabled: r.get(4)?,
            locked_until: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A wrong password or code: one try less; the last one locks and ends
/// the sessions.
fn failed_try(c: &Connection, id: &str, login: &str, ip: &str) -> ApiResult<()> {
    let failed: i64 = c.query_row("SELECT failed FROM admins WHERE id = ?1", [id], |r| {
        r.get(0)
    })?;
    let failed = failed + 1;
    if failed >= TRIES {
        c.execute(
            "UPDATE admins SET failed = 0, locked_until = ?2 WHERE id = ?1",
            params![id, now_ms() + LOCK_MS],
        )?;
        end_sessions(c, id)?;
        event_on(c, "admin.locked", ip, login, "");
    } else {
        c.execute(
            "UPDATE admins SET failed = ?2 WHERE id = ?1",
            params![id, failed],
        )?;
    }
    Ok(())
}

/// A new code from the app, for what cannot be undone.
pub(crate) fn fresh(c: &Connection, admin: &Admin, code: &str) -> ApiResult<()> {
    let (secret, last): (String, i64) = c.query_row(
        "SELECT totp_secret, last_step FROM admins WHERE id = ?1",
        [&admin.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    match totp::check(&secret, code, now_ms(), last) {
        Some(step) => {
            c.execute(
                "UPDATE admins SET last_step = ?2, failed = 0 WHERE id = ?1",
                params![admin.id, step],
            )?;
            Ok(())
        }
        None => {
            event_on(c, "admin.code_refused", &admin.ip, &admin.login, "");
            failed_try(c, &admin.id, &admin.login, &admin.ip)?;
            Err(ApiError::Forbidden("admin.code".into()))
        }
    }
}

/// An Argon2 hash to check against when the login is unknown: as long as a
/// wrong password, so the logins that exist are not told by the time.
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash_password("not anyone's password").unwrap_or_default())
}

/// The session cookie's token, from the `Cookie` header(s).
pub(crate) fn cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.trim().to_owned())
        .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn set_cookie(token: &str, max_age_s: i64) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!(
        "{COOKIE}={token}; Path=/admin; Max-Age={max_age_s}; HttpOnly; Secure; SameSite=Strict"
    ))
    .ok()
}

impl Server {
    /// The administrator of a session: still allowed, from the address it
    /// started at, neither idle too long nor too old.
    pub(crate) fn admin_session(&self, token: &str, ip: &str) -> ApiResult<Admin> {
        let c = lock(&self.control)?;
        let now = now_ms();
        let row: Option<(String, String, String, String, i64, i64)> = c
            .query_row(
                "SELECT a.id, a.login, a.name, s.ip, s.created_at, s.seen_at
                 FROM admin_sessions s JOIN admins a ON a.id = s.admin_id
                 WHERE s.token_hash = ?1 AND a.disabled_at IS NULL
                   AND (a.locked_until IS NULL OR a.locked_until < ?2)",
                params![hash(token), now],
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
        let Some((id, login, name, from, created, seen)) = row else {
            return Err(ApiError::Unauthorized);
        };
        if from != ip || seen + IDLE_MS < now || created + MAX_MS < now {
            c.execute(
                "DELETE FROM admin_sessions WHERE token_hash = ?1",
                [hash(token)],
            )?;
            if from != ip {
                // A cookie carried elsewhere: stolen, most likely.
                event_on(&c, "admin.session_moved", ip, &login, &from);
            }
            return Err(ApiError::Unauthorized);
        }
        c.execute(
            "UPDATE admin_sessions SET seen_at = ?2 WHERE token_hash = ?1",
            params![hash(token), now],
        )?;
        Ok(Admin {
            id,
            login,
            name,
            ip: ip.to_owned(),
        })
    }
}

#[derive(Deserialize)]
pub(crate) struct LoginRequest {
    login: String,
    password: String,
    code: String,
}

#[derive(Serialize)]
struct Me {
    login: String,
    name: String,
}

pub(crate) async fn login(
    State(server): State<Arc<Server>>,
    axum::Extension(ClientIp(ip)): axum::Extension<ClientIp>,
    axum::Json(req): axum::Json<LoginRequest>,
) -> ApiResult<Response> {
    if server.knock(&format!("admin:{ip}"), PER_MINUTE) {
        return Err(ApiError::TooMany);
    }
    if req.password.len() > 256 || req.login.len() > 64 || req.code.len() > 16 {
        return Err(ApiError::Unauthorized);
    }
    let (me, token) = server
        .control(move |c| {
            let login = req.login.trim().to_ascii_lowercase();
            type Row = (String, String, String, String, i64, Option<i64>, Option<i64>);
            let row: Option<Row> = c
                .query_row(
                    "SELECT id, name, password_hash, totp_secret, last_step, locked_until, disabled_at
                     FROM admins WHERE login = ?1",
                    [&login],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                        ))
                    },
                )
                .optional()?;
            let Some((id, name, stored, secret, last, locked_until, None)) = row else {
                let _ = password_is(dummy_hash(), &req.password);
                event_on(c, "admin.login_failed", &ip, &login, "");
                return Err(ApiError::Unauthorized);
            };
            if locked_until.is_some_and(|t| t > now_ms()) {
                let _ = password_is(dummy_hash(), &req.password);
                event_on(c, "admin.login_failed", &ip, &login, "locked");
                return Err(ApiError::Unauthorized);
            }
            // Both are checked, whatever the first gives.
            let password_ok = password_is(&stored, &req.password);
            let step = totp::check(&secret, &req.code, now_ms(), last);
            let Some(step) = step.filter(|_| password_ok) else {
                event_on(
                    c,
                    "admin.login_failed",
                    &ip,
                    &login,
                    if password_ok { "code" } else { "password" },
                );
                failed_try(c, &id, &login, &ip)?;
                return Err(ApiError::Unauthorized);
            };
            c.execute(
                "UPDATE admins SET failed = 0, locked_until = NULL, last_step = ?2 WHERE id = ?1",
                params![id, step],
            )?;
            let token = new_token()?;
            let now = now_ms();
            c.execute(
                "INSERT INTO admin_sessions (token_hash, admin_id, ip, created_at, seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![hash(&token), id, ip, now],
            )?;
            log_action(c, &login, &ip, "login", "", "")?;
            Ok((Me { login, name }, token))
        })
        .await?;
    let mut res = axum::Json(me).into_response();
    if let Some(v) = set_cookie(&token, MAX_MS / 1000) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    Ok(res)
}

pub(crate) async fn logout(State(server): State<Arc<Server>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie(&headers) {
        let _ = server
            .control(move |c| {
                c.execute(
                    "DELETE FROM admin_sessions WHERE token_hash = ?1",
                    [hash(&token)],
                )?;
                Ok(())
            })
            .await;
    }
    let mut res = axum::http::StatusCode::NO_CONTENT.into_response();
    if let Some(v) = set_cookie("", 0) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    res
}

pub(crate) async fn me(admin: Admin) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "login": admin.login, "name": admin.name }))
}

#[derive(Deserialize)]
pub(crate) struct PasswordChange {
    old: String,
    new: String,
    code: String,
}

pub(crate) async fn change_password(
    State(server): State<Arc<Server>>,
    admin: Admin,
    axum::Json(req): axum::Json<PasswordChange>,
) -> ApiResult<axum::http::StatusCode> {
    check_password(&req.new)?;
    server
        .control(move |c| {
            fresh(c, &admin, &req.code)?;
            let stored: String = c.query_row(
                "SELECT password_hash FROM admins WHERE id = ?1",
                [&admin.id],
                |r| r.get(0),
            )?;
            if !password_is(&stored, &req.old) {
                failed_try(c, &admin.id, &admin.login, &admin.ip)?;
                return Err(ApiError::Forbidden("admin.password_wrong".into()));
            }
            c.execute(
                "UPDATE admins SET password_hash = ?2 WHERE id = ?1",
                params![admin.id, hash_password(&req.new)?],
            )?;
            // Other sessions end; this one stays.
            c.execute(
                "DELETE FROM admin_sessions WHERE admin_id = ?1 AND ip != ?2",
                params![admin.id, admin.ip],
            )?;
            log_action(
                c,
                &admin.login,
                &admin.ip,
                "admin.password",
                &admin.login,
                "",
            )?;
            Ok(axum::http::StatusCode::NO_CONTENT)
        })
        .await
}

pub(crate) async fn list(
    State(server): State<Arc<Server>>,
    _admin: Admin,
) -> ApiResult<axum::Json<Vec<AdminInfo>>> {
    Ok(axum::Json(server.control(list_admins).await?))
}

#[derive(Deserialize)]
pub(crate) struct AddRequest {
    login: String,
    name: String,
    password: String,
    code: String,
}

pub(crate) async fn add(
    State(server): State<Arc<Server>>,
    admin: Admin,
    axum::Json(req): axum::Json<AddRequest>,
) -> ApiResult<axum::Json<NewAdmin>> {
    server
        .control(move |c| {
            fresh(c, &admin, &req.code)?;
            let made = add_admin(c, &req.login, &req.name, &req.password)?;
            log_action(c, &admin.login, &admin.ip, "admin.add", &made.login, "")?;
            Ok(axum::Json(made))
        })
        .await
}

pub(crate) async fn disable(
    State(server): State<Arc<Server>>,
    admin: Admin,
    UrlPath(id): UrlPath<String>,
    axum::Json(req): axum::Json<Confirm>,
) -> ApiResult<axum::http::StatusCode> {
    if id == admin.id {
        return Err(ApiError::BadRequest("admin.not_yourself".into()));
    }
    server
        .control(move |c| {
            fresh(c, &admin, &req.code)?;
            let login: String = c
                .query_row("SELECT login FROM admins WHERE id = ?1", [&id], |r| {
                    r.get(0)
                })
                .optional()?
                .ok_or(ApiError::NotFound)?;
            c.execute(
                "UPDATE admins SET disabled_at = ?2 WHERE id = ?1",
                params![id, now_ms()],
            )?;
            end_sessions(c, &id)?;
            log_action(c, &admin.login, &admin.ip, "admin.disable", &login, "")?;
            Ok(axum::http::StatusCode::NO_CONTENT)
        })
        .await
}

/// `nb-server admin …`: the administrators, from the server's own command
/// line (whoever runs it holds the server already). `password` is asked
/// only by the commands that set one.
pub fn cli(
    dir: &Path,
    args: &[String],
    password: impl FnOnce() -> Option<String>,
) -> Result<String, String> {
    let c = crate::control_db(dir).map_err(|e| format!("{e:?}"))?;
    let err = |e: ApiError| match e {
        ApiError::BadRequest(m) | ApiError::Conflict(m) => m,
        ApiError::NotFound => "no such administrator".to_owned(),
        other => format!("{other:?}"),
    };
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    let shown = |n: &NewAdmin| {
        format!(
            "Scan this with the authenticator app (Google Authenticator, Aegis, …):\n\n{}\n\
             Or type the key: {}\n({})\n",
            totp::qr_text(&n.uri),
            n.secret,
            n.uri
        )
    };
    match arg(0) {
        "add" => {
            let name = args.get(2..).map(|r| r.join(" ")).unwrap_or_default();
            let pw = password().ok_or("a password is needed")?;
            let made = add_admin(&c, arg(1), &name, &pw).map_err(err)?;
            log_action(&c, "cli", "local", "admin.add", &made.login, "").map_err(err)?;
            Ok(format!(
                "Administrator {} made.\n{}",
                made.login,
                shown(&made)
            ))
        }
        "list" => {
            let rows = list_admins(&c).map_err(err)?;
            Ok(rows
                .iter()
                .map(|a| {
                    format!(
                        "{}\t{}\t{}",
                        a.login,
                        a.name,
                        if a.disabled { "disabled" } else { "active" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "reset-2fa" => {
            let id = admin_id(&c, arg(1)).map_err(err)?;
            let secret = totp::new_secret()?;
            c.execute(
                "UPDATE admins SET totp_secret = ?2, last_step = 0 WHERE id = ?1",
                params![id, secret],
            )
            .map_err(|e| e.to_string())?;
            end_sessions(&c, &id).map_err(err)?;
            log_action(&c, "cli", "local", "admin.reset_2fa", arg(1), "").map_err(err)?;
            Ok(shown(&second_factor(&arg(1).to_ascii_lowercase(), secret)))
        }
        "password" => {
            let id = admin_id(&c, arg(1)).map_err(err)?;
            let pw = password().ok_or("a password is needed")?;
            check_password(&pw).map_err(err)?;
            c.execute(
                "UPDATE admins SET password_hash = ?2 WHERE id = ?1",
                params![id, hash_password(&pw).map_err(err)?],
            )
            .map_err(|e| e.to_string())?;
            end_sessions(&c, &id).map_err(err)?;
            log_action(&c, "cli", "local", "admin.password", arg(1), "").map_err(err)?;
            Ok("Password changed; its sessions ended.".to_owned())
        }
        "disable" | "enable" | "unlock" => {
            let id = admin_id(&c, arg(1)).map_err(err)?;
            let sql = match arg(0) {
                "disable" => "UPDATE admins SET disabled_at = ?2 WHERE id = ?1",
                "enable" => "UPDATE admins SET disabled_at = NULL WHERE id = ?1 AND ?2 > 0",
                _ => "UPDATE admins SET locked_until = NULL, failed = 0 WHERE id = ?1 AND ?2 > 0",
            };
            c.execute(sql, params![id, now_ms()])
                .map_err(|e| e.to_string())?;
            if arg(0) == "disable" {
                end_sessions(&c, &id).map_err(err)?;
            }
            log_action(&c, "cli", "local", &format!("admin.{}", arg(0)), arg(1), "")
                .map_err(err)?;
            Ok("Done.".to_owned())
        }
        _ => Err(
            "usage: nb-server admin add <login> <name> | list | reset-2fa <login> | \
                  password <login> | disable <login> | enable <login> | unlock <login>"
                .to_owned(),
        ),
    }
}

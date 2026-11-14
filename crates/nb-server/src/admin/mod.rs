//! The platform's administration (ADR-0008): the operator of a server that
//! sells the service sees the shops, their accounts and devices, their
//! subscriptions and payments, what attacks the server, and the server
//! itself; and stops, lifts, extends, restores.
//!
//! It is a separate door (`admin_router`), on its own address: by default
//! this machine only (`127.0.0.1:8789`), reached through an SSH tunnel or a
//! proxy that lets in the operator's addresses only. Who comes in:
//! - an administrator made on the server's command line (`nb-server admin
//!   add`), never a default one;
//! - with a password (Argon2id) **and** a code from an authenticator app
//!   (TOTP), a code working once;
//! - five wrong tries lock the administrator a quarter of an hour, and an
//!   address knocking more than ten times a minute waits;
//! - the session is a random token in a cookie the page's scripts cannot
//!   read (`HttpOnly`), sent to this door only (`SameSite=Strict`,
//!   `Path=/admin`), tied to the address it started from, ended after 30
//!   minutes idle or 12 hours;
//! - every change asks for a header no other site can send without the
//!   server's leave (`x-nb-admin`), and from the same origin;
//! - what cannot be undone (deleting a shop, restoring a copy, adding an
//!   administrator) asks for a new code from the app;
//! - every change is written in a log each entry of which seals the one
//!   before it (a hash chain), in a table that refuses changes.
//!
//! The page is three files in the program (no other file is ever served),
//! with a strict content policy: no script but its own, no inline code.
//! Its words come from `nb-i18n` (`admin.*`).

mod auth;
mod billing;
mod shops;
mod system;
pub mod totp;

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{ApiError, ApiResult, ClientIp, Server, client_address, hash, lock, now_ms};
pub use auth::{AdminInfo, NewAdmin, cli};

/// A day, in milliseconds.
pub(crate) const DAY: i64 = 86_400_000;

pub(crate) fn schema(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS admins (
             id            TEXT PRIMARY KEY,
             login         TEXT NOT NULL UNIQUE,
             name          TEXT NOT NULL,
             password_hash TEXT NOT NULL,
             totp_secret   TEXT NOT NULL,
             last_step     INTEGER NOT NULL DEFAULT 0,
             failed        INTEGER NOT NULL DEFAULT 0,
             locked_until  INTEGER,
             disabled_at   INTEGER,
             created_at    INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS admin_sessions (
             token_hash  TEXT PRIMARY KEY,
             admin_id    TEXT NOT NULL REFERENCES admins(id),
             ip          TEXT NOT NULL,
             created_at  INTEGER NOT NULL,
             seen_at     INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS admin_log (
             seq     INTEGER PRIMARY KEY AUTOINCREMENT,
             at      INTEGER NOT NULL,
             admin   TEXT NOT NULL,
             ip      TEXT NOT NULL,
             action  TEXT NOT NULL,
             target  TEXT NOT NULL,
             detail  TEXT NOT NULL,
             prev    TEXT NOT NULL,
             hash    TEXT NOT NULL
         );
         CREATE TRIGGER IF NOT EXISTS admin_log_kept BEFORE UPDATE ON admin_log
         BEGIN SELECT RAISE(ABORT, 'the administration log is append-only'); END;
         CREATE TRIGGER IF NOT EXISTS admin_log_not_deleted BEFORE DELETE ON admin_log
         BEGIN SELECT RAISE(ABORT, 'the administration log is append-only'); END;
         CREATE TABLE IF NOT EXISTS security_events (
             id       INTEGER PRIMARY KEY AUTOINCREMENT,
             at       INTEGER NOT NULL,
             kind     TEXT NOT NULL,
             ip       TEXT NOT NULL,
             subject  TEXT NOT NULL DEFAULT '',
             detail   TEXT NOT NULL DEFAULT ''
         );
         CREATE INDEX IF NOT EXISTS security_events_at ON security_events (at);
         CREATE TABLE IF NOT EXISTS blocked_ips (
             ip          TEXT PRIMARY KEY,
             reason      TEXT NOT NULL,
             until       INTEGER,
             created_at  INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS plans (
             code          TEXT PRIMARY KEY,
             name          TEXT NOT NULL,
             price_minor   INTEGER NOT NULL CHECK (price_minor >= 0),
             currency      TEXT NOT NULL,
             months        INTEGER NOT NULL CHECK (months > 0),
             max_devices   INTEGER CHECK (max_devices > 0),
             max_branches  INTEGER CHECK (max_branches > 0),
             active        INTEGER NOT NULL DEFAULT 1,
             created_at    INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS subscriptions (
             org_id      TEXT PRIMARY KEY,
             plan        TEXT,
             status      TEXT NOT NULL CHECK (status IN ('trial', 'active', 'cancelled')),
             ends_at     INTEGER,
             note        TEXT NOT NULL DEFAULT '',
             updated_at  INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS payments (
             id            TEXT PRIMARY KEY,
             org_id        TEXT NOT NULL,
             plan          TEXT NOT NULL,
             amount_minor  INTEGER NOT NULL CHECK (amount_minor >= 0),
             currency      TEXT NOT NULL,
             method        TEXT NOT NULL,
             reference     TEXT NOT NULL,
             months        INTEGER NOT NULL CHECK (months > 0),
             admin         TEXT NOT NULL,
             at            INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS platform_settings (
             key    TEXT PRIMARY KEY,
             value  TEXT NOT NULL
         );",
    )?;
    add_column(c, "accounts", "disabled_at", "INTEGER")?;
    add_column(c, "accounts", "reset_hash", "TEXT")?;
    add_column(c, "accounts", "reset_until", "INTEGER")?;
    add_column(c, "orgs", "suspended_at", "INTEGER")?;
    add_column(c, "memberships", "user_id", "TEXT")?;
    add_column(c, "devices", "added_by", "TEXT")?;
    add_column(c, "orgs", "suspend_reason", "TEXT NOT NULL DEFAULT ''")?;
    Ok(())
}

/// `table` and `column` come from this file, never from a request.
fn add_column(c: &Connection, table: &str, column: &str, decl: &str) -> rusqlite::Result<()> {
    let has: bool = c.query_row(
        "SELECT EXISTS (SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        params![table, column],
        |r| r.get(0),
    )?;
    if !has {
        c.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Addresses and networks

/// An address, or a network (`10.0.0.0/8`, `2001:db8::/32`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Net {
    addr: IpAddr,
    bits: u8,
}

impl std::str::FromStr for Net {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        let (a, bits) = match s.trim().split_once('/') {
            Some((a, b)) => (a, Some(b.parse::<u8>().map_err(|_| ())?)),
            None => (s.trim(), None),
        };
        let addr: IpAddr = a.parse().map_err(|_| ())?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let bits = bits.unwrap_or(max);
        if bits > max {
            return Err(());
        }
        Ok(Net { addr, bits })
    }
}

impl Net {
    pub fn contains(&self, ip: &str) -> bool {
        let Ok(ip) = ip.parse::<IpAddr>() else {
            return false;
        };
        // An IPv4 address seen through IPv6 (`::ffff:a.b.c.d`) is itself.
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
            v4 => v4,
        };
        let (a, b, width) = match (self.addr, ip) {
            (IpAddr::V4(n), IpAddr::V4(i)) => {
                (u128::from(u32::from(n)), u128::from(u32::from(i)), 32u32)
            }
            (IpAddr::V6(n), IpAddr::V6(i)) => (u128::from(n), u128::from(i), 128),
            _ => return false,
        };
        let bits = u32::from(self.bits);
        if bits == 0 {
            return true;
        }
        let shift = width - bits;
        (a >> shift) == (b >> shift)
    }
}

impl Server {
    fn admin_ip_allowed(&self, ip: &str) -> bool {
        self.admin_allow.is_empty() || self.admin_allow.iter().any(|n| n.contains(ip))
    }

    /// Whether `ip` is refused everywhere (a block that ended is lifted).
    pub(crate) fn is_blocked(&self, ip: &str) -> bool {
        let Ok(mut map) = self.blocked.lock() else {
            return false;
        };
        match map.get(ip) {
            None => false,
            Some(None) => true,
            Some(Some(until)) if *until > now_ms() => true,
            Some(Some(_)) => {
                map.remove(ip);
                false
            }
        }
    }
}

pub(crate) fn load_blocks(c: &Connection) -> ApiResult<HashMap<String, Option<i64>>> {
    let mut st = c.prepare("SELECT ip, until FROM blocked_ips")?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ---------------------------------------------------------------------------
// The security log and the administration log

fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Writes what happened in the security log (the registry already open).
pub(crate) fn event_on(c: &Connection, kind: &str, ip: &str, subject: &str, detail: &str) {
    let _ = c.execute(
        "INSERT INTO security_events (at, kind, ip, subject, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            now_ms(),
            kind,
            cut(ip, 64),
            cut(subject, 200),
            cut(detail, 500)
        ],
    );
}

impl Server {
    /// Writes what happened in the security log.
    pub(crate) fn event(&self, kind: &str, ip: &str, subject: &str, detail: &str) {
        if let Ok(c) = self.control.lock() {
            event_on(&c, kind, ip, subject, detail);
        }
    }

    /// Forgets security events older than `keep_days`, and sessions that
    /// ended (run each hour with the backups).
    pub fn tidy(&self, now: i64, keep_days: i64) -> ApiResult<()> {
        let c = lock(&self.control)?;
        c.execute(
            "DELETE FROM security_events WHERE at < ?1",
            [now - keep_days * DAY],
        )?;
        c.execute(
            "DELETE FROM admin_sessions WHERE seen_at < ?1 OR created_at < ?2",
            params![now - auth::IDLE_MS, now - auth::MAX_MS],
        )?;
        c.execute("DELETE FROM sessions WHERE expires_at < ?1", [now])?;
        c.execute(
            "DELETE FROM blocked_ips WHERE until IS NOT NULL AND until < ?1",
            [now],
        )?;
        Ok(())
    }
}

fn chain(
    prev: &str,
    at: i64,
    admin: &str,
    ip: &str,
    action: &str,
    target: &str,
    detail: &str,
) -> String {
    let sep = '\u{1f}';
    hash(&format!(
        "{prev}{sep}{at}{sep}{admin}{sep}{ip}{sep}{action}{sep}{target}{sep}{detail}"
    ))
}

/// Writes a change an administrator made; each entry seals the one before.
pub(crate) fn log_action(
    c: &Connection,
    admin: &str,
    ip: &str,
    action: &str,
    target: &str,
    detail: &str,
) -> ApiResult<()> {
    let prev: String = c
        .query_row(
            "SELECT hash FROM admin_log ORDER BY seq DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_else(|| "genesis".to_owned());
    let at = now_ms();
    let detail = cut(detail, 1000);
    let h = chain(&prev, at, admin, ip, action, target, &detail);
    c.execute(
        "INSERT INTO admin_log (at, admin, ip, action, target, detail, prev, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![at, admin, ip, action, target, detail, prev, h],
    )?;
    Ok(())
}

/// The first entry of the log that does not follow from the ones before
/// (`None`: the whole log holds).
pub(crate) fn broken_link(c: &Connection) -> ApiResult<Option<i64>> {
    let mut st = c.prepare(
        "SELECT seq, at, admin, ip, action, target, detail, prev, hash FROM admin_log ORDER BY seq",
    )?;
    let mut rows = st.query([])?;
    let mut prev = "genesis".to_owned();
    while let Some(r) = rows.next()? {
        let seq: i64 = r.get(0)?;
        let at: i64 = r.get(1)?;
        let (admin, ip, action, target, detail, p, h): (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
        ) = (
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
        );
        if p != prev || chain(&p, at, &admin, &ip, &action, &target, &detail) != h {
            return Ok(Some(seq));
        }
        prev = h;
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// The door

/// The administration's routes (served on their own address).
pub fn admin_router(server: Arc<Server>) -> Router {
    let api = Router::new()
        .route("/strings", get(strings))
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/password", post(auth::change_password))
        .route("/admins", get(auth::list).post(auth::add))
        .route("/admins/{id}/disable", post(auth::disable))
        .route("/overview", get(shops::overview))
        .route("/orgs", get(shops::orgs))
        .route("/orgs/{id}", get(shops::org))
        .route("/orgs/{id}/suspend", post(shops::suspend))
        .route("/orgs/{id}/resume", post(shops::resume))
        .route("/orgs/{id}/delete", post(shops::delete))
        .route("/orgs/{id}/data", get(shops::data))
        .route("/orgs/{id}/subscription", post(billing::set_subscription))
        .route("/orgs/{id}/payments", post(billing::add_payment))
        .route("/orgs/{id}/backups", get(system::backups))
        .route("/orgs/{id}/backups/run", post(system::backup_now))
        .route("/orgs/{id}/backups/restore", post(system::restore))
        .route("/devices/{id}/revoke", post(shops::revoke_device))
        .route("/accounts", get(shops::accounts))
        .route("/accounts/{id}", get(shops::account))
        .route("/accounts/{id}/{action}", post(shops::account_action))
        .route("/plans", get(billing::plans).post(billing::save_plan))
        .route(
            "/settings",
            get(billing::settings).post(billing::save_settings),
        )
        .route("/payments", get(billing::payments))
        .route("/security", get(system::security))
        .route("/blocks", post(system::block))
        .route("/blocks/remove", post(system::unblock))
        .route("/log", get(system::log))
        .route("/server", get(system::server_info));
    Router::new()
        .route(
            "/admin",
            get(|| async { axum::response::Redirect::to("/admin/") }),
        )
        .route("/admin/", get(page))
        .route("/admin/app.js", get(script))
        .route("/admin/app.css", get(style))
        .nest("/admin/api", api)
        .layer(axum::extract::DefaultBodyLimit::max(crate::MAX_SMALL))
        .layer(axum::middleware::from_fn_with_state(server.clone(), guard))
        .with_state(server)
}

/// Before anything: the address may come in; a change comes from this
/// page (its header, its origin). After: the page's protections.
async fn guard(
    State(server): State<Arc<Server>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let ip = client_address(&req);
    if !server.admin_ip_allowed(&ip) || server.is_blocked(&ip) {
        server.event("admin.address_refused", &ip, "", req.uri().path());
        // Not even said to exist.
        return StatusCode::NOT_FOUND.into_response();
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let marked = req
            .headers()
            .get("x-nb-admin")
            .is_some_and(|v| v.as_bytes() == b"1");
        if !marked || !same_origin(req.headers()) {
            server.event("admin.forged_request", &ip, "", req.uri().path());
            return ApiError::Forbidden("csrf".into()).into_response();
        }
    }
    req.extensions_mut().insert(ClientIp(ip));
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    for (k, v) in [
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
             connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
        ("x-frame-options", "DENY"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
        ),
        (
            "strict-transport-security",
            "max-age=31536000; includeSubDomains",
        ),
    ] {
        if let Ok(v) = HeaderValue::from_str(v) {
            h.insert(k, v);
        }
    }
    res
}

/// The request's `Origin`, when it has one, is this server (its `Host`).
fn same_origin(headers: &axum::http::HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        // Browsers send it with every change; tools may not.
        return true;
    };
    let host = headers
        .get("x-forwarded-host")
        .filter(|_| std::env::var("NB_SERVER_TRUST_PROXY").is_ok_and(|v| v == "1"))
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let bare = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or("");
    !host.is_empty() && bare == host
}

/// The administrator a request comes from (its session cookie).
#[derive(Debug, Clone)]
pub(crate) struct Admin {
    pub id: String,
    pub login: String,
    pub name: String,
    pub ip: String,
}

impl FromRequestParts<Arc<Server>> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, server: &Arc<Server>) -> Result<Self, ApiError> {
        let ip = parts
            .extensions
            .get::<ClientIp>()
            .map(|c| c.0.clone())
            .ok_or(ApiError::Unauthorized)?;
        let token = auth::cookie(&parts.headers).ok_or(ApiError::Unauthorized)?;
        let server = server.clone();
        crate::blocking(move || server.admin_session(&token, &ip)).await
    }
}

/// A change's body: what it is, and when it cannot be undone, a fresh code.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct Confirm {
    #[serde(default)]
    pub code: String,
}

impl Server {
    /// Runs `f` on the registry, alone, off the async threads.
    pub(crate) async fn control<T: Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(&Connection) -> ApiResult<T> + Send + 'static,
    ) -> ApiResult<T> {
        let server = self.clone();
        crate::blocking(move || {
            let c = lock(&server.control)?;
            f(&c)
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// The page

const PAGE: &str = include_str!("ui/index.html");
const SCRIPT: &str = include_str!("ui/app.js");
const STYLE: &str = include_str!("ui/app.css");

async fn page() -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], PAGE).into_response()
}

async fn script() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        SCRIPT,
    )
        .into_response()
}

async fn style() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], STYLE).into_response()
}

#[derive(serde::Deserialize)]
struct LangQuery {
    #[serde(default)]
    lang: String,
}

/// The page's words in a language (`admin.*` and the few it borrows).
async fn strings(Query(q): Query<LangQuery>) -> Response {
    let lang = nb_i18n::Lang::from_tag(&q.lang).unwrap_or(nb_i18n::Lang::Ar);
    let words: serde_json::Map<String, serde_json::Value> = nb_i18n::all_keys()
        .into_iter()
        .filter(|k| k.starts_with("admin."))
        .map(|k| {
            (
                k.to_owned(),
                serde_json::Value::String(nb_i18n::tr(lang, k).into_owned()),
            )
        })
        .collect();
    let rtl = lang.direction() == nb_i18n::Direction::Rtl;
    axum::Json(serde_json::json!({
        "lang": lang.code(),
        "dir": if rtl { "rtl" } else { "ltr" },
        "words": words,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::Net;

    #[test]
    fn networks_hold_their_addresses() {
        let n: Net = "10.0.0.0/8".parse().unwrap_or(Net {
            addr: std::net::Ipv4Addr::UNSPECIFIED.into(),
            bits: 32,
        });
        assert!(n.contains("10.20.30.40"));
        assert!(n.contains("::ffff:10.1.2.3"));
        assert!(!n.contains("11.0.0.1"));
        assert!(!n.contains("not an ip"));
        let one: Result<Net, ()> = "203.0.113.7".parse();
        assert!(one.is_ok_and(|n| n.contains("203.0.113.7") && !n.contains("203.0.113.8")));
        let v6: Result<Net, ()> = "2001:db8::/32".parse();
        assert!(v6.is_ok_and(|n| n.contains("2001:db8:1::5") && !n.contains("2001:db9::1")));
        assert!("10.0.0.0/33".parse::<Net>().is_err());
        assert!("hello".parse::<Net>().is_err());
    }
}

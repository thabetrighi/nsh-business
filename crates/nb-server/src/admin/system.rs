//! What attacks the server, who did what in the administration, the
//! server itself, and the shops' copies.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use rusqlite::params;
use serde::Deserialize;

use super::{Admin, DAY, auth::fresh, broken_link, log_action, shops::known_org};
use crate::{ApiError, ApiResult, Server, lock, now_ms};

#[derive(Deserialize, Default)]
pub(crate) struct EventQuery {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    ip: String,
}

pub(crate) async fn security(
    State(server): State<Arc<Server>>,
    _admin: Admin,
    Query(q): Query<EventQuery>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    server
        .control(move |c| {
            let now = now_ms();
            let mut st = c.prepare(
                "SELECT at, kind, ip, subject, detail FROM security_events
                 WHERE (?1 = '' OR kind = ?1) AND (?2 = '' OR ip = ?2)
                 ORDER BY id DESC LIMIT 300",
            )?;
            let events: Vec<serde_json::Value> = st
                .query_map(params![q.kind, q.ip], |r| {
                    Ok(serde_json::json!({
                        "at": r.get::<_, i64>(0)?,
                        "kind": r.get::<_, String>(1)?,
                        "ip": r.get::<_, String>(2)?,
                        "subject": r.get::<_, String>(3)?,
                        "detail": r.get::<_, String>(4)?,
                    }))
                })?
                .collect::<Result<_, _>>()?;
            let mut st = c.prepare(
                "SELECT ip, COUNT(*), MAX(at) FROM security_events WHERE at > ?1
                 GROUP BY ip ORDER BY 2 DESC LIMIT 15",
            )?;
            let top: Vec<serde_json::Value> = st
                .query_map([now - DAY], |r| {
                    Ok(serde_json::json!({
                        "ip": r.get::<_, String>(0)?,
                        "count": r.get::<_, i64>(1)?,
                        "last": r.get::<_, i64>(2)?,
                    }))
                })?
                .collect::<Result<_, _>>()?;
            let mut st = c.prepare(
                "SELECT ip, reason, until, created_at FROM blocked_ips ORDER BY created_at DESC",
            )?;
            let blocked: Vec<serde_json::Value> = st
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "ip": r.get::<_, String>(0)?,
                        "reason": r.get::<_, String>(1)?,
                        "until": r.get::<_, Option<i64>>(2)?,
                        "created_at": r.get::<_, i64>(3)?,
                    }))
                })?
                .collect::<Result<_, _>>()?;
            Ok(axum::Json(serde_json::json!({
                "events": events,
                "top": top,
                "blocked": blocked,
            })))
        })
        .await
}

#[derive(Deserialize)]
pub(crate) struct BlockRequest {
    ip: String,
    #[serde(default)]
    reason: String,
    /// Hours; none: until lifted.
    hours: Option<i64>,
}

pub(crate) async fn block(
    State(server): State<Arc<Server>>,
    admin: Admin,
    axum::Json(req): axum::Json<BlockRequest>,
) -> ApiResult<StatusCode> {
    let ip = req
        .ip
        .trim()
        .parse::<std::net::IpAddr>()
        .map_err(|_| ApiError::BadRequest("admin.ip_invalid".into()))?
        .to_string();
    if ip == admin.ip {
        return Err(ApiError::BadRequest("admin.not_yourself".into()));
    }
    if req.hours.is_some_and(|h| !(1..=24 * 365).contains(&h)) {
        return Err(ApiError::BadRequest("admin.hours_invalid".into()));
    }
    let until = req.hours.map(|h| now_ms() + h * 3_600_000);
    let srv = server.clone();
    server
        .control(move |c| {
            let reason: String = req.reason.trim().chars().take(200).collect();
            c.execute(
                "INSERT INTO blocked_ips (ip, reason, until, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (ip) DO UPDATE SET reason = ?2, until = ?3",
                params![ip, reason, until, now_ms()],
            )?;
            lock(&srv.blocked)?.insert(ip.clone(), until);
            log_action(c, &admin.login, &admin.ip, "ip.block", &ip, &reason)?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

#[derive(Deserialize)]
pub(crate) struct IpOnly {
    ip: String,
}

pub(crate) async fn unblock(
    State(server): State<Arc<Server>>,
    admin: Admin,
    axum::Json(req): axum::Json<IpOnly>,
) -> ApiResult<StatusCode> {
    let srv = server.clone();
    server
        .control(move |c| {
            let ip = req.ip.trim().to_owned();
            c.execute("DELETE FROM blocked_ips WHERE ip = ?1", [&ip])?;
            lock(&srv.blocked)?.remove(&ip);
            log_action(c, &admin.login, &admin.ip, "ip.unblock", &ip, "")?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

pub(crate) async fn log(
    State(server): State<Arc<Server>>,
    _admin: Admin,
) -> ApiResult<axum::Json<serde_json::Value>> {
    server
        .control(|c| {
            let mut st = c.prepare(
                "SELECT seq, at, admin, ip, action, target, detail FROM admin_log
                 ORDER BY seq DESC LIMIT 300",
            )?;
            let rows: Vec<serde_json::Value> = st
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "seq": r.get::<_, i64>(0)?,
                        "at": r.get::<_, i64>(1)?,
                        "admin": r.get::<_, String>(2)?,
                        "ip": r.get::<_, String>(3)?,
                        "action": r.get::<_, String>(4)?,
                        "target": r.get::<_, String>(5)?,
                        "detail": r.get::<_, String>(6)?,
                    }))
                })?
                .collect::<Result<_, _>>()?;
            Ok(axum::Json(serde_json::json!({
                "rows": rows,
                "broken_at": broken_link(c)?,
            })))
        })
        .await
}

fn dir_size(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

pub(crate) async fn server_info(
    State(server): State<Arc<Server>>,
    _admin: Admin,
) -> ApiResult<axum::Json<serde_json::Value>> {
    let srv = server.clone();
    crate::blocking(move || {
        let dir = srv.dir.clone();
        let open = lock(&srv.tenants)?.len();
        let live: usize = lock(&srv.waves)?.values().map(|w| w.receiver_count()).sum();
        Ok(axum::Json(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "started_at": srv.started_at,
            "now": now_ms(),
            "billing": srv.billing,
            "admin_networks": !srv.admin_allow.is_empty(),
            "open_signup": srv.open_signup,
            "disk_free": fs4::available_space(&dir).ok(),
            "disk_total": fs4::total_space(&dir).ok(),
            "size_tenants": dir_size(&dir.join("tenants")),
            "size_backups": dir_size(&dir.join("backups")),
            "size_deleted": dir_size(&dir.join("deleted")),
            "size_control": dir_size(&dir).saturating_sub(
                dir_size(&dir.join("tenants")) + dir_size(&dir.join("backups")) + dir_size(&dir.join("deleted"))
            ),
            "open_shops": open,
            "live_connections": live,
        })))
    })
    .await
}

/// A copy's name: `yyyy-mm-dd` or `yyyy-mm-dd-hhmm`.
fn copy_name_ok(name: &str) -> bool {
    let b = name.as_bytes();
    let digits =
        |r: std::ops::Range<usize>| b.get(r).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    (b.len() == 10 || b.len() == 15)
        && digits(0..4)
        && b.get(4) == Some(&b'-')
        && digits(5..7)
        && b.get(7) == Some(&b'-')
        && digits(8..10)
        && (b.len() == 10 || (b.get(10) == Some(&b'-') && digits(11..15)))
}

pub(crate) async fn backups(
    State(server): State<Arc<Server>>,
    _admin: Admin,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Vec<serde_json::Value>>> {
    let srv = server.clone();
    crate::blocking(move || {
        known_org(&*lock(&srv.control)?, &id)?;
        let dir = srv.backups_dir(&id);
        let mut list: Vec<(String, u64)> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let stem = name.strip_suffix(".sqlite")?.to_owned();
                        copy_name_ok(&stem)
                            .then(|| (stem, e.metadata().map(|m| m.len()).unwrap_or(0)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        list.sort();
        list.reverse();
        Ok(axum::Json(
            list.into_iter()
                .map(|(name, size)| serde_json::json!({ "name": name, "size": size }))
                .collect(),
        ))
    })
    .await
}

pub(crate) async fn backup_now(
    State(server): State<Arc<Server>>,
    admin: Admin,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    let srv = server.clone();
    crate::blocking(move || {
        known_org(&*lock(&srv.control)?, &id)?;
        let now = now_ms();
        let (y, m, d) = nb_store::time::civil_date(now, 60);
        let mins = (now / 60_000 + 60).rem_euclid(24 * 60);
        let name = format!("{y:04}-{m:02}-{d:02}-{:02}{:02}", mins / 60, mins % 60);
        let made = srv.backup_org(&id, &name)?;
        log_action(
            &*lock(&srv.control)?,
            &admin.login,
            &admin.ip,
            "backup.run",
            &id,
            &name,
        )?;
        Ok(axum::Json(
            serde_json::json!({ "name": name, "made": made }),
        ))
    })
    .await
}

#[derive(Deserialize)]
pub(crate) struct RestoreRequest {
    name: String,
    code: String,
}

/// A shop goes back to one of its copies (after a disaster): the current
/// file is kept aside (`.bak`), and the shop's devices take the copy as a
/// new start (a new epoch), as after a repair.
pub(crate) async fn restore(
    State(server): State<Arc<Server>>,
    admin: Admin,
    Path(id): Path<String>,
    axum::Json(req): axum::Json<RestoreRequest>,
) -> ApiResult<StatusCode> {
    if !copy_name_ok(&req.name) {
        return Err(ApiError::BadRequest("admin.copy_invalid".into()));
    }
    let srv = server.clone();
    crate::blocking(move || {
        {
            let c = lock(&srv.control)?;
            known_org(&c, &id)?;
            fresh(&c, &admin, &req.code)?;
        }
        let file = srv.backups_dir(&id).join(format!("{}.sqlite", req.name));
        let bytes = std::fs::read(&file).map_err(|_| ApiError::NotFound)?;
        {
            let tenant = srv.tenant(&id)?;
            let mut guard = lock(&tenant)?;
            *guard = None;
            srv.install(&bytes, Some(&id))?;
        }
        let c = lock(&srv.control)?;
        c.execute("UPDATE orgs SET epoch = epoch + 1 WHERE id = ?1", [&id])?;
        log_action(
            &c,
            &admin.login,
            &admin.ip,
            "backup.restore",
            &id,
            &req.name,
        )?;
        drop(c);
        srv.ripple(&id, "");
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn copy_names_are_dates_only() {
        assert!(super::copy_name_ok("2026-11-14"));
        assert!(super::copy_name_ok("2026-11-14-0930"));
        for bad in [
            "../control",
            "2026-11-14/../../x",
            "2026-1-14",
            "",
            "2026-11-14-09:30",
        ] {
            assert!(!super::copy_name_ok(bad), "{bad}");
        }
    }
}

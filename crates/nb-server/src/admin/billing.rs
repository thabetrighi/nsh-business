//! Plans, subscriptions and payments (a platform that sells the service).
//! Money is in minor units (`i64`), never floats. A month is 30 days.
//!
//! Enforced only when the server runs with billing on
//! (`NB_SERVER_BILLING=1`): a new shop gets a trial; a shop whose
//! subscription ended, past its grace days, stops syncing (402) and its
//! devices go on selling, unsynced; a plan's device count is a limit.
//! A shop the operator stopped (suspended) is refused in any case.

use std::sync::Arc;

use axum::extract::{Path, State};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{Admin, DAY, log_action};
use crate::{ApiError, ApiResult, Server, new_token, now_ms};

/// Days of trial a new shop gets, and of grace after its end (defaults).
const TRIAL_DAYS: i64 = 30;
const GRACE_DAYS: i64 = 7;
const METHODS: [&str; 6] = ["baridimob", "ccp", "cash", "bank", "card", "other"];

fn setting(c: &Connection, key: &str, default: i64) -> ApiResult<i64> {
    Ok(c.query_row(
        "SELECT value FROM platform_settings WHERE key = ?1",
        [key],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .and_then(|v| v.parse().ok())
    .unwrap_or(default))
}

impl Server {
    /// Whether `org` may sync now (see the module).
    pub(crate) fn gate(&self, c: &Connection, org: &str) -> ApiResult<()> {
        let suspended: Option<i64> = c
            .query_row("SELECT suspended_at FROM orgs WHERE id = ?1", [org], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        if suspended.is_some() {
            return Err(ApiError::Forbidden("org.suspended".into()));
        }
        if !self.billing {
            return Ok(());
        }
        let now = now_ms();
        let sub: Option<(String, Option<i64>)> = c
            .query_row(
                "SELECT status, ends_at FROM subscriptions WHERE org_id = ?1",
                [org],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match sub {
            None => {
                let days = setting(c, "trial_days", TRIAL_DAYS)?;
                c.execute(
                    "INSERT INTO subscriptions (org_id, plan, status, ends_at, updated_at)
                     VALUES (?1, NULL, 'trial', ?2, ?3)",
                    params![org, now + days * DAY, now],
                )?;
                Ok(())
            }
            Some((status, _)) if status == "cancelled" => {
                Err(ApiError::PaymentRequired("subscription.ended".into()))
            }
            Some((_, Some(ends))) if ends + setting(c, "grace_days", GRACE_DAYS)? * DAY < now => {
                Err(ApiError::PaymentRequired("subscription.ended".into()))
            }
            Some(_) => Ok(()),
        }
    }

    /// Whether `org` may have one more device under its plan.
    pub(crate) fn room_for_device(&self, c: &Connection, org: &str) -> ApiResult<()> {
        if !self.billing {
            return Ok(());
        }
        let max: Option<i64> = c
            .query_row(
                "SELECT p.max_devices FROM subscriptions s JOIN plans p ON p.code = s.plan
                 WHERE s.org_id = ?1",
                [org],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let Some(max) = max else {
            return Ok(());
        };
        let used: i64 = c.query_row(
            "SELECT COUNT(*) FROM devices WHERE org_id = ?1 AND revoked_at IS NULL",
            [org],
            |r| r.get(0),
        )?;
        if used >= max {
            return Err(ApiError::Forbidden("plan.devices".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Plan {
    code: String,
    name: String,
    price_minor: i64,
    currency: String,
    months: i64,
    max_devices: Option<i64>,
    max_branches: Option<i64>,
    #[serde(default = "yes")]
    active: bool,
}

fn yes() -> bool {
    true
}

fn plans_of(c: &Connection) -> ApiResult<Vec<Plan>> {
    let mut st = c.prepare(
        "SELECT code, name, price_minor, currency, months, max_devices, max_branches, active
         FROM plans ORDER BY price_minor",
    )?;
    let rows = st.query_map([], |r| {
        Ok(Plan {
            code: r.get(0)?,
            name: r.get(1)?,
            price_minor: r.get(2)?,
            currency: r.get(3)?,
            months: r.get(4)?,
            max_devices: r.get(5)?,
            max_branches: r.get(6)?,
            active: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(crate) async fn plans(
    State(server): State<Arc<Server>>,
    _admin: Admin,
) -> ApiResult<axum::Json<Vec<Plan>>> {
    Ok(axum::Json(server.control(plans_of).await?))
}

fn code_ok(s: &str) -> bool {
    (2..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
}

pub(crate) async fn save_plan(
    State(server): State<Arc<Server>>,
    admin: Admin,
    axum::Json(p): axum::Json<Plan>,
) -> ApiResult<axum::Json<Vec<Plan>>> {
    let ok = code_ok(&p.code)
        && !p.name.trim().is_empty()
        && p.name.chars().count() <= 60
        && p.price_minor >= 0
        && (1..=36).contains(&p.months)
        && p.currency.len() == 3
        && p.currency.bytes().all(|b| b.is_ascii_uppercase())
        && p.max_devices.is_none_or(|n| (1..=1000).contains(&n))
        && p.max_branches.is_none_or(|n| (1..=1000).contains(&n));
    if !ok {
        return Err(ApiError::BadRequest("admin.plan_invalid".into()));
    }
    server
        .control(move |c| {
            c.execute(
                "INSERT INTO plans (code, name, price_minor, currency, months, max_devices, max_branches, active, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (code) DO UPDATE SET name = ?2, price_minor = ?3, currency = ?4, months = ?5,
                     max_devices = ?6, max_branches = ?7, active = ?8",
                params![
                    p.code,
                    p.name.trim(),
                    p.price_minor,
                    p.currency,
                    p.months,
                    p.max_devices,
                    p.max_branches,
                    p.active,
                    now_ms()
                ],
            )?;
            log_action(
                c,
                &admin.login,
                &admin.ip,
                "plan.save",
                &p.code,
                &format!("{} {} / {} months", p.price_minor, p.currency, p.months),
            )?;
            plans_of(c).map(axum::Json)
        })
        .await
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Settings {
    trial_days: i64,
    grace_days: i64,
}

pub(crate) async fn settings(
    State(server): State<Arc<Server>>,
    _admin: Admin,
) -> ApiResult<axum::Json<serde_json::Value>> {
    let billing = server.billing;
    server
        .control(move |c| {
            Ok(axum::Json(serde_json::json!({
                "billing": billing,
                "trial_days": setting(c, "trial_days", TRIAL_DAYS)?,
                "grace_days": setting(c, "grace_days", GRACE_DAYS)?,
            })))
        })
        .await
}

pub(crate) async fn save_settings(
    State(server): State<Arc<Server>>,
    admin: Admin,
    axum::Json(s): axum::Json<Settings>,
) -> ApiResult<axum::http::StatusCode> {
    if !(0..=365).contains(&s.trial_days) || !(0..=90).contains(&s.grace_days) {
        return Err(ApiError::BadRequest("admin.settings_invalid".into()));
    }
    server
        .control(move |c| {
            for (k, v) in [("trial_days", s.trial_days), ("grace_days", s.grace_days)] {
                c.execute(
                    "INSERT INTO platform_settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT (key) DO UPDATE SET value = ?2",
                    params![k, v.to_string()],
                )?;
            }
            log_action(
                c,
                &admin.login,
                &admin.ip,
                "settings.save",
                "",
                &format!("trial {} / grace {}", s.trial_days, s.grace_days),
            )?;
            Ok(axum::http::StatusCode::NO_CONTENT)
        })
        .await
}

#[derive(Deserialize)]
pub(crate) struct SubscriptionChange {
    plan: Option<String>,
    status: String,
    ends_at: Option<i64>,
    #[serde(default)]
    note: String,
}

pub(crate) async fn set_subscription(
    State(server): State<Arc<Server>>,
    admin: Admin,
    Path(org): Path<String>,
    axum::Json(req): axum::Json<SubscriptionChange>,
) -> ApiResult<axum::http::StatusCode> {
    if !["trial", "active", "cancelled"].contains(&req.status.as_str()) {
        return Err(ApiError::BadRequest("admin.status_invalid".into()));
    }
    server
        .control(move |c| {
            super::shops::known_org(c, &org)?;
            if let Some(p) = &req.plan {
                let known: bool = c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM plans WHERE code = ?1)",
                    [p],
                    |r| r.get(0),
                )?;
                if !known {
                    return Err(ApiError::BadRequest("admin.plan_unknown".into()));
                }
            }
            let note: String = req.note.trim().chars().take(300).collect();
            c.execute(
                "INSERT INTO subscriptions (org_id, plan, status, ends_at, note, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (org_id) DO UPDATE SET plan = ?2, status = ?3, ends_at = ?4, note = ?5, updated_at = ?6",
                params![org, req.plan, req.status, req.ends_at, note, now_ms()],
            )?;
            log_action(
                c,
                &admin.login,
                &admin.ip,
                "subscription.set",
                &org,
                &format!(
                    "{} {} {}",
                    req.plan.as_deref().unwrap_or("-"),
                    req.status,
                    req.ends_at.map(|t| t.to_string()).unwrap_or_default()
                ),
            )?;
            Ok(axum::http::StatusCode::NO_CONTENT)
        })
        .await
}

#[derive(Deserialize)]
pub(crate) struct PaymentRequest {
    plan: String,
    amount_minor: i64,
    method: String,
    #[serde(default)]
    reference: String,
    months: i64,
}

/// A payment received (BaridiMob, CCP, cash…): written, and the
/// subscription runs `months` × 30 days further (from its end, or from
/// today if it already ended), active, on that plan.
pub(crate) async fn add_payment(
    State(server): State<Arc<Server>>,
    admin: Admin,
    Path(org): Path<String>,
    axum::Json(req): axum::Json<PaymentRequest>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    if req.amount_minor < 0
        || !(1..=36).contains(&req.months)
        || !METHODS.contains(&req.method.as_str())
        || req.reference.chars().count() > 120
    {
        return Err(ApiError::BadRequest("admin.payment_invalid".into()));
    }
    server
        .control(move |c| {
            super::shops::known_org(c, &org)?;
            let currency: String = c
                .query_row("SELECT currency FROM plans WHERE code = ?1", [&req.plan], |r| {
                    r.get(0)
                })
                .optional()?
                .ok_or_else(|| ApiError::BadRequest("admin.plan_unknown".into()))?;
            let now = now_ms();
            let ends: Option<i64> = c
                .query_row(
                    "SELECT ends_at FROM subscriptions WHERE org_id = ?1",
                    [&org],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            let from = ends.filter(|e| *e > now).unwrap_or(now);
            let until = from + req.months * 30 * DAY;
            c.execute(
                "INSERT INTO payments (id, org_id, plan, amount_minor, currency, method, reference, months, admin, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    new_token()?,
                    org,
                    req.plan,
                    req.amount_minor,
                    currency,
                    req.method,
                    req.reference.trim(),
                    req.months,
                    admin.login,
                    now
                ],
            )?;
            c.execute(
                "INSERT INTO subscriptions (org_id, plan, status, ends_at, updated_at)
                 VALUES (?1, ?2, 'active', ?3, ?4)
                 ON CONFLICT (org_id) DO UPDATE SET plan = ?2, status = 'active', ends_at = ?3, updated_at = ?4",
                params![org, req.plan, until, now],
            )?;
            log_action(
                c,
                &admin.login,
                &admin.ip,
                "payment.add",
                &org,
                &format!(
                    "{} {currency} {} {} months → {until}",
                    req.amount_minor, req.method, req.months
                ),
            )?;
            Ok(axum::Json(serde_json::json!({ "ends_at": until })))
        })
        .await
}

pub(crate) fn payments_of(c: &Connection, org: Option<&str>) -> ApiResult<Vec<serde_json::Value>> {
    let mut st = c.prepare(
        "SELECT p.id, p.org_id, COALESCE(o.name, ''), p.plan, p.amount_minor, p.currency, p.method,
                p.reference, p.months, p.admin, p.at
         FROM payments p LEFT JOIN orgs o ON o.id = p.org_id
         WHERE ?1 IS NULL OR p.org_id = ?1
         ORDER BY p.at DESC LIMIT 200",
    )?;
    let rows = st.query_map([org], |r| {
        Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?,
            "org_id": r.get::<_, String>(1)?,
            "org_name": r.get::<_, String>(2)?,
            "plan": r.get::<_, String>(3)?,
            "amount_minor": r.get::<_, i64>(4)?,
            "currency": r.get::<_, String>(5)?,
            "method": r.get::<_, String>(6)?,
            "reference": r.get::<_, String>(7)?,
            "months": r.get::<_, i64>(8)?,
            "admin": r.get::<_, String>(9)?,
            "at": r.get::<_, i64>(10)?,
        }))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(crate) async fn payments(
    State(server): State<Arc<Server>>,
    _admin: Admin,
) -> ApiResult<axum::Json<Vec<serde_json::Value>>> {
    Ok(axum::Json(server.control(|c| payments_of(c, None)).await?))
}

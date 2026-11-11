//! Roles, grants, credentials, approvals and feature switches.

use nb_core::OrganizationId;
use nb_domain::access::{BuiltinRole, Grant, Permission, PermissionSet, Scope, Target};
use nb_domain::features::Feature;
use nb_domain::{ApprovalId, RuleError, UserId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::parse_id;
use crate::uow::Writer;
use crate::{Error, Result};

/// A role: built-in (`id` = its code) or defined by the organization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRow {
    pub id: String,
    /// Empty for built-in roles (named by `role.<id>` messages).
    pub name: String,
    pub builtin: bool,
    pub permissions: Vec<String>,
    /// Users holding it (anywhere).
    pub users: u32,
}

/// A role granted to a user over a scope, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantRow {
    pub role: String,
    /// `org`, `branch`, `warehouse` or `register`.
    pub scope_kind: String,
    pub scope_id: Option<String>,
}

impl GrantRow {
    pub fn org(role: &str) -> Self {
        Self {
            role: role.into(),
            scope_kind: "org".into(),
            scope_id: None,
        }
    }

    pub fn scope(&self) -> Result<Scope> {
        let id = || {
            self.scope_id
                .as_deref()
                .ok_or(Error::from(RuleError::on("scope", "required")))
        };
        Ok(match self.scope_kind.as_str() {
            "org" => Scope::Org,
            "branch" => Scope::Branch(parse_id(id()?)?),
            "warehouse" => Scope::Warehouse(parse_id(id()?)?),
            "register" => Scope::Register(parse_id(id()?)?),
            _ => return Err(RuleError::on("scope", "invalid_value").into()),
        })
    }
}

/// A user with their roles, for the users screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRow {
    pub id: String,
    pub name: String,
    pub phone: Option<String>,
    pub active: bool,
    pub login: Option<String>,
    pub has_password: bool,
    pub has_pin: bool,
    pub locked_until: Option<i64>,
    pub grants: Vec<GrantRow>,
}

/// The permissions of a role id (built-in code or custom id); `None` if it
/// no longer exists.
pub fn role_permissions(conn: &Connection, role: &str) -> Result<Option<PermissionSet>> {
    if let Some(b) = BuiltinRole::from_code(role) {
        return Ok(Some(b.permissions()));
    }
    let stored: Option<String> = conn
        .query_row(
            "SELECT permissions FROM roles WHERE id = ?1 AND deleted_at IS NULL",
            [role],
            |r| r.get(0),
        )
        .optional()?;
    match stored {
        None => Ok(None),
        Some(json) => {
            let codes: Vec<String> = serde_json::from_str(&json)?;
            Ok(Some(PermissionSet::from_codes(&codes)?))
        }
    }
}

pub fn user_grant_rows(conn: &Connection, user: UserId) -> Result<Vec<GrantRow>> {
    let mut s = conn.prepare_cached(
        "SELECT role, scope_kind, scope_id FROM user_roles
         WHERE user_id = ?1 AND deleted_at IS NULL ORDER BY created_at, role",
    )?;
    let rows = s.query_map([user.to_string()], |r| {
        Ok(GrantRow {
            role: r.get(0)?,
            scope_kind: r.get(1)?,
            scope_id: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Whether the user is active, and what their roles grant where.
pub fn grants(conn: &Connection, user: UserId) -> Result<(bool, Vec<Grant>)> {
    let active: Option<bool> = conn
        .query_row(
            "SELECT active FROM users WHERE id = ?1 AND deleted_at IS NULL",
            [user.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(active) = active else {
        return Ok((false, vec![]));
    };
    let mut out = Vec::new();
    for g in user_grant_rows(conn, user)? {
        if let Some(permissions) = role_permissions(conn, &g.role)? {
            out.push(Grant {
                permissions,
                scope: g.scope()?,
            });
        }
    }
    Ok((active, out))
}

pub fn insert_grant(w: &Writer<'_>, org: OrganizationId, user: UserId, g: &GrantRow) -> Result<()> {
    w.tx().execute(
        "INSERT INTO user_roles (id, org_id, user_id, role, scope_kind, scope_id, created_at, created_by, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            nb_core::OperationId::new().to_string(),
            org.to_string(),
            user.to_string(),
            g.role,
            g.scope_kind,
            g.scope_id,
            w.now(),
            w.actor_text(),
            w.device().id.to_string()
        ],
    )?;
    Ok(())
}

/// Ends all of a user's grants (they are replaced as a whole).
pub fn end_grants(w: &Writer<'_>, user: UserId) -> Result<()> {
    w.tx().execute(
        "UPDATE user_roles SET deleted_at = ?2 WHERE user_id = ?1 AND deleted_at IS NULL",
        params![user.to_string(), w.now()],
    )?;
    Ok(())
}

/// Active users holding the owner role over the organization.
pub fn active_owners(conn: &Connection, org: OrganizationId) -> Result<Vec<String>> {
    let mut s = conn.prepare(
        "SELECT DISTINCT u.id FROM users u JOIN user_roles g ON g.user_id = u.id
         WHERE u.org_id = ?1 AND u.deleted_at IS NULL AND u.active = 1
           AND g.deleted_at IS NULL AND g.role = 'owner' AND g.scope_kind = 'org'",
    )?;
    let rows = s.query_map([org.to_string()], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn roles(conn: &Connection, org: OrganizationId) -> Result<Vec<RoleRow>> {
    let count = |role: &str| -> Result<u32> {
        Ok(conn.query_row(
            "SELECT COUNT(DISTINCT g.user_id) FROM user_roles g JOIN users u ON u.id = g.user_id
             WHERE g.role = ?1 AND g.deleted_at IS NULL AND u.deleted_at IS NULL AND u.org_id = ?2",
            params![role, org.to_string()],
            |r| r.get(0),
        )?)
    };
    let mut out = Vec::new();
    for b in BuiltinRole::ALL {
        out.push(RoleRow {
            id: b.code().into(),
            name: String::new(),
            builtin: true,
            permissions: b
                .permissions()
                .codes()
                .into_iter()
                .map(Into::into)
                .collect(),
            users: count(b.code())?,
        });
    }
    let mut s = conn.prepare(
        "SELECT id, name, permissions FROM roles WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY lower(name)",
    )?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (id, name, json) = row?;
        let permissions: Vec<String> = serde_json::from_str(&json)?;
        let users = count(&id)?;
        out.push(RoleRow {
            id,
            name,
            builtin: false,
            permissions,
            users,
        });
    }
    Ok(out)
}

/// Credentials of a user: (login, password hash, PIN hash, failures, locked until).
pub type Credentials = (
    Option<String>,
    Option<String>,
    Option<String>,
    u32,
    Option<i64>,
);

pub fn credentials(conn: &Connection, user: UserId) -> Result<Option<Credentials>> {
    Ok(conn
        .query_row(
            "SELECT login, password_hash, pin_hash, failures, locked_until FROM user_credentials WHERE user_id = ?1",
            [user.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?)
}

/// The user signing in with `login` (case-insensitive).
pub fn user_by_login(
    conn: &Connection,
    org: OrganizationId,
    login: &str,
) -> Result<Option<UserId>> {
    let id: Option<String> = conn
        .query_row(
            "SELECT user_id FROM user_credentials WHERE org_id = ?1 AND login = ?2",
            params![org.to_string(), login.trim().to_lowercase()],
            |r| r.get(0),
        )
        .optional()?;
    id.map(|s| parse_id(&s)).transpose()
}

/// Makes sure a credentials row exists for the user.
pub fn ensure_credentials(w: &Writer<'_>, org: OrganizationId, user: UserId) -> Result<()> {
    w.tx().execute(
        "INSERT INTO user_credentials (user_id, org_id, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (user_id) DO NOTHING",
        params![user.to_string(), org.to_string(), w.now()],
    )?;
    Ok(())
}

pub fn set_login(w: &Writer<'_>, user: UserId, login: Option<&str>) -> Result<()> {
    let r = w.tx().execute(
        "UPDATE user_credentials SET login = ?2, updated_at = ?3 WHERE user_id = ?1",
        params![user.to_string(), login, w.now()],
    );
    match r {
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(RuleError::on("login", "access.login_taken").into())
        }
        other => {
            other?;
            Ok(())
        }
    }
}

/// Stores a new secret hash (`password_hash` or `pin_hash`) and clears failures.
pub fn set_secret(w: &Writer<'_>, user: UserId, column: Secret, hash: Option<&str>) -> Result<()> {
    let sql = match column {
        Secret::Password => {
            "UPDATE user_credentials SET password_hash = ?2, failures = 0, locked_until = NULL, updated_at = ?3 WHERE user_id = ?1"
        }
        Secret::Pin => {
            "UPDATE user_credentials SET pin_hash = ?2, failures = 0, locked_until = NULL, updated_at = ?3 WHERE user_id = ?1"
        }
    };
    w.tx()
        .execute(sql, params![user.to_string(), hash, w.now()])?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Secret {
    Password,
    Pin,
}

pub fn set_failures(
    w: &Writer<'_>,
    user: UserId,
    failures: u32,
    locked_until: Option<i64>,
) -> Result<()> {
    w.tx().execute(
        "UPDATE user_credentials SET failures = ?2, locked_until = ?3 WHERE user_id = ?1",
        params![user.to_string(), failures, locked_until],
    )?;
    Ok(())
}

/// Users of the organization with their roles and sign-in state.
pub fn users(conn: &Connection, org: OrganizationId) -> Result<Vec<UserRow>> {
    let mut s = conn.prepare(
        "SELECT u.id, u.name, u.phone, u.active, c.login, c.password_hash IS NOT NULL,
                c.pin_hash IS NOT NULL, c.locked_until
         FROM users u LEFT JOIN user_credentials c ON c.user_id = u.id
         WHERE u.org_id = ?1 AND u.deleted_at IS NULL
         ORDER BY u.active DESC, u.created_at",
    )?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok(UserRow {
            id: r.get(0)?,
            name: r.get(1)?,
            phone: r.get(2)?,
            active: r.get(3)?,
            login: r.get(4)?,
            has_password: r.get::<_, Option<bool>>(5)?.unwrap_or(false),
            has_pin: r.get::<_, Option<bool>>(6)?.unwrap_or(false),
            locked_until: r.get(7)?,
            grants: vec![],
        })
    })?;
    let mut out: Vec<UserRow> = rows.collect::<std::result::Result<_, _>>()?;
    for u in &mut out {
        u.grants = user_grant_rows(conn, parse_id(&u.id)?)?;
    }
    Ok(out)
}

pub fn user(conn: &Connection, org: OrganizationId, id: UserId) -> Result<UserRow> {
    users(conn, org)?
        .into_iter()
        .find(|u| u.id == id.to_string())
        .ok_or(Error::NotFound("user"))
}

/// Records a supervisor's approval.
#[allow(clippy::too_many_arguments)]
pub fn insert_approval(
    w: &Writer<'_>,
    org: OrganizationId,
    id: ApprovalId,
    p: Permission,
    requested_by: UserId,
    approved_by: UserId,
    reason: &str,
    t: &Target,
    expires_at: i64,
) -> Result<()> {
    w.tx().execute(
        "INSERT INTO approvals (id, org_id, permission, requested_by, approved_by, reason, branch_id, warehouse_id, register_id, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id.to_string(),
            org.to_string(),
            p.code(),
            requested_by.to_string(),
            approved_by.to_string(),
            reason,
            t.branch.map(|b| b.to_string()),
            t.warehouse.map(|x| x.to_string()),
            t.register.map(|x| x.to_string()),
            w.now(),
            expires_at
        ],
    )?;
    Ok(())
}

/// Uses an approval up: it must be for this permission, this requester and
/// this target, unused and not expired.
pub fn use_approval(
    w: &Writer<'_>,
    id: ApprovalId,
    p: Permission,
    actor: UserId,
    t: &Target,
) -> Result<()> {
    let n = w.tx().execute(
        "UPDATE approvals SET used_at = ?6, used_by_operation = ?7
         WHERE id = ?1 AND permission = ?2 AND requested_by = ?3 AND used_at IS NULL AND expires_at > ?6
           AND branch_id IS ?4 AND (warehouse_id IS ?5 OR warehouse_id IS NULL) AND (register_id IS ?8 OR register_id IS NULL)",
        params![
            id.to_string(),
            p.code(),
            actor.to_string(),
            t.branch.map(|b| b.to_string()),
            t.warehouse.map(|x| x.to_string()),
            w.now(),
            w.meta().id.to_string(),
            t.register.map(|x| x.to_string())
        ],
    )?;
    if n == 1 {
        Ok(())
    } else {
        Err(RuleError::new("access.approval_invalid").into())
    }
}

pub fn feature_overrides(conn: &Connection, org: OrganizationId) -> Result<Vec<(Feature, bool)>> {
    let mut s = conn.prepare("SELECT feature, enabled FROM org_features WHERE org_id = ?1")?;
    let rows = s.query_map([org.to_string()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (code, on) = row?;
        // A switch from a newer version is ignored rather than fatal.
        if let Some(f) = Feature::from_code(&code) {
            out.push((f, on));
        }
    }
    Ok(out)
}

pub fn set_feature(
    w: &Writer<'_>,
    org: OrganizationId,
    f: Feature,
    on: Option<bool>,
) -> Result<()> {
    match on {
        Some(on) => w.tx().execute(
            "INSERT INTO org_features (org_id, feature, enabled, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (org_id, feature) DO UPDATE SET enabled = ?3, updated_at = ?4",
            params![org.to_string(), f.code(), on, w.now()],
        )?,
        None => w.tx().execute(
            "DELETE FROM org_features WHERE org_id = ?1 AND feature = ?2",
            params![org.to_string(), f.code()],
        )?,
    };
    Ok(())
}

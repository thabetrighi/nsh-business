//! Users, roles, sign-in and supervisor approvals (M4.5–M4.8).
//!
//! - Managing users needs `users.manage`; defining roles `roles.manage`.
//!   Nobody can hand out a permission they do not hold themselves, and the
//!   organization always keeps an active owner.
//! - Sign-in by login and password, or by choosing one's name and typing a
//!   PIN (cashiers). Failures are counted and lock the account for a while
//!   (`nb_domain::access::after_failure`); every attempt is audited.
//! - A sensitive action a user lacks can be approved on the spot by a
//!   supervisor's PIN: the approval is recorded, expires after a few
//!   minutes, and is used up by the one operation it was given for.
//!
//! Secrets are hashed (argon2id) before the transaction and never appear
//! in events or audit records.

use std::sync::OnceLock;

use nb_core::OrganizationId;
use nb_domain::access::{
    self, BuiltinRole, Decision, Permission, PermissionSet, Target, check_password, check_pin,
};
use nb_domain::{ApprovalId, BranchId, RegisterId, RoleId, RuleError, UserId, WarehouseId};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::repo::access::{self as repo, GrantRow, RoleRow, Secret, UserRow};
use crate::repo::{org, parse_id};
use crate::uow::{OpMeta, Writer};
use crate::{Error, Result, Store, secret};

/// How long a supervisor's approval stays usable.
pub const APPROVAL_TTL_MS: i64 = 5 * 60_000;

// ---------------------------------------------------------------------------
// Users

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInput {
    pub name: String,
    pub phone: Option<String>,
    /// Sign-in name (`a-z 0-9 . _ -`, 3 to 32), or none for PIN-only users.
    pub login: Option<String>,
    pub password: Option<String>,
    pub pin: Option<String>,
    pub grants: Vec<GrantRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserUpdate {
    pub id: String,
    pub name: String,
    pub phone: Option<String>,
    pub login: Option<String>,
    pub active: bool,
    pub grants: Vec<GrantRow>,
}

fn normalize_login(login: Option<&str>) -> Result<Option<String>> {
    let Some(l) = login
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty())
    else {
        return Ok(None);
    };
    let ok = (3..=32).contains(&l.chars().count())
        && l.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(Some(l))
    } else {
        Err(RuleError::on("login", "access.login_format").into())
    }
}

/// What the actor may hand out: the permissions they hold.
fn actor_perms(w: &Writer<'_>) -> Result<PermissionSet> {
    let (active, grants) = repo::grants(w.tx(), w.actor()?)?;
    Ok(if active {
        access::reachable(&grants)
    } else {
        PermissionSet::default()
    })
}

/// Checks grants: roles and scopes exist, and none hands out more than the
/// actor holds (`added` only: keeping what a user already had is fine).
fn check_grants(w: &Writer<'_>, org: OrganizationId, added: &[&GrantRow]) -> Result<()> {
    let mine = actor_perms(w)?;
    for g in added {
        let perms = repo::role_permissions(w.tx(), &g.role)?
            .ok_or(Error::from(RuleError::on("roles", "access.role_unknown")))?;
        if perms.iter().any(|p| !mine.contains(p)) {
            return Err(RuleError::on("roles", "access.escalation").into());
        }
        let exists = |table: &str, id: &str| -> Result<bool> {
            Ok(w.tx().query_row(
                &format!(
                    "SELECT COUNT(*) FROM {table} WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL"
                ),
                params![id, org.to_string()],
                |r| r.get::<_, i64>(0),
            )? == 1)
        };
        let found = match (g.scope()?, g.scope_id.as_deref()) {
            (access::Scope::Org, _) => true,
            (access::Scope::Branch(_), Some(id)) => exists("branches", id)?,
            (access::Scope::Warehouse(_), Some(id)) => exists("warehouses", id)?,
            (access::Scope::Register(_), Some(id)) => exists("cash_registers", id)?,
            _ => false,
        };
        if !found {
            return Err(RuleError::on("scope", "not_found").into());
        }
    }
    Ok(())
}

fn hash_opt(s: Option<&str>) -> Result<Option<String>> {
    s.map(secret::hash).transpose()
}

/// A new user as it travels to other devices: its password and PIN only
/// as their Argon2id hashes (computed once, on the device that made the
/// user), never as typed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewUser {
    pub name: String,
    pub phone: Option<String>,
    pub login: Option<String>,
    pub password_hash: Option<String>,
    pub pin_hash: Option<String>,
    pub grants: Vec<GrantRow>,
}

/// A stored hash, never a password: what may travel in the log.
fn check_hash(field: &'static str, h: Option<&str>) -> Result<()> {
    match h {
        Some(h) if !h.starts_with("$argon2") || h.len() > 512 => {
            Err(RuleError::on(field, "invalid_value").into())
        }
        _ => Ok(()),
    }
}

/// Makes a user. The password and PIN are checked and hashed here; what is
/// logged (and runs on the other devices) is [`create_user_hashed`].
pub fn create_user(store: &mut Store, meta: OpMeta, input: UserInput) -> Result<UserRow> {
    let mut errors = Vec::new();
    if input.name.trim().is_empty() {
        errors.push(RuleError::on("name", "required"));
    }
    let login = match normalize_login(input.login.as_deref()) {
        Ok(l) => l,
        Err(Error::Rules(mut e)) => {
            errors.append(&mut e);
            None
        }
        Err(e) => return Err(e),
    };
    if let Some(Err(e)) = input.password.as_deref().map(check_password) {
        errors.push(e);
    }
    let no_login = input.login.as_deref().is_none_or(|l| l.trim().is_empty());
    if input.password.is_some() && no_login {
        errors.push(RuleError::on("login", "required"));
    }
    if let Some(Err(e)) = input.pin.as_deref().map(check_pin) {
        errors.push(e);
    }
    if input.grants.is_empty() {
        errors.push(RuleError::on("roles", "access.no_role"));
    }
    if !errors.is_empty() {
        return Err(Error::Rules(errors));
    }
    let password_hash = hash_opt(input.password.as_deref())?;
    let pin_hash = hash_opt(input.pin.as_deref())?;
    create_user_hashed(
        store,
        meta,
        NewUser {
            name: input.name,
            phone: input.phone,
            login,
            password_hash,
            pin_hash,
            grants: input.grants,
        },
    )
}

/// [`create_user`] with the secrets already hashed: the operation as it is
/// logged and run again elsewhere.
pub fn create_user_hashed(store: &mut Store, meta: OpMeta, input: NewUser) -> Result<UserRow> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::AccessCreateUserHashed {
            input: input.clone(),
        },
    )?;
    if input.name.trim().is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    if input.grants.is_empty() {
        return Err(RuleError::on("roles", "access.no_role").into());
    }
    let login = normalize_login(input.login.as_deref())?;
    if input.password_hash.is_some() && login.is_none() {
        return Err(RuleError::on("login", "required").into());
    }
    check_hash("password", input.password_hash.as_deref())?;
    check_hash("pin", input.pin_hash.as_deref())?;
    let (password, pin) = (input.password_hash.clone(), input.pin_hash.clone());
    store.write(meta, |w| {
        let org = w.org()?;
        w.require(Permission::UsersManage, &Target::org())?;
        check_grants(w, org, &input.grants.iter().collect::<Vec<_>>())?;
        let id = UserId::new();
        let primary = input.grants.first().map_or("", |g| g.role.as_str());
        org::insert_user(w, org, id, input.name.trim(), primary)?;
        w.tx().execute(
            "UPDATE users SET phone = ?2 WHERE id = ?1",
            params![
                id.to_string(),
                input
                    .phone
                    .as_deref()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
            ],
        )?;
        for g in &input.grants {
            repo::insert_grant(w, org, id, g)?;
        }
        repo::ensure_credentials(w, org, id)?;
        repo::set_login(w, id, login.as_deref())?;
        repo::set_secret(w, id, Secret::Password, password.as_deref())?;
        repo::set_secret(w, id, Secret::Pin, pin.as_deref())?;
        let row = repo::user(w.tx(), org, id)?;
        w.event("user.created", "user", &row.id, &row)?;
        w.audit("create", "user", &row.id, &row)?;
        Ok(row)
    })
}

pub fn update_user(store: &mut Store, meta: OpMeta, input: UserUpdate) -> Result<UserRow> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::AccessUpdateUser {
            input: input.clone(),
        },
    )?;
    let id: UserId = parse_id(&input.id)?;
    if input.name.trim().is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    if input.grants.is_empty() {
        return Err(RuleError::on("roles", "access.no_role").into());
    }
    let login = normalize_login(input.login.as_deref())?;
    store.write(meta, |w| {
        let org = w.org()?;
        w.require(Permission::UsersManage, &Target::org())?;
        let before = repo::user(w.tx(), org, id)?;
        let added: Vec<&GrantRow> = input
            .grants
            .iter()
            .filter(|g| !before.grants.contains(g))
            .collect();
        check_grants(w, org, &added)?;
        // Taking a role away is handing nothing out, but removing an owner
        // takes an owner: only an owner may do that.
        let had_owner = before.grants.iter().any(|g| g.role == BuiltinRole::Owner.code());
        let keeps_owner = input.grants.iter().any(|g| g.role == BuiltinRole::Owner.code());
        if had_owner && (!keeps_owner || !input.active) && !actor_perms(w)?.contains(Permission::RolesManage) {
            return Err(RuleError::on("roles", "access.escalation").into());
        }
        w.tx().execute(
            "UPDATE users SET name = ?2, phone = ?3, active = ?4, role = ?5, updated_at = ?6, version = version + 1
             WHERE id = ?1",
            params![
                id.to_string(),
                input.name.trim(),
                input.phone.as_deref().map(str::trim).filter(|p| !p.is_empty()),
                input.active,
                input.grants.first().map_or("", |g| g.role.as_str()),
                w.now()
            ],
        )?;
        if before.grants != input.grants {
            repo::end_grants(w, id)?;
            for g in &input.grants {
                repo::insert_grant(w, org, id, g)?;
            }
        }
        repo::ensure_credentials(w, org, id)?;
        repo::set_login(w, id, login.as_deref())?;
        if repo::active_owners(w.tx(), org)?.is_empty() {
            return Err(RuleError::on("roles", "access.last_owner").into());
        }
        let row = repo::user(w.tx(), org, id)?;
        w.event("user.updated", "user", &row.id, &row)?;
        w.audit("update", "user", &row.id, &serde_json::json!({ "before": before, "after": row }))?;
        Ok(row)
    })
}

/// Sets or clears (`None`) a user's password or PIN. Users change their
/// own; changing another's needs `users.manage`. Checked and hashed here;
/// logged as [`set_secret_hashed`].
pub fn set_secret(
    store: &mut Store,
    meta: OpMeta,
    user: UserId,
    kind: Secret,
    value: Option<String>,
) -> Result<()> {
    if let Some(v) = value.as_deref() {
        match kind {
            Secret::Password => check_password(v)?,
            Secret::Pin => check_pin(v)?,
        }
    }
    let hash = hash_opt(value.as_deref())?;
    let which = match kind {
        Secret::Password => "password",
        Secret::Pin => "pin",
    };
    set_secret_hashed(store, meta, user, which.to_owned(), hash)
}

/// [`set_secret`] with the secret already hashed (`which`: `password` or
/// `pin`): the operation as it is logged and run again elsewhere.
pub fn set_secret_hashed(
    store: &mut Store,
    meta: OpMeta,
    user: UserId,
    which: String,
    hash: Option<String>,
) -> Result<()> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::AccessSetSecretHashed {
            user: user.clone(),
            which: which.clone(),
            hash: hash.clone(),
        },
    )?;
    let kind = match which.as_str() {
        "password" => Secret::Password,
        "pin" => Secret::Pin,
        _ => return Err(RuleError::on("secret", "invalid_value").into()),
    };
    check_hash("secret", hash.as_deref())?;
    let what = match kind {
        Secret::Password => "password",
        Secret::Pin => "pin",
    };
    store.write(meta, |w| {
        let org = w.org()?;
        if w.actor()? != user {
            w.require(Permission::UsersManage, &Target::org())?;
        }
        repo::user(w.tx(), org, user)?;
        repo::ensure_credentials(w, org, user)?;
        if kind == Secret::Password && hash.is_some() {
            let (login, ..) = repo::credentials(w.tx(), user)?.unwrap_or_default();
            if login.is_none() {
                return Err(RuleError::on("login", "required").into());
            }
        }
        repo::set_secret(w, user, kind, hash.as_deref())?;
        let detail = serde_json::json!({ "secret": what, "set": hash.is_some() });
        w.event("user.secret_changed", "user", &user.to_string(), &detail)?;
        w.audit("secret", "user", &user.to_string(), &detail)?;
        Ok(())
    })
}

pub fn list_users(store: &Store) -> Result<Vec<UserRow>> {
    store.read(|r| {
        let Some(o) = org::the_org(r.conn())? else {
            return Ok(vec![]);
        };
        repo::users(r.conn(), o.org_id()?)
    })
}

// ---------------------------------------------------------------------------
// Roles

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleInput {
    /// `None` to create.
    pub id: Option<String>,
    pub name: String,
    pub permissions: Vec<String>,
}

pub fn save_role(store: &mut Store, meta: OpMeta, input: RoleInput) -> Result<RoleRow> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::AccessSaveRole {
            input: input.clone(),
        },
    )?;
    let name = input.name.trim().to_owned();
    if name.is_empty() {
        return Err(RuleError::on("name", "required").into());
    }
    let perms = PermissionSet::from_codes(&input.permissions)?;
    if perms.is_empty() {
        return Err(RuleError::on("permissions", "access.no_permission").into());
    }
    if let Some(id) = input.id.as_deref()
        && BuiltinRole::from_code(id).is_some()
    {
        return Err(RuleError::on("role", "access.builtin_role").into());
    }
    store.write(meta, |w| {
        let org = w.org()?;
        w.require(Permission::RolesManage, &Target::org())?;
        let mine = actor_perms(w)?;
        if perms.iter().any(|p| !mine.contains(p)) {
            return Err(RuleError::on("permissions", "access.escalation").into());
        }
        let json = serde_json::to_string(&perms.codes())?;
        let id = match input.id.as_deref() {
            Some(id) => {
                let n = w.tx().execute(
                    "UPDATE roles SET name = ?3, permissions = ?4, updated_at = ?5, version = version + 1
                     WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
                    params![id, org.to_string(), name, json, w.now()],
                );
                if unique_violation(&n) {
                    return Err(RuleError::on("name", "access.role_name_taken").into());
                }
                if n? == 0 {
                    return Err(Error::NotFound("role"));
                }
                id.to_owned()
            }
            None => {
                let id = RoleId::new().to_string();
                let n = w.tx().execute(
                    "INSERT INTO roles (id, org_id, name, permissions, created_at, updated_at, created_by, device_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
                    params![id, org.to_string(), name, json, w.now(), w.actor_text(), w.device().id.to_string()],
                );
                if unique_violation(&n) {
                    return Err(RuleError::on("name", "access.role_name_taken").into());
                }
                n?;
                id
            }
        };
        let row = repo::roles(w.tx(), org)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or(Error::NotFound("role"))?;
        w.event("role.saved", "role", &row.id, &row)?;
        w.audit("save", "role", &row.id, &row)?;
        Ok(row)
    })
}

fn unique_violation<T>(r: &std::result::Result<T, rusqlite::Error>) -> bool {
    matches!(r, Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation)
}

/// Deletes a custom role nobody holds.
pub fn delete_role(store: &mut Store, meta: OpMeta, id: &str) -> Result<()> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::AccessDeleteRole { id: id.to_owned() },
    )?;
    if BuiltinRole::from_code(id).is_some() {
        return Err(RuleError::on("role", "access.builtin_role").into());
    }
    store.write(meta, |w| {
        let org = w.org()?;
        w.require(Permission::RolesManage, &Target::org())?;
        let held: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM user_roles WHERE role = ?1 AND deleted_at IS NULL",
            [id],
            |r| r.get(0),
        )?;
        if held > 0 {
            return Err(RuleError::on("role", "access.role_in_use").into());
        }
        let n = w.tx().execute(
            "UPDATE roles SET deleted_at = ?3, updated_at = ?3, version = version + 1
             WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
            params![id, org.to_string(), w.now()],
        )?;
        if n == 0 {
            return Err(Error::NotFound("role"));
        }
        w.event("role.deleted", "role", id, &id)?;
        w.audit("delete", "role", id, &id)?;
        Ok(())
    })
}

pub fn list_roles(store: &Store) -> Result<Vec<RoleRow>> {
    store.read(|r| {
        let Some(o) = org::the_org(r.conn())? else {
            return Ok(vec![]);
        };
        repo::roles(r.conn(), o.org_id()?)
    })
}

// ---------------------------------------------------------------------------
// Sign-in

/// The person now using the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedIn {
    pub user_id: String,
    pub name: String,
    /// Codes of the permissions held somewhere (to show screens and buttons).
    pub permissions: Vec<String>,
    /// Minutes of inactivity before the screen locks (0 = never).
    pub auto_lock_min: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignInOutcome {
    SignedIn(SignedIn),
    /// `access.bad_credentials`, `access.locked` (until the given time) or
    /// `access.inactive`.
    Refused {
        code: String,
        locked_until: Option<i64>,
    },
}

/// A valid hash to verify against when the user is unknown, so that the
/// answer takes the same time and says nothing about who exists.
fn decoy() -> &'static str {
    static DECOY: OnceLock<String> = OnceLock::new();
    DECOY.get_or_init(|| secret::hash("decoy-secret").unwrap_or_default())
}

fn signed_in(conn: &Connection, org: OrganizationId, user: UserId) -> Result<SignedIn> {
    let row = repo::user(conn, org, user)?;
    let (_, grants) = repo::grants(conn, user)?;
    let auto_lock_min: u32 = conn.query_row(
        "SELECT auto_lock_min FROM organizations WHERE id = ?1",
        [org.to_string()],
        |r| r.get(0),
    )?;
    Ok(SignedIn {
        user_id: row.id,
        name: row.name,
        permissions: access::reachable(&grants)
            .codes()
            .into_iter()
            .map(Into::into)
            .collect(),
        auto_lock_min,
    })
}

/// Checks a secret against a user's stored hash and records the attempt.
fn attempt(
    store: &mut Store,
    user: Option<UserId>,
    kind: Secret,
    typed: &str,
) -> Result<SignInOutcome> {
    let o = store
        .read(|r| org::the_org(r.conn()))?
        .ok_or(Error::NotFound("organization"))?;
    let org = o.org_id()?;
    let creds = match user {
        Some(u) => store.read(|r| repo::credentials(r.conn(), u))?,
        None => None,
    };
    let stored = creds.as_ref().and_then(|(_, pw, pin, ..)| match kind {
        Secret::Password => pw.clone(),
        Secret::Pin => pin.clone(),
    });
    // Verify before the transaction: hashing is slow on purpose.
    let matches = secret::verify(typed, stored.as_deref().unwrap_or(decoy())) && stored.is_some();
    let meta = OpMeta::new("auth.sign_in", Some(org), user);
    store.write(meta, |w| {
        let method = match kind {
            Secret::Password => "password",
            Secret::Pin => "pin",
        };
        let refuse =
            |w: &mut Writer<'_>, code: &str, until: Option<i64>| -> Result<SignInOutcome> {
                let detail = serde_json::json!({ "method": method, "refused": code });
                let who = user.map(|u| u.to_string()).unwrap_or_default();
                w.event("auth.refused", "user", &who, &detail)?;
                w.audit("sign_in.refused", "user", &who, &detail)?;
                Ok(SignInOutcome::Refused {
                    code: code.into(),
                    locked_until: until,
                })
            };
        let (Some(u), Some((_, _, _, failures, locked))) = (user, creds.clone()) else {
            return refuse(w, "access.bad_credentials", None);
        };
        if access::is_locked(locked, w.now()) {
            return refuse(w, "access.locked", locked);
        }
        let (active, _) = repo::grants(w.tx(), u)?;
        if !matches {
            let (n, until) = access::after_failure(failures, w.now());
            repo::set_failures(w, u, n, until)?;
            return match until {
                Some(t) => refuse(w, "access.locked", Some(t)),
                None => refuse(w, "access.bad_credentials", None),
            };
        }
        if !active {
            return refuse(w, "access.inactive", None);
        }
        repo::set_failures(w, u, 0, None)?;
        let detail = serde_json::json!({ "method": method });
        w.event("auth.signed_in", "user", &u.to_string(), &detail)?;
        w.audit("sign_in", "user", &u.to_string(), &detail)?;
        Ok(SignInOutcome::SignedIn(signed_in(w.tx(), org, u)?))
    })
}

pub fn sign_in_password(store: &mut Store, login: &str, password: &str) -> Result<SignInOutcome> {
    let user = store.read(|r| match org::the_org(r.conn())? {
        Some(o) => repo::user_by_login(r.conn(), o.org_id()?, login),
        None => Ok(None),
    })?;
    attempt(store, user, Secret::Password, password)
}

/// Sign-in by choosing one's name on the lock screen and typing a PIN.
pub fn sign_in_pin(store: &mut Store, user: UserId, pin: &str) -> Result<SignInOutcome> {
    attempt(store, Some(user), Secret::Pin, pin)
}

/// Whether this device asks who is there on start: as soon as an active
/// user has a password or PIN. Until then the owner works without signing in.
pub fn sign_in_required(store: &Store) -> Result<bool> {
    store.read(|r| {
        Ok(r.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM user_credentials c JOIN users u ON u.id = c.user_id
             WHERE u.active = 1 AND u.deleted_at IS NULL
               AND (c.password_hash IS NOT NULL OR c.pin_hash IS NOT NULL))",
            [],
            |row| row.get(0),
        )?)
    })
}

/// The session of a user without signing in (a device whose users have no
/// secrets yet; see [`sign_in_required`]).
pub fn session_of(store: &Store, user: UserId) -> Result<SignedIn> {
    store.read(|r| {
        let o = org::the_org(r.conn())?.ok_or(Error::NotFound("organization"))?;
        signed_in(r.conn(), o.org_id()?, user)
    })
}

// ---------------------------------------------------------------------------
// Supervisor approval (offline)

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveInput {
    pub supervisor_id: String,
    pub pin: String,
    pub permission: String,
    pub branch_id: Option<String>,
    pub warehouse_id: Option<String>,
    pub register_id: Option<String>,
    pub reason: String,
}

impl ApproveInput {
    fn target(&self) -> Result<Target> {
        Ok(Target {
            branch: self
                .branch_id
                .as_deref()
                .map(parse_id::<BranchId>)
                .transpose()?,
            warehouse: self
                .warehouse_id
                .as_deref()
                .map(parse_id::<WarehouseId>)
                .transpose()?,
            register: self
                .register_id
                .as_deref()
                .map(parse_id::<RegisterId>)
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalOutcome {
    /// Pass `id` with the operation it was asked for, before `expires_at`.
    Approved { id: String, expires_at: i64 },
    /// `access.bad_credentials`, `access.locked`, `access.not_supervisor`.
    Refused { code: String },
}

/// A supervisor approves, with their PIN, a sensitive action for the user
/// running `meta` (the requester).
pub fn approve(store: &mut Store, meta: OpMeta, input: ApproveInput) -> Result<ApprovalOutcome> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::AccessApprove {
            input: input.clone(),
        },
    )?;
    let p = Permission::from_code(&input.permission)
        .filter(|p| p.is_sensitive())
        .ok_or(Error::from(RuleError::on("permission", "invalid_value")))?;
    if input.reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    let supervisor: UserId = parse_id(&input.supervisor_id)?;
    let target = input.target()?;
    let creds = store.read(|r| repo::credentials(r.conn(), supervisor))?;
    let stored = creds.as_ref().and_then(|c| c.2.clone());
    let matches =
        secret::verify(&input.pin, stored.as_deref().unwrap_or(decoy())) && stored.is_some();
    store.write(meta, |w| {
        let org = w.org()?;
        let requester = w.actor()?;
        if requester == supervisor {
            return Err(RuleError::on("supervisor", "access.self_approval").into());
        }
        let refuse = |w: &mut Writer<'_>, code: &str| -> Result<ApprovalOutcome> {
            let detail = serde_json::json!({
                "permission": p.code(), "supervisor": supervisor.to_string(), "refused": code
            });
            w.event("approval.refused", "user", &requester.to_string(), &detail)?;
            w.audit("approval.refused", "user", &requester.to_string(), &detail)?;
            Ok(ApprovalOutcome::Refused { code: code.into() })
        };
        let Some((_, _, _, failures, locked)) = creds.clone() else {
            return refuse(w, "access.bad_credentials");
        };
        if access::is_locked(locked, w.now()) {
            return refuse(w, "access.locked");
        }
        if !matches {
            let (n, until) = access::after_failure(failures, w.now());
            repo::set_failures(w, supervisor, n, until)?;
            return refuse(
                w,
                if until.is_some() {
                    "access.locked"
                } else {
                    "access.bad_credentials"
                },
            );
        }
        repo::set_failures(w, supervisor, 0, None)?;
        let (active, grants) = repo::grants(w.tx(), supervisor)?;
        if access::authorize(&grants, active, p, &target) != Decision::Allow {
            return refuse(w, "access.not_supervisor");
        }
        let id = ApprovalId::new();
        let expires_at = w.now() + APPROVAL_TTL_MS;
        repo::insert_approval(
            w,
            org,
            id,
            p,
            requester,
            supervisor,
            input.reason.trim(),
            &target,
            expires_at,
        )?;
        let detail = serde_json::json!({
            "permission": p.code(),
            "requested_by": requester.to_string(),
            "approved_by": supervisor.to_string(),
            "reason": input.reason.trim(),
        });
        w.event("approval.granted", "approval", &id.to_string(), &detail)?;
        w.audit("approve", "approval", &id.to_string(), &detail)?;
        Ok(ApprovalOutcome::Approved {
            id: id.to_string(),
            expires_at,
        })
    })
}

/// Users who may approve `permission` on `target` (for the approval dialog).
pub fn supervisors(store: &Store, permission: Permission, target: Target) -> Result<Vec<UserRow>> {
    store.read(|r| {
        let Some(o) = org::the_org(r.conn())? else {
            return Ok(vec![]);
        };
        let mut out = Vec::new();
        for u in repo::users(r.conn(), o.org_id()?)? {
            let (active, grants) = repo::grants(r.conn(), parse_id(&u.id)?)?;
            if u.has_pin
                && access::authorize(&grants, active, permission, &target) == Decision::Allow
            {
                out.push(u);
            }
        }
        Ok(out)
    })
}

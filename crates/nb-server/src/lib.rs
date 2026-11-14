//! The sync server (M11/M1, ADR-0006). Each organization is one SQLite
//! file running the same `nb-store` use cases as the devices; a device
//! sends its operation log and the server runs each operation again, as
//! the same operation (same id, time, device): the server's copy becomes
//! the device's, row for row, which `fingerprint` shows.
//!
//! A small registry (`control.sqlite`) knows the organizations and their
//! devices (a token each, kept hashed). Linking a device sends its whole
//! database once; afterwards only operations travel.
//!
//! API (JSON unless said):
//! - `GET  /v1/health`
//! - `POST /v1/link`     enrollment key; body: the database (bytes) → device token
//! - `POST /v1/push`     device token; `{ops: [LoggedOp]}` → what was taken
//! - `GET  /v1/state`    device token → organization, fingerprint, operations
//! - `GET  /v1/snapshot` device token → the organization's database (bytes)
//! - `POST /v1/reseed?after=` device token; body: the database → replaces the
//!   copy (only a device that has every other device's operations), and
//!   starts a new epoch of the log: the other devices take the copy again
//! - `POST /v1/invite`   device token → a code for a new device (an hour, once)
//! - `POST /v1/join`     `{code, name}` → the new device's identity and token
//! - `GET  /v1/pull?after=` device token → the other devices' operations
//! - `GET  /v1/devices`  device token → the organization's devices
//! - `POST /v1/devices/{id}/revoke` device token → that device's token stops
//!
//! `/v1/snapshot` tells the position the copy is at in `x-nb-cursor` and
//! `x-nb-epoch`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use nb_store::ops::OpDevice;
pub use nb_store::sync::{
    CURSOR_HEADER, DeviceInfo, EPOCH_HEADER, Invite, Joined, Linked, Position, PullResult,
    PushFailure, PushRequest, PushResult, StateInfo,
};
use nb_store::{Store, StoreOptions};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

/// Largest database a device may send (a big shop is a few hundred MB).
pub const MAX_DB: usize = 1 << 30;

/// What went wrong, as the API answers it.
#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    BadRequest(String),
    Conflict(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, detail) = match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", String::new()),
            ApiError::BadRequest(d) => (StatusCode::BAD_REQUEST, "bad_request", d),
            ApiError::Conflict(d) => (StatusCode::CONFLICT, "conflict", d),
            ApiError::Internal(d) => (StatusCode::INTERNAL_SERVER_ERROR, "internal", d),
        };
        (
            status,
            axum::Json(serde_json::json!({ "error": code, "detail": detail })),
        )
            .into_response()
    }
}

impl From<nb_store::Error> for ApiError {
    fn from(e: nb_store::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// The server's state: where the organizations live, the registry, the
/// open organizations (one writer each).
pub struct Server {
    dir: PathBuf,
    enrollment_key: String,
    control: Mutex<Connection>,
    tenants: Mutex<HashMap<String, Tenant>>,
}

/// An open organization: `None` once its file was replaced (a reseed), so
/// a request that waited for it opens the new one.
type Tenant = Arc<Mutex<Option<Store>>>;

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn new_token() -> ApiResult<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn lock<T>(m: &Mutex<T>) -> ApiResult<std::sync::MutexGuard<'_, T>> {
    m.lock()
        .map_err(|_| ApiError::Internal("lock poisoned".into()))
}

/// A registered device, as its token says.
#[derive(Debug, Clone)]
struct DeviceRow {
    id: String,
    org: String,
    prefix: String,
}

impl Server {
    /// Opens (or creates) the server's data in `dir`. `enrollment_key` is
    /// what a device shows to link a new organization.
    pub fn open(dir: &Path, enrollment_key: &str) -> ApiResult<Arc<Self>> {
        if enrollment_key.trim().len() < 12 {
            return Err(ApiError::BadRequest(
                "the enrollment key needs at least 12 characters".into(),
            ));
        }
        std::fs::create_dir_all(dir.join("tenants"))
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let control = Connection::open(dir.join("control.sqlite"))?;
        control.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS orgs (
                 id          TEXT PRIMARY KEY,
                 name        TEXT NOT NULL,
                 created_at  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS devices (
                 id           TEXT PRIMARY KEY,
                 org_id       TEXT NOT NULL REFERENCES orgs(id),
                 prefix       TEXT NOT NULL,
                 name         TEXT NOT NULL,
                 token_hash   TEXT NOT NULL UNIQUE,
                 created_at   INTEGER NOT NULL,
                 last_seen_at INTEGER,
                 revoked_at   INTEGER,
                 UNIQUE (org_id, prefix)
             );
             CREATE TABLE IF NOT EXISTS invites (
                 code_hash   TEXT PRIMARY KEY,
                 org_id      TEXT NOT NULL REFERENCES orgs(id),
                 made_by     TEXT NOT NULL,
                 expires_at  INTEGER NOT NULL,
                 used_at     INTEGER
             );",
        )?;
        let has_epoch: bool = control.query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('orgs') WHERE name = 'epoch')",
            [],
            |r| r.get(0),
        )?;
        if !has_epoch {
            control.execute(
                "ALTER TABLE orgs ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        Ok(Arc::new(Self {
            dir: dir.to_path_buf(),
            enrollment_key: enrollment_key.trim().to_owned(),
            control: Mutex::new(control),
            tenants: Mutex::new(HashMap::new()),
        }))
    }

    fn tenant_path(&self, org: &str) -> PathBuf {
        self.dir.join("tenants").join(format!("{org}.sqlite"))
    }

    fn tenant(&self, org: &str) -> ApiResult<Tenant> {
        let mut map = lock(&self.tenants)?;
        if let Some(t) = map.get(org) {
            return Ok(t.clone());
        }
        let store = Store::open(&self.tenant_path(org), StoreOptions::default())?;
        let t = Arc::new(Mutex::new(Some(store)));
        map.insert(org.to_owned(), t.clone());
        Ok(t)
    }

    /// Runs `f` on `org`'s database, alone (one writer per organization).
    fn with_tenant<R>(
        &self,
        org: &str,
        f: impl FnOnce(&mut Store) -> ApiResult<R>,
    ) -> ApiResult<R> {
        for _ in 0..3 {
            let tenant = self.tenant(org)?;
            let mut guard = lock(&tenant)?;
            if let Some(store) = guard.as_mut() {
                return f(store);
            }
            // Replaced while this waited: the map has the new one.
        }
        Err(ApiError::Internal("organization busy".into()))
    }

    /// Which copy of `org`'s log positions count in.
    fn epoch(&self, org: &str) -> ApiResult<i64> {
        Ok(lock(&self.control)?
            .query_row("SELECT epoch FROM orgs WHERE id = ?1", [org], |r| r.get(0))?)
    }

    fn device(&self, headers: &HeaderMap) -> ApiResult<DeviceRow> {
        let token = bearer(headers).ok_or(ApiError::Unauthorized)?;
        let control = lock(&self.control)?;
        let row = control
            .query_row(
                "SELECT id, org_id, prefix FROM devices WHERE token_hash = ?1 AND revoked_at IS NULL",
                [hash(&token)],
                |r| {
                    Ok(DeviceRow {
                        id: r.get(0)?,
                        org: r.get(1)?,
                        prefix: r.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or(ApiError::Unauthorized)?;
        control.execute(
            "UPDATE devices SET last_seen_at = ?2 WHERE id = ?1",
            params![row.id, now_ms()],
        )?;
        Ok(row)
    }

    /// Checks a database a device sent and puts it in place as `org`'s copy
    /// (`None`: the organization it holds, new here). Returns the
    /// organization, its name and the device in it.
    fn install(
        &self,
        db: &[u8],
        expect_org: Option<&str>,
    ) -> ApiResult<(String, String, String, String)> {
        let tmp = self.dir.join(format!("incoming-{}.sqlite", new_token()?));
        std::fs::write(&tmp, db).map_err(|e| ApiError::Internal(e.to_string()))?;
        let checked = (|| -> ApiResult<(String, String, String, String)> {
            let store = Store::open(&tmp, StoreOptions::default())
                .map_err(|e| ApiError::BadRequest(format!("not a database of this app: {e}")))?;
            let health = store.health()?;
            if !health.is_sound() {
                return Err(ApiError::BadRequest(format!(
                    "database not sound: {health:?}"
                )));
            }
            let org = store
                .read(|r| nb_store::repo::org::the_org(r.conn()))?
                .ok_or_else(|| ApiError::BadRequest("no organization in the database".into()))?;
            let org_id = org.org_id()?.to_string();
            if let Some(e) = expect_org
                && e != org_id
            {
                return Err(ApiError::Conflict("another organization".into()));
            }
            let device = store.device().clone();
            Ok((org_id, org.name, device.id.to_string(), device.prefix))
        })();
        let (org_id, name, device, prefix) = match checked {
            Ok(v) => v,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        // Close the open copy, keep the old file, put the new one in place.
        lock(&self.tenants)?.remove(&org_id);
        let path = self.tenant_path(&org_id);
        if path.exists() {
            let keep = self
                .dir
                .join("tenants")
                .join(format!("{org_id}.{}.bak", now_ms()));
            std::fs::rename(&path, keep).map_err(|e| ApiError::Internal(e.to_string()))?;
            for ext in ["-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
            }
        }
        std::fs::rename(&tmp, &path).map_err(|e| ApiError::Internal(e.to_string()))?;
        for ext in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", tmp.display()));
        }
        Ok((org_id, name, device, prefix))
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|t| t.trim().to_owned())
}

/// The routes.
pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/link", post(link))
        .route("/v1/push", post(push))
        .route("/v1/state", get(state))
        .route("/v1/snapshot", get(snapshot))
        .route("/v1/reseed", post(reseed))
        .route("/v1/invite", post(invite))
        .route("/v1/join", post(join))
        .route("/v1/pull", get(pull))
        .route("/v1/devices", get(devices))
        .route("/v1/devices/{id}/revoke", post(revoke))
        .layer(DefaultBodyLimit::max(MAX_DB))
        .with_state(server)
}

async fn health() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

/// Runs blocking work (SQLite) off the async threads.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
}

async fn link(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<axum::Json<Linked>> {
    let key = bearer(&headers).ok_or(ApiError::Unauthorized)?;
    if hash(&key) != hash(&server.enrollment_key) {
        return Err(ApiError::Unauthorized);
    }
    let name = headers
        .get("x-device-name")
        .and_then(|v| v.to_str().ok())
        .map(percent_decoded)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "device".to_owned());
    blocking(move || {
        // An organization links once; its other devices join it (M2).
        let probe = {
            let tmp = server.dir.join(format!("probe-{}.sqlite", new_token()?));
            std::fs::write(&tmp, &body).map_err(|e| ApiError::Internal(e.to_string()))?;
            let org = Store::open(&tmp, StoreOptions::default())
                .ok()
                .and_then(|s| s.read(|r| nb_store::repo::org::the_org(r.conn())).ok().flatten())
                .and_then(|o| o.org_id().ok())
                .map(|o| o.to_string());
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", tmp.display()));
            }
            org
        };
        if let Some(org) = &probe {
            let known: bool = lock(&server.control)?.query_row(
                "SELECT EXISTS (SELECT 1 FROM orgs WHERE id = ?1)",
                [org],
                |r| r.get(0),
            )?;
            if known {
                return Err(ApiError::Conflict("organization already linked".into()));
            }
        }
        let (org_id, org_name, device_id, prefix) = server.install(&body, None)?;
        let token = new_token()?;
        let control = lock(&server.control)?;
        control.execute(
            "INSERT INTO orgs (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![org_id, org_name, now_ms()],
        )?;
        control.execute(
            "INSERT INTO devices (id, org_id, prefix, name, token_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![device_id, org_id, prefix, name, hash(&token), now_ms()],
        )?;
        Ok(axum::Json(Linked {
            org_id,
            org_name,
            device_id,
            token,
        }))
    })
    .await
}

async fn push(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<PushRequest>,
) -> ApiResult<axum::Json<PushResult>> {
    let device = server.device(&headers)?;
    blocking(move || {
        server.with_tenant(&device.org.clone(), |store| push_into(store, &device, req))
    })
    .await
}

fn push_into(
    store: &mut Store,
    device: &DeviceRow,
    req: PushRequest,
) -> ApiResult<axum::Json<PushResult>> {
    {
        let me = OpDevice {
            id: device
                .id
                .parse()
                .map_err(|_| ApiError::Internal("device id".into()))?,
            prefix: device.prefix.clone(),
        };
        let mut accepted = Vec::new();
        let mut failed = None;
        for mut op in req.ops {
            // The server decides who ran it: the device the token belongs
            // to, in its organization.
            if op.org.map(|o| o.to_string()) != Some(device.org.clone()) {
                failed = Some(PushFailure {
                    id: op.id,
                    code: "wrong_org".into(),
                    detail: String::new(),
                });
                break;
            }
            op.device = Some(me.clone());
            let id = op.id;
            let opaque = matches!(op.call, nb_store::ops::Call::Opaque { .. });
            match store.replay_log(vec![op]) {
                Ok(()) => accepted.push(id),
                Err(e) => {
                    failed = Some(PushFailure {
                        id,
                        code: if opaque {
                            "opaque".into()
                        } else {
                            "rejected".into()
                        },
                        detail: e.to_string(),
                    });
                    break;
                }
            }
        }
        let fingerprint = store.fingerprint()?;
        Ok(axum::Json(PushResult {
            accepted,
            failed,
            fingerprint,
        }))
    }
}

async fn state(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<StateInfo>> {
    let device = server.device(&headers)?;
    blocking(move || {
        let epoch = server.epoch(&device.org)?;
        server.with_tenant(&device.org, |store| {
            Ok(axum::Json(state_of(store, &device.org, epoch)?))
        })
    })
    .await
}

fn state_of(store: &Store, org: &str, epoch: i64) -> ApiResult<StateInfo> {
    let operations: i64 =
        store
            .connection()
            .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))?;
    Ok(StateInfo {
        org_id: org.to_owned(),
        fingerprint: store.fingerprint()?,
        operations,
        cursor: store.log_end()?,
        epoch,
    })
}

async fn snapshot(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult<Response> {
    let device = server.device(&headers)?;
    blocking(move || {
        let epoch = server.epoch(&device.org)?;
        let (db, cursor) = server.with_tenant(&device.org, |store| {
            Ok((store.snapshot()?, store.log_end()?))
        })?;
        Ok((
            [
                (CURSOR_HEADER, cursor.to_string()),
                (EPOCH_HEADER, epoch.to_string()),
            ],
            db,
        )
            .into_response())
    })
    .await
}

async fn reseed(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<PullQuery>,
    body: Bytes,
) -> ApiResult<axum::Json<StateInfo>> {
    let device = server.device(&headers)?;
    blocking(move || {
        let epoch = server.epoch(&device.org)?;
        {
            // Alone on the organization: nothing lands between the check
            // and the new file.
            let tenant = server.tenant(&device.org)?;
            let mut guard = lock(&tenant)?;
            let store = guard
                .as_ref()
                .ok_or_else(|| ApiError::Conflict("busy".into()))?;
            // Only a device that has what the others sent may replace the
            // copy (or it would lose it).
            if q.epoch != epoch {
                return Err(ApiError::Conflict("behind".into()));
            }
            let mut after = q.after;
            loop {
                let rows = store.log_after(after, 1_000)?;
                if rows
                    .iter()
                    .any(|r| r.2.as_deref() != Some(device.id.as_str()))
                {
                    return Err(ApiError::Conflict("behind".into()));
                }
                match rows.last() {
                    Some(r) if rows.len() == 1_000 => after = r.0,
                    _ => break,
                }
            }
            // Closed first; who waited for the old file opens the new one.
            *guard = None;
            server.install(&body, Some(&device.org))?;
        }
        let epoch = epoch + 1;
        lock(&server.control)?.execute(
            "UPDATE orgs SET epoch = ?2 WHERE id = ?1",
            params![device.org, epoch],
        )?;
        server.with_tenant(&device.org, |store| {
            Ok(axum::Json(state_of(store, &device.org, epoch)?))
        })
    })
    .await
}

/// Codes are short enough to read aloud: no 0/O, 1/I/L.
const CODE_LETTERS: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

fn new_code() -> ApiResult<String> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(b.iter()
        .map(|x| CODE_LETTERS[usize::from(*x) % CODE_LETTERS.len()] as char)
        .collect())
}

/// The first prefix no device of `org` has: A … Z, then AA … ZZ.
fn free_prefix(control: &Connection, org: &str) -> ApiResult<String> {
    let used: Vec<String> = {
        let mut st = control.prepare("SELECT prefix FROM devices WHERE org_id = ?1")?;
        st.query_map([org], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    let letters: Vec<char> = ('A'..='Z').collect();
    let singles = letters.iter().map(|c| c.to_string());
    let doubles = letters
        .iter()
        .flat_map(|a| letters.iter().map(move |b| format!("{a}{b}")));
    singles
        .chain(doubles)
        .find(|p| !used.contains(p))
        .ok_or_else(|| ApiError::Conflict("no prefix left".into()))
}

/// A linked device makes a code (valid an hour, once) for a new device of
/// its organization.
async fn invite(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Invite>> {
    let device = server.device(&headers)?;
    let code = new_code()?;
    let expires_at = now_ms() + 3_600_000;
    lock(&server.control)?.execute(
        "INSERT INTO invites (code_hash, org_id, made_by, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![hash(&code), device.org, device.id, expires_at],
    )?;
    Ok(axum::Json(Invite { code, expires_at }))
}

/// What a new device sends to join.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct JoinRequest {
    pub code: String,
    #[serde(default)]
    pub name: String,
}

async fn join(
    State(server): State<Arc<Server>>,
    axum::Json(req): axum::Json<JoinRequest>,
) -> ApiResult<axum::Json<Joined>> {
    let code = req.code.trim().to_uppercase().replace([' ', '-'], "");
    blocking(move || {
        let (org_id, org_name, device_id, prefix, token) = {
            let control = lock(&server.control)?;
            let org: Option<String> = control
                .query_row(
                    "SELECT org_id FROM invites WHERE code_hash = ?1 AND used_at IS NULL AND expires_at > ?2",
                    params![hash(&code), now_ms()],
                    |r| r.get(0),
                )
                .optional()?;
            let org = org.ok_or(ApiError::Unauthorized)?;
            control.execute(
                "UPDATE invites SET used_at = ?2 WHERE code_hash = ?1",
                params![hash(&code), now_ms()],
            )?;
            let name: String =
                control.query_row("SELECT name FROM orgs WHERE id = ?1", [&org], |r| r.get(0))?;
            let prefix = free_prefix(&control, &org)?;
            let device_id = nb_core::DeviceId::new().to_string();
            let token = new_token()?;
            control.execute(
                "INSERT INTO devices (id, org_id, prefix, name, token_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    device_id,
                    org,
                    prefix,
                    if req.name.trim().is_empty() { "device" } else { req.name.trim() },
                    hash(&token),
                    now_ms()
                ],
            )?;
            (org, name, device_id, prefix, token)
        };
        let cursor = server.with_tenant(&org_id, |store| Ok(store.log_end()?))?;
        Ok(axum::Json(Joined {
            org_id,
            org_name,
            device_id,
            prefix,
            token,
            cursor,
        }))
    })
    .await
}

#[derive(Debug, serde::Deserialize)]
pub struct PullQuery {
    #[serde(default)]
    pub after: i64,
    #[serde(default)]
    pub epoch: i64,
}

/// The operations of the other devices after the device's position.
async fn pull(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<PullQuery>,
) -> ApiResult<axum::Json<PullResult>> {
    let device = server.device(&headers)?;
    blocking(move || {
        let prefixes: HashMap<String, String> = {
            let control = lock(&server.control)?;
            let mut st = control.prepare("SELECT id, prefix FROM devices WHERE org_id = ?1")?;
            st.query_map([&device.org], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        let epoch = server.epoch(&device.org)?;
        server.with_tenant(&device.org, |store| {
            const BATCH: u32 = 500;
            let rows = store.log_after(q.after, BATCH)?;
            let more = rows.len() == BATCH as usize;
            let cursor = rows.last().map_or(q.after.max(0), |r| r.0);
            let mut ops = Vec::new();
            for (_, mut op, dev) in rows {
                // Its own operations the device has already.
                if dev.as_deref() == Some(device.id.as_str()) {
                    continue;
                }
                op.device = match dev.and_then(|d| prefixes.get(&d).map(|p| (d, p.clone()))) {
                    Some((d, prefix)) => Some(OpDevice {
                        id: d
                            .parse()
                            .map_err(|_| ApiError::Internal("device id".into()))?,
                        prefix,
                    }),
                    None => None,
                };
                ops.push(op);
            }
            let end = store.log_end()?;
            let cursor = if more { cursor } else { end.max(cursor) };
            Ok(axum::Json(PullResult {
                ops,
                cursor,
                more,
                fingerprint: store.fingerprint()?,
                epoch,
            }))
        })
    })
    .await
}

async fn devices(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Vec<DeviceInfo>>> {
    let me = server.device(&headers)?;
    let control = lock(&server.control)?;
    let mut st = control.prepare(
        "SELECT id, prefix, name, created_at, last_seen_at, revoked_at FROM devices
         WHERE org_id = ?1 ORDER BY created_at, prefix",
    )?;
    let rows = st.query_map([&me.org], |r| {
        let id: String = r.get(0)?;
        Ok(DeviceInfo {
            this: id == me.id,
            id,
            prefix: r.get(1)?,
            name: r.get(2)?,
            created_at: r.get(3)?,
            last_seen_at: r.get(4)?,
            revoked_at: r.get(5)?,
        })
    })?;
    Ok(axum::Json(rows.collect::<Result<_, _>>()?))
}

/// A lost or replaced device stops: its token no longer opens anything.
/// Its letter stays taken (its numbers stay its own).
async fn revoke(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<StatusCode> {
    let me = server.device(&headers)?;
    let changed = lock(&server.control)?.execute(
        "UPDATE devices SET revoked_at = ?3 WHERE id = ?1 AND org_id = ?2 AND revoked_at IS NULL",
        params![id, me.org, now_ms()],
    )?;
    if changed == 0 {
        return Err(ApiError::BadRequest("no such device".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `%D8%A3…` back to text (a device name sent in a header).
fn percent_decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| char::from(c).to_digit(16);
        match (
            b[i],
            b.get(i + 1).copied().and_then(hex),
            b.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).chars().take(80).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_come_back_from_headers() {
        assert_eq!(
            super::percent_decoded("Android%20%C2%B7%20%D9%83%D8%B1%D9%8A%D9%85"),
            "Android · كريم"
        );
        assert_eq!(super::percent_decoded("plain"), "plain");
        assert_eq!(super::percent_decoded("bad%zz"), "bad%zz");
    }
}

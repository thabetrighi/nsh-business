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
//! - `POST /v1/reseed`   device token; body: the database → replaces the copy

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
pub use nb_store::sync::{Linked, PushFailure, PushRequest, PushResult, StateInfo};
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
    tenants: Mutex<HashMap<String, Arc<Mutex<Store>>>>,
}

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
             );",
        )?;
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

    fn tenant(&self, org: &str) -> ApiResult<Arc<Mutex<Store>>> {
        let mut map = lock(&self.tenants)?;
        if let Some(t) = map.get(org) {
            return Ok(t.clone());
        }
        let store = Store::open(&self.tenant_path(org), StoreOptions::default())?;
        let t = Arc::new(Mutex::new(store));
        map.insert(org.to_owned(), t.clone());
        Ok(t)
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
        .unwrap_or("device")
        .to_owned();
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
        let tenant = server.tenant(&device.org)?;
        let mut store = lock(&tenant)?;
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
    })
    .await
}

async fn state(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<StateInfo>> {
    let device = server.device(&headers)?;
    blocking(move || {
        let tenant = server.tenant(&device.org)?;
        let store = lock(&tenant)?;
        let operations: i64 =
            store
                .connection()
                .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))?;
        Ok(axum::Json(StateInfo {
            org_id: device.org.clone(),
            fingerprint: store.fingerprint()?,
            operations,
        }))
    })
    .await
}

async fn snapshot(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult<Vec<u8>> {
    let device = server.device(&headers)?;
    blocking(move || {
        let tenant = server.tenant(&device.org)?;
        let store = lock(&tenant)?;
        Ok(store.snapshot()?)
    })
    .await
}

async fn reseed(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<axum::Json<StateInfo>> {
    let device = server.device(&headers)?;
    blocking(move || {
        let (org_id, _, _, _) = server.install(&body, Some(&device.org))?;
        let tenant = server.tenant(&org_id)?;
        let store = lock(&tenant)?;
        let operations: i64 =
            store
                .connection()
                .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))?;
        Ok(axum::Json(StateInfo {
            org_id,
            fingerprint: store.fingerprint()?,
            operations,
        }))
    })
    .await
}

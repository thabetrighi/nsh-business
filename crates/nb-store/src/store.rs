//! Opening the database and identifying this device.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nb_core::{Clock, DeviceId, Hlc, SystemClock};
use rusqlite::{Connection, OptionalExtension};

use crate::migrate;
use crate::{Error, Result};

/// How to open a store.
#[derive(Clone)]
pub struct StoreOptions {
    pub clock: Arc<dyn Clock>,
    /// Letters put before this device's document numbers (1–3, A–Z).
    pub device_prefix: String,
    pub device_name: String,
    /// The store's offset from UTC, in minutes (Algeria: 60): business dates.
    pub utc_offset_min: i32,
    /// This device's id when the database is new (default: a new one). A
    /// replica of a device's data (the server's, a test's) takes its id.
    pub device_id: Option<DeviceId>,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            device_prefix: "A".into(),
            device_name: "device".into(),
            utc_offset_min: 60,
            device_id: None,
        }
    }
}

/// This device, as recorded in its database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub prefix: String,
}

/// What [`Store::health`] found; all empty or zero when sound.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Health {
    /// SQLite's own findings on the file.
    pub integrity: Vec<String>,
    /// Rows pointing at nothing.
    pub foreign_keys: usize,
    /// Entries whose debits and credits differ.
    pub unbalanced: usize,
    /// Stock balances that differ from their replayed movements.
    pub stock_drift: usize,
    /// The first audit row whose hash does not follow.
    pub audit_broken_at: Option<i64>,
    /// Customers whose balance differs from their receivable lines.
    #[serde(default)]
    pub customer_drift: usize,
    /// Suppliers whose balance differs from their payable lines.
    #[serde(default)]
    pub supplier_drift: usize,
}

impl Health {
    pub fn is_sound(&self) -> bool {
        self.integrity.is_empty()
            && self.foreign_keys == 0
            && self.unbalanced == 0
            && self.stock_drift == 0
            && self.audit_broken_at.is_none()
            && self.customer_drift == 0
            && self.supplier_drift == 0
    }
}

/// The local database. One per organization file; used from one thread.
pub struct Store {
    pub(crate) conn: Connection,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) hlc: Hlc,
    pub(crate) device: Device,
    pub(crate) utc_offset_min: i32,
    path: Option<PathBuf>,
    /// Calls recorded by use cases, waiting for their operation's write
    /// (by operation id; a use case that fails before writing leaves one,
    /// dropped when it gets old).
    pub(crate) calls: Vec<(nb_core::OperationId, String)>,
    /// Inside `replay_log`.
    pub(crate) replaying: bool,
}

/// Something running another device's operation here left to look at
/// (`stock_shortfall`: goods sold on two devices that were not there).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Review {
    pub id: String,
    pub kind: String,
    pub entity: String,
    pub entity_id: String,
    /// JSON.
    pub detail: String,
    pub created_at: i64,
}

impl Store {
    /// Opens (or creates) the database file at `path` and migrates it.
    pub fn open(path: &Path, opts: StoreOptions) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        // WAL: readers never block the writer; FULL: a committed sale
        // survives a power cut (ADR-0002).
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        Self::configure(&mut conn)?;
        migrate::migrate_to(&mut conn, migrate::latest(), Some(path))?;
        Self::finish(conn, opts, Some(path.to_path_buf()))
    }

    /// A database in memory (tests, demos).
    pub fn open_in_memory(opts: StoreOptions) -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        Self::configure(&mut conn)?;
        migrate::migrate_to(&mut conn, migrate::latest(), None)?;
        Self::finish(conn, opts, None)
    }

    fn configure(conn: &mut Connection) -> Result<()> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(())
    }

    fn finish(conn: Connection, opts: StoreOptions, path: Option<PathBuf>) -> Result<Self> {
        nb_domain::numbering::validate_device_prefix(&opts.device_prefix)?;
        let existing: Option<(String, String)> = conn
            .query_row("SELECT id, prefix FROM device LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let device = match existing {
            Some((id, prefix)) => Device {
                id: id
                    .parse()
                    .map_err(|_| Error::Corrupt(format!("device id {id}")))?,
                prefix,
            },
            None => {
                let d = Device {
                    id: opts.device_id.unwrap_or_default(),
                    prefix: opts.device_prefix.clone(),
                };
                conn.execute(
                    "INSERT INTO device (id, prefix, name, created_at) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        d.id.to_string(),
                        d.prefix,
                        opts.device_name,
                        opts.clock.now_ms()
                    ],
                )?;
                d
            }
        };
        // The device id's last 8 bytes order concurrent events between devices.
        let node = u64::from_be_bytes(
            device.id.uuid().as_bytes()[8..16]
                .try_into()
                .map_err(|_| Error::Corrupt("device id".into()))?,
        );
        Ok(Self {
            conn,
            clock: opts.clock,
            hlc: Hlc::new(node),
            device,
            utc_offset_min: opts.utc_offset_min,
            path,
            calls: Vec::new(),
            replaying: false,
        })
    }

    /// Records the call operation `meta` is about to write (kept with the
    /// operation for replicas, ADR-0006).
    pub fn record(&mut self, meta: crate::OpMeta, call: &crate::ops::Call) -> Result<()> {
        let text = serde_json::to_string(call)?;
        self.calls.retain(|(id, _)| *id != meta.id);
        self.calls.push((meta.id, text));
        if self.calls.len() > 16 {
            self.calls.remove(0);
        }
        Ok(())
    }

    /// Runs the operations of `log` again here, in order: (operation id,
    /// kind, organization, actor, time, call). Operations already here are
    /// skipped (their stored result comes back).
    pub fn replay_log(&mut self, log: Vec<crate::ops::LoggedOp>) -> Result<()> {
        let real = self.clock.clone();
        let own = self.device.clone();
        for op in log {
            self.clock = std::sync::Arc::new(nb_core::FixedClock::new(op.at));
            self.device = match &op.device {
                Some(d) => Device {
                    id: d.id,
                    prefix: d.prefix.clone(),
                },
                None => own.clone(),
            };
            let meta = crate::OpMeta {
                id: op.id,
                kind: crate::ops::intern(&op.kind),
                org: op.org,
                actor: op.actor,
            };
            let below = self.below_zero()?;
            self.replaying = true;
            let run = nb_core::id::idgen::deterministic(op.id.uuid(), op.at, || {
                crate::ops::replay(self, meta, op.call)
            });
            self.replaying = false;
            if run.is_ok() {
                self.note_shortfalls(&below, op.id, op.at)?;
            }
            if let Err(e) = run {
                self.clock = real;
                self.device = own;
                return Err(e);
            }
        }
        self.clock = real;
        self.device = own;
        Ok(())
    }

    /// Stock below zero now: (product, warehouse) → quantity (thousandths).
    fn below_zero(&self) -> Result<std::collections::BTreeMap<(String, String), i64>> {
        let mut st = self.conn.prepare(
            "SELECT product_id, warehouse_id, on_hand_milli FROM stock_balances WHERE on_hand_milli < 0",
        )?;
        let rows = st.query_map([], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Notes stock that an operation run again took (further) below zero:
    /// the goods left on another device before this one knew.
    fn note_shortfalls(
        &mut self,
        before: &std::collections::BTreeMap<(String, String), i64>,
        op: nb_core::OperationId,
        at: i64,
    ) -> Result<()> {
        let after = self.below_zero()?;
        for ((product, warehouse), qty) in after {
            let was = before
                .get(&(product.clone(), warehouse.clone()))
                .copied()
                .unwrap_or(0);
            if qty < was {
                self.conn.execute(
                    "INSERT INTO reviews (id, kind, entity, entity_id, detail, operation_id, created_at)
                     VALUES (?1, 'stock_shortfall', 'product', ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        nb_core::OperationId::new().to_string(),
                        product,
                        serde_json::json!({ "warehouse": warehouse, "on_hand_milli": qty, "before_milli": was })
                            .to_string(),
                        op.to_string(),
                        at
                    ],
                )?;
            }
        }
        Ok(())
    }

    /// Open review items: (id, kind, entity, entity id, detail, time).
    pub fn open_reviews(&self) -> Result<Vec<Review>> {
        let mut st = self.conn.prepare(
            "SELECT id, kind, entity, entity_id, detail, created_at FROM reviews
             WHERE resolved_at IS NULL ORDER BY created_at DESC LIMIT 500",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Review {
                id: r.get(0)?,
                kind: r.get(1)?,
                entity: r.get(2)?,
                entity_id: r.get(3)?,
                detail: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Takes this device's identity in a database that came from elsewhere
    /// (the server's copy, when a device joins or catches up): its rows and
    /// numbers from now on carry `id` and `prefix`.
    pub fn adopt_device(&mut self, id: nb_core::DeviceId, prefix: &str) -> Result<()> {
        nb_domain::numbering::validate_device_prefix(prefix)?;
        self.conn.execute(
            "UPDATE device SET id = ?1, prefix = ?2",
            rusqlite::params![id.to_string(), prefix],
        )?;
        self.device = Device {
            id,
            prefix: prefix.to_owned(),
        };
        let node = u64::from_be_bytes(
            id.uuid().as_bytes()[8..16]
                .try_into()
                .map_err(|_| Error::Corrupt("device id".into()))?,
        );
        self.hlc = Hlc::new(node);
        Ok(())
    }

    /// The operations recorded here with their calls, oldest first.
    pub fn operation_log(&self) -> Result<Vec<crate::ops::LoggedOp>> {
        self.logged("WHERE call IS NOT NULL ORDER BY created_at, rowid")
    }

    /// The operations not sent to the server yet, oldest first (at most
    /// `limit`).
    pub fn unpushed(&self, limit: u32) -> Result<Vec<crate::ops::LoggedOp>> {
        self.logged(&format!(
            "WHERE call IS NOT NULL AND pushed_at IS NULL ORDER BY created_at, rowid LIMIT {}",
            limit.clamp(1, 10_000)
        ))
    }

    /// How many operations wait to be sent.
    pub fn unpushed_count(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM operations WHERE call IS NOT NULL AND pushed_at IS NULL",
            [],
            |r| r.get(0),
        )?)
    }

    /// Notes that the server took these operations.
    pub fn mark_pushed(&mut self, ids: &[nb_core::OperationId]) -> Result<()> {
        let now = self.clock.now_ms();
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE operations SET pushed_at = ?2 WHERE id = ?1",
                rusqlite::params![id.to_string(), now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Notes that everything up to now is on the server (it received the
    /// whole database).
    pub fn mark_all_pushed(&mut self) -> Result<()> {
        let now = self.clock.now_ms();
        self.conn.execute(
            "UPDATE operations SET pushed_at = ?1 WHERE pushed_at IS NULL",
            [now],
        )?;
        Ok(())
    }

    /// The log after position `after` (the server's order), at most
    /// `limit`: (position, operation, the device that ran it). The server
    /// sends these to the other devices.
    pub fn log_after(
        &self,
        after: i64,
        limit: u32,
    ) -> Result<Vec<(i64, crate::ops::LoggedOp, Option<String>)>> {
        let mut st = self.conn.prepare(
            "SELECT rowid, id, kind, org_id, actor_id, created_at, call, device_id FROM operations
             WHERE rowid > ?1 AND call IS NOT NULL ORDER BY rowid LIMIT ?2",
        )?;
        let rows = st.query_map(rusqlite::params![after, limit.clamp(1, 5_000)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (pos, id, kind, org, actor, at, call, device) = row?;
            let bad = |what: &str| Error::Corrupt(format!("operation {id}: {what}"));
            out.push((
                pos,
                crate::ops::LoggedOp {
                    id: id.parse().map_err(|_| bad("id"))?,
                    kind,
                    org: org.map(|o| o.parse()).transpose().map_err(|_| bad("org"))?,
                    actor: actor
                        .map(|a| a.parse())
                        .transpose()
                        .map_err(|_| bad("actor"))?,
                    at,
                    device: None,
                    call: serde_json::from_str(&call)?,
                },
                device,
            ));
        }
        Ok(out)
    }

    /// The last position in the log.
    pub fn log_end(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(MAX(rowid), 0) FROM operations", [], |r| {
                r.get(0)
            })?)
    }

    fn logged(&self, tail: &str) -> Result<Vec<crate::ops::LoggedOp>> {
        let own_prefix = self.device.prefix.clone();
        let own_id = self.device.id.to_string();
        let mut st = self.conn.prepare(&format!(
            "SELECT id, kind, org_id, actor_id, created_at, call, device_id FROM operations {tail}"
        ))?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, kind, org, actor, at, call, device) = row?;
            let bad = |what: &str| Error::Corrupt(format!("operation {id}: {what}"));
            out.push(crate::ops::LoggedOp {
                id: id.parse().map_err(|_| bad("id"))?,
                kind,
                org: org.map(|o| o.parse()).transpose().map_err(|_| bad("org"))?,
                actor: actor
                    .map(|a| a.parse())
                    .transpose()
                    .map_err(|_| bad("actor"))?,
                at,
                // Operations of this device carry it (for the server).
                device: match device {
                    Some(d) if d == own_id => Some(crate::ops::OpDevice {
                        id: d.parse().map_err(|_| bad("device"))?,
                        prefix: own_prefix.clone(),
                    }),
                    _ => None,
                },
                call: serde_json::from_str(&call)?,
            });
        }
        Ok(out)
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// The state of the business data as one hash (ADR-0006): two replicas
    /// that went through the same operations have the same fingerprint.
    /// Left out: what belongs to this device only (its identity, its
    /// operation log, outbox, audit chain, parked carts, backups, password
    /// hashes with their random salts) and the local insertion order
    /// (`seq` columns).
    pub fn fingerprint(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        for (t, part) in self.fingerprints()? {
            h.update(t.as_bytes());
            h.update(part.as_bytes());
        }
        Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    /// [`Store::fingerprint`] table by table (to find where two replicas
    /// differ).
    pub fn fingerprints(&self) -> Result<Vec<(String, String)>> {
        use sha2::{Digest, Sha256};
        let mut out = Vec::new();
        const LOCAL: [&str; 11] = [
            "reviews",
            "sync_link",
            "device",
            "operations",
            "outbox_events",
            "audit_log",
            "held_carts",
            "backup_settings",
            "backup_log",
            "user_credentials",
            "sqlite_sequence",
        ];
        let tables: Vec<String> = {
            let mut st = self.conn.prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )?;
            st.query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        // A search index's own tables (`*_fts_data`, …) are laid out by the
        // order rows arrived in; what it finds is the same.
        let derived = |t: &str| t.contains("_fts_");
        for t in tables
            .iter()
            .filter(|t| !LOCAL.contains(&t.as_str()) && !derived(t))
        {
            let mut h = Sha256::new();
            let cols: Vec<String> = {
                let mut st = self.conn.prepare(&format!("PRAGMA table_info(\"{t}\")"))?;
                st.query_map([], |r| r.get::<_, String>(1))?
                    .collect::<std::result::Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter(|c| c != "seq" && !c.ends_with("_seq"))
                    .collect()
            };
            if cols.is_empty() {
                continue;
            }
            let list = cols
                .iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let mut st = self.conn.prepare(&format!("SELECT {list} FROM \"{t}\""))?;
            let n = cols.len();
            let mut rows: Vec<String> = st
                .query_map([], |r| {
                    let mut line = String::new();
                    for k in 0..n {
                        use rusqlite::types::ValueRef;
                        match r.get_ref(k)? {
                            ValueRef::Null => line.push_str("N|"),
                            ValueRef::Integer(i) => line.push_str(&format!("I{i}|")),
                            ValueRef::Real(f) => line.push_str(&format!("R{f:?}|")),
                            ValueRef::Text(b) => {
                                line.push_str(&format!("T{}:", b.len()));
                                line.push_str(&String::from_utf8_lossy(b));
                                line.push('|');
                            }
                            ValueRef::Blob(b) => {
                                line.push_str(&format!("B{}:", b.len()));
                                for x in b {
                                    line.push_str(&format!("{x:02x}"));
                                }
                                line.push('|');
                            }
                        }
                    }
                    Ok(line)
                })?
                .collect::<std::result::Result<_, _>>()?;
            rows.sort_unstable();
            h.update((rows.len() as u64).to_be_bytes());
            for r in rows {
                h.update((r.len() as u64).to_be_bytes());
                h.update(r.as_bytes());
            }
            out.push((
                t.clone(),
                h.finalize().iter().map(|b| format!("{b:02x}")).collect(),
            ));
        }
        Ok(out)
    }

    /// The raw connection, for tests and maintenance tools only.
    #[doc(hidden)]
    /// The database checked from end to end (at start, after a crash, on
    /// demand): the file's own pages and keys, every entry balanced, the
    /// stock equal to its replayed movements, and the audit chain whole.
    pub fn health(&self) -> Result<Health> {
        let integrity: Vec<String> = {
            let mut s = self.conn.prepare("PRAGMA integrity_check")?;
            s.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|m| m != "ok")
                .collect()
        };
        let foreign_keys: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })?;
        let unbalanced = crate::repo::ledger::unbalanced_entries(&self.conn)?.len();
        let stock_drift = match crate::repo::org::the_org(&self.conn)? {
            Some(o) => crate::repo::stock::rebuild_balances(&self.conn, o.currency()?)?.len(),
            None => 0,
        };
        let audit_broken_at = self.verify_audit_chain()?;
        let customer_drift = crate::usecases::customers::customer_drift(&self.conn)?.len();
        let supplier_drift = crate::usecases::purchasing::supplier_drift(&self.conn)?.len();
        Ok(Health {
            integrity,
            foreign_keys: usize::try_from(foreign_keys).unwrap_or(usize::MAX),
            unbalanced,
            stock_drift,
            audit_broken_at,
            customer_drift,
            supplier_drift,
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// The schema version of the open database.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn scratch_file() -> PathBuf {
    let tag: String = nb_core::random_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::env::temp_dir().join(format!("nb-{tag}.db"))
}

/// What a restore found once the database was back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    /// The schema the backup had; the store migrated it to the latest.
    pub from_schema: u32,
    /// Stock balances that did not match their movements (rebuilt).
    pub stock_fixed: usize,
    pub health: Health,
    /// The database as it was before, kept next to its file.
    pub previous: Option<PathBuf>,
}

/// Checks a database file before it replaces the store's: a sound SQLite
/// file of this app, of a schema it knows. Returns that schema.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn checked_schema(file: &Path) -> Result<u32> {
    let bad = || Error::from(nb_domain::RuleError::on("backup", "backup.damaged"));
    let src = Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| bad())?;
    let ok: String = src
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|_| bad())?;
    if ok != "ok" {
        return Err(bad());
    }
    let v: u32 = src
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|_| bad())?;
    let has_org: i64 = src
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'organizations'",
            [],
            |r| r.get(0),
        )
        .map_err(|_| bad())?;
    if has_org == 0 || v == 0 {
        return Err(bad());
    }
    if v > migrate::latest() {
        return Err(nb_domain::RuleError::on("backup", "backup.too_new").into());
    }
    Ok(v)
}

/// Checks a snapshot's bytes as [`checked_schema`] does.
#[cfg(not(target_arch = "wasm32"))]
pub fn check_snapshot(db: &[u8]) -> Result<u32> {
    let tmp = scratch_file();
    std::fs::write(&tmp, db).map_err(|e| Error::Backup(e.to_string()))?;
    let r = checked_schema(&tmp);
    let _ = std::fs::remove_file(&tmp);
    r
}

#[cfg(not(target_arch = "wasm32"))]
impl Store {
    /// A consistent, compact copy of the database's file (`VACUUM INTO`),
    /// taken between operations.
    pub fn snapshot(&self) -> Result<Vec<u8>> {
        let tmp = scratch_file();
        let path = tmp
            .to_str()
            .ok_or_else(|| Error::Backup("temporary path is not UTF-8".into()))?;
        self.conn.execute("VACUUM INTO ?1", [path])?;
        let bytes = std::fs::read(&tmp).map_err(|e| Error::Backup(e.to_string()));
        let _ = std::fs::remove_file(&tmp);
        bytes
    }

    /// Replaces the whole database with `db` (a snapshot's bytes), in
    /// place: checked first (a sound SQLite file of a schema this app
    /// knows), the current one kept next to its file, then migrated, the
    /// device reread, the stock projection rebuilt and everything checked.
    pub fn restore_snapshot(&mut self, db: &[u8]) -> Result<Restored> {
        let tmp = scratch_file();
        std::fs::write(&tmp, db).map_err(|e| Error::Backup(e.to_string()))?;
        let result = self.restore_from(&tmp);
        let _ = std::fs::remove_file(&tmp);
        result
    }

    fn restore_from(&mut self, file: &Path) -> Result<Restored> {
        let from_schema = checked_schema(file)?;
        let previous = match &self.path {
            Some(p) => {
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| Error::Backup("database path has no file name".into()))?;
                let keep = p.with_file_name(format!("{name}.before-restore"));
                if keep.exists() {
                    std::fs::remove_file(&keep).map_err(|e| Error::Backup(e.to_string()))?;
                }
                let k = keep
                    .to_str()
                    .ok_or_else(|| Error::Backup("path is not UTF-8".into()))?;
                self.conn.execute("VACUUM INTO ?1", [k])?;
                Some(keep)
            }
            None => None,
        };
        self.conn.restore(
            rusqlite::MAIN_DB,
            file,
            None::<fn(rusqlite::backup::Progress)>,
        )?;
        migrate::migrate_to(&mut self.conn, migrate::latest(), None)?;
        let (id, prefix): (String, String) =
            self.conn
                .query_row("SELECT id, prefix FROM device LIMIT 1", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        self.device = Device {
            id: id
                .parse()
                .map_err(|_| Error::Corrupt(format!("device id {id}")))?,
            prefix,
        };
        let node = u64::from_be_bytes(
            self.device.id.uuid().as_bytes()[8..16]
                .try_into()
                .map_err(|_| Error::Corrupt("device id".into()))?,
        );
        self.hlc = Hlc::new(node);
        let stock_fixed = match crate::repo::org::the_org(&self.conn)? {
            Some(o) => crate::repo::stock::rebuild_balances(&self.conn, o.currency()?)?.len(),
            None => 0,
        };
        crate::repo::tracking::rebuild_lot_balances(&self.conn)?;
        Ok(Restored {
            from_schema,
            stock_fixed,
            health: self.health()?,
            previous,
        })
    }
}

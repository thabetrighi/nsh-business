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
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            device_prefix: "A".into(),
            device_name: "device".into(),
            utc_offset_min: 60,
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
                    id: DeviceId::new(),
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
        })
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
}

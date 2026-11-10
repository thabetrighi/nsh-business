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
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

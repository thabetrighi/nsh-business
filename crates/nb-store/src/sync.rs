//! What a device and its server say to each other (M11/M1, ADR-0006), and
//! where this device syncs to. The server (`nb-server`) and the app use the
//! same types.

use serde::{Deserialize, Serialize};

use crate::ops::LoggedOp;
use crate::{Result, Store};

/// What linking a database answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Linked {
    pub org_id: String,
    pub org_name: String,
    pub device_id: String,
    /// Given once: the device keeps it.
    pub token: String,
}

/// Operations a device sends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    pub ops: Vec<LoggedOp>,
}

/// What the server took: every operation up to the first it could not run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResult {
    pub accepted: Vec<nb_core::OperationId>,
    pub failed: Option<PushFailure>,
    /// The server's copy after these operations.
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushFailure {
    pub id: nb_core::OperationId,
    /// `opaque`: the operation cannot travel by itself (it holds a secret):
    /// send the whole database again. `rejected`: the server could not run
    /// it. `wrong_org`: not this device's organization.
    pub code: String,
    pub detail: String,
}

/// The server's copy, to compare with this one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateInfo {
    pub org_id: String,
    pub fingerprint: String,
    pub operations: i64,
}

/// Where this device syncs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncLink {
    pub server_url: String,
    pub token: String,
    pub linked_at: i64,
    pub last_push_at: Option<i64>,
    pub last_error: String,
}

impl Store {
    pub fn sync_link(&self) -> Result<Option<SyncLink>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .connection()
            .query_row(
                "SELECT server_url, token, linked_at, last_push_at, last_error FROM sync_link WHERE id = 1",
                [],
                |r| {
                    Ok(SyncLink {
                        server_url: r.get(0)?,
                        token: r.get(1)?,
                        linked_at: r.get(2)?,
                        last_push_at: r.get(3)?,
                        last_error: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Links this device to `server_url` with `token` (after the server
    /// took its database): everything here counts as sent.
    pub fn set_sync_link(&mut self, server_url: &str, token: &str) -> Result<()> {
        let now = self.now_ms();
        self.connection().execute(
            "INSERT INTO sync_link (id, server_url, token, linked_at, last_push_at) VALUES (1, ?1, ?2, ?3, ?3)
             ON CONFLICT (id) DO UPDATE SET server_url = ?1, token = ?2, linked_at = ?3,
                    last_push_at = ?3, last_error = ''",
            rusqlite::params![server_url.trim_end_matches('/'), token, now],
        )?;
        self.mark_all_pushed()
    }

    /// Stops syncing (the server keeps its copy).
    pub fn clear_sync_link(&mut self) -> Result<()> {
        self.connection().execute("DELETE FROM sync_link", [])?;
        Ok(())
    }

    /// Notes how the last exchange went (empty `error`: well).
    pub fn note_sync(&mut self, error: &str) -> Result<()> {
        let now = self.now_ms();
        if error.is_empty() {
            self.connection().execute(
                "UPDATE sync_link SET last_push_at = ?1, last_error = '' WHERE id = 1",
                [now],
            )?;
        } else {
            self.connection()
                .execute("UPDATE sync_link SET last_error = ?1 WHERE id = 1", [error])?;
        }
        Ok(())
    }

    /// The database as the server gets it: without where and how this
    /// device syncs (its token stays here).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn snapshot_for_sync(&self) -> Result<Vec<u8>> {
        let db = self.snapshot()?;
        let tmp = std::env::temp_dir().join(format!("nb-sync-{}.db", nb_core::OperationId::new()));
        std::fs::write(&tmp, &db).map_err(|e| crate::Error::Backup(e.to_string()))?;
        let out = (|| {
            let c = rusqlite::Connection::open(&tmp)?;
            c.execute("DELETE FROM sync_link", [])?;
            drop(c);
            std::fs::read(&tmp).map_err(|e| crate::Error::Backup(e.to_string()))
        })();
        let _ = std::fs::remove_file(&tmp);
        out
    }
}

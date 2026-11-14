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

/// What the server's log holds after a device's position: operations of
/// the other devices (each with its device), the new position, and the
/// server's fingerprint at that position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResult {
    pub ops: Vec<LoggedOp>,
    pub cursor: i64,
    /// More waits after `cursor`.
    pub more: bool,
    pub fingerprint: String,
    /// The copy of the log `cursor` counts in (see [`Position`]).
    #[serde(default)]
    pub epoch: i64,
}

/// Where a device is in the server's log: `cursor` counts in copy `epoch`
/// of it. A device that sends its whole database again (a secret) starts a
/// new copy; the others then take the server's database and run again what
/// they had not sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub cursor: i64,
    pub epoch: i64,
}

/// Headers of `/v1/snapshot`: the position the database is at.
pub const CURSOR_HEADER: &str = "x-nb-cursor";
pub const EPOCH_HEADER: &str = "x-nb-epoch";

/// A code an existing device makes for a new one to join (an hour).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub code: String,
    pub expires_at: i64,
}

/// What a device joining an organization gets: who it is, its token, the
/// server's position (the database comes from `/v1/snapshot`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Joined {
    pub org_id: String,
    pub org_name: String,
    pub device_id: String,
    pub prefix: String,
    pub token: String,
    pub cursor: i64,
}

/// A device of the organization, as `/v1/devices` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub prefix: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked_at: Option<i64>,
    /// The device asking.
    pub this: bool,
}

/// The server's copy, to compare with this one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateInfo {
    pub org_id: String,
    pub fingerprint: String,
    pub operations: i64,
    /// The server's position (after a reseed: the device's new one).
    #[serde(default)]
    pub cursor: i64,
    #[serde(default)]
    pub epoch: i64,
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

    /// Where this device is in the server's log.
    pub fn pull_position(&self) -> Result<Position> {
        use rusqlite::OptionalExtension;
        Ok(self
            .connection()
            .query_row(
                "SELECT pull_cursor, pull_epoch FROM sync_link WHERE id = 1",
                [],
                |r| {
                    Ok(Position {
                        cursor: r.get(0)?,
                        epoch: r.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    pub fn pull_cursor(&self) -> Result<i64> {
        Ok(self.pull_position()?.cursor)
    }

    pub fn set_pull_position(&mut self, at: Position) -> Result<()> {
        self.connection().execute(
            "UPDATE sync_link SET pull_cursor = ?1, pull_epoch = ?2 WHERE id = 1",
            [at.cursor, at.epoch],
        )?;
        Ok(())
    }

    fn set_pull_cursor(&mut self, cursor: i64) -> Result<()> {
        self.connection().execute(
            "UPDATE sync_link SET pull_cursor = ?1 WHERE id = 1",
            [cursor],
        )?;
        Ok(())
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

    /// Runs the other devices' operations from the server (a pull) and
    /// moves this device's position to `cursor`. They are the server's
    /// already, so they are not sent back.
    pub fn apply_pulled(&mut self, ops: Vec<LoggedOp>, cursor: i64) -> Result<()> {
        let ids: Vec<nb_core::OperationId> = ops.iter().map(|o| o.id).collect();
        self.replay_log(ops)?;
        self.mark_pushed(&ids)?;
        self.set_pull_cursor(cursor)
    }

    /// Becomes a device of an organization that already exists: its
    /// database is the server's copy (`snapshot`), with this device's
    /// identity, linked, at the position the snapshot is at.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn join_with(
        &mut self,
        snapshot: &[u8],
        at: Position,
        joined: &Joined,
        server_url: &str,
    ) -> Result<()> {
        self.restore_snapshot(snapshot)?;
        let id = joined
            .device_id
            .parse()
            .map_err(|_| crate::Error::Corrupt("device id".into()))?;
        self.adopt_device(id, &joined.prefix)?;
        self.set_sync_link(server_url, &joined.token)?;
        self.set_pull_position(at)
    }

    /// Catches up with the server by taking its copy (`snapshot`, at
    /// position `at`) when replaying its log is not enough (another device
    /// sent its whole database, or the result differs): this device's
    /// operations not sent yet are run again on top, and sent later. Not
    /// while one of them holds a secret (it cannot be run again): that
    /// device sends its database first.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resync(&mut self, snapshot: &[u8], at: Position) -> Result<()> {
        let link = self
            .sync_link()?
            .ok_or(crate::Error::NotFound("sync link"))?;
        let me = self.device().clone();
        let till = self.own_register()?;
        let pending = self.unpushed(10_000)?;
        if pending.len() >= 10_000 {
            return Err(crate::Error::Corrupt("too many operations waiting".into()));
        }
        if let Some(op) = pending
            .iter()
            .find(|o| matches!(o.call, crate::ops::Call::Opaque { .. }))
        {
            return Err(crate::Error::Corrupt(format!(
                "operation {} holds a secret and is not sent yet",
                op.kind
            )));
        }
        self.restore_snapshot(snapshot)?;
        self.adopt_device(me.id, &me.prefix)?;
        self.set_own_register(till.as_deref())?;
        self.set_sync_link(&link.server_url, &link.token)?;
        self.set_pull_position(at)?;
        let pending: Vec<LoggedOp> = pending
            .into_iter()
            .map(|mut op| {
                op.device = Some(crate::ops::OpDevice {
                    id: me.id,
                    prefix: me.prefix.clone(),
                });
                op
            })
            .collect();
        // Already on the server (an answer lost): run once, as before.
        let ids: Vec<nb_core::OperationId> = pending.iter().map(|o| o.id).collect();
        self.replay_log(pending)?;
        // What did not exist there waits to be sent again.
        let tx = self.connection();
        for id in ids {
            tx.execute(
                "UPDATE operations SET pushed_at = NULL WHERE id = ?1 AND call IS NOT NULL",
                [id.to_string()],
            )?;
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

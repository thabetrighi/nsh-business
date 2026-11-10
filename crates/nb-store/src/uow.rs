//! The unit of work (M2.4–M2.8). Every business operation runs in
//! [`Store::write`]: one transaction that holds its changes, its outbox
//! events and its audit records, and that is recorded under its
//! [`OperationId`]. Running the same operation again returns the first
//! result and changes nothing (a double tap pays once).

use nb_core::{HlcTimestamp, OperationId, OrganizationId};
use nb_domain::UserId;
use nb_domain::numbering::{self, DocKind};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::store::{Device, Store};
use crate::time::civil_date;
use crate::{Error, Result};

/// Who runs an operation, and which one it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpMeta {
    pub id: OperationId,
    /// Stable name of the use case (`product.create`).
    pub kind: &'static str,
    pub org: Option<OrganizationId>,
    pub actor: Option<UserId>,
}

impl OpMeta {
    pub fn new(kind: &'static str, org: Option<OrganizationId>, actor: Option<UserId>) -> Self {
        Self {
            id: OperationId::new(),
            kind,
            org,
            actor,
        }
    }
}

/// Read access inside [`Store::read`].
pub struct Reader<'a> {
    pub(crate) conn: &'a rusqlite::Connection,
    pub utc_offset_min: i32,
}

impl Reader<'_> {
    pub fn conn(&self) -> &rusqlite::Connection {
        self.conn
    }
}

/// Write access inside [`Store::write`]: the transaction and what an
/// operation records besides its rows.
pub struct Writer<'a> {
    tx: Transaction<'a>,
    meta: OpMeta,
    now: i64,
    device: Device,
    hlc: &'a mut nb_core::Hlc,
    clock: &'a dyn nb_core::Clock,
    utc_offset_min: i32,
    events: u32,
}

/// Sortable text of a hybrid timestamp (outbox order across devices).
pub fn hlc_text(t: HlcTimestamp) -> String {
    format!("{:013}-{:06}-{:016x}", t.wall_ms, t.counter, t.node)
}

impl<'a> Writer<'a> {
    pub fn tx(&self) -> &Transaction<'a> {
        &self.tx
    }
    pub fn meta(&self) -> OpMeta {
        self.meta
    }
    /// The operation's time (the same for all its rows).
    pub fn now(&self) -> i64 {
        self.now
    }
    pub fn device(&self) -> &Device {
        &self.device
    }
    pub fn org(&self) -> Result<OrganizationId> {
        self.meta.org.ok_or(Error::NotFound("organization"))
    }
    pub fn actor_text(&self) -> Option<String> {
        self.meta.actor.map(|a| a.to_string())
    }
    /// The business date of the operation in the store's time zone.
    pub fn date(&self) -> (i32, u32, u32) {
        civil_date(self.now, self.utc_offset_min)
    }

    /// Records an event for sync, in this transaction (M2.5).
    pub fn event(
        &mut self,
        kind: &str,
        entity: &str,
        entity_id: &str,
        payload: &impl Serialize,
    ) -> Result<()> {
        let stamp = hlc_text(self.hlc.tick(self.clock));
        self.tx.execute(
            "INSERT INTO outbox_events (id, operation_id, org_id, kind, entity, entity_id, payload, hlc, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                OperationId::new().to_string(),
                self.meta.id.to_string(),
                self.meta.org.map(|o| o.to_string()),
                kind,
                entity,
                entity_id,
                serde_json::to_string(payload)?,
                stamp,
                self.now
            ],
        )?;
        self.events += 1;
        Ok(())
    }

    /// Appends an audit record chained to the previous one (M2.7).
    pub fn audit(
        &mut self,
        action: &str,
        entity: &str,
        entity_id: &str,
        detail: &impl Serialize,
    ) -> Result<()> {
        let prev: String = self
            .tx
            .query_row(
                "SELECT hash FROM audit_log ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| GENESIS.to_owned());
        let rec = AuditRecord {
            operation_id: self.meta.id.to_string(),
            org_id: self.meta.org.map(|o| o.to_string()),
            actor_id: self.actor_text(),
            action: action.to_owned(),
            entity: entity.to_owned(),
            entity_id: entity_id.to_owned(),
            detail: serde_json::to_string(detail)?,
            created_at: self.now,
        };
        let hash = rec.hash(&prev);
        self.tx.execute(
            "INSERT INTO audit_log (operation_id, org_id, actor_id, action, entity, entity_id, detail, created_at, prev_hash, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                rec.operation_id,
                rec.org_id,
                rec.actor_id,
                rec.action,
                rec.entity,
                rec.entity_id,
                rec.detail,
                rec.created_at,
                prev,
                hash
            ],
        )?;
        Ok(())
    }

    /// The next document number for this device (M2.8).
    pub fn next_number(
        &mut self,
        kind: DocKind,
        branch_id: &str,
        branch_code: &str,
    ) -> Result<String> {
        let (year, _, _) = self.date();
        let last: i64 = self.tx.query_row(
            "INSERT INTO doc_counters (kind, branch_id, year, last) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT (kind, branch_id, year) DO UPDATE SET last = last + 1
             RETURNING last",
            params![kind.code(), branch_id, year],
            |r| r.get(0),
        )?;
        Ok(numbering::format(
            kind,
            branch_code,
            year,
            &self.device.prefix,
            u64::try_from(last).map_err(|_| Error::Corrupt("counter".into()))?,
        ))
    }
}

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

struct AuditRecord {
    operation_id: String,
    org_id: Option<String>,
    actor_id: Option<String>,
    action: String,
    entity: String,
    entity_id: String,
    detail: String,
    created_at: i64,
}

impl AuditRecord {
    fn hash(&self, prev: &str) -> String {
        let mut h = Sha256::new();
        for part in [
            prev,
            &self.operation_id,
            self.org_id.as_deref().unwrap_or(""),
            self.actor_id.as_deref().unwrap_or(""),
            &self.action,
            &self.entity,
            &self.entity_id,
            &self.detail,
        ] {
            // Length-prefixed, so fields cannot be shifted into each other.
            h.update((part.len() as u64).to_be_bytes());
            h.update(part.as_bytes());
        }
        h.update(self.created_at.to_be_bytes());
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl Store {
    /// Runs `f` as the operation `meta`: all or nothing, recorded once.
    pub fn write<R, F>(&mut self, meta: OpMeta, f: F) -> Result<R>
    where
        R: Serialize + DeserializeOwned,
        F: FnOnce(&mut Writer<'_>) -> Result<R>,
    {
        let done: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT kind, result FROM operations WHERE id = ?1",
                [meta.id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((kind, result)) = done {
            if kind != meta.kind {
                return Err(Error::OperationReused(kind));
            }
            return Ok(serde_json::from_str(&result)?);
        }
        let now = self.clock.now_ms();
        let device = self.device.clone();
        let utc_offset_min = self.utc_offset_min;
        let clock = self.clock.clone();
        let tx = self.conn.transaction()?;
        // The operation row first, so events can reference it.
        tx.execute(
            "INSERT INTO operations (id, kind, org_id, actor_id, result, created_at) VALUES (?1, ?2, ?3, ?4, '', ?5)",
            params![
                meta.id.to_string(),
                meta.kind,
                meta.org.map(|o| o.to_string()),
                meta.actor.map(|a| a.to_string()),
                now
            ],
        )?;
        let mut w = Writer {
            tx,
            meta,
            now,
            device,
            hlc: &mut self.hlc,
            clock: clock.as_ref(),
            utc_offset_min,
            events: 0,
        };
        let out = f(&mut w)?; // dropping `w` rolls back
        if w.events == 0 {
            return Err(Error::NoEvent);
        }
        w.tx.execute(
            "UPDATE operations SET result = ?1 WHERE id = ?2",
            params![serde_json::to_string(&out)?, meta.id.to_string()],
        )?;
        w.tx.commit()?;
        Ok(out)
    }

    /// Runs read-only `f` (no transaction needed: WAL snapshot per statement).
    pub fn read<R>(&self, f: impl FnOnce(&Reader<'_>) -> Result<R>) -> Result<R> {
        f(&Reader {
            conn: &self.conn,
            utc_offset_min: self.utc_offset_min,
        })
    }

    /// Checks the audit chain; returns the first broken record's sequence.
    pub fn verify_audit_chain(&self) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, operation_id, org_id, actor_id, action, entity, entity_id, detail, created_at, prev_hash, hash
             FROM audit_log ORDER BY seq",
        )?;
        let mut rows = stmt.query([])?;
        let mut prev = GENESIS.to_owned();
        while let Some(r) = rows.next()? {
            let seq: i64 = r.get(0)?;
            let rec = AuditRecord {
                operation_id: r.get(1)?,
                org_id: r.get(2)?,
                actor_id: r.get(3)?,
                action: r.get(4)?,
                entity: r.get(5)?,
                entity_id: r.get(6)?,
                detail: r.get(7)?,
                created_at: r.get(8)?,
            };
            let stored_prev: String = r.get(9)?;
            let stored: String = r.get(10)?;
            if stored_prev != prev || rec.hash(&prev) != stored {
                return Ok(Some(seq));
            }
            prev = stored;
        }
        Ok(None)
    }

    /// Events not sent yet, oldest first: (seq, kind, entity_id, payload).
    pub fn pending_events(&self, limit: u32) -> Result<Vec<(i64, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, kind, entity_id, payload FROM outbox_events WHERE sent_at IS NULL ORDER BY seq LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Number of events waiting to be sent.
    pub fn pending_count(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM outbox_events WHERE sent_at IS NULL",
            [],
            |r| r.get(0),
        )?)
    }
}

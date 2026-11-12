//! Backups and restore (M14.1, M14.2), as the app runs them: the owner sets
//! a passphrase, a schedule, how many to keep and where; the app backs up
//! when one is due or when asked, checks a backup on demand, and restores
//! one in place. Every attempt is logged, failures too. The file format is
//! in [`crate::backup`].

use std::path::{Path, PathBuf};

use nb_domain::RuleError;
use nb_domain::access::{Permission, Target};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::backup::{self, BackupKey, Header};
use crate::store::Restored;
use crate::uow::OpMeta;
use crate::usecases::sales::the_org;
use crate::{Error, Result, Store};

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSettings {
    /// `off`, `daily` or `weekly`.
    pub schedule: String,
    pub keep: u32,
    pub directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SettingsInput {
    /// Empty: keep the current one (required the first time).
    pub passphrase: String,
    pub schedule: String,
    pub keep: u32,
    pub directory: String,
}

/// A line of the backup log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRow {
    pub kind: String,
    pub path: String,
    pub size: i64,
    pub ok: bool,
    pub message: String,
    pub created_at: i64,
}

/// A backup file found in the directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupFile {
    pub path: String,
    pub size: u64,
    pub header: Option<Header>,
}

fn load(store: &Store) -> Result<Option<(BackupSettings, BackupKey)>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let row: Option<(String, String, u32, String)> = r
            .conn()
            .query_row(
                "SELECT key, schedule, keep, directory FROM backup_settings WHERE org_id = ?1",
                [org.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        row.map(|(key, schedule, keep, directory)| {
            Ok((
                BackupSettings {
                    schedule,
                    keep,
                    directory,
                },
                BackupKey::from_stored(&key)?,
            ))
        })
        .transpose()
    })
}

/// How backups are set up, if they are.
pub fn backup_settings(store: &Store) -> Result<Option<BackupSettings>> {
    Ok(load(store)?.map(|(s, _)| s))
}

/// Sets the passphrase (first time, or to change it), schedule, how many
/// to keep and the directory.
pub fn save_backup_settings(
    store: &mut Store,
    meta: OpMeta,
    input: SettingsInput,
) -> Result<BackupSettings> {
    if !matches!(input.schedule.as_str(), "off" | "daily" | "weekly") {
        return Err(RuleError::on("schedule", "invalid_value").into());
    }
    if !(1..=365).contains(&input.keep) {
        return Err(RuleError::on("keep", "invalid_value").into());
    }
    let directory = input.directory.trim().to_owned();
    if directory.is_empty() {
        return Err(RuleError::on("directory", "required").into());
    }
    let current = load(store)?;
    // Deriving takes a moment: outside the transaction.
    let key = match (input.passphrase.is_empty(), &current) {
        (true, Some((_, k))) => k.clone(),
        (true, None) => {
            return Err(RuleError::on("passphrase", "backup.passphrase_needed").into());
        }
        (false, _) => BackupKey::new(&input.passphrase)?,
    };
    let out = BackupSettings {
        schedule: input.schedule.clone(),
        keep: input.keep,
        directory: directory.clone(),
    };
    store.write(meta, |w| {
        w.require(Permission::BackupManage, &Target::org())?;
        w.tx().execute(
            "INSERT INTO backup_settings (org_id, key, schedule, keep, directory, updated_at, updated_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (org_id) DO UPDATE SET key = ?2, schedule = ?3, keep = ?4, directory = ?5,
                    updated_at = ?6, updated_by = ?7",
            params![
                w.org()?.to_string(),
                key.to_stored(),
                input.schedule,
                input.keep,
                directory,
                w.now(),
                w.actor_text()
            ],
        )?;
        let shown = serde_json::json!({
            "schedule": input.schedule, "keep": input.keep, "directory": directory,
            "passphrase_changed": !input.passphrase.is_empty()
        });
        w.event("backup.configured", "organization", &w.org()?.to_string(), &shown)?;
        w.audit("configure", "backup", &w.org()?.to_string(), &shown)?;
        Ok(out.clone())
    })
}

fn log(
    store: &Store,
    kind: &str,
    path: &Path,
    size: u64,
    outcome: &Result<()>,
    actor: Option<&str>,
) {
    let message = match outcome {
        Ok(()) => String::new(),
        Err(e) => e
            .rule_codes()
            .first()
            .map(|c| c.1.to_owned())
            .filter(|c| *c != "internal")
            .unwrap_or_else(|| e.to_string()),
    };
    // The log is a convenience: a failure to write it never hides the
    // outcome it records.
    let _ = store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        r.conn().execute(
            "INSERT INTO backup_log (id, org_id, kind, path, size, ok, message, created_at, created_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                nb_core::OperationId::new().to_string(),
                org.to_string(),
                kind,
                path.to_string_lossy(),
                i64::try_from(size).unwrap_or(i64::MAX),
                outcome.is_ok(),
                message,
                store.now_ms(),
                actor
            ],
        )?;
        Ok(())
    });
}

/// The latest log lines, newest first.
pub fn backup_log(store: &Store, limit: u32) -> Result<Vec<LogRow>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT kind, path, size, ok, message, created_at FROM backup_log
             WHERE org_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = s
            .query_map(params![org.to_string(), limit], |r| {
                Ok(LogRow {
                    kind: r.get(0)?,
                    path: r.get(1)?,
                    size: r.get(2)?,
                    ok: r.get(3)?,
                    message: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    })
}

/// When the last backup that worked was made.
pub fn last_backup_at(store: &Store) -> Result<Option<i64>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        Ok(r.conn().query_row(
            "SELECT MAX(created_at) FROM backup_log WHERE org_id = ?1 AND kind = 'backup' AND ok = 1",
            [org.to_string()],
            |r| r.get(0),
        )?)
    })
}

/// Whether a scheduled backup is due now.
pub fn backup_due(store: &Store) -> Result<bool> {
    let Some((s, _)) = load(store)? else {
        return Ok(false);
    };
    let every = match s.schedule.as_str() {
        "daily" => DAY_MS,
        "weekly" => 7 * DAY_MS,
        _ => return Ok(false),
    };
    Ok(last_backup_at(store)?.is_none_or(|t| store.now_ms() - t >= every))
}

/// Backs up now into the set directory; keeps the newest `keep`.
pub fn backup_now(store: &Store, actor: Option<&str>) -> Result<PathBuf> {
    let (settings, key) = load(store)?.ok_or(RuleError::on("backup", "backup.not_set_up"))?;
    let dir = PathBuf::from(&settings.directory);
    let now = store.now_ms();
    let path = dir.join(backup::file_name(now, store.utc_offset_min));
    let mut size = 0;
    let outcome = (|| -> Result<()> {
        std::fs::create_dir_all(&dir).map_err(|e| Error::Backup(e.to_string()))?;
        let db = store.snapshot()?;
        let organization = store.read(|r| {
            Ok(crate::repo::org::the_org(r.conn())?
                .map(|o| o.name)
                .unwrap_or_default())
        })?;
        let device: String = store.read(|r| {
            Ok(r.conn()
                .query_row("SELECT name FROM device LIMIT 1", [], |r| r.get(0))?)
        })?;
        let header = Header {
            format: 1,
            created_at: now,
            app_version: env!("CARGO_PKG_VERSION").into(),
            schema: store.schema_version()?,
            organization,
            device,
            size: 0,
            sha256: String::new(),
            kdf: String::new(),
            m_kib: 0,
            t_cost: 0,
            p_cost: 0,
            salt: String::new(),
            nonce: String::new(),
        };
        let file = backup::seal(&db, header, &key)?;
        size = file.len() as u64;
        backup::write_atomic(&path, &file)?;
        backup::prune(&dir, settings.keep as usize)?;
        Ok(())
    })();
    log(store, "backup", &path, size, &outcome, actor);
    outcome.map(|()| path)
}

/// The backups in the set directory, newest first, with their headers.
pub fn list_backups(store: &Store) -> Result<Vec<BackupFile>> {
    let Some((s, _)) = load(store)? else {
        return Ok(Vec::new());
    };
    Ok(backup::list(Path::new(&s.directory))?
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).ok();
            BackupFile {
                path: p.to_string_lossy().into_owned(),
                size: bytes.as_ref().map_or(0, |b| b.len() as u64),
                header: bytes.and_then(|b| backup::header_of(&b).ok()),
            }
        })
        .collect())
}

/// Opens a backup file: the set key if it is one of ours, else the
/// passphrase it was made with.
fn open_file(store: &Store, path: &Path, passphrase: &str) -> Result<(Header, Vec<u8>)> {
    let file = std::fs::read(path).map_err(|e| Error::Backup(e.to_string()))?;
    let header = backup::header_of(&file)?;
    let stored = load(store)?.map(|(_, k)| k);
    let key = backup::key_for(&header, stored.as_ref(), passphrase)?;
    backup::open(&file, &key)
}

/// Checks that a backup opens and holds a sound database, without
/// restoring it.
pub fn verify_backup(store: &Store, path: &Path, passphrase: &str) -> Result<Header> {
    let mut size = 0;
    let outcome = (|| {
        let (h, db) = open_file(store, path, passphrase)?;
        size = h.size;
        crate::store::check_snapshot(&db)?;
        Ok(h)
    })();
    log(
        store,
        "verify",
        path,
        size,
        &outcome.as_ref().map(|_| ()).map_err(clone_err),
        None,
    );
    outcome
}

fn clone_err(e: &Error) -> Error {
    match e.rule_codes().first() {
        Some((_, code)) if *code != "internal" => RuleError::on("backup", code).into(),
        _ => Error::Backup(e.to_string()),
    }
}

/// Replaces the store's data with a backup's (the right to manage backups
/// is needed). The data before is kept next to the database file.
pub fn restore_backup(
    store: &mut Store,
    meta: OpMeta,
    path: &Path,
    passphrase: &str,
) -> Result<Restored> {
    let actor = meta.actor.map(|a| a.to_string());
    store.write(meta, |w| {
        w.require(Permission::BackupManage, &Target::org())?;
        w.audit(
            "restore",
            "backup",
            &w.org()?.to_string(),
            &serde_json::json!({ "path": path.to_string_lossy() }),
        )?;
        w.event(
            "backup.restoring",
            "organization",
            &w.org()?.to_string(),
            &serde_json::json!({ "path": path.to_string_lossy() }),
        )?;
        Ok(())
    })?;
    let (h, db) = open_file(store, path, passphrase)?;
    let restored = store.restore_snapshot(&db)?;
    log(store, "restore", path, h.size, &Ok(()), actor.as_deref());
    Ok(restored)
}

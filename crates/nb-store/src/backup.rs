//! Backups (M14.1, M14.2): a consistent copy of the database, compressed
//! and sealed, that only the owner's passphrase opens; and its restore into
//! the open store.
//!
//! A backup file (`.nbk`) is:
//!
//! ```text
//! "NBK1" | header length (u32 LE) | header (JSON) | sealed body
//! ```
//!
//! The header is readable without the passphrase (when, what, from which
//! schema) and is bound to the body: the body is the database file,
//! deflated, then sealed with XChaCha20-Poly1305 under a key derived from
//! the passphrase by Argon2id (its salt and costs in the header), with the
//! magic and header as associated data. Any change to either, or the wrong
//! passphrase, fails to open; the database's SHA-256 is checked after.
//!
//! The whole file is handled in memory: fine for the databases of a shop
//! (tens of MB); streaming waits for larger ones.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use nb_domain::RuleError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

const MAGIC: &[u8; 4] = b"NBK1";
/// The extension of backup files.
pub const EXTENSION: &str = "nbk";
/// Backups this app writes start with this; only those are pruned.
pub const PREFIX: &str = "nsh-business-";
/// The shortest passphrase accepted.
pub const MIN_PASSPHRASE: usize = 8;

/// Argon2id costs (OWASP's second profile: 19 MiB, 2 passes).
const M_KIB: u32 = 19_456;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

fn io(e: std::io::Error) -> Error {
    Error::Backup(e.to_string())
}

fn rule(code: &'static str) -> Error {
    RuleError::on("backup", code).into()
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| Error::Backup(format!("random: {e}")))?;
    Ok(b)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Result<[u8; N]> {
    let mut out = [0u8; N];
    if s.len() != 2 * N {
        return Err(rule("backup.not_a_backup"));
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2).unwrap_or("zz"), 16)
            .map_err(|_| rule("backup.not_a_backup"))?;
    }
    Ok(out)
}

/// A key derived from the owner's passphrase, with its salt.
#[derive(Clone, PartialEq, Eq)]
pub struct BackupKey {
    pub salt: [u8; 16],
    key: [u8; 32],
}

impl std::fmt::Debug for BackupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupKey")
            .field("salt", &hex(&self.salt))
            .finish_non_exhaustive()
    }
}

impl BackupKey {
    /// Derives the key of `passphrase` with `salt`.
    pub fn derive(passphrase: &str, salt: [u8; 16]) -> Result<Self> {
        let params = Params::new(M_KIB, T_COST, P_COST, Some(32))
            .map_err(|e| Error::Backup(format!("argon2: {e}")))?;
        let mut key = [0u8; 32];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(passphrase.as_bytes(), &salt, &mut key)
            .map_err(|e| Error::Backup(format!("argon2: {e}")))?;
        Ok(Self { salt, key })
    }

    /// A new key for `passphrase`, with a fresh salt.
    pub fn new(passphrase: &str) -> Result<Self> {
        if passphrase.chars().count() < MIN_PASSPHRASE {
            return Err(RuleError::on("passphrase", "backup.passphrase_short").into());
        }
        Self::derive(passphrase, random()?)
    }

    /// The key as stored (`salt:key`, hex).
    pub fn to_stored(&self) -> String {
        format!("{}:{}", hex(&self.salt), hex(&self.key))
    }

    pub fn from_stored(s: &str) -> Result<Self> {
        let (salt, key) = s.split_once(':').ok_or(rule("backup.not_set_up"))?;
        Ok(Self {
            salt: unhex(salt)?,
            key: unhex(key)?,
        })
    }
}

/// What a backup says about itself, readable without the passphrase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub format: u32,
    /// Milliseconds since the epoch.
    pub created_at: i64,
    pub app_version: String,
    /// The database's schema version (`user_version`).
    pub schema: u32,
    pub organization: String,
    pub device: String,
    /// The database file, before compression.
    pub size: u64,
    pub sha256: String,
    pub kdf: String,
    pub m_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub salt: String,
    pub nonce: String,
}

/// Seals a database file's bytes.
pub fn seal(db: &[u8], mut header: Header, key: &BackupKey) -> Result<Vec<u8>> {
    let nonce: [u8; 24] = random()?;
    header.format = 1;
    header.size = db.len() as u64;
    header.sha256 = hex(&Sha256::digest(db));
    header.kdf = "argon2id".into();
    (header.m_kib, header.t_cost, header.p_cost) = (M_KIB, T_COST, P_COST);
    header.salt = hex(&key.salt);
    header.nonce = hex(&nonce);
    let head = serde_json::to_vec(&header)?;
    let mut deflate =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    deflate.write_all(db).map_err(io)?;
    let packed = deflate.finish().map_err(io)?;
    let mut aad = MAGIC.to_vec();
    aad.extend_from_slice(&head);
    let body = XChaCha20Poly1305::new((&key.key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &packed,
                aad: &aad,
            },
        )
        .map_err(|_| Error::Backup("seal".into()))?;
    let len = u32::try_from(head.len()).map_err(|_| Error::Backup("header".into()))?;
    let mut out = Vec::with_capacity(8 + head.len() + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&head);
    out.extend_from_slice(&body);
    Ok(out)
}

/// The header and where the body starts.
fn split(file: &[u8]) -> Result<(Header, usize)> {
    if file.len() < 8 || &file[..4] != MAGIC {
        return Err(rule("backup.not_a_backup"));
    }
    let len = u32::from_le_bytes([file[4], file[5], file[6], file[7]]) as usize;
    let end = 8usize
        .checked_add(len)
        .filter(|e| *e <= file.len())
        .ok_or(rule("backup.not_a_backup"))?;
    let header: Header =
        serde_json::from_slice(&file[8..end]).map_err(|_| rule("backup.not_a_backup"))?;
    if header.format != 1 || header.kdf != "argon2id" {
        return Err(rule("backup.too_new"));
    }
    Ok((header, end))
}

/// Reads a backup's header (no passphrase needed).
pub fn header_of(file: &[u8]) -> Result<Header> {
    split(file).map(|(h, _)| h)
}

/// The key that opens this backup: `stored` if its salt is the backup's,
/// else derived from `passphrase`.
pub fn key_for(header: &Header, stored: Option<&BackupKey>, passphrase: &str) -> Result<BackupKey> {
    let salt: [u8; 16] = unhex(&header.salt)?;
    if let Some(k) = stored.filter(|k| k.salt == salt)
        && passphrase.is_empty()
    {
        return Ok(k.clone());
    }
    if passphrase.is_empty() {
        return Err(RuleError::on("passphrase", "backup.passphrase_needed").into());
    }
    let params = Params::new(header.m_kib, header.t_cost, header.p_cost, Some(32))
        .map_err(|_| rule("backup.not_a_backup"))?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), &salt, &mut key)
        .map_err(|e| Error::Backup(format!("argon2: {e}")))?;
    Ok(BackupKey { salt, key })
}

/// Opens a backup: the database file's bytes, checked.
pub fn open(file: &[u8], key: &BackupKey) -> Result<(Header, Vec<u8>)> {
    let (header, start) = split(file)?;
    let nonce: [u8; 24] = unhex(&header.nonce)?;
    let aad = &file[..start];
    let mut aad_buf = MAGIC.to_vec();
    aad_buf.extend_from_slice(&aad[8..]);
    let packed = XChaCha20Poly1305::new((&key.key).into())
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &file[start..],
                aad: &aad_buf,
            },
        )
        .map_err(|_| RuleError::on("passphrase", "backup.wrong_passphrase"))?;
    let mut db = Vec::with_capacity(usize::try_from(header.size).unwrap_or(0));
    flate2::read::DeflateDecoder::new(&packed[..])
        .read_to_end(&mut db)
        .map_err(|_| rule("backup.damaged"))?;
    if db.len() as u64 != header.size || hex(&Sha256::digest(&db)) != header.sha256 {
        return Err(rule("backup.damaged"));
    }
    Ok((header, db))
}

/// A backup file's name for a moment (local time, sortable).
pub fn file_name(created_at: i64, utc_offset_min: i32) -> String {
    let t = created_at / 1000 + i64::from(utc_offset_min) * 60;
    let (y, m, d) = crate::time::civil_date(created_at, utc_offset_min);
    let secs = t.rem_euclid(86_400);
    format!(
        "{PREFIX}{y:04}{m:02}{d:02}-{:02}{:02}{:02}.{EXTENSION}",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// Writes `bytes` to `path` through a temporary file, so a crash leaves
/// either the old file or the whole new one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("part");
    {
        let mut f = std::fs::File::create(&tmp).map_err(io)?;
        f.write_all(bytes).map_err(io)?;
        f.sync_all().map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(io)
}

/// This app's backups in `dir`, newest first.
pub fn list(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension().is_some_and(|e| e == EXTENSION)
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(PREFIX))
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(io(e)),
    };
    out.sort();
    out.reverse();
    Ok(out)
}

/// Keeps the newest `keep` backups in `dir`; returns those removed.
pub fn prune(dir: &Path, keep: usize) -> Result<Vec<PathBuf>> {
    let old: Vec<PathBuf> = list(dir)?.into_iter().skip(keep.max(1)).collect();
    for p in &old {
        std::fs::remove_file(p).map_err(io)?;
    }
    Ok(old)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Header {
        Header {
            format: 0,
            created_at: 1_700_000_000_000,
            app_version: "0.1.0".into(),
            schema: 21,
            organization: "Épicerie".into(),
            device: "caisse".into(),
            size: 0,
            sha256: String::new(),
            kdf: String::new(),
            m_kib: 0,
            t_cost: 0,
            p_cost: 0,
            salt: String::new(),
            nonce: String::new(),
        }
    }

    #[test]
    fn a_sealed_backup_opens_only_with_its_passphrase() -> Result<()> {
        let key = BackupKey::new("correct horse")?;
        let db = b"SQLite format 3\0 and some pages".repeat(100);
        let file = seal(&db, header(), &key)?;
        assert!(file.len() < db.len(), "compressed");
        let h = header_of(&file)?;
        assert_eq!((h.schema, h.size), (21, db.len() as u64));
        // The stored key opens it; so does the passphrase.
        let (_, back) = open(&file, &key_for(&h, Some(&key), "")?)?;
        assert_eq!(back, db);
        let (_, back) = open(&file, &key_for(&h, None, "correct horse")?)?;
        assert_eq!(back, db);
        let wrong = key_for(&h, None, "wrong horse")?;
        assert_eq!(
            open(&file, &wrong).err().map(|e| e.rule_codes()),
            Some(vec![(Some("passphrase"), "backup.wrong_passphrase")])
        );
        assert!(key_for(&h, None, "").is_err());
        // Another key's salt: the stored key does not apply.
        let other = BackupKey::new("another phrase")?;
        assert!(key_for(&h, Some(&other), "").is_err());
        Ok(())
    }

    #[test]
    fn any_change_is_caught() -> Result<()> {
        let key = BackupKey::new("correct horse")?;
        let db = vec![7u8; 4096];
        let file = seal(&db, header(), &key)?;
        // The body.
        let mut bad = file.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(open(&bad, &key).is_err());
        // The header (its organization's name).
        let mut bad = file.clone();
        if let Some(i) = bad.windows(4).position(|w| w == b"cais") {
            bad[i] = b'k';
        }
        assert!(open(&bad, &key).is_err());
        assert_eq!(
            header_of(b"PK\x03\x04 a zip").err().map(|e| e.rule_codes()),
            Some(vec![(Some("backup"), "backup.not_a_backup")])
        );
        assert!(BackupKey::new("short").is_err());
        let stored = key.to_stored();
        assert_eq!(BackupKey::from_stored(&stored)?, key);
        Ok(())
    }

    #[test]
    fn names_sort_by_time() {
        // 2023-11-14 22:13:20 UTC is 23:13:20 in Algiers.
        assert_eq!(
            file_name(1_700_000_000_000, 60),
            "nsh-business-20231114-231320.nbk"
        );
    }
}

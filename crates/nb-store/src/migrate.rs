//! Numbered migrations (M2.2). The schema version is SQLite's
//! `user_version`. A published migration is never edited: a change is a new
//! file. Before migrating an existing file database, a copy is written next
//! to it (`<file>.bak-v<from>`), so an upgrade can always be undone.

use std::path::Path;

use rusqlite::Connection;

use crate::{Error, Result};

/// The migrations, in order. Version N is the N-th entry.
pub const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_core", include_str!("../migrations/0001_core.sql")),
    ("0002_org", include_str!("../migrations/0002_org.sql")),
    (
        "0003_catalog",
        include_str!("../migrations/0003_catalog.sql"),
    ),
    ("0004_stock", include_str!("../migrations/0004_stock.sql")),
    ("0005_ledger", include_str!("../migrations/0005_ledger.sql")),
    ("0006_access", include_str!("../migrations/0006_access.sql")),
    (
        "0007_catalog_more",
        include_str!("../migrations/0007_catalog_more.sql"),
    ),
    (
        "0008_inventory",
        include_str!("../migrations/0008_inventory.sql"),
    ),
    (
        "0009_tracking",
        include_str!("../migrations/0009_tracking.sql"),
    ),
    (
        "0010_accounting",
        include_str!("../migrations/0010_accounting.sql"),
    ),
    ("0011_sales", include_str!("../migrations/0011_sales.sql")),
    (
        "0012_free_lines",
        include_str!("../migrations/0012_free_lines.sql"),
    ),
    (
        "0013_returns_holds",
        include_str!("../migrations/0013_returns_holds.sql"),
    ),
    (
        "0014_sales_documents",
        include_str!("../migrations/0014_sales_documents.sql"),
    ),
    (
        "0015_customers",
        include_str!("../migrations/0015_customers.sql"),
    ),
    (
        "0016_receipts",
        include_str!("../migrations/0016_receipts.sql"),
    ),
    (
        "0017_installments",
        include_str!("../migrations/0017_installments.sql"),
    ),
    (
        "0018_adjustments_expenses",
        include_str!("../migrations/0018_adjustments_expenses.sql"),
    ),
    (
        "0019_purchasing",
        include_str!("../migrations/0019_purchasing.sql"),
    ),
    (
        "0020_purchase_orders",
        include_str!("../migrations/0020_purchase_orders.sql"),
    ),
    (
        "0021_exchanges",
        include_str!("../migrations/0021_exchanges.sql"),
    ),
    (
        "0022_backups",
        include_str!("../migrations/0022_backups.sql"),
    ),
    (
        "0023_report_indexes",
        include_str!("../migrations/0023_report_indexes.sql"),
    ),
    (
        "0024_product_indexes",
        include_str!("../migrations/0024_product_indexes.sql"),
    ),
    (
        "0025_scale_prefix_20",
        include_str!("../migrations/0025_scale_prefix_20.sql"),
    ),
    (
        "0026_documents_balances_edits",
        include_str!("../migrations/0026_documents_balances_edits.sql"),
    ),
    (
        "0027_fifo_layers",
        include_str!("../migrations/0027_fifo_layers.sql"),
    ),
    (
        "0028_document_templates",
        include_str!("../migrations/0028_document_templates.sql"),
    ),
    (
        "0029_product_favorites",
        include_str!("../migrations/0029_product_favorites.sql"),
    ),
];

pub fn latest() -> u32 {
    u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX)
}

pub fn version(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Brings the schema to `target`, one transaction per migration. A file
/// database that already has data is copied first.
pub fn migrate_to(conn: &mut Connection, target: u32, file: Option<&Path>) -> Result<u32> {
    let from = version(conn)?;
    if from > latest() {
        return Err(Error::Migration {
            version: from,
            message: "the database is newer than this app; update the app".into(),
        });
    }
    if from >= target {
        return Ok(from);
    }
    if let Some(path) = file
        && from > 0
    {
        backup(conn, path, from)?;
    }
    for v in from + 1..=target {
        let (name, sql) = MIGRATIONS[(v - 1) as usize];
        let tx = conn.transaction()?;
        tx.execute_batch(sql).map_err(|e| Error::Migration {
            version: v,
            message: format!("{name}: {e}"),
        })?;
        tx.pragma_update(None, "user_version", v)?;
        tx.commit()?;
    }
    Ok(target)
}

/// Copies the database next to its file (`VACUUM INTO`: consistent, compact).
pub fn backup(conn: &Connection, path: &Path, version: u32) -> Result<std::path::PathBuf> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Backup("database path has no file name".into()))?;
    let dest = path.with_file_name(format!("{name}.bak-v{version}"));
    if dest.exists() {
        std::fs::remove_file(&dest).map_err(|e| Error::Backup(e.to_string()))?;
    }
    let dest_str = dest
        .to_str()
        .ok_or_else(|| Error::Backup("backup path is not UTF-8".into()))?;
    conn.execute("VACUUM INTO ?1", [dest_str])?;
    Ok(dest)
}

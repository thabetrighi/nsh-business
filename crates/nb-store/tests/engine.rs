//! The local engine's guarantees (M2): migrations with backup, atomic
//! operations with outbox and audit, idempotency, append-only tables,
//! tamper-evident audit, offline numbering.

mod common;

use common::*;
use nb_core::OperationId;
use nb_domain::numbering::DocKind;
use nb_store::migrate::{MIGRATIONS, latest, migrate_to, version};
use nb_store::usecases::products::create_product;
use nb_store::{Error, OpMeta, Store};

type R = Result<(), Error>;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("nb-store-{name}-{}", OperationId::new()));
    let _ = std::fs::create_dir_all(&d);
    d
}

#[test]
fn opens_at_latest_version_and_keeps_its_device() -> R {
    let dir = temp_dir("open");
    let path = dir.join("org.db");
    let device = {
        let s = Store::open(&path, options("B"))?;
        assert_eq!(version(s.connection())?, latest());
        s.device().clone()
    };
    let again = Store::open(&path, options("Z"))?;
    assert_eq!(again.device(), &device, "the device is recorded once");
    assert_eq!(again.device().prefix, "B");
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn upgrading_backs_up_and_keeps_data() -> R {
    let dir = temp_dir("upgrade");
    let path = dir.join("org.db");
    {
        let mut conn = rusqlite::Connection::open(&path)?;
        migrate_to(&mut conn, 1, Some(&path))?;
        conn.execute(
            "INSERT INTO device (id, prefix, name, created_at) VALUES (?1, 'C', 'old', 0)",
            [nb_core::DeviceId::new().to_string()],
        )?;
    }
    let s = Store::open(&path, options("A"))?;
    assert_eq!(version(s.connection())?, latest());
    assert_eq!(s.device().prefix, "C", "data from v1 is kept");
    let backup = dir.join("org.db.bak-v1");
    assert!(backup.exists(), "a copy is made before migrating");
    let old = rusqlite::Connection::open(&backup)?;
    assert_eq!(version(&old)?, 1);
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn a_newer_database_is_refused() -> R {
    let mut conn = rusqlite::Connection::open_in_memory()?;
    conn.pragma_update(None, "user_version", latest() + 1)?;
    assert!(matches!(
        migrate_to(&mut conn, latest(), None),
        Err(Error::Migration { .. })
    ));
    assert_eq!(MIGRATIONS.len() as u32, latest());
    Ok(())
}

#[test]
fn a_failed_operation_leaves_no_trace() -> R {
    let (mut s, setup) = store();
    let before = (
        count(&s, "SELECT COUNT(*) FROM products"),
        count(&s, "SELECT COUNT(*) FROM outbox_events"),
        count(&s, "SELECT COUNT(*) FROM audit_log"),
        count(&s, "SELECT COUNT(*) FROM operations"),
    );
    let meta = meta(&setup, "test.fail");
    let r: Result<(), Error> = s.write(meta, |w| {
        w.tx().execute(
            "INSERT INTO categories (id, org_id, name, created_at, updated_at, device_id) VALUES ('c1', ?1, 'x', 0, 0, 'd')",
            [setup.org.to_string()],
        )?;
        w.event("x", "category", "c1", &"x")?;
        w.audit("x", "category", "c1", &"x")?;
        Err(Error::NotFound("on purpose"))
    });
    assert!(r.is_err());
    let after = (
        count(&s, "SELECT COUNT(*) FROM products"),
        count(&s, "SELECT COUNT(*) FROM outbox_events"),
        count(&s, "SELECT COUNT(*) FROM audit_log"),
        count(&s, "SELECT COUNT(*) FROM operations"),
    );
    assert_eq!(before, after);
    assert_eq!(count(&s, "SELECT COUNT(*) FROM categories"), 0);
    Ok(())
}

#[test]
fn an_operation_runs_once() -> R {
    let (mut s, setup) = store();
    let meta = meta(&setup, "product.create");
    let first = create_product(&mut s, meta, product("Lait", "", 12_000))?;
    let again = create_product(&mut s, meta, product("Lait", "", 12_000))?;
    assert_eq!(first, again, "the stored result is returned");
    assert_eq!(count(&s, "SELECT COUNT(*) FROM products"), 1);
    let events = count(
        &s,
        &format!(
            "SELECT COUNT(*) FROM outbox_events WHERE operation_id = '{}'",
            meta.id
        ),
    );
    assert!(events >= 1);
    // The same id for another operation is a bug, reported as such.
    let other = OpMeta {
        kind: "product.update",
        ..meta
    };
    assert!(matches!(
        create_product(&mut s, other, product("Lait", "", 1)),
        Err(Error::OperationReused(_))
    ));
    Ok(())
}

#[test]
fn no_operation_without_event() -> R {
    let (mut s, setup) = store();
    let r: Result<u8, Error> = s.write(meta(&setup, "test.silent"), |_| Ok(1));
    assert!(matches!(r, Err(Error::NoEvent)));
    // And no event without its operation: every event points to one.
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM outbox_events e LEFT JOIN operations o ON o.id = e.operation_id WHERE o.id IS NULL"
        ),
        0
    );
    Ok(())
}

#[test]
fn audit_chain_detects_tampering() -> R {
    let (mut s, setup) = store();
    for i in 0..3 {
        create_product(
            &mut s,
            meta(&setup, "product.create"),
            product(&format!("P{i}"), "", 100),
        )?;
    }
    assert_eq!(s.verify_audit_chain()?, None);
    // The table refuses edits...
    assert!(
        s.connection()
            .execute("UPDATE audit_log SET detail = 'x' WHERE seq = 2", [])
            .is_err()
    );
    assert!(
        s.connection()
            .execute("DELETE FROM audit_log WHERE seq = 2", [])
            .is_err()
    );
    // ...and someone with raw file access is still caught.
    s.connection().execute_batch(
        "DROP TRIGGER audit_log_no_update; UPDATE audit_log SET detail = '{\"price\":1}' WHERE seq = 2;",
    )?;
    assert_eq!(s.verify_audit_chain()?, Some(2));
    Ok(())
}

#[test]
fn numbers_are_unique_across_devices_and_years() -> R {
    let (mut a, setup) = store();
    let mut b = Store::open_in_memory(options("B"))?;
    let mut numbers = std::collections::HashSet::new();
    for store in [&mut a, &mut b] {
        for _ in 0..50 {
            let n: String = store.write(meta(&setup, "test.number"), |w| {
                let n = w.next_number(DocKind::Invoice, "branch", "01")?;
                w.event("n", "doc", &n, &n)?;
                Ok(n)
            })?;
            assert!(numbers.insert(n));
        }
    }
    assert!(numbers.contains("FV-01-26-A00001"));
    assert!(numbers.contains("FV-01-26-B00050"));
    Ok(())
}

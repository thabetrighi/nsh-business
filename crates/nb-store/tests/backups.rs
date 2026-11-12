//! Backups (M14.1) and restore (M14.2): sealed with the owner's
//! passphrase, made when due and pruned; checked without restoring;
//! restored in place, the data before kept, the projections rebuilt.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use nb_core::FixedClock;
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::backups::*;
use nb_store::usecases::products::create_product;
use nb_store::usecases::setup::setup_organization;
use nb_store::{Error, OpMeta, Store, StoreOptions};

type R = Result<(), Error>;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nb-{name}-{}",
        nb_core::uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("temp dir: {e}"));
    dir
}

fn codes<T: std::fmt::Debug>(r: Result<T, Error>) -> Vec<&'static str> {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
    }
}

fn products(s: &Store) -> i64 {
    s.connection()
        .query_row("SELECT COUNT(*) FROM products", [], |r| r.get(0))
        .unwrap_or(-1)
}

#[test]
fn backups_are_made_when_due_checked_and_restored() -> R {
    let dir = temp_dir("backup");
    let clock = Arc::new(FixedClock::new(T0));
    let mut s = Store::open(
        &dir.join("store.db"),
        StoreOptions {
            clock: clock.clone(),
            ..options("A")
        },
    )?;
    let setup = setup_organization(&mut s, setup_input())?;
    create_product(&mut s, meta(&setup, "p"), product("Lait", "", 1_190))?;
    let shelf = dir.join("backups");

    assert_eq!(codes(backup_now(&s, None)), ["backup.not_set_up"]);
    assert!(!backup_due(&s)?);
    let input = |pass: &str| SettingsInput {
        passphrase: pass.into(),
        schedule: "daily".into(),
        keep: 2,
        directory: shelf.to_string_lossy().into_owned(),
    };
    assert_eq!(
        codes(save_backup_settings(
            &mut s,
            meta(&setup, "b"),
            input("short")
        )),
        ["backup.passphrase_short"]
    );
    assert_eq!(
        codes(save_backup_settings(&mut s, meta(&setup, "b"), input(""))),
        ["backup.passphrase_needed"]
    );
    save_backup_settings(&mut s, meta(&setup, "b"), input("correct horse battery"))?;
    assert!(backup_due(&s)?);

    // The first backup: one product.
    let first = backup_now(&s, None)?;
    assert!(first.exists());
    assert!(!backup_due(&s)?, "done for today");
    let raw = std::fs::read(&first).unwrap_or_default();
    assert!(
        !raw.windows(4).any(|w| w == b"Lait"),
        "nothing readable in the file"
    );

    // A day later, another product and another backup; then a third: only
    // the newest two are kept.
    clock.set(T0 + 86_400_000);
    assert!(backup_due(&s)?);
    create_product(&mut s, meta(&setup, "p"), product("Huile", "", 3_570))?;
    let second = backup_now(&s, None)?;
    clock.set(T0 + 86_400_000 + 1_000);
    create_product(&mut s, meta(&setup, "p"), product("Sel", "", 238))?;
    let third = backup_now(&s, None)?;
    let kept = list_backups(&s)?;
    assert_eq!(
        kept.iter()
            .map(|b| PathBuf::from(&b.path))
            .collect::<Vec<_>>(),
        [third.clone(), second.clone()]
    );
    assert!(!first.exists());
    let h = kept[1].header.clone().unwrap_or_else(|| panic!("header"));
    assert_eq!(h.organization, setup_input().org_name);

    // Checked: with the set key, or the passphrase; a wrong one fails.
    verify_backup(&s, &second, "")?;
    verify_backup(&s, &second, "correct horse battery")?;
    assert_eq!(
        codes(verify_backup(&s, &second, "wrong horse battery")),
        ["backup.wrong_passphrase"]
    );
    let junk = dir.join("junk.nbk");
    std::fs::write(&junk, b"not a backup at all").unwrap_or_default();
    assert_eq!(codes(verify_backup(&s, &junk, "")), ["backup.not_a_backup"]);

    // Only who manages backups restores.
    let cashier = create_user(
        &mut s,
        meta(&setup, "user"),
        UserInput {
            name: "Caissier".into(),
            phone: None,
            login: None,
            password: None,
            pin: None,
            grants: vec![GrantRow::org("cashier")],
        },
    )?;
    let as_cashier = OpMeta::new(
        "backup.restore",
        Some(setup.org),
        Some(cashier.id.parse().unwrap_or_default()),
    );
    assert_eq!(
        codes(restore_backup(&mut s, as_cashier, &second, "")),
        ["access.denied"]
    );

    // Restored: back to two products, sound, the data before kept.
    assert_eq!(products(&s), 3);
    let r = restore_backup(&mut s, meta(&setup, "restore"), &second, "")?;
    assert_eq!(products(&s), 2);
    assert!(r.health.is_sound(), "{:?}", r.health);
    assert!(r.previous.as_ref().is_some_and(|p| p.exists()));
    // The store keeps working, and the restore is in the log.
    create_product(&mut s, meta(&setup, "p"), product("Sucre", "", 1_500))?;
    assert_eq!(products(&s), 3);
    let log = backup_log(&s, 10)?;
    assert_eq!(
        log.first().map(|l| (l.kind.as_str(), l.ok)),
        Some(("restore", true))
    );
    // The settings came back with the data: backups still work.
    clock.set(T0 + 3 * 86_400_000);
    backup_now(&s, None)?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn a_backup_opens_on_a_new_device_with_its_passphrase() -> R {
    let dir = temp_dir("restore-new");
    let mut s = Store::open_in_memory(options("A"))?;
    let setup = setup_organization(&mut s, setup_input())?;
    create_product(&mut s, meta(&setup, "p"), product("Lait", "", 1_190))?;
    save_backup_settings(
        &mut s,
        meta(&setup, "b"),
        SettingsInput {
            passphrase: "correct horse battery".into(),
            schedule: "off".into(),
            keep: 5,
            directory: dir.to_string_lossy().into_owned(),
        },
    )?;
    assert!(!backup_due(&s)?, "no schedule");
    let file = backup_now(&s, None)?;

    // A new device, set up on its own: the passphrase opens the backup.
    let mut fresh = Store::open_in_memory(options("B"))?;
    let other = setup_organization(&mut fresh, setup_input())?;
    assert_eq!(
        codes(restore_backup(&mut fresh, meta(&other, "r"), &file, "")),
        ["backup.passphrase_needed"]
    );
    let r = restore_backup(
        &mut fresh,
        meta(&other, "r"),
        &file,
        "correct horse battery",
    )?;
    assert_eq!(products(&fresh), 1);
    assert!(r.health.is_sound());
    assert_eq!(r.previous, None, "nothing kept for a store in memory");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

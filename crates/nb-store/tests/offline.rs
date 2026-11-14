//! Out of touch for too long (ADR-0007): a device linked to a server that
//! has not reached it for 14 days stops changing settings, users and
//! permissions until it does; selling and everything else goes on. A
//! device that was never linked has no such limit.

use std::sync::Arc;

use nb_core::FixedClock;
use nb_store::usecases::customers::{CustomerInput, save_customer};
use nb_store::usecases::org::set_feature;
use nb_store::usecases::setup::{SetupInput, setup_organization};
use nb_store::{Error, OpMeta, Store, StoreOptions};

const DAY: i64 = 86_400_000;

#[test]
fn settings_wait_for_the_server_after_two_weeks_apart() -> Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(FixedClock::new(1_794_304_800_000));
    let mut s = Store::open_in_memory(StoreOptions {
        clock: clock.clone(),
        ..Default::default()
    })?;
    let setup = setup_organization(
        &mut s,
        SetupInput {
            org_name: "Superette".into(),
            activity: "grocery".into(),
            country: "DZ".into(),
            currency: "DZD".into(),
            currency_exp: 2,
            branch_name: "Biskra".into(),
            owner_name: "Ahmed".into(),
            language: "ar".into(),
            fiscal_year_start: 1,
        },
    )?;
    let meta = |k: &'static str| OpMeta::new(k, Some(setup.org), Some(setup.owner));
    let customer = |name: &str| CustomerInput {
        kind: "person".into(),
        name: name.into(),
        status: "active".into(),
        ..Default::default()
    };

    // Never linked: a month apart changes nothing.
    clock.advance(30 * DAY);
    set_feature(&mut s, meta("f"), "promotions", Some(true))?;

    // Linked, and in touch: fine.
    s.set_sync_link("https://sync.example.dz", "token")?;
    clock.advance(13 * DAY);
    set_feature(&mut s, meta("f"), "promotions", Some(false))?;

    // Two weeks and more without the server: settings wait, sales do not.
    clock.advance(2 * DAY);
    match set_feature(&mut s, meta("f"), "promotions", Some(true)) {
        Err(Error::Rules(e)) => assert_eq!(e[0].code, "sync.offline_too_long"),
        other => panic!("expected the lock, got {other:?}"),
    }
    save_customer(&mut s, meta("c"), customer("Toujours servi"))?;

    // Back in touch: open again.
    s.note_sync("")?;
    set_feature(&mut s, meta("f"), "promotions", Some(true))?;
    Ok(())
}

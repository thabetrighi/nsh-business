//! A device and the server (M11/M1): linked once with its database, then
//! only its operations travel; the server's copy stays the device's, row
//! for row. Each operation counts once; secrets travel as a new copy.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nb_core::FixedClock;
use nb_server::{Linked, PushRequest, PushResult, StateInfo, router};
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, create_user};
use nb_store::usecases::customers::{CustomerInput, save_customer};
use nb_store::usecases::products::{ProductInput, create_product};
use nb_store::usecases::setup::{SetupInput, SetupResult, setup_organization};
use nb_store::{OpMeta, Store, StoreOptions};
use tower::ServiceExt;

type R = Result<(), Box<dyn std::error::Error>>;

const KEY: &str = "enroll-key-0123456789";

fn device() -> Result<(Store, SetupResult), Box<dyn std::error::Error>> {
    let mut s = Store::open_in_memory(StoreOptions {
        clock: Arc::new(FixedClock::new(1_794_304_800_000)),
        ..Default::default()
    })?;
    let setup = setup_organization(
        &mut s,
        SetupInput {
            org_name: "Magasin En-Nour".into(),
            activity: "grocery".into(),
            country: "DZ".into(),
            currency: "DZD".into(),
            currency_exp: 2,
            branch_name: "El Oued".into(),
            owner_name: "Ahmed".into(),
            language: "fr".into(),
            fiscal_year_start: 1,
        },
    )?;
    Ok((s, setup))
}

fn meta(s: &SetupResult, kind: &'static str) -> OpMeta {
    OpMeta::new(kind, Some(s.org), Some(s.owner))
}

fn customer(name: &str) -> CustomerInput {
    CustomerInput {
        kind: "person".into(),
        name: name.into(),
        status: "active".into(),
        ..Default::default()
    }
}

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Vec<u8>,
) -> Result<(StatusCode, Vec<u8>), Box<dyn std::error::Error>> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if path == "/v1/push" {
        req = req.header("content-type", "application/json");
    }
    let res = app.clone().oneshot(req.body(Body::from(body))?).await?;
    let status = res.status();
    let bytes = res.into_body().collect().await?.to_bytes().to_vec();
    Ok((status, bytes))
}

async fn push(
    app: &axum::Router,
    token: &str,
    s: &mut Store,
) -> Result<PushResult, Box<dyn std::error::Error>> {
    let ops = s.unpushed(500)?;
    let body = serde_json::to_vec(&PushRequest { ops })?;
    let (status, bytes) = call(app, "POST", "/v1/push", token, body).await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let r: PushResult = serde_json::from_slice(&bytes)?;
    s.mark_pushed(&r.accepted)?;
    Ok(r)
}

#[tokio::test]
async fn a_device_and_its_copy_on_the_server() -> R {
    let dir = std::env::temp_dir().join(format!("nb-server-test-{}", nb_core::OperationId::new()));
    let server = nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?;
    let app = router(server);
    let (mut dev, setup) = device()?;
    save_customer(&mut dev, meta(&setup, "c"), customer("Avant le lien"))?;

    // Linking: the enrollment key, the whole database once.
    let (status, _) = call(
        &app,
        "POST",
        "/v1/link",
        "wrong-key-000000",
        dev.snapshot()?,
    )
    .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, bytes) = call(&app, "POST", "/v1/link", KEY, dev.snapshot()?).await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let linked: Linked = serde_json::from_slice(&bytes)?;
    assert_eq!(linked.org_id, setup.org.to_string());
    dev.mark_all_pushed()?;
    let (status, _) = call(&app, "POST", "/v1/link", KEY, dev.snapshot()?).await?;
    assert_eq!(status, StatusCode::CONFLICT);

    // Then only operations: the server's copy is the device's.
    for name in ["Lait", "Pain", "Café"] {
        let mut p = ProductInput {
            name: name.into(),
            kind: "goods".into(),
            base_unit: "pc".into(),
            tax_bps: 1_900,
            price_minor: 10_000,
            ..Default::default()
        };
        p.sku = String::new();
        create_product(&mut dev, meta(&setup, "p"), p)?;
    }
    save_customer(&mut dev, meta(&setup, "c"), customer("Après le lien"))?;
    assert_eq!(dev.unpushed_count()?, 4);
    let r = push(&app, &linked.token, &mut dev).await?;
    assert_eq!(r.accepted.len(), 4);
    assert!(r.failed.is_none());
    assert_eq!(r.fingerprint, dev.fingerprint()?);
    assert_eq!(dev.unpushed_count()?, 0);

    // Sent twice (a lost answer): counted once.
    let again = serde_json::to_vec(&PushRequest {
        ops: dev.operation_log()?.into_iter().rev().take(2).collect(),
    })?;
    let (status, bytes) = call(&app, "POST", "/v1/push", &linked.token, again).await?;
    assert_eq!(status, StatusCode::OK);
    let r: PushResult = serde_json::from_slice(&bytes)?;
    assert_eq!(r.fingerprint, dev.fingerprint()?);

    // A user with a PIN travels like any operation (the PIN as its hash):
    // the server's copy stays the device's.
    create_user(
        &mut dev,
        meta(&setup, "user"),
        UserInput {
            name: "Yacine".into(),
            phone: None,
            login: None,
            password: None,
            pin: Some("4826".into()),
            grants: vec![GrantRow::org("cashier")],
        },
    )?;
    save_customer(&mut dev, meta(&setup, "c"), customer("Encore"))?;
    let r = push(&app, &linked.token, &mut dev).await?;
    assert!(r.failed.is_none(), "{:?}", r.failed);
    assert_eq!(r.fingerprint, dev.fingerprint()?);
    // A device alone cannot replace the shop's database.
    let (status, _) = call(&app, "POST", "/v1/reseed", &linked.token, dev.snapshot()?).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The state and the copy, for a new device or a restore.
    let (_, bytes) = call(&app, "GET", "/v1/state", &linked.token, Vec::new()).await?;
    let state: StateInfo = serde_json::from_slice(&bytes)?;
    assert_eq!(state.fingerprint, dev.fingerprint()?);
    let (status, copy) = call(&app, "GET", "/v1/snapshot", &linked.token, Vec::new()).await?;
    assert_eq!(status, StatusCode::OK);
    let path = dir.join("restored.sqlite");
    std::fs::write(&path, copy)?;
    let restored = Store::open(&path, StoreOptions::default())?;
    assert_eq!(restored.fingerprint()?, dev.fingerprint()?);

    // A copy a day, the oldest going.
    let server = nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?;
    let day = 86_400_000;
    let start = 1_794_304_800_000;
    for k in 0..5 {
        assert_eq!(
            server
                .backup_all(start + k * day, 3)
                .map_err(|e| format!("{e:?}"))?,
            1
        );
    }
    assert_eq!(
        server
            .backup_all(start + 4 * day, 3)
            .map_err(|e| format!("{e:?}"))?,
        0,
        "once a day"
    );
    let kept: Vec<_> = std::fs::read_dir(dir.join("backups").join(&linked.org_id))?
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(kept.len(), 3);
    let newest = dir
        .join("backups")
        .join(&linked.org_id)
        .join("2026-11-14.sqlite");
    assert!(newest.exists());
    let copy = Store::open(&newest, StoreOptions::default())?;
    assert_eq!(copy.fingerprint()?, dev.fingerprint()?);

    // A token that is not a device's.
    let (status, _) = call(&app, "GET", "/v1/state", "nope", Vec::new()).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

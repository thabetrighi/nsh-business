//! Accounts (ADR-0007) and organizations apart (M11/M3): a phone and a
//! password, recovery codes, a shop linked by an account, a device added
//! to it without a code; one account with two shops sees both, another
//! account sees neither, and a device of one shop never reaches the other.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nb_core::FixedClock;
use nb_server::{Joined, Linked, OrgInfo, PullResult, PushRequest, PushResult, Session, router};
use nb_store::usecases::customers::{CustomerInput, save_customer};
use nb_store::usecases::setup::{SetupInput, SetupResult, setup_organization};
use nb_store::{OpMeta, Store, StoreOptions};
use serde_json::json;
use tower::ServiceExt;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const KEY: &str = "enroll-key-0123456789";

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Vec<u8>,
) -> Res<(StatusCode, Vec<u8>)> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if !token.is_empty() {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let res = app.clone().oneshot(req.body(Body::from(body))?).await?;
    let status = res.status();
    Ok((status, res.into_body().collect().await?.to_bytes().to_vec()))
}

async fn json_call<T: serde::de::DeserializeOwned>(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: serde_json::Value,
) -> Res<T> {
    let (status, bytes) = call(app, method, path, token, body.to_string().into_bytes()).await?;
    assert!(
        status.is_success(),
        "{method} {path}: {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn shop(name: &str) -> Res<(Store, SetupResult)> {
    let mut s = Store::open_in_memory(StoreOptions {
        clock: Arc::new(FixedClock::new(1_794_304_800_000)),
        ..Default::default()
    })?;
    let setup = setup_organization(
        &mut s,
        SetupInput {
            org_name: name.into(),
            activity: "grocery".into(),
            country: "DZ".into(),
            currency: "DZD".into(),
            currency_exp: 2,
            branch_name: "Centre".into(),
            owner_name: "Ahmed".into(),
            language: "ar".into(),
            fiscal_year_start: 1,
        },
    )?;
    Ok((s, setup))
}

#[tokio::test]
async fn accounts_own_their_shops_and_nothing_else() -> Res<()> {
    let dir = std::env::temp_dir().join(format!("nb-accounts-{}", nb_core::OperationId::new()));
    let app = router(nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?);

    // A private server: making an account asks for its key.
    let me = json!({"phone": "0550 12 34 56", "password": "kesra-2026", "name": "Ahmed"});
    let (status, _) = call(&app, "POST", "/v1/signup", "", me.to_string().into_bytes()).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let mut with_key = me.clone();
    with_key["key"] = json!(KEY);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/signup",
        "",
        json!({"phone": "0550123456", "password": "short", "name": "A", "key": KEY})
            .to_string()
            .into_bytes(),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a password of 8 at least");
    let signed: Session = json_call(&app, "POST", "/v1/signup", "", with_key.clone()).await?;
    assert_eq!(signed.phone, "+213550123456");
    assert_eq!(signed.recovery_codes.len(), 10);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/signup",
        "",
        with_key.to_string().into_bytes(),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT, "one account per phone");

    // Signing in, written another way.
    let session: Session = json_call(
        &app,
        "POST",
        "/v1/login",
        "",
        json!({"phone": "+213 550 12 34 56", "password": "kesra-2026"}),
    )
    .await?;
    let (status, _) = call(
        &app,
        "POST",
        "/v1/login",
        "",
        json!({"phone": "0550123456", "password": "wrong-one"})
            .to_string()
            .into_bytes(),
    )
    .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Two shops linked by the account: both are its.
    let (mut nour, nour_setup) = shop("Magasin En-Nour")?;
    let (_, bytes) = call(&app, "POST", "/v1/link", &session.token, nour.snapshot()?).await?;
    let linked_nour: Linked = serde_json::from_slice(&bytes)?;
    let (mut baraka, baraka_setup) = shop("Superette Baraka")?;
    let (_, bytes) = call(&app, "POST", "/v1/link", &session.token, baraka.snapshot()?).await?;
    let linked_baraka: Linked = serde_json::from_slice(&bytes)?;
    nour.mark_all_pushed()?;
    baraka.mark_all_pushed()?;
    let mine: Vec<OrgInfo> = json_call(&app, "GET", "/v1/orgs", &session.token, json!({})).await?;
    let names: Vec<&str> = mine.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["Magasin En-Nour", "Superette Baraka"]);
    assert!(mine.iter().all(|o| o.role == "owner" && o.devices == 1));

    // A new device of En-Nour, without a code: the account says so.
    let joined: Joined = json_call(
        &app,
        "POST",
        &format!("/v1/orgs/{}/devices", linked_nour.org_id),
        &session.token,
        json!({"name": "Caisse 2"}),
    )
    .await?;
    assert_eq!(joined.prefix, "B");
    assert_eq!(joined.org_name, "Magasin En-Nour");

    // B's app is removed and installed again: the new install takes B's
    // place (its letter), and B's old token stops at once.
    let listed: Vec<nb_server::DeviceInfo> = json_call(
        &app,
        "GET",
        &format!("/v1/orgs/{}/devices", linked_nour.org_id),
        &session.token,
        json!({}),
    )
    .await?;
    assert_eq!(listed.len(), 2);
    let again: Joined = json_call(
        &app,
        "POST",
        &format!("/v1/orgs/{}/devices", linked_nour.org_id),
        &session.token,
        json!({"name": "Caisse 2 (réinstallée)", "replaces": joined.device_id}),
    )
    .await?;
    assert_eq!(again.prefix, "B");
    assert_eq!(again.device_id, joined.device_id);
    let (status, _) = call(&app, "GET", "/v1/state", &joined.token, Vec::new()).await?;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the old install is cut off"
    );
    let (status, _) = call(&app, "GET", "/v1/state", &again.token, Vec::new()).await?;
    assert_eq!(status, StatusCode::OK);
    let joined = again;

    // Another account sees none of it, and cannot add itself a device.
    let other: Session = json_call(
        &app,
        "POST",
        "/v1/signup",
        "",
        json!({"phone": "0661 00 00 00", "password": "autre-2026", "name": "Karim", "key": KEY}),
    )
    .await?;
    let theirs: Vec<OrgInfo> = json_call(&app, "GET", "/v1/orgs", &other.token, json!({})).await?;
    assert!(theirs.is_empty());
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/orgs/{}/devices", linked_nour.org_id),
        &other.token,
        b"{}".to_vec(),
    )
    .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A device of En-Nour sends and takes En-Nour's only: Baraka's sale
    // never reaches it.
    save_customer(
        &mut baraka,
        OpMeta::new("c", Some(baraka_setup.org), Some(baraka_setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: "Client de Baraka".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    let body = serde_json::to_vec(&PushRequest {
        ops: baraka.unpushed(100)?,
    })?;
    let (_, bytes) = call(&app, "POST", "/v1/push", &linked_baraka.token, body).await?;
    let _: PushResult = serde_json::from_slice(&bytes)?;
    save_customer(
        &mut nour,
        OpMeta::new("c", Some(nour_setup.org), Some(nour_setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: "Client d'En-Nour".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    let body = serde_json::to_vec(&PushRequest {
        ops: nour.unpushed(100)?,
    })?;
    let (_, bytes) = call(&app, "POST", "/v1/push", &linked_nour.token, body).await?;
    let r: PushResult = serde_json::from_slice(&bytes)?;
    assert!(r.failed.is_none());
    let (_, bytes) = call(&app, "GET", "/v1/pull?after=0", &joined.token, Vec::new()).await?;
    let pulled: PullResult = serde_json::from_slice(&bytes)?;
    let text = serde_json::to_string(&pulled.ops)?;
    assert!(text.contains("Client d'En-Nour"));
    assert!(!text.contains("Baraka"), "nothing of the other shop");
    // A Baraka operation sent with En-Nour's token is not taken as Baraka's.
    let mut stray = baraka.operation_log()?.into_iter().last().ok_or("no op")?;
    stray.id = nb_core::OperationId::new();
    let body = serde_json::to_vec(&PushRequest { ops: vec![stray] })?;
    let (_, bytes) = call(&app, "POST", "/v1/push", &linked_nour.token, body).await?;
    let r: PushResult = serde_json::from_slice(&bytes)?;
    assert!(
        r.failed.is_some(),
        "another organization's operation is refused"
    );

    // A recovery code opens the account once, with a new password; the old
    // sessions end.
    let code = signed.recovery_codes[3].clone();
    let recovered: Session = json_call(
        &app,
        "POST",
        "/v1/recover",
        "",
        json!({"phone": "0550123456", "code": code.to_lowercase(), "password": "nouveau-2026"}),
    )
    .await?;
    let (status, _) = call(&app, "GET", "/v1/orgs", &session.token, Vec::new()).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the old session ended");
    let mine: Vec<OrgInfo> =
        json_call(&app, "GET", "/v1/orgs", &recovered.token, json!({})).await?;
    assert_eq!(mine.len(), 2);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/recover",
        "",
        json!({"phone": "0550123456", "code": code, "password": "encore-2026"})
            .to_string()
            .into_bytes(),
    )
    .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a code works once");

    // Guessing a password: the account waits after a few tries.
    let mut last = StatusCode::OK;
    for _ in 0..9 {
        (last, _) = call(
            &app,
            "POST",
            "/v1/login",
            "",
            json!({"phone": "0550123456", "password": "guess-guess"})
                .to_string()
                .into_bytes(),
        )
        .await?;
    }
    assert_eq!(last, StatusCode::CONFLICT, "locked for a while");
    let (status, _) = call(
        &app,
        "POST",
        "/v1/login",
        "",
        json!({"phone": "0550123456", "password": "nouveau-2026"})
            .to_string()
            .into_bytes(),
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "even the right one, until it ends"
    );

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

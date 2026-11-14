//! Employees who sign in from their own phone (ADR-0009): the owner gives
//! a user of the shop an account; the employee signs in, sees the shop as
//! that user and nothing more; the owner takes it back.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nb_core::FixedClock;
use nb_server::{Joined, Linked, OrgInfo, PushRequest, Session, router};
use nb_store::repo::access::GrantRow;
use nb_store::usecases::access::{UserInput, UserUpdate, create_user, update_user};
use nb_store::usecases::setup::{SetupInput, setup_organization};
use nb_store::{OpMeta, Store, StoreOptions};
use serde_json::{Value, json};
use tower::ServiceExt;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const KEY: &str = "enroll-key-0123456789";

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Res<(StatusCode, Value)> {
    let mut req = Request::builder().method(method).uri(path);
    if !token.is_empty() {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let mut req = req.body(body)?;
    let addr: SocketAddr = "198.51.100.30:4000".parse()?;
    req.extensions_mut().insert(ConnectInfo(addr));
    let res = app.clone().oneshot(req).await?;
    let status = res.status();
    let bytes = res.into_body().collect().await?.to_bytes();
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

async fn push(app: &axum::Router, token: &str, store: &mut Store) -> Res<()> {
    let ops = store.unpushed(500)?;
    let (status, _) = call(
        app,
        "POST",
        "/v1/push",
        token,
        Some(serde_json::to_value(PushRequest { ops })?),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    store.mark_all_pushed()?;
    Ok(())
}

async fn sign_up(app: &axum::Router, phone: &str, name: &str) -> Res<Session> {
    let (status, v) = call(
        app,
        "POST",
        "/v1/signup",
        "",
        Some(json!({"phone": phone, "password": "kesra-2026", "name": name, "key": KEY})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{v}");
    Ok(serde_json::from_value(v)?)
}

async fn login(
    app: &axum::Router,
    phone: &str,
    password: &str,
) -> Res<(StatusCode, Option<Session>)> {
    let (status, v) = call(
        app,
        "POST",
        "/v1/login",
        "",
        Some(json!({"phone": phone, "password": password})),
    )
    .await?;
    Ok((status, serde_json::from_value(v).ok()))
}

#[tokio::test]
async fn an_employee_signs_in_as_its_user_and_nothing_more() -> Res<()> {
    let dir = std::env::temp_dir().join(format!("nb-staff-{}", nb_core::OperationId::new()));
    let app = router(nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?);

    // The owner's shop, with a cashier and a stock keeper.
    let owner = sign_up(&app, "0550 12 34 56", "Ahmed").await?;
    let mut shop = Store::open_in_memory(StoreOptions {
        clock: Arc::new(FixedClock::new(1_794_304_800_000)),
        ..Default::default()
    })?;
    let setup = setup_organization(
        &mut shop,
        SetupInput {
            org_name: "Magasin En-Nour".into(),
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
    let meta = |k: &'static str| OpMeta::new(k, Some(setup.org), Some(setup.owner));
    let user = |name: &str, role: &str| UserInput {
        name: name.into(),
        phone: None,
        login: None,
        password: None,
        pin: Some("2580".into()),
        grants: vec![GrantRow::org(role)],
    };
    let karim = create_user(&mut shop, meta("u"), user("Karim", "cashier"))?;
    let samir = create_user(&mut shop, meta("u"), user("Samir", "stock_keeper"))?;
    let mut req = Request::builder()
        .method("POST")
        .uri("/v1/link")
        .header("authorization", format!("Bearer {}", owner.token))
        .body(Body::from(shop.snapshot()?))?;
    let addr: SocketAddr = "198.51.100.30:4000".parse()?;
    req.extensions_mut().insert(ConnectInfo(addr));
    let res = app.clone().oneshot(req).await?;
    let linked: Linked = serde_json::from_slice(&res.into_body().collect().await?.to_bytes())?;
    shop.mark_all_pushed()?;
    let shop_token = linked.token.clone();

    // The owner gives Karim access: an account on his phone, with the
    // password the owner chose. Not the owner's own user, not someone
    // unknown, not too short a password.
    let grant = |user_id: &str, phone: &str, password: &str| json!({"user_id": user_id, "phone": phone, "name": "Karim", "password": password});
    for (body, want) in [
        (
            grant(&setup.owner.to_string(), "0661 00 00 01", "caisse-2026"),
            StatusCode::BAD_REQUEST,
        ),
        (
            grant(
                "00000000-0000-0000-0000-000000000000",
                "0661 00 00 01",
                "caisse-2026",
            ),
            StatusCode::BAD_REQUEST,
        ),
        (
            grant("x' OR 1=1 --", "0661 00 00 01", "caisse-2026"),
            StatusCode::BAD_REQUEST,
        ),
        (
            grant(&karim.id, "0661 00 00 01", "short"),
            StatusCode::BAD_REQUEST,
        ),
        (
            grant(&karim.id, "0550 12 34 56", "caisse-2026"),
            StatusCode::CONFLICT,
        ),
    ] {
        let (status, v) = call(&app, "PUT", "/v1/staff", &shop_token, Some(body)).await?;
        assert_eq!(status, want, "{v}");
    }
    let (status, v) = call(
        &app,
        "PUT",
        "/v1/staff",
        &shop_token,
        Some(grant(&karim.id, "0661 00 00 01", "caisse-2026")),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["existing"], false);
    let (_, list) = call(&app, "GET", "/v1/staff", &shop_token, None).await?;
    assert_eq!(list[0]["user_id"], karim.id.as_str());
    assert_eq!(list[0]["phone"], "+213661000001");
    // Without the shop's token: nothing.
    let (status, _) = call(&app, "GET", "/v1/staff", "", None).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Karim signs in on his phone: the shop is there, as Karim.
    let (status, s) = login(&app, "0661000001", "caisse-2026").await?;
    assert_eq!(status, StatusCode::OK);
    let karim_session = s.ok_or("no session")?;
    let (_, orgs) = call(&app, "GET", "/v1/orgs", &karim_session.token, None).await?;
    let orgs: Vec<OrgInfo> = serde_json::from_value(orgs)?;
    assert_eq!(orgs.len(), 1);
    assert_eq!(orgs[0].role, "staff");
    assert_eq!(orgs[0].user_id.as_deref(), Some(karim.id.as_str()));
    let (status, v) = call(
        &app,
        "POST",
        &format!("/v1/orgs/{}/devices", linked.org_id),
        &karim_session.token,
        Some(json!({"name": "Android · Karim"})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{v}");
    let joined: Joined = serde_json::from_value(v)?;
    assert_eq!(joined.user_id.as_deref(), Some(karim.id.as_str()));
    let (status, _) = call(&app, "GET", "/v1/state", &joined.token, None).await?;
    assert_eq!(status, StatusCode::OK);

    // His phone cannot invite devices, stop the owner's, or give access.
    let (status, _) = call(&app, "POST", "/v1/invite", &joined.token, None).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/devices/{}/revoke", linked.device_id),
        &joined.token,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        "PUT",
        "/v1/staff",
        &joined.token,
        Some(grant(&samir.id, "0661 00 00 09", "caisse-2026")),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&app, "GET", "/v1/state", &shop_token, None).await?;
    assert_eq!(status, StatusCode::OK, "the owner's device still works");

    // His devices only: he cannot take the place of the owner's device…
    let (_, devices) = call(
        &app,
        "GET",
        &format!("/v1/orgs/{}/devices", linked.org_id),
        &karim_session.token,
        None,
    )
    .await?;
    assert_eq!(devices.as_array().map(Vec::len), Some(1));
    assert_eq!(devices[0]["id"], joined.device_id.as_str());
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/orgs/{}/devices", linked.org_id),
        &karim_session.token,
        Some(json!({"name": "x", "replaces": linked.device_id})),
    )
    .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "not the owner's device");
    // …nor repair (replace) the shop's database.
    let mut req = Request::builder()
        .method("POST")
        .uri("/v1/reseed")
        .header("authorization", format!("Bearer {}", joined.token))
        .header("x-account", karim_session.token.clone())
        .body(Body::from(shop.snapshot()?))?;
    req.extensions_mut().insert(ConnectInfo(addr));
    let res = app.clone().oneshot(req).await?;
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "an employee cannot repair"
    );
    // He can take back his own device's place (a reinstall).
    let (status, v) = call(
        &app,
        "POST",
        &format!("/v1/orgs/{}/devices", linked.org_id),
        &karim_session.token,
        Some(json!({"name": "Android · Karim", "replaces": joined.device_id})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{v}");
    let joined: Joined = serde_json::from_value(v)?;
    assert_eq!(joined.user_id.as_deref(), Some(karim.id.as_str()));

    // A forgotten password: the owner sets it again (the account is only
    // this shop's employee).
    let (status, _) = call(
        &app,
        "PUT",
        "/v1/staff",
        &shop_token,
        Some(grant(&karim.id, "0661 00 00 01", "nouvelle-caisse")),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        login(&app, "0661000001", "caisse-2026").await?.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, s) = login(&app, "0661000001", "nouvelle-caisse").await?;
    assert_eq!(status, StatusCode::OK);
    let karim_session = s.ok_or("no session")?;

    // Someone with an account of their own (another shop's owner) becomes
    // Samir's account: theirs keeps its password.
    let other = sign_up(&app, "0770 11 22 33", "Samir").await?;
    let mut req = Request::builder()
        .method("POST")
        .uri("/v1/link")
        .header("authorization", format!("Bearer {}", other.token))
        .body(Body::from({
            let mut s2 = Store::open_in_memory(StoreOptions::default())?;
            setup_organization(
                &mut s2,
                SetupInput {
                    org_name: "Kiosque Samir".into(),
                    activity: "grocery".into(),
                    country: "DZ".into(),
                    currency: "DZD".into(),
                    currency_exp: 2,
                    branch_name: "Centre".into(),
                    owner_name: "Samir".into(),
                    language: "ar".into(),
                    fiscal_year_start: 1,
                },
            )?;
            s2.snapshot()?
        }))?;
    req.extensions_mut().insert(ConnectInfo(addr));
    app.clone().oneshot(req).await?;
    let (status, v) = call(&app, "PUT", "/v1/staff", &shop_token, Some(json!({"user_id": samir.id, "phone": "0770112233", "name": "Samir", "password": "changed-by-owner"}))).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["existing"], true);
    assert_eq!(
        login(&app, "0770112233", "kesra-2026").await?.0,
        StatusCode::OK,
        "his own password"
    );
    let (_, orgs) = call(&app, "GET", "/v1/orgs", &other.token, None).await?;
    assert_eq!(
        orgs.as_array().map(Vec::len),
        Some(2),
        "his shop and En-Nour"
    );

    // Karim stops working in the shop (inactive): no new device.
    update_user(
        &mut shop,
        meta("u2"),
        UserUpdate {
            id: karim.id.clone(),
            name: "Karim".into(),
            phone: None,
            login: None,
            active: false,
            grants: vec![GrantRow::org("cashier")],
        },
    )?;
    push(&app, &shop_token, &mut shop).await?;
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/orgs/{}/devices", linked.org_id),
        &karim_session.token,
        Some(json!({"name": "Tablette"})),
    )
    .await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "an inactive user");

    // The owner takes the access back: the shop leaves his list, and his
    // device stops at once.
    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/v1/staff/{}", karim.id),
        &shop_token,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, orgs) = call(&app, "GET", "/v1/orgs", &karim_session.token, None).await?;
    assert_eq!(orgs.as_array().map(Vec::len), Some(0));
    let (status, _) = call(&app, "GET", "/v1/state", &joined.token, None).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "his device stopped");
    let (status, _) = call(&app, "GET", "/v1/state", &shop_token, None).await?;
    assert_eq!(status, StatusCode::OK, "the owner's goes on");

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

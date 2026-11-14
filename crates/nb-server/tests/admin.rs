//! The platform's administration under attack (ADR-0008, docs/SECURITY.md):
//! who may come in (no default administrator, password and a one-time code,
//! a code once, locks, limits, addresses), a stolen cookie, forged requests,
//! injected searches, files outside the program; then the operator's work
//! (shops stopped and resumed, plans and payments, subscriptions that end,
//! accounts, devices, copies, a shop deleted) and the log that keeps it.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use nb_core::FixedClock;
use nb_server::admin::totp;
use nb_server::{Linked, Options, PushRequest, Session, admin::admin_router, router};
use nb_store::usecases::customers::{CustomerInput, save_customer};
use nb_store::usecases::setup::{SetupInput, setup_organization};
use nb_store::{OpMeta, Store, StoreOptions};
use serde_json::{Value, json};
use tower::ServiceExt;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const KEY: &str = "enroll-key-0123456789";
const PASSWORD: &str = "a long admin password 2026";
const HOST: &str = "admin.example.dz";

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

/// A request from `ip`, as a browser on the page sends it (unless the
/// headers say otherwise).
async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    ip: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Res<Reply> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", HOST);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let mut req = req.body(body)?;
    let addr: SocketAddr = format!("{ip}:40000").parse()?;
    req.extensions_mut().insert(ConnectInfo(addr));
    let res = app.clone().oneshot(req).await?;
    let status = res.status();
    let headers = res.headers().clone();
    let body = res.into_body().collect().await?.to_bytes().to_vec();
    Ok(Reply {
        status,
        headers,
        body,
    })
}

/// A signed-in administrator's request (cookie and the page's header).
async fn as_admin(
    app: &axum::Router,
    cookie: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Res<Reply> {
    send(
        app,
        method,
        path,
        "198.51.100.10",
        &[("cookie", cookie), ("x-nb-admin", "1")],
        body,
    )
    .await
}

fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nb-admin-{tag}-{}", nb_core::OperationId::new()))
}

/// Makes an administrator on the command line; its app's key back.
fn make_admin(dir: &Path, login: &str) -> Res<String> {
    let out = nb_server::admin::cli(
        dir,
        &["add".into(), login.into(), "The".into(), "Boss".into()],
        || Some(PASSWORD.to_owned()),
    )?;
    let key = out
        .lines()
        .find_map(|l| l.strip_prefix("Or type the key: "))
        .ok_or("no key shown")?;
    Ok(key.trim().to_owned())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
}

fn code(secret: &str) -> String {
    totp::code_at(secret, totp::step_of(now_ms())).unwrap_or_default()
}

/// A code the server takes again: the test forgets the last step used (a
/// real administrator waits 30 seconds for the next code).
fn fresh_code(dir: &Path, secret: &str) -> Res<String> {
    let c = rusqlite::Connection::open(dir.join("control.sqlite"))?;
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    c.execute("UPDATE admins SET last_step = 0", [])?;
    Ok(code(secret))
}

async fn sign_in(app: &axum::Router, dir: &Path, secret: &str) -> Res<String> {
    let r = send(
        app,
        "POST",
        "/admin/api/login",
        "198.51.100.10",
        &[("x-nb-admin", "1")],
        Some(json!({"login": "boss", "password": PASSWORD, "code": fresh_code(dir, secret)?})),
    )
    .await?;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let set = r
        .headers
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .ok_or("no cookie")?;
    Ok(set.split(';').next().unwrap_or_default().to_owned())
}

#[tokio::test]
async fn the_door_keeps_strangers_and_guessers_out() -> Res<()> {
    let dir = temp_dir("door");
    let server = nb_server::Server::open_with(&dir, KEY, Options::default())
        .map_err(|e| format!("{e:?}"))?;
    let admin = admin_router(server.clone());
    let me = "198.51.100.10";

    // The page: its own script only, no frames, nothing cached.
    let page = send(&admin, "GET", "/admin/", me, &[], None).await?;
    assert_eq!(page.status, StatusCode::OK);
    let csp = page
        .headers
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"));
    assert!(!csp.contains("unsafe"), "{csp}");
    assert_eq!(
        page.headers.get("x-frame-options").map(|v| v.as_bytes()),
        Some(&b"DENY"[..])
    );
    // Nothing but the program's three files is ever served.
    for path in [
        "/admin/../control.sqlite",
        "/admin/control.sqlite",
        "/admin/%2e%2e/control.sqlite",
        "/admin/api/../../tenants",
    ] {
        let r = send(&admin, "GET", path, me, &[], None).await?;
        assert!(
            r.status == StatusCode::NOT_FOUND || r.status == StatusCode::UNAUTHORIZED,
            "{path}: {}",
            r.status
        );
        assert!(!String::from_utf8_lossy(&r.body).contains("SQLite"));
    }

    // No administrator exists until the server's command line makes one.
    let r = send(&admin, "GET", "/admin/api/me", me, &[], None).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = send(
        &admin,
        "POST",
        "/admin/api/login",
        me,
        &[("x-nb-admin", "1")],
        Some(json!({"login": "admin", "password": "admin", "code": "000000"})),
    )
    .await?;
    assert_eq!(
        r.status,
        StatusCode::UNAUTHORIZED,
        "no default administrator"
    );
    // A short password is refused on the command line too.
    let short = nb_server::admin::cli(&dir, &["add".into(), "boss".into(), "B".into()], || {
        Some("short".into())
    });
    assert!(short.is_err());
    let secret = make_admin(&dir, "boss")?;

    // The password alone, or the code alone, is not enough.
    let try_login = |password: &'static str, code: String, ip: &'static str| {
        let admin = admin.clone();
        async move {
            send(
                &admin,
                "POST",
                "/admin/api/login",
                ip,
                &[("x-nb-admin", "1")],
                Some(json!({"login": "boss", "password": password, "code": code})),
            )
            .await
        }
    };
    let r = try_login(PASSWORD, "123456".into(), me).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "the code is needed");
    let r = try_login("a wrong password here", code(&secret), me).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "the password is needed");

    // Both: in, with a cookie the page's scripts cannot read.
    let used = fresh_code(&dir, &secret)?;
    let r = try_login(PASSWORD, used.clone(), me).await?;
    assert_eq!(r.status, StatusCode::OK);
    let set = r
        .headers
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    for part in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/admin"] {
        assert!(set.contains(part), "{set}");
    }
    let cookie = set.split(';').next().unwrap_or_default().to_owned();
    // The same code again: refused (replay), even with the password.
    let r = try_login(PASSWORD, used, me).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "a code works once");

    let r = as_admin(&admin, &cookie, "GET", "/admin/api/me", None).await?;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["login"], "boss");

    // A forged change: without the page's header, or from another origin.
    let r = send(
        &admin,
        "POST",
        "/admin/api/blocks",
        me,
        &[("cookie", &cookie)],
        Some(json!({"ip": "203.0.113.1"})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "no header");
    let r = send(
        &admin,
        "POST",
        "/admin/api/blocks",
        me,
        &[
            ("cookie", &cookie),
            ("x-nb-admin", "1"),
            ("origin", "https://evil.example"),
        ],
        Some(json!({"ip": "203.0.113.1"})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "another origin");
    let r = send(
        &admin,
        "POST",
        "/admin/api/blocks",
        me,
        &[
            ("cookie", &cookie),
            ("x-nb-admin", "1"),
            ("origin", &format!("https://{HOST}")),
        ],
        Some(json!({"ip": "203.0.113.1", "hours": 1})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "the page itself");

    // The cookie carried to another address: the session ends there and here.
    let r = send(
        &admin,
        "GET",
        "/admin/api/me",
        "203.0.113.50",
        &[("cookie", &cookie)],
        None,
    )
    .await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "a stolen cookie");
    let r = as_admin(&admin, &cookie, "GET", "/admin/api/me", None).await?;
    assert_eq!(
        r.status,
        StatusCode::UNAUTHORIZED,
        "and the session is over"
    );
    // A forged cookie value.
    let r = as_admin(&admin, "nb_admin=' OR 1=1 --", "GET", "/admin/api/me", None).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // Guessing: five wrong tries lock the administrator, even the right
    // password then waits.
    for _ in 0..5 {
        let r = try_login("guess guess guess", code(&secret), "192.0.2.1").await?;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    let r = try_login(PASSWORD, fresh_code(&dir, &secret)?, "192.0.2.2").await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "locked");
    nb_server::admin::cli(&dir, &["unlock".into(), "boss".into()], || None)?;
    let r = try_login(PASSWORD, fresh_code(&dir, &secret)?, "192.0.2.2").await?;
    assert_eq!(r.status, StatusCode::OK, "unlocked on the command line");

    // An address knocking more than ten times a minute waits.
    let mut last = StatusCode::OK;
    for _ in 0..12 {
        last = send(
            &admin,
            "POST",
            "/admin/api/login",
            "192.0.2.77",
            &[("x-nb-admin", "1")],
            Some(json!({"login": "nobody", "password": "x", "code": "1"})),
        )
        .await?
        .status;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);

    // Everything above is in the security log.
    let cookie = sign_in(&admin, &dir, &secret).await?;
    let r = as_admin(&admin, &cookie, "GET", "/admin/api/security", None).await?;
    let text = String::from_utf8_lossy(&r.body).to_string();
    for kind in [
        "admin.login_failed",
        "admin.locked",
        "admin.session_moved",
        "admin.forged_request",
        "rate_limited",
    ] {
        assert!(text.contains(kind), "{kind} not logged");
    }

    // Only the operator's addresses, when the server says so.
    let dir2 = temp_dir("nets");
    let only = nb_server::Server::open_with(
        &dir2,
        KEY,
        Options {
            admin_allow_ips: vec!["10.0.0.0/8".into(), "2001:db8::/32".into()],
            ..Options::default()
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    let gated = admin_router(only);
    let r = send(&gated, "GET", "/admin/", "203.0.113.9", &[], None).await?;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "not even said to exist");
    let r = send(&gated, "GET", "/admin/", "10.20.30.40", &[], None).await?;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        nb_server::Server::open_with(
            &temp_dir("bad"),
            KEY,
            Options {
                admin_allow_ips: vec!["10.0.0.0/99".into()],
                ..Options::default()
            }
        )
        .is_err(),
        "a wrong network is refused at the start"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
    Ok(())
}

/// The page cannot run text that came from a shop: no markup is ever
/// built from strings.
#[test]
fn the_page_never_runs_foreign_text() {
    let script = include_str!("../src/admin/ui/app.js");
    for bad in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
        "new Function",
        "setTimeout('",
        "setTimeout(\"",
    ] {
        assert!(!script.contains(bad), "app.js uses {bad}");
    }
    let page = include_str!("../src/admin/ui/index.html");
    assert!(!page.contains("<script>"), "no inline script");
    assert!(!page.contains("style="), "no inline style");
    assert!(
        !page.contains("http://") && !page.contains("https://"),
        "nothing from elsewhere"
    );
}

fn shop(name: &str) -> Res<(Store, nb_store::usecases::setup::SetupResult)> {
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

async fn public(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Vec<u8>,
) -> Res<Reply> {
    let mut req = Request::builder().method(method).uri(path);
    if !token.is_empty() {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    req = req.header("content-type", "application/json");
    let mut req = req.body(Body::from(body))?;
    let addr: SocketAddr = "203.0.113.20:5000".parse()?;
    req.extensions_mut().insert(ConnectInfo(addr));
    let res = app.clone().oneshot(req).await?;
    let status = res.status();
    let headers = res.headers().clone();
    Ok(Reply {
        status,
        headers,
        body: res.into_body().collect().await?.to_bytes().to_vec(),
    })
}

#[tokio::test]
async fn the_operator_runs_the_shops() -> Res<()> {
    let dir = temp_dir("ops");
    let server = nb_server::Server::open_with(
        &dir,
        KEY,
        Options {
            billing: true,
            ..Options::default()
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    let app = router(server.clone());
    let admin = admin_router(server.clone());
    let secret = make_admin(&dir, "boss")?;
    let cookie = sign_in(&admin, &dir, &secret).await?;

    // An owner signs up and links a shop with a customer whose name is a
    // trap for a careless page.
    let owner: Session = serde_json::from_slice(
        &public(
            &app,
            "POST",
            "/v1/signup",
            "",
            json!({"phone": "0550 12 34 56", "password": "kesra-2026", "name": "Ahmed", "key": KEY})
                .to_string()
                .into_bytes(),
        )
        .await?
        .body,
    )?;
    let (mut nour, setup) = shop("Magasin En-Nour")?;
    let trap = "<img src=x onerror=alert(1)>";
    save_customer(
        &mut nour,
        OpMeta::new("c", Some(setup.org), Some(setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: trap.into(),
            phone: "0661000000".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    let linked: Linked = serde_json::from_slice(
        &public(&app, "POST", "/v1/link", &owner.token, nour.snapshot()?)
            .await?
            .body,
    )?;
    nour.mark_all_pushed()?;
    let org = linked.org_id.clone();

    // Billing on: the shop's first sync starts its trial.
    let r = public(&app, "GET", "/v1/state", &linked.token, Vec::new()).await?;
    assert_eq!(r.status, StatusCode::OK);
    let r = as_admin(&admin, &cookie, "GET", "/admin/api/overview", None).await?;
    assert_eq!(r.json()["orgs"], 1);
    assert_eq!(r.json()["trial"], 1);

    // Found by its owner's phone; an injected search finds nothing more.
    let r = as_admin(&admin, &cookie, "GET", "/admin/api/orgs?q=550123", None).await?;
    assert_eq!(r.json()["total"], 1);
    for q in [
        "%27%20OR%201%3D1%20--",
        "%25",
        "x%27%3B%20DROP%20TABLE%20orgs%3B--",
    ] {
        let r = as_admin(
            &admin,
            &cookie,
            "GET",
            &format!("/admin/api/orgs?q={q}&filter=x%27%20OR%201%3D1"),
            None,
        )
        .await?;
        assert_eq!(r.status, StatusCode::OK, "{q}");
        assert!(r.json()["total"].as_i64().unwrap_or(9) <= 1, "{q}");
    }
    let r = as_admin(&admin, &cookie, "GET", "/admin/api/orgs", None).await?;
    assert_eq!(r.json()["total"], 1, "the table is still there");
    let r = as_admin(
        &admin,
        &cookie,
        "GET",
        "/admin/api/orgs/..%2F..%2Fcontrol/data",
        None,
    )
    .await?;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // Inside the shop, read only: the trap comes back as text.
    let r = as_admin(
        &admin,
        &cookie,
        "GET",
        &format!("/admin/api/orgs/{org}/data?part=customers"),
        None,
    )
    .await?;
    assert_eq!(r.json()["data"]["rows"][0]["name"], trap);
    let r = as_admin(
        &admin,
        &cookie,
        "GET",
        &format!("/admin/api/orgs/{org}/data"),
        None,
    )
    .await?;
    assert_eq!(r.json()["data"]["customers"], 1);
    assert_eq!(r.json()["currency"], "DZD");

    // Stopped: the shop's devices are refused (and told why); resumed: back.
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/orgs/{org}/suspend"),
        Some(json!({"reason": "unpaid"})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = public(&app, "GET", "/v1/state", &linked.token, Vec::new()).await?;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.json()["error"], "org.suspended");
    as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/orgs/{org}/resume"),
        Some(json!({})),
    )
    .await?;
    let r = public(&app, "GET", "/v1/state", &linked.token, Vec::new()).await?;
    assert_eq!(r.status, StatusCode::OK);

    // A plan of one device, paid: active; a second device is refused.
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        "/admin/api/plans",
        Some(json!({
            "code": "solo", "name": "Solo", "price_minor": 150_000, "currency": "DZD",
            "months": 1, "max_devices": 1, "max_branches": 1, "active": true
        })),
    )
    .await?;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        "/admin/api/plans",
        Some(json!({
            "code": "Bad Code!", "name": "x", "price_minor": -1, "currency": "dzd", "months": 0
        })),
    )
    .await?;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = as_admin(&admin, &cookie, "POST", &format!("/admin/api/orgs/{org}/payments"), Some(json!({
        "plan": "solo", "amount_minor": 150_000, "method": "baridimob", "reference": "BM-778812", "months": 1
    }))).await?;
    assert_eq!(r.status, StatusCode::OK);
    let ends = r.json()["ends_at"].as_i64().unwrap_or(0);
    assert!(ends > now_ms() + 29 * 86_400_000);
    let r = public(
        &app,
        "POST",
        &format!("/v1/orgs/{org}/devices"),
        &owner.token,
        json!({"name": "Caisse 2"}).to_string().into_bytes(),
    )
    .await?;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.json()["error"], "plan.devices");

    // The subscription ended, its grace too: syncing stops (402), selling
    // goes on in the shop; a payment opens it again.
    as_admin(
        &admin,
        &cookie,
        "POST",
        "/admin/api/settings",
        Some(json!({"trial_days": 30, "grace_days": 0})),
    )
    .await?;
    as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/orgs/{org}/subscription"),
        Some(json!({
            "plan": "solo", "status": "active", "ends_at": now_ms() - 1000
        })),
    )
    .await?;
    let r = public(&app, "GET", "/v1/state", &linked.token, Vec::new()).await?;
    assert_eq!(r.status, StatusCode::PAYMENT_REQUIRED);
    save_customer(
        &mut nour,
        OpMeta::new("c2", Some(setup.org), Some(setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: "Pendant la coupure".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/orgs/{org}/payments"),
        Some(json!({
            "plan": "solo", "amount_minor": 150_000, "method": "ccp", "months": 1
        })),
    )
    .await?;
    let body = serde_json::to_vec(&PushRequest {
        ops: nour.unpushed(100)?,
    })?;
    let r = public(&app, "POST", "/v1/push", &linked.token, body).await?;
    assert_eq!(r.status, StatusCode::OK, "what waited goes through");

    // Accounts: stopped (a fresh code), its sign-in refused; a reset code
    // from the support opens it once.
    let accounts = as_admin(&admin, &cookie, "GET", "/admin/api/accounts?q=Ahmed", None).await?;
    let account = accounts.json()["rows"][0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/accounts/{account}/disable"),
        Some(json!({"code": "000000"})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "a fresh code is needed");
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/accounts/{account}/disable"),
        Some(json!({"code": fresh_code(&dir, &secret)?})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::OK);
    let login = json!({"phone": "0550123456", "password": "kesra-2026"})
        .to_string()
        .into_bytes();
    let r = public(&app, "POST", "/v1/login", "", login.clone()).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = public(&app, "GET", "/v1/orgs", &owner.token, Vec::new()).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "its sessions ended");
    as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/accounts/{account}/enable"),
        Some(json!({})),
    )
    .await?;
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/accounts/{account}/reset"),
        Some(json!({"code": fresh_code(&dir, &secret)?})),
    )
    .await?;
    let reset = r.json()["code"].as_str().unwrap_or_default().to_owned();
    assert_eq!(reset.len(), 9);
    let recover = |c: &str| {
        json!({"phone": "0550123456", "code": c, "password": "nouveau-2026"})
            .to_string()
            .into_bytes()
    };
    let r = public(&app, "POST", "/v1/recover", "", recover(&reset)).await?;
    assert_eq!(r.status, StatusCode::OK);
    let r = public(&app, "POST", "/v1/recover", "", recover(&reset)).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "once");

    // Copies: one now, then the shop goes back to it.
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/orgs/{org}/backups/run"),
        Some(json!({})),
    )
    .await?;
    let copy = r.json()["name"].as_str().unwrap_or_default().to_owned();
    let r = as_admin(
        &admin,
        &cookie,
        "GET",
        &format!("/admin/api/orgs/{org}/backups"),
        None,
    )
    .await?;
    assert_eq!(r.json()[0]["name"], copy.as_str());
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/orgs/{org}/backups/restore"),
        Some(json!({"name": "../../control", "code": fresh_code(&dir, &secret)?})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "only copies' names");
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/orgs/{org}/backups/restore"),
        Some(json!({"name": copy, "code": fresh_code(&dir, &secret)?})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = as_admin(
        &admin,
        &cookie,
        "GET",
        &format!("/admin/api/orgs/{org}"),
        None,
    )
    .await?;
    assert_eq!(
        r.json()["epoch"],
        1,
        "the devices start again from the copy"
    );

    // An address blocked everywhere, then lifted; not one's own.
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        "/admin/api/blocks",
        Some(json!({"ip": "203.0.113.20", "reason": "flood"})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = public(&app, "GET", "/v1/health", "", Vec::new()).await?;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    as_admin(
        &admin,
        &cookie,
        "POST",
        "/admin/api/blocks/remove",
        Some(json!({"ip": "203.0.113.20"})),
    )
    .await?;
    let r = public(&app, "GET", "/v1/health", "", Vec::new()).await?;
    assert_eq!(r.status, StatusCode::OK);
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        "/admin/api/blocks",
        Some(json!({"ip": "198.51.100.10"})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "not yourself");

    // A device stopped from here.
    let r = as_admin(
        &admin,
        &cookie,
        "GET",
        &format!("/admin/api/orgs/{org}"),
        None,
    )
    .await?;
    let device = r.json()["devices"][0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/devices/{device}/revoke"),
        Some(json!({})),
    )
    .await?;
    let r = public(&app, "GET", "/v1/state", &linked.token, Vec::new()).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // Deleting a shop: its exact name and a fresh code; the file is kept
    // aside, not lost.
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/orgs/{org}/delete"),
        Some(json!({"name": "En-Nour", "code": fresh_code(&dir, &secret)?})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = as_admin(
        &admin,
        &cookie,
        "POST",
        &format!("/admin/api/orgs/{org}/delete"),
        Some(json!({"name": "Magasin En-Nour", "code": fresh_code(&dir, &secret)?})),
    )
    .await?;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert!(!dir.join("tenants").join(format!("{org}.sqlite")).exists());
    assert_eq!(std::fs::read_dir(dir.join("deleted"))?.count(), 1);
    let r = as_admin(&admin, &cookie, "GET", "/admin/api/orgs", None).await?;
    assert_eq!(r.json()["total"], 0);

    // The log holds every change, sealed; it refuses to be changed, and a
    // change made around the server shows.
    let r = as_admin(&admin, &cookie, "GET", "/admin/api/log", None).await?;
    let log = r.json();
    assert!(log["broken_at"].is_null());
    let actions: Vec<&str> = log["rows"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["action"].as_str()).collect())
        .unwrap_or_default();
    for a in [
        "org.suspend",
        "org.resume",
        "plan.save",
        "payment.add",
        "account.disable",
        "account.reset",
        "backup.restore",
        "ip.block",
        "device.revoke",
        "org.delete",
    ] {
        assert!(actions.contains(&a), "{a} not in the log");
    }
    let c = rusqlite::Connection::open(dir.join("control.sqlite"))?;
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    assert!(
        c.execute("UPDATE admin_log SET detail = 'nothing' WHERE seq = 2", [])
            .is_err()
    );
    assert!(c.execute("DELETE FROM admin_log", []).is_err());
    c.execute_batch(
        "DROP TRIGGER admin_log_kept; UPDATE admin_log SET detail = 'hidden' WHERE seq = 3;",
    )?;
    let r = as_admin(&admin, &cookie, "GET", "/admin/api/log", None).await?;
    assert_eq!(r.json()["broken_at"], 3, "the tampered entry is found");

    // An account's failed sign-in is in the security log.
    public(&app, "POST", "/v1/login", "", login).await?;
    let r = as_admin(
        &admin,
        &cookie,
        "GET",
        "/admin/api/security?kind=account.login_failed",
        None,
    )
    .await?;
    assert!(String::from_utf8_lossy(&r.body).contains("+213550123456"));

    // Signing out ends the session.
    as_admin(
        &admin,
        &cookie,
        "POST",
        "/admin/api/logout",
        Some(json!({})),
    )
    .await?;
    let r = as_admin(&admin, &cookie, "GET", "/admin/api/me", None).await?;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Demo data for looking at the page (screenshots):
/// `NB_SEED_DIR=… cargo test -p nb-server --test admin -- --ignored seed`,
/// then `NB_SERVER_DIR=… NB_SERVER_BILLING=1 nb-server`. The
/// administrator `boss` (password in `PASSWORD`), its app's key in
/// `admin-key.txt`.
#[tokio::test]
#[ignore = "makes demo data on request"]
async fn seed_a_demo_platform() -> Res<()> {
    let Ok(dir) = std::env::var("NB_SEED_DIR") else {
        return Ok(());
    };
    let dir = PathBuf::from(dir);
    let server = nb_server::Server::open_with(
        &dir,
        KEY,
        Options {
            billing: true,
            ..Options::default()
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    let app = router(server.clone());
    let admin = admin_router(server.clone());
    let secret = make_admin(&dir, "boss")?;
    std::fs::write(dir.join("admin-key.txt"), &secret)?;
    let cookie = sign_in(&admin, &dir, &secret).await?;
    for (code, name, price, devices) in [
        ("starter", "Starter", 150_000, Some(1)),
        ("pro", "Pro", 350_000, Some(5)),
        ("chain", "Chaîne", 900_000, None::<i64>),
    ] {
        as_admin(
            &admin,
            &cookie,
            "POST",
            "/admin/api/plans",
            Some(json!({
                "code": code, "name": name, "price_minor": price, "currency": "DZD",
                "months": 1, "max_devices": devices, "max_branches": devices, "active": true
            })),
        )
        .await?;
    }
    let shops = [
        ("0550 12 34 56", "Ahmed Benali", "متجر النور", "pro"),
        (
            "0661 22 33 44",
            "Karim Saidi",
            "Superette Baraka",
            "starter",
        ),
        ("0770 98 76 54", "Nadia Khelif", "Librairie El Fikr", ""),
        ("0555 44 11 22", "Yacine Mansouri", "قصابة الوفاء", "chain"),
    ];
    for (i, (phone, owner, name, plan)) in shops.iter().enumerate() {
        let s: Session = serde_json::from_slice(
            &public(
                &app,
                "POST",
                "/v1/signup",
                "",
                json!({"phone": phone, "password": "kesra-2026", "name": owner, "key": KEY})
                    .to_string()
                    .into_bytes(),
            )
            .await?
            .body,
        )?;
        let (mut store, setup) = shop(name)?;
        for (j, c) in [
            "Mohamed Amine",
            "Fatima Zohra",
            "<img src=x onerror=alert(1)>",
            "Salim Haddad",
        ]
        .iter()
        .enumerate()
        {
            save_customer(
                &mut store,
                OpMeta::new("c", Some(setup.org), Some(setup.owner)),
                CustomerInput {
                    kind: "person".into(),
                    name: (*c).into(),
                    phone: format!("06610000{i}{j}"),
                    city: "Biskra".into(),
                    status: "active".into(),
                    ..Default::default()
                },
            )?;
        }
        let linked: Linked = serde_json::from_slice(
            &public(&app, "POST", "/v1/link", &s.token, store.snapshot()?)
                .await?
                .body,
        )?;
        public(&app, "GET", "/v1/state", &linked.token, Vec::new()).await?;
        if !plan.is_empty() {
            as_admin(&admin, &cookie, "POST", &format!("/admin/api/orgs/{}/payments", linked.org_id), Some(json!({
                "plan": plan, "amount_minor": if *plan == "pro" { 350_000 } else if *plan == "chain" { 900_000 } else { 150_000 },
                "method": if i % 2 == 0 { "baridimob" } else { "ccp" }, "reference": format!("BM-77{i}812"), "months": 1
            }))).await?;
        }
        if i == 1 {
            as_admin(
                &admin,
                &cookie,
                "POST",
                &format!("/admin/api/orgs/{}/subscription", linked.org_id),
                Some(json!({
                    "plan": "starter", "status": "active", "ends_at": now_ms() + 3 * 86_400_000
                })),
            )
            .await?;
        }
        if i == 2 {
            as_admin(
                &admin,
                &cookie,
                "POST",
                &format!("/admin/api/orgs/{}/suspend", linked.org_id),
                Some(json!({"reason": "Paiement en retard"})),
            )
            .await?;
        }
        as_admin(
            &admin,
            &cookie,
            "POST",
            &format!("/admin/api/orgs/{}/backups/run", linked.org_id),
            Some(json!({})),
        )
        .await?;
    }
    // Some attacks to look at.
    for _ in 0..6 {
        public(
            &app,
            "POST",
            "/v1/login",
            "",
            json!({"phone": "0550123456", "password": "guess"})
                .to_string()
                .into_bytes(),
        )
        .await?;
    }
    for _ in 0..3 {
        send(
            &admin,
            "POST",
            "/admin/api/login",
            "192.0.2.66",
            &[("x-nb-admin", "1")],
            Some(json!({"login": "admin", "password": "admin", "code": "123456"})),
        )
        .await?;
    }
    as_admin(
        &admin,
        &cookie,
        "POST",
        "/admin/api/blocks",
        Some(
            json!({"ip": "192.0.2.66", "reason": "Tentatives sur l'administration", "hours": 168}),
        ),
    )
    .await?;
    Ok(())
}

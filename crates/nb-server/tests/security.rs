//! The server under attack (security review, 2026-11): what a hostile
//! device, page or guesser tries, and what it gets.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use nb_core::FixedClock;
use nb_server::{Linked, Options, PushRequest, router};
use nb_store::usecases::customers::{CustomerInput, save_customer};
use nb_store::usecases::setup::{SetupInput, SetupResult, setup_organization};
use nb_store::{OpMeta, Store, StoreOptions};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const KEY: &str = "enroll-key-0123456789";

fn shop() -> Res<(Store, SetupResult)> {
    let mut s = Store::open_in_memory(StoreOptions {
        clock: Arc::new(FixedClock::new(1_794_304_800_000)),
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
    Ok((s, setup))
}

async fn send(
    app: &axum::Router,
    req: Request<Body>,
) -> Res<(StatusCode, axum::http::HeaderMap, Vec<u8>)> {
    let res = app.clone().oneshot(req).await?;
    let (status, headers) = (res.status(), res.headers().clone());
    Ok((
        status,
        headers,
        res.into_body().collect().await?.to_bytes().to_vec(),
    ))
}

fn post(path: &str, token: &str, body: Vec<u8>) -> Res<Request<Body>> {
    Ok(Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body))?)
}

fn temp() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("nb-security-{}", nb_core::OperationId::new()))
}

/// A database crafted to run its own code inside the server's copy: a
/// trigger that wipes the sales whenever a customer is added.
fn with_trap(db: &[u8], sql: &str) -> Res<Vec<u8>> {
    let path = temp().with_extension("sqlite");
    std::fs::write(&path, db)?;
    let c = rusqlite::Connection::open(&path)?;
    c.execute_batch(sql)?;
    drop(c);
    let bytes = std::fs::read(&path)?;
    let _ = std::fs::remove_file(&path);
    Ok(bytes)
}

#[tokio::test]
async fn crafted_databases_are_refused_before_they_run() -> Res<()> {
    let dir = temp();
    let app = router(nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?);
    let (dev, _) = shop()?;
    let clean = dev.snapshot()?;
    for trap in [
        "CREATE TRIGGER evil AFTER INSERT ON customers BEGIN DELETE FROM sales; END;",
        "CREATE TABLE extra (x TEXT);",
        "CREATE VIEW peek AS SELECT * FROM user_credentials;",
        "DROP INDEX IF EXISTS idx_sales_number; CREATE INDEX idx_other ON sales(id);",
    ] {
        let (status, _, body) =
            send(&app, post("/v1/link", KEY, with_trap(&clean, trap)?)?).await?;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{trap}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    // Not a database at all.
    let (status, _, _) = send(&app, post("/v1/link", KEY, b"hello".to_vec())?).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The clean one links.
    let (status, _, _) = send(&app, post("/v1/link", KEY, clean)?).await?;
    assert_eq!(status, StatusCode::OK);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[tokio::test]
async fn floods_and_guessing_are_cut_short() -> Res<()> {
    let dir = temp();
    let app = router(nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?);

    // A sign-in far larger than any real one.
    let huge = serde_json::json!({"phone": "0550123456", "password": "x".repeat(200_000)});
    let (status, _, _) = send(&app, post("/v1/login", "", huge.to_string().into_bytes())?).await?;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    // Guessing invitation codes or passwords from one address: slowed down.
    let mut last = StatusCode::OK;
    for k in 0..25 {
        let body = serde_json::json!({"code": format!("GUESS{k:03}")});
        (last, _, _) = send(&app, post("/v1/join", "", body.to_string().into_bytes())?).await?;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);

    // Too many operations in one push.
    let (mut dev, setup) = shop()?;
    let (_, _, body) = send(&app, post("/v1/link", KEY, dev.snapshot()?)?).await?;
    let linked: Linked = serde_json::from_slice(&body)?;
    dev.mark_all_pushed()?;
    save_customer(
        &mut dev,
        OpMeta::new("c", Some(setup.org), Some(setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: "Client".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    let one = dev.unpushed(1)?.remove(0);
    let many = PushRequest {
        ops: vec![one; nb_server::MAX_OPS + 1],
    };
    let (status, _, _) = send(
        &app,
        post("/v1/push", &linked.token, serde_json::to_vec(&many)?)?,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Answers are not cached nor sniffed.
    let (_, headers, _) = send(
        &app,
        Request::builder().uri("/v1/health").body(Body::empty())?,
    )
    .await?;
    assert_eq!(
        headers.get("cache-control").map(|v| v.as_bytes()),
        Some(&b"no-store"[..])
    );
    assert_eq!(
        headers.get("x-content-type-options").map(|v| v.as_bytes()),
        Some(&b"nosniff"[..])
    );
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[tokio::test]
async fn only_the_web_app_may_call_from_a_browser() -> Res<()> {
    let dir = temp();
    let app = router(
        nb_server::Server::open_with(
            &dir,
            KEY,
            Options {
                allow_origins: vec!["https://app.example.dz".into()],
                ..Default::default()
            },
        )
        .map_err(|e| format!("{e:?}"))?,
    );
    let preflight = |origin: &str| -> Res<Request<Body>> {
        Ok(Request::builder()
            .method("OPTIONS")
            .uri("/v1/pull")
            .header("origin", origin)
            .header("access-control-request-method", "GET")
            .header("access-control-request-headers", "authorization")
            .body(Body::empty())?)
    };
    let (_, headers, _) = send(&app, preflight("https://app.example.dz")?).await?;
    assert_eq!(
        headers
            .get("access-control-allow-origin")
            .map(|v| v.as_bytes()),
        Some(&b"https://app.example.dz"[..])
    );
    let (_, headers, _) = send(&app, preflight("https://evil.example.com")?).await?;
    assert!(headers.get("access-control-allow-origin").is_none());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[tokio::test]
async fn devices_hear_of_each_other_over_a_websocket() -> Res<()> {
    let dir = temp();
    let server = nb_server::Server::open(&dir, KEY).map_err(|e| format!("{e:?}"))?;
    let app = router(server);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let serving = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });
    let base = format!("http://{addr}");
    let http = |path: &str| format!("{base}{path}");
    let client = |req: Request<Body>| async move {
        // A plain HTTP call over the socket (the same server).
        let (parts, body) = req.into_parts();
        let bytes = body.collect().await?.to_bytes().to_vec();
        let mut stream = tokio::net::TcpStream::connect(addr).await?;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut head = format!(
            "{} {} HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\ncontent-length: {}\r\n",
            parts.method,
            parts.uri,
            bytes.len()
        );
        for (k, v) in &parts.headers {
            head.push_str(&format!("{k}: {}\r\n", v.to_str()?));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&bytes).await?;
        let mut out = Vec::new();
        stream.read_to_end(&mut out).await?;
        let text = String::from_utf8_lossy(&out).to_string();
        let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
        Res::Ok(body)
    };
    let _ = http;

    let (mut a, setup) = shop()?;
    let body = client(post("/v1/link", KEY, a.snapshot()?)?).await?;
    let linked: Linked = serde_json::from_str(&body)?;
    a.mark_all_pushed()?;

    // A native device: its token in the header.
    let mut req = format!("ws://{addr}/v1/ws").into_client_request()?;
    req.headers_mut()
        .insert("authorization", format!("Bearer {}", linked.token).parse()?);
    let (mut native, _) = tokio_tungstenite::connect_async(req).await?;
    let first = tokio::time::timeout(Duration::from_secs(5), native.next())
        .await?
        .ok_or("closed")??;
    assert_eq!(first, Message::Text(r#"{"ready":true}"#.into()));

    // A browser: no header, its token as the first message. A wrong one
    // is shown the door.
    let (mut bad, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/ws")).await?;
    bad.send(Message::Text(r#"{"token":"nope"}"#.into()))
        .await?;
    let answer = tokio::time::timeout(Duration::from_secs(5), bad.next()).await?;
    assert!(
        matches!(answer, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
        "{answer:?}"
    );

    // A second device joins; when it sends, the first hears it at once.
    let invite: nb_server::Invite =
        serde_json::from_str(&client(post("/v1/invite", &linked.token, Vec::new())?).await?)?;
    let joined: nb_server::Joined = serde_json::from_str(
        &client(post(
            "/v1/join",
            "",
            serde_json::json!({"code": invite.code})
                .to_string()
                .into_bytes(),
        )?)
        .await?,
    )?;
    let (mut browser, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/ws")).await?;
    browser
        .send(Message::Text(
            format!(r#"{{"token":"{}"}}"#, joined.token).into(),
        ))
        .await?;
    let first = tokio::time::timeout(Duration::from_secs(5), browser.next())
        .await?
        .ok_or("closed")??;
    assert_eq!(first, Message::Text(r#"{"ready":true}"#.into()));

    save_customer(
        &mut a,
        OpMeta::new("c", Some(setup.org), Some(setup.owner)),
        CustomerInput {
            kind: "person".into(),
            name: "Client".into(),
            status: "active".into(),
            ..Default::default()
        },
    )?;
    let started = std::time::Instant::now();
    client(post(
        "/v1/push",
        &linked.token,
        serde_json::to_vec(&PushRequest {
            ops: a.unpushed(10)?,
        })?,
    )?)
    .await?;
    let news = tokio::time::timeout(Duration::from_secs(5), browser.next())
        .await?
        .ok_or("closed")??;
    assert_eq!(news, Message::Text(r#"{"changed":true}"#.into()));
    assert!(started.elapsed() < Duration::from_secs(2));
    // The sender hears nothing of its own push.
    let own = tokio::time::timeout(Duration::from_millis(500), native.next()).await;
    assert!(own.is_err(), "no notice of its own work: {own:?}");

    // A stopped device is cut off (at the next check).
    let _ = client(post(
        &format!("/v1/devices/{}/revoke", joined.device_id),
        &linked.token,
        Vec::new(),
    )?)
    .await?;

    serving.abort();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

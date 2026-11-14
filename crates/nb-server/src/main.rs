//! `nb-server`: the sync server (ADR-0006).
//!
//! Settings from the environment:
//! - `NB_SERVER_DIR`  where the data lives (default `./nb-server-data`)
//! - `NB_SERVER_ADDR` address to listen on (default `0.0.0.0:8787`)
//! - `NB_SERVER_KEY`  the enrollment key devices show to link (required,
//!   at least 12 characters)
//! - `NB_SERVER_OPEN_SIGNUP=1` anyone may make an account (a public
//!   server); otherwise making one asks for `NB_SERVER_KEY`
//!
//! - `NB_SERVER_ALLOW_ORIGINS` the web app's origins (`https://…`,
//!   comma-separated) allowed to call the server from a browser
//! - `NB_SERVER_TRUST_PROXY=1` behind a reverse proxy: the caller's address
//!   is the last `X-Forwarded-For` entry (for the limits on guessing)
//! - `NB_SERVER_KEEP_DAYS` daily copies kept per shop (default 14), in
//!   `NB_SERVER_DIR/backups`
//!
//! Put it behind TLS (a reverse proxy) when it faces the internet.

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let dir =
        PathBuf::from(std::env::var("NB_SERVER_DIR").unwrap_or_else(|_| "nb-server-data".into()));
    let addr = std::env::var("NB_SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8787".into());
    let Ok(key) = std::env::var("NB_SERVER_KEY") else {
        eprintln!("NB_SERVER_KEY is required (the enrollment key devices show to link).");
        std::process::exit(2);
    };
    // A public server lets anyone make an account; a private one (a shop's
    // or a partner's) asks for the key.
    let options = nb_server::Options {
        open_signup: std::env::var("NB_SERVER_OPEN_SIGNUP").is_ok_and(|v| v == "1"),
        allow_origins: std::env::var("NB_SERVER_ALLOW_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|o| o.trim().to_owned())
            .filter(|o| !o.is_empty())
            .collect(),
    };
    let server = match nb_server::Server::open_with(&dir, &key, options) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open {}: {e:?}", dir.display());
            std::process::exit(1);
        }
    };
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("nb-server listening on {addr}, data in {}", dir.display());
    // A copy of every shop each day, the last 14 kept (`NB_SERVER_KEEP_DAYS`).
    let keep: u32 = std::env::var("NB_SERVER_KEEP_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14);
    {
        let server = server.clone();
        tokio::spawn(async move {
            loop {
                let s = server.clone();
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
                match tokio::task::spawn_blocking(move || s.backup_all(now, keep)).await {
                    Ok(Ok(n)) if n > 0 => eprintln!("backups: {n} shop(s) copied"),
                    Ok(Err(e)) => eprintln!("backups failed: {e:?}"),
                    _ => {}
                }
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });
    }
    let app = nb_server::router(server);
    // The caller's address, for the limits on guessing (behind a proxy see
    // `NB_SERVER_TRUST_PROXY`).
    let app = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    {
        eprintln!("server stopped: {e}");
        std::process::exit(1);
    }
}

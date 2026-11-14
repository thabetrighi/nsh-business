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
//! - `NB_SERVER_BILLING=1` subscriptions are enforced (a platform selling
//!   the service): trials, plans' device limits, syncing stops after the
//!   end and its grace
//! - `NB_ADMIN_ADDR` the administration's own address (default
//!   `127.0.0.1:8789`: this machine only; reach it through an SSH tunnel);
//!   `off` closes it
//! - `NB_ADMIN_ALLOW_IPS` addresses or networks allowed into the
//!   administration (comma-separated), whatever reaches its address
//!
//! Put it behind TLS (a reverse proxy) when it faces the internet.
//!
//! Administrators are made here, never by default:
//! `nb-server admin add <login> <name>` (the password from
//! `NB_ADMIN_PASSWORD` or typed), then scan the code shown with an
//! authenticator app. Also `admin list | reset-2fa | password | disable |
//! enable | unlock <login>`.

use std::path::PathBuf;

fn env_list(key: &str) -> Vec<String> {
    std::env::var(key)
        .unwrap_or_default()
        .split(',')
        .map(|o| o.trim().to_owned())
        .filter(|o| !o.is_empty())
        .collect()
}

/// A password from `NB_ADMIN_PASSWORD`, or typed (twice).
fn ask_password() -> Option<String> {
    if let Ok(p) = std::env::var("NB_ADMIN_PASSWORD") {
        return Some(p);
    }
    let read = |prompt: &str| -> Option<String> {
        eprint!("{prompt}");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok()?;
        Some(line.trim_end_matches(['\r', '\n']).to_owned())
    };
    let first = read("Password (12 characters at least): ")?;
    let again = read("Again: ")?;
    (first == again).then_some(first)
}

#[tokio::main]
async fn main() {
    let dir =
        PathBuf::from(std::env::var("NB_SERVER_DIR").unwrap_or_else(|_| "nb-server-data".into()));
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("admin") {
        match nb_server::admin::cli(&dir, &args[1..], ask_password) {
            Ok(out) => {
                println!("{out}");
                return;
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
    }
    let addr = std::env::var("NB_SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8787".into());
    let Ok(key) = std::env::var("NB_SERVER_KEY") else {
        eprintln!("NB_SERVER_KEY is required (the enrollment key devices show to link).");
        std::process::exit(2);
    };
    // A public server lets anyone make an account; a private one (a shop's
    // or a partner's) asks for the key.
    let options = nb_server::Options {
        open_signup: std::env::var("NB_SERVER_OPEN_SIGNUP").is_ok_and(|v| v == "1"),
        allow_origins: env_list("NB_SERVER_ALLOW_ORIGINS"),
        billing: std::env::var("NB_SERVER_BILLING").is_ok_and(|v| v == "1"),
        admin_allow_ips: env_list("NB_ADMIN_ALLOW_IPS"),
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
                match tokio::task::spawn_blocking(move || {
                    let made = s.backup_all(now, keep);
                    // The security log keeps 90 days; ended sessions go.
                    if let Err(e) = s.tidy(now, 90) {
                        eprintln!("tidying failed: {e:?}");
                    }
                    made
                })
                .await
                {
                    Ok(Ok(n)) if n > 0 => eprintln!("backups: {n} shop(s) copied"),
                    Ok(Err(e)) => eprintln!("backups failed: {e:?}"),
                    _ => {}
                }
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });
    }
    // The administration, on its own address (this machine by default).
    let admin_addr = std::env::var("NB_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:8789".into());
    if admin_addr != "off" {
        match tokio::net::TcpListener::bind(&admin_addr).await {
            Ok(l) => {
                eprintln!("administration on http://{admin_addr}/admin/");
                let admin = nb_server::admin::admin_router(server.clone())
                    .into_make_service_with_connect_info::<std::net::SocketAddr>();
                tokio::spawn(async move {
                    if let Err(e) = axum::serve(l, admin).await {
                        eprintln!("administration stopped: {e}");
                    }
                });
            }
            Err(e) => eprintln!("administration not started ({admin_addr}): {e}"),
        }
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

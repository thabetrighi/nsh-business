//! `nb-server`: the sync server (ADR-0006).
//!
//! Settings from the environment:
//! - `NB_SERVER_DIR`  where the data lives (default `./nb-server-data`)
//! - `NB_SERVER_ADDR` address to listen on (default `0.0.0.0:8787`)
//! - `NB_SERVER_KEY`  the enrollment key devices show to link (required,
//!   at least 12 characters)
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
    let server = match nb_server::Server::open(&dir, &key) {
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
    let app = nb_server::router(server);
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

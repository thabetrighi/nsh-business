//! The device's side of sync (M11/M1, ADR-0006): linking to a server (the
//! whole database once) and then sending the operations made here, a batch
//! at a time, from the root (no page change stops it). An operation that
//! cannot travel alone (a password or PIN) makes the device send its whole
//! database again. Everything keeps working without a server.

use nb_store::sync::{Linked, PushRequest, PushResult, StateInfo};
use nsh_core::platform::HttpRequest;
use nsh_core::prelude::*;

use crate::state::AppState;

/// What the screens show about sync.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncView {
    pub linked: bool,
    pub server: String,
    pub pending: i64,
    pub last_ok_at: Option<i64>,
    /// The last failure (empty: none).
    pub error: String,
    pub busy: bool,
}

/// Why an exchange failed, as a translation key and its detail.
pub struct Failed {
    pub key: &'static str,
    pub detail: String,
}

fn failed(key: &'static str, detail: impl Into<String>) -> Failed {
    Failed {
        key,
        detail: detail.into(),
    }
}

async fn send(req: HttpRequest) -> Result<Vec<u8>, Failed> {
    let res = fetch(req)
        .await
        .map_err(|e| failed("sync.err.network", e))?;
    match res.status() {
        200..=299 => Ok(res.body),
        401 => Err(failed("sync.err.unauthorized", "")),
        409 => Err(failed("sync.err.already_linked", "")),
        s => Err(failed(
            "sync.err.server",
            format!("{s} {}", String::from_utf8_lossy(&res.body)),
        )),
    }
}

/// Reads the device's sync state into `st.sync`.
pub async fn refresh(st: AppState) {
    let r = st
        .db()
        .call(|s| Ok((s.sync_link()?, s.unpushed_count()?)))
        .await;
    if let Ok((link, pending)) = r {
        st.sync.update(|v| {
            v.linked = link.is_some();
            v.server = link
                .as_ref()
                .map(|l| l.server_url.clone())
                .unwrap_or_default();
            v.last_ok_at = link.as_ref().and_then(|l| l.last_push_at);
            v.error = link.map(|l| l.last_error).unwrap_or_default();
            v.pending = pending;
        });
    }
}

/// Links this device's database to the server at `url` with the server's
/// enrollment `key`.
#[cfg(not(target_arch = "wasm32"))]
pub async fn link(st: AppState, url: String, key: String) -> Result<Linked, Failed> {
    let url = url.trim().trim_end_matches('/').to_owned();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(failed("sync.err.url", ""));
    }
    let db = st.db();
    let snapshot = db
        .call(|s| s.snapshot_for_sync())
        .await
        .map_err(|e| failed("sync.err.local", e.message))?;
    let name = st
        .user
        .get_untracked()
        .map(|u| u.name)
        .unwrap_or_else(|| "device".into());
    let body = send(
        HttpRequest::new("POST", format!("{url}/v1/link"))
            .header("authorization", format!("Bearer {}", key.trim()))
            .header("x-device-name", name)
            .body(snapshot),
    )
    .await?;
    let linked: Linked =
        serde_json::from_slice(&body).map_err(|e| failed("sync.err.server", e.to_string()))?;
    let token = linked.token.clone();
    db.call(move |s| s.set_sync_link(&url, &token))
        .await
        .map_err(|e| failed("sync.err.local", e.message))?;
    refresh(st).await;
    Ok(linked)
}

/// Sends what waits (one batch after another); `Ok(n)`: operations sent.
pub async fn push_now(st: AppState) -> Result<usize, Failed> {
    if st.sync.get_untracked().busy {
        return Ok(0);
    }
    st.sync.update(|v| v.busy = true);
    let r = push_all(st).await;
    let note = match &r {
        Ok(_) => String::new(),
        Err(f) => {
            let mut t = st.tn(f.key);
            if !f.detail.is_empty() {
                t.push_str(" — ");
                t.push_str(&f.detail);
            }
            t
        }
    };
    let _ = st.db().call(move |s| s.note_sync(&note)).await;
    st.sync.update(|v| v.busy = false);
    refresh(st).await;
    r
}

async fn push_all(st: AppState) -> Result<usize, Failed> {
    let db = st.db();
    let mut sent = 0;
    for _ in 0..50 {
        let (link, ops) = db
            .call(|s| Ok((s.sync_link()?, s.unpushed(300)?)))
            .await
            .map_err(|e| failed("sync.err.local", e.message))?;
        let Some(link) = link else {
            return Ok(sent);
        };
        if ops.is_empty() {
            return Ok(sent);
        }
        let body = serde_json::to_vec(&PushRequest { ops })
            .map_err(|e| failed("sync.err.local", e.to_string()))?;
        let answer = send(
            HttpRequest::new("POST", format!("{}/v1/push", link.server_url))
                .header("authorization", format!("Bearer {}", link.token))
                .header("content-type", "application/json")
                .body(body),
        )
        .await?;
        let r: PushResult = serde_json::from_slice(&answer)
            .map_err(|e| failed("sync.err.server", e.to_string()))?;
        let accepted = r.accepted.clone();
        sent += accepted.len();
        db.call(move |s| s.mark_pushed(&accepted))
            .await
            .map_err(|e| failed("sync.err.local", e.message))?;
        match r.failed {
            None => {}
            Some(f) if f.code == "opaque" => reseed(st, &link.server_url, &link.token).await?,
            Some(f) => return Err(failed("sync.err.rejected", f.detail)),
        }
    }
    Ok(sent)
}

/// Sends the whole database again (an operation holding a secret).
async fn reseed(st: AppState, server: &str, token: &str) -> Result<(), Failed> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let db = st.db();
        let snapshot = db
            .call(|s| s.snapshot_for_sync())
            .await
            .map_err(|e| failed("sync.err.local", e.message))?;
        send(
            HttpRequest::new("POST", format!("{server}/v1/reseed"))
                .header("authorization", format!("Bearer {token}"))
                .body(snapshot),
        )
        .await?;
        db.call(|s| s.mark_all_pushed())
            .await
            .map_err(|e| failed("sync.err.local", e.message))?;
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (st, server, token);
        Err(failed("sync.err.local", "web"))
    }
}

/// Whether the server's copy is this device's, row for row.
pub async fn verify(st: AppState) -> Result<bool, Failed> {
    let db = st.db();
    let (link, mine) = db
        .call(|s| Ok((s.sync_link()?, s.fingerprint()?)))
        .await
        .map_err(|e| failed("sync.err.local", e.message))?;
    let Some(link) = link else {
        return Err(failed("sync.err.not_linked", ""));
    };
    let body = send(
        HttpRequest::new("GET", format!("{}/v1/state", link.server_url))
            .header("authorization", format!("Bearer {}", link.token)),
    )
    .await?;
    let state: StateInfo =
        serde_json::from_slice(&body).map_err(|e| failed("sync.err.server", e.to_string()))?;
    Ok(state.fingerprint == mine)
}

/// Stops syncing here (the server keeps its copy).
pub async fn unlink(st: AppState) {
    let _ = st.db().call(|s| s.clear_sync_link()).await;
    refresh(st).await;
}

/// The background exchange, from the root: soon after the start, then
/// every twenty seconds while something waits.
pub fn start(st: AppState) {
    let tick = move || {
        if st.session.get_untracked().is_none() {
            return;
        }
        let task = spawn(async move {
            refresh(st).await;
            let v = st.sync.get_untracked();
            if v.linked && v.pending > 0 && !v.busy {
                let _ = push_now(st).await;
            }
        });
        drop(task); // runs to completion under the root
    };
    let first = set_timeout(std::time::Duration::from_secs(5), tick);
    drop(first); // runs while the root lives
    let every = set_interval(std::time::Duration::from_secs(20), tick);
    drop(every); // runs while the root lives
}

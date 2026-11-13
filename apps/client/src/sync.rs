//! The device's side of sync (M11/M1–M2, ADR-0006): linking to a server
//! (the whole database once) or joining an organization already there
//! (a code from one of its devices), then sending the operations made here
//! and running those of the other devices, a batch at a time, from the root
//! (no page change stops it). An operation that cannot travel alone (a
//! password or PIN) makes the device send its whole database again, once it
//! has everything the others sent; they then take that copy. Everything
//! keeps working without a server.

use nb_store::sync::{
    Invite, Joined, Linked, Position, PullResult, PushRequest, PushResult, StateInfo,
};
use nsh_core::platform::HttpRequest;
use nsh_core::prelude::*;

use crate::state::AppState;

/// What the screens show about sync.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncView {
    pub linked: bool,
    pub server: String,
    pub pending: i64,
    /// What running other devices' operations here left to look at.
    pub reviews: i64,
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
    Ok(send_full(req).await?.body)
}

async fn send_full(req: HttpRequest) -> Result<HttpResponse, Failed> {
    let res = fetch(req)
        .await
        .map_err(|e| failed("sync.err.network", e))?;
    match res.status() {
        200..=299 => Ok(res),
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
        .call(|s| Ok((s.sync_link()?, s.unpushed_count()?, s.open_reviews()?.len())))
        .await;
    if let Ok((link, pending, reviews)) = r {
        st.sync.update(|v| {
            v.linked = link.is_some();
            v.server = link
                .as_ref()
                .map(|l| l.server_url.clone())
                .unwrap_or_default();
            v.last_ok_at = link.as_ref().and_then(|l| l.last_push_at);
            v.error = link.map(|l| l.last_error).unwrap_or_default();
            v.pending = pending;
            v.reviews = i64::try_from(reviews).unwrap_or(i64::MAX);
        });
    }
}

/// Links this device's database to the server at `url` with the server's
/// enrollment `key`.
#[cfg(not(target_arch = "wasm32"))]
pub async fn link(st: AppState, url: String, key: String) -> Result<Linked, Failed> {
    let url = clean_url(&url)?;
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

fn local(e: crate::db::DbError) -> Failed {
    failed("sync.err.local", e.message)
}

fn decode<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Failed> {
    serde_json::from_slice(body).map_err(|e| failed("sync.err.server", e.to_string()))
}

fn clean_url(url: &str) -> Result<String, Failed> {
    let url = url.trim().trim_end_matches('/').to_owned();
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url)
    } else {
        Err(failed("sync.err.url", ""))
    }
}

/// The server's database and the position it is at.
async fn server_copy(server: &str, token: &str) -> Result<(Vec<u8>, Position), Failed> {
    let res = send_full(
        HttpRequest::new("GET", format!("{server}/v1/snapshot"))
            .header("authorization", format!("Bearer {token}")),
    )
    .await?;
    let num = |name: &str| {
        res.header(name)
            .and_then(|v| v.trim().parse::<i64>().ok())
            .ok_or_else(|| failed("sync.err.server", name.to_owned()))
    };
    let at = Position {
        cursor: num(nb_store::sync::CURSOR_HEADER)?,
        epoch: num(nb_store::sync::EPOCH_HEADER)?,
    };
    Ok((res.body, at))
}

/// Joins the organization of the device that made `code`, on the server at
/// `url`: this device's (empty) database becomes the organization's.
#[cfg(not(target_arch = "wasm32"))]
pub async fn join(st: AppState, url: String, code: String) -> Result<Joined, Failed> {
    let url = clean_url(&url)?;
    let body = serde_json::json!({ "code": code.trim(), "name": st.tn("sync.this_device") });
    let answer = send(
        HttpRequest::new("POST", format!("{url}/v1/join"))
            .header("content-type", "application/json")
            .body(body.to_string().into_bytes()),
    )
    .await
    .map_err(|f| match f.key {
        "sync.err.unauthorized" => failed("sync.err.code", ""),
        _ => f,
    })?;
    let joined: Joined = decode(&answer)?;
    let (db, at) = server_copy(&url, &joined.token).await?;
    st.db()
        .join(db, at, joined.clone(), url)
        .await
        .map_err(local)?;
    refresh(st).await;
    Ok(joined)
}

/// A code for a new device of this organization (an hour, once).
pub async fn invite(st: AppState) -> Result<Invite, Failed> {
    let link = st
        .db()
        .call(|s| s.sync_link())
        .await
        .map_err(local)?
        .ok_or_else(|| failed("sync.err.not_linked", ""))?;
    let body = send(
        HttpRequest::new("POST", format!("{}/v1/invite", link.server_url))
            .header("authorization", format!("Bearer {}", link.token)),
    )
    .await?;
    decode(&body)
}

/// Sends what waits, then runs what the other devices did (one batch after
/// another); `Ok(n)`: operations sent.
pub async fn push_now(st: AppState) -> Result<usize, Failed> {
    if st.sync.get_untracked().busy {
        return Ok(0);
    }
    st.sync.update(|v| v.busy = true);
    let r: Result<usize, Failed> = async {
        let n = push_all(st).await?;
        pull_all(st).await?;
        Ok(n)
    }
    .await;
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
            Some(f) if f.code == "opaque" => {
                // Everything the others sent first, or their work is lost.
                pull_all(st).await?;
                reseed(st, &link.server_url, &link.token).await?;
            }
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
        let (snapshot, at) = db
            .call(|s| Ok((s.snapshot_for_sync()?, s.pull_position()?)))
            .await
            .map_err(local)?;
        let answer = send(
            HttpRequest::new(
                "POST",
                format!("{server}/v1/reseed?after={}&epoch={}", at.cursor, at.epoch),
            )
            .header("authorization", format!("Bearer {token}"))
            .body(snapshot),
        )
        .await
        .map_err(|f| match f.key {
            "sync.err.already_linked" => failed("sync.err.behind", ""),
            _ => f,
        })?;
        let state: StateInfo = decode(&answer)?;
        db.call(move |s| {
            s.mark_all_pushed()?;
            s.set_pull_position(Position {
                cursor: state.cursor,
                epoch: state.epoch,
            })
        })
        .await
        .map_err(local)?;
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (st, server, token);
        Err(failed("sync.err.local", "web"))
    }
}

/// Runs the other devices' operations, from this device's position. When
/// the server's log started again (a device sent its database) or the
/// result is not the server's, takes the server's copy and runs again what
/// waits to be sent.
async fn pull_all(st: AppState) -> Result<(), Failed> {
    let db = st.db();
    for _ in 0..50 {
        let (link, at) = db
            .call(|s| Ok((s.sync_link()?, s.pull_position()?)))
            .await
            .map_err(local)?;
        let Some(link) = link else {
            return Ok(());
        };
        let body = send(
            HttpRequest::new(
                "GET",
                format!(
                    "{}/v1/pull?after={}&epoch={}",
                    link.server_url, at.cursor, at.epoch
                ),
            )
            .header("authorization", format!("Bearer {}", link.token)),
        )
        .await?;
        let r: PullResult = decode(&body)?;
        if r.epoch != at.epoch {
            return catch_up(st, &link.server_url, &link.token).await;
        }
        let (more, theirs, cursor) = (r.more, r.fingerprint, r.cursor);
        let got = !r.ops.is_empty();
        let (pending, mine) = db
            .call(move |s| {
                s.apply_pulled(r.ops, cursor)?;
                Ok((s.unpushed_count()?, s.fingerprint()?))
            })
            .await
            .map_err(local)?;
        if got {
            st.changed();
        }
        if !more {
            if pending == 0 && mine != theirs {
                log::warn!("sync: this copy differs from the server's; taking it again");
                return catch_up(st, &link.server_url, &link.token).await;
            }
            return Ok(());
        }
    }
    Ok(())
}

/// Takes the server's copy, keeping what waits to be sent.
async fn catch_up(st: AppState, server: &str, token: &str) -> Result<(), Failed> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (db, at) = server_copy(server, token).await?;
        st.db()
            .call(move |s| s.resync(&db, at))
            .await
            .map_err(local)?;
        st.changed();
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
/// every twenty seconds (what waits goes, what the others did comes).
pub fn start(st: AppState) {
    let tick = move || {
        if st.session.get_untracked().is_none() {
            return;
        }
        let task = spawn(async move {
            refresh(st).await;
            let v = st.sync.get_untracked();
            if v.linked && !v.busy {
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

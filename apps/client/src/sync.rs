//! The device's side of sync (M11/M1–M2, ADR-0006): linking to a server
//! (the whole database once) or joining an organization already there
//! (a code from one of its devices), then sending the operations made here
//! and running those of the other devices, a batch at a time, from the root
//! (no page change stops it). An operation that cannot travel alone (a
//! password or PIN) makes the device send its whole database again, once it
//! has everything the others sent; they then take that copy. Everything
//! keeps working without a server.

use nb_store::sync::{
    DeviceInfo, Invite, Joined, Linked, OrgInfo, Position, PullResult, PushRequest, PushResult,
    Session, StateInfo,
};
use nsh_core::platform::HttpRequest;
use nsh_core::prelude::*;

use crate::state::AppState;

/// After this long without reaching the server, the top bar warns (the
/// store locks settings at 14 days, ADR-0007).
pub const STALE_MS: i64 = 7 * 86_400_000;

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

impl Failed {}

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
        // What the conflict is (`phone.taken`, `account.locked`, `behind`…)
        // stays in the detail for whoever asked.
        409 => Err(failed(
            "sync.err.already_linked",
            serde_json::from_slice::<serde_json::Value>(&res.body)
                .ok()
                .and_then(|v| v["detail"].as_str().map(str::to_owned))
                .unwrap_or_default(),
        )),
        // The platform stopped the shop (its operator), or its
        // subscription ended: the shop goes on selling here, unsynced.
        402 => Err(failed("sync.err.subscription", "")),
        403 => {
            let code = serde_json::from_slice::<serde_json::Value>(&res.body)
                .ok()
                .and_then(|v| v["error"].as_str().map(str::to_owned))
                .unwrap_or_default();
            Err(match code.as_str() {
                "org.suspended" => failed("sync.err.suspended", ""),
                "plan.devices" => failed("sync.err.plan_devices", ""),
                _ => failed("sync.err.blocked", code),
            })
        }
        s => Err(failed(
            "sync.err.server",
            format!("{s} {}", String::from_utf8_lossy(&res.body)),
        )),
    }
}

/// What the other devices see this one as: its kind of system, and who
/// uses it when known ("Android · Karim").
fn device_name(st: AppState) -> String {
    let os = st.tn(match std::env::consts::OS {
        "android" => "device.os.android",
        "ios" => "device.os.ios",
        "windows" => "device.os.windows",
        "macos" => "device.os.macos",
        "linux" => "device.os.linux",
        _ => "device.os.web",
    });
    match st.user.get_untracked() {
        Some(u) if !u.name.trim().is_empty() => format!("{os} · {}", u.name.trim()),
        _ => os,
    }
}

/// Reads the device's sync state into `st.sync`.
pub async fn refresh(st: AppState) {
    let r = st
        .db()
        .call(|s| Ok((s.sync_link()?, s.unpushed_count()?, s.open_reviews()?.len())))
        .await;
    if let Ok((link, pending, reviews)) = r {
        let mut v = st.sync.get_untracked();
        v.linked = link.is_some();
        v.server = link
            .as_ref()
            .map(|l| l.server_url.clone())
            .unwrap_or_default();
        v.last_ok_at = link.as_ref().and_then(|l| l.last_push_at);
        v.error = link.map(|l| l.last_error).unwrap_or_default();
        v.pending = pending;
        v.reviews = i64::try_from(reviews).unwrap_or(i64::MAX);
        // Every few seconds: what shows it is drawn again only on a change.
        if v != st.sync.get_untracked() {
            st.sync.set(v);
        }
    }
}

/// Links this device's database to the server at `url` with the server's
/// enrollment `key`.
pub async fn link(st: AppState, url: String, key: String) -> Result<Linked, Failed> {
    let url = clean_url(&url)?;
    let db = st.db();
    let snapshot = db
        .call(|s| s.snapshot_for_sync())
        .await
        .map_err(|e| failed("sync.err.local", e.message))?;
    // A header holds ASCII only: the name goes percent-encoded.
    let name: String = device_name(st)
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
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

/// Signs in to an account on the server at `url` (ADR-0007): a session for
/// linking this shop or adding this device to one of the account's shops.
/// The session is not kept: the device gets its own token.
pub async fn sign_in(url: &str, phone: &str, password: &str) -> Result<Session, Failed> {
    let url = clean_url(url)?;
    let body = serde_json::json!({ "phone": phone.trim(), "password": password });
    let answer = send(
        HttpRequest::new("POST", format!("{url}/v1/login"))
            .header("content-type", "application/json")
            .body(body.to_string().into_bytes()),
    )
    .await
    .map_err(account_error)?;
    decode(&answer)
}

/// Makes an account; its recovery codes come back once (to be kept).
pub async fn sign_up(
    url: &str,
    name: &str,
    phone: &str,
    password: &str,
    key: &str,
) -> Result<Session, Failed> {
    let url = clean_url(url)?;
    let body = serde_json::json!({
        "name": name.trim(),
        "phone": phone.trim(),
        "password": password,
        "key": key.trim(),
    });
    let answer = send(
        HttpRequest::new("POST", format!("{url}/v1/signup"))
            .header("content-type", "application/json")
            .body(body.to_string().into_bytes()),
    )
    .await
    .map_err(account_error)?;
    decode(&answer)
}

/// A forgotten password: a recovery code (each works once) and a new
/// password; the account's other sessions end.
pub async fn recover(
    url: &str,
    phone: &str,
    code: &str,
    password: &str,
) -> Result<Session, Failed> {
    let url = clean_url(url)?;
    let body =
        serde_json::json!({ "phone": phone.trim(), "code": code.trim(), "password": password });
    let answer = send(
        HttpRequest::new("POST", format!("{url}/v1/recover"))
            .header("content-type", "application/json")
            .body(body.to_string().into_bytes()),
    )
    .await
    .map_err(|f| match f.key {
        "sync.err.unauthorized" => failed("account.err.code", ""),
        _ => account_error(f),
    })?;
    decode(&answer)
}

/// The account's shops.
pub async fn my_orgs(url: &str, session: &str) -> Result<Vec<OrgInfo>, Failed> {
    let url = clean_url(url)?;
    let body = send(
        HttpRequest::new("GET", format!("{url}/v1/orgs"))
            .header("authorization", format!("Bearer {session}")),
    )
    .await?;
    decode(&body)
}

/// The devices of one of the account's shops (a reinstall picks the one it
/// replaces).
pub async fn shop_devices(url: &str, session: &str, org: &str) -> Result<Vec<DeviceInfo>, Failed> {
    let url = clean_url(url)?;
    let body = send(
        HttpRequest::new("GET", format!("{url}/v1/orgs/{org}/devices"))
            .header("authorization", format!("Bearer {session}")),
    )
    .await?;
    decode(&body)
}

/// This (new) device joins one of the account's shops: no code needed.
pub async fn join_mine(
    st: AppState,
    url: String,
    session: String,
    org: String,
    replaces: Option<String>,
) -> Result<Joined, Failed> {
    let url = clean_url(&url)?;
    let mine = st
        .db()
        .call(|s| Ok(s.device().id.to_string()))
        .await
        .map_err(local)?;
    let body = serde_json::json!({
        "name": device_name(st),
        "device_id": mine,
        "replaces": replaces,
    });
    let answer = send(
        HttpRequest::new("POST", format!("{url}/v1/orgs/{org}/devices"))
            .header("authorization", format!("Bearer {session}"))
            .header("content-type", "application/json")
            .body(body.to_string().into_bytes()),
    )
    .await?;
    let joined: Joined = decode(&answer)?;
    finish_join(st, url, joined).await
}

/// What the server says about an account, in the app's words.
fn account_error(f: Failed) -> Failed {
    let key = match (f.key, f.detail.as_str()) {
        ("sync.err.unauthorized", _) => "account.err.wrong",
        (_, d) if d.contains("phone.taken") => "account.err.taken",
        (_, d) if d.contains("phone.invalid") => "account.err.phone",
        (_, d) if d.contains("password.short") => "account.err.short",
        (_, d) if d.contains("account.locked") => "account.err.locked",
        (_, d) if d.contains("name.required") => "account.err.name",
        _ => return f,
    };
    failed(key, "")
}

fn local(e: crate::db::DbError) -> Failed {
    failed("sync.err.local", e.message)
}

fn decode<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Failed> {
    serde_json::from_slice(body).map_err(|e| failed("sync.err.server", e.to_string()))
}

fn clean_url(url: &str) -> Result<String, Failed> {
    let url = url.trim().trim_end_matches('/').to_owned();
    if url.starts_with("https://") {
        Ok(url)
    } else if let Some(rest) = url.strip_prefix("http://") {
        // Passwords and tokens would cross the internet readable: plain
        // HTTP only to a server on the shop's own network.
        let host = rest.split(['/', '?']).next().unwrap_or("");
        let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
        if is_local_host(host) {
            Ok(url)
        } else {
            Err(failed("sync.err.insecure", ""))
        }
    } else {
        Err(failed("sync.err.url", ""))
    }
}

/// This computer, or an address of a local network (RFC 1918, `.local`).
fn is_local_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".local") || host == "::1" {
        return true;
    }
    match host.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::clean_url;

    #[test]
    fn plain_http_only_inside_the_shop() {
        assert!(clean_url("https://sync.example.dz/").is_ok());
        for local in [
            "http://127.0.0.1:8787",
            "http://localhost:8787",
            "http://192.168.1.20:8787",
            "http://10.0.0.5",
            "http://caisse.local:8787",
        ] {
            assert!(clean_url(local).is_ok(), "{local}");
        }
        for far in [
            "http://sync.example.dz",
            "http://41.200.1.1:8787",
            "ftp://x",
        ] {
            assert!(clean_url(far).is_err(), "{far}");
        }
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
pub async fn join(st: AppState, url: String, code: String) -> Result<Joined, Failed> {
    let url = clean_url(&url)?;
    let mine = st
        .db()
        .call(|s| Ok(s.device().id.to_string()))
        .await
        .map_err(local)?;
    let body = serde_json::json!({
        "code": code.trim(),
        "name": device_name(st),
        "device_id": mine,
    });
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
    finish_join(st, url, joined).await
}

/// Takes the shop's database from the server as this device's, with its
/// own till.
async fn finish_join(st: AppState, url: String, joined: Joined) -> Result<Joined, Failed> {
    let (db, at) = server_copy(&url, &joined.token).await?;
    let till = nb_i18n::tr_args(
        st.lang(),
        "sync.register_name",
        &[("letter", &joined.prefix)],
    );
    st.db()
        .join(db, at, joined.clone(), url, till)
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

/// The organization's devices, as the server knows them.
pub async fn devices(st: AppState) -> Result<Vec<DeviceInfo>, Failed> {
    let link = st
        .db()
        .call(|s| s.sync_link())
        .await
        .map_err(local)?
        .ok_or_else(|| failed("sync.err.not_linked", ""))?;
    let body = send(
        HttpRequest::new("GET", format!("{}/v1/devices", link.server_url))
            .header("authorization", format!("Bearer {}", link.token)),
    )
    .await?;
    decode(&body)
}

/// Stops a lost or replaced device: its token no longer opens anything.
pub async fn revoke(st: AppState, id: String) -> Result<(), Failed> {
    let link = st
        .db()
        .call(|s| s.sync_link())
        .await
        .map_err(local)?
        .ok_or_else(|| failed("sync.err.not_linked", ""))?;
    send(
        HttpRequest::new(
            "POST",
            format!("{}/v1/devices/{id}/revoke", link.server_url),
        )
        .header("authorization", format!("Bearer {}", link.token)),
    )
    .await?;
    Ok(())
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
            // An operation from an older version that holds a secret: it
            // cannot travel; the owner repairs the shop (`docs/SERVER.md`).
            Some(f) if f.code == "opaque" => return Err(failed("sync.err.opaque", f.detail)),
            Some(f) => {
                // Not sent again: undone here (the server's copy), kept for
                // a person to look at; what else waited goes on.
                set_aside(st, &link.server_url, &link.token, f.id, &f.detail).await?;
            }
        }
    }
    Ok(sent)
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

/// Sets aside an operation the server refused (see `Store::set_aside`).
async fn set_aside(
    st: AppState,
    server: &str,
    token: &str,
    refused: nb_core::OperationId,
    why: &str,
) -> Result<(), Failed> {
    let (db, at) = server_copy(server, token).await?;
    let why = why.to_owned();
    st.db()
        .call(move |s| s.set_aside(refused, &why, &db, at))
        .await
        .map_err(local)?;
    st.changed();
    Ok(())
}

/// Takes the server's copy, keeping what waits to be sent.
async fn catch_up(st: AppState, server: &str, token: &str) -> Result<(), Failed> {
    let (db, at) = server_copy(server, token).await?;
    st.db()
        .call(move |s| s.resync(&db, at))
        .await
        .map_err(local)?;
    st.changed();
    Ok(())
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

/// A shared computer: everything this browser kept of the shop goes, after
/// what waits here reached the server (nothing is lost: the server and the
/// other devices keep it all). The page starts again on its first screen.
#[cfg(target_arch = "wasm32")]
pub async fn forget_browser(st: AppState) -> Result<(), Failed> {
    refresh(st).await;
    if st.sync.get_untracked().linked && st.sync.get_untracked().pending > 0 {
        push_now(st).await?;
        refresh(st).await;
        if st.sync.get_untracked().pending > 0 {
            return Err(failed("sync.err.unsent", ""));
        }
    }
    crate::web_store::stop_saving();
    let _ = st.db().call(|s| s.clear_sync_link()).await;
    crate::web_store::clear()
        .await
        .map_err(|e| failed("sync.err.local", e))?;
    if let Some(w) = web_sys::window() {
        let _ = w.location().reload();
    }
    Ok(())
}

/// Stops syncing here (the server keeps its copy).
pub async fn unlink(st: AppState) {
    let _ = st.db().call(|s| s.clear_sync_link()).await;
    refresh(st).await;
}

/// The background exchange, from the root: soon after the start, then
/// every two seconds (what waits here goes out; the others' news comes
/// through [`listen_after`]).
pub fn start(st: AppState) {
    let tick = move || {
        if st.session.get_untracked().is_none() {
            return;
        }
        let task = spawn(async move {
            refresh(st).await;
            keep_live(st).await;
            let v = st.sync.get_untracked();
            // Sends only when something waits (a local count, cheap); what
            // the others do comes through `listen`.
            if v.linked && !v.busy && v.pending > 0 {
                let _ = push_now(st).await;
            }
        });
        drop(task); // runs to completion under the root
    };
    let first = set_timeout(std::time::Duration::from_secs(5), tick);
    drop(first); // runs while the root lives
    // What is made here goes out within seconds; news from the others
    // arrives through `listen` as soon as they send it.
    let every = set_interval(std::time::Duration::from_secs(2), tick);
    drop(every); // runs while the root lives
    listen_after(st, 6);
}

thread_local! {
    /// The live socket for this device's server and token, while linked.
    static LIVE: std::cell::RefCell<Option<crate::live::Live>> = const { std::cell::RefCell::new(None) };
    /// Whether it is up now (then no waiting requests).
    static LIVE_UP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Which socket the notices being read belong to (a newer one wins).
    static LIVE_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Keeps one live socket for the current link: started when linked,
/// replaced when the server or token changes, stopped when unlinked.
async fn keep_live(st: AppState) {
    let link = st.db().call(|s| s.sync_link()).await.ok().flatten();
    let current = LIVE.with(|l| {
        l.borrow()
            .as_ref()
            .map(|x| (x.server.clone(), x.token.clone()))
    });
    let wanted = link.map(|l| (l.server_url, l.token));
    if current == wanted {
        return;
    }
    LIVE.with(|l| *l.borrow_mut() = None);
    LIVE_UP.with(|u| u.set(false));
    let Some((server, token)) = wanted else {
        return;
    };
    let Some((live, mut rx)) = crate::live::start(&server, &token) else {
        return;
    };
    let generation = LIVE_GEN.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    LIVE.with(|l| *l.borrow_mut() = Some(live));
    let task = st.root.run(|| {
        spawn(async move {
            while let Some(notice) = rx.recv().await {
                if LIVE_GEN.with(std::cell::Cell::get) != generation {
                    return; // an older socket's last words
                }
                match notice {
                    crate::live::Notice::Up => {
                        LIVE_UP.with(|u| u.set(true));
                        // What came while it was down.
                        news(st);
                    }
                    crate::live::Notice::Changed => news(st),
                    crate::live::Notice::Down => LIVE_UP.with(|u| u.set(false)),
                }
            }
        })
    });
    drop(task); // reads until the socket ends, under the root
}

/// Another device sent something: take it (now, or as soon as the running
/// exchange ends).
fn news(st: AppState) {
    st.root.run(|| {
        let task = spawn(async move {
            if st.sync.get_untracked().busy {
                let again = set_timeout(std::time::Duration::from_secs(2), move || news(st));
                drop(again); // fires once, under the root
                return;
            }
            let _ = push_now(st).await;
        });
        drop(task); // runs to completion under the root
    });
}

/// Waits on the server for the other devices' operations (it answers the
/// moment one sends something, or after 25 s), runs them, and waits again.
fn listen_after(st: AppState, secs: u64) {
    st.root.run(|| {
        let next = set_timeout(std::time::Duration::from_secs(secs), move || {
            let task = st.root.run(|| {
                spawn(async move {
                    let again = match listen_once(st).await {
                        // Another exchange is running: it takes them.
                        Ok(true) if st.sync.get_untracked().busy => 2,
                        Ok(true) => {
                            let _ = push_now(st).await;
                            0
                        }
                        // In touch, nothing new: that counts as in touch.
                        Ok(false) => {
                            let _ = st.db().call(|s| s.note_sync("")).await;
                            0
                        }
                        // Not linked, no network, or the socket is up: look
                        // again a bit later.
                        Err(()) => 20,
                    };
                    listen_after(st, again);
                })
            });
            drop(task); // runs to completion under the root
        });
        drop(next); // fires once, under the root
    });
}

async fn listen_once(st: AppState) -> Result<bool, ()> {
    if st.session.get_untracked().is_none() {
        return Err(());
    }
    // The socket brings the news while it is up: no waiting request.
    if LIVE_UP.with(std::cell::Cell::get) {
        return Err(());
    }
    let (link, at) = st
        .db()
        .call(|s| Ok((s.sync_link()?, s.pull_position()?)))
        .await
        .map_err(|_| ())?;
    let link = link.ok_or(())?;
    let body = send(
        HttpRequest::new(
            "GET",
            format!(
                "{}/v1/wait?after={}&epoch={}",
                link.server_url, at.cursor, at.epoch
            ),
        )
        .header("authorization", format!("Bearer {}", link.token)),
    )
    .await
    .map_err(|_| ())?;
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|_| ())?;
    Ok(v["changed"].as_bool().unwrap_or(false))
}

//! `GET /v1/ws`: a WebSocket that tells a device, the moment it happens,
//! that another device of its organization sent operations (it then pulls
//! them). Only a notice travels here, never data: the data still goes
//! through `/v1/pull`, checked and in order.
//!
//! The device shows its token in the `Authorization` header (native
//! devices) or, from a browser that cannot set headers, in its first text
//! message `{"token": "…"}` within 10 seconds. Messages it receives:
//! `{"ready": true}` once in, then `{"changed": true}` each time. The
//! server pings every 25 seconds and closes the socket when the device is
//! stopped (revoked) or silent too long.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;

use crate::{DeviceRow, Server, bearer};

/// Longest wait for a browser's first message (its token).
const HELLO: Duration = Duration::from_secs(10);
/// How often the server pings (and checks the device is still allowed).
const PING: Duration = Duration::from_secs(25);
/// Largest message a device may send (only its token, or a pong).
const MAX_MESSAGE: usize = 4 << 10;

pub(crate) async fn upgrade(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // Native devices are known before the upgrade; a browser after.
    let known = bearer(&headers).and_then(|t| server.device_of(&t).ok().flatten());
    ws.max_message_size(MAX_MESSAGE)
        .max_frame_size(MAX_MESSAGE)
        .on_upgrade(move |socket| serve(server, socket, known))
}

async fn serve(server: Arc<Server>, mut socket: WebSocket, known: Option<(DeviceRow, String)>) {
    let (device, token) = match known {
        Some(k) => k,
        None => match hello(&server, &mut socket).await {
            Some(k) => k,
            None => {
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
        },
    };
    let Ok(mut wave) = server.wave(&device.org) else {
        return;
    };
    if socket
        .send(Message::Text(r#"{"ready":true}"#.into()))
        .await
        .is_err()
    {
        return;
    }
    let mut ping = tokio::time::interval(PING);
    ping.tick().await;
    let mut silent = 0u32;
    loop {
        tokio::select! {
            changed = wave.changed() => {
                if changed.is_err() {
                    return;
                }
                // Its own operations are no news to it.
                let from = wave.borrow_and_update().1.clone();
                if from == device.id {
                    continue;
                }
                if socket.send(Message::Text(r#"{"changed":true}"#.into())).await.is_err() {
                    return;
                }
            }
            _ = ping.tick() => {
                // A stopped device is cut off at once, not at its next request.
                if !matches!(server.device_of(&token), Ok(Some(_))) {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
                silent += 1;
                if silent > 3 || socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    Some(Ok(_)) => silent = 0,
                }
            }
        }
    }
}

/// A browser's first message: its token.
async fn hello(server: &Server, socket: &mut WebSocket) -> Option<(DeviceRow, String)> {
    let first = tokio::time::timeout(HELLO, socket.recv())
        .await
        .ok()??
        .ok()?;
    let Message::Text(text) = first else {
        return None;
    };
    let v: serde_json::Value = serde_json::from_str(text.as_str()).ok()?;
    let token = v["token"].as_str()?.to_owned();
    server.device_of(&token).ok().flatten()
}

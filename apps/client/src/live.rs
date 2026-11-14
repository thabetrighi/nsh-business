//! The live sync socket (`/v1/ws`): the server tells this device, the
//! moment another one sends operations, that there is news; the app then
//! pulls them. Only notices come this way; the data still goes through
//! `/v1/pull`.
//!
//! The socket lives on its own thread (its own small runtime), connects
//! again by itself after a cut (2 s, then up to a minute between tries),
//! and hands notices to the app through a channel. While it is down the
//! app falls back on waiting requests (`/v1/wait`), so nothing is missed.

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[cfg(not(target_arch = "wasm32"))]
pub use native::{Live, start};
#[cfg(target_arch = "wasm32")]
pub use web::{Live, start};

/// What the socket tells the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// In: news will come this way.
    Up,
    /// Another device sent operations.
    Changed,
    /// Cut: the app waits by requests until it is back.
    Down,
}

/// `https://host` → `wss://host/v1/ws` (`http` → `ws`, a local server).
pub fn socket_url(server: &str) -> Option<String> {
    let s = server.trim_end_matches('/');
    if let Some(rest) = s.strip_prefix("https://") {
        Some(format!("wss://{rest}/v1/ws"))
    } else {
        s.strip_prefix("http://")
            .map(|rest| format!("ws://{rest}/v1/ws"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    use super::{Notice, UnboundedReceiver, UnboundedSender, socket_url, unbounded_channel};

    /// A running socket for one server and device token. Dropping it stops
    /// the thread at its next step.
    pub struct Live {
        pub server: String,
        pub token: String,
        stop: Arc<AtomicBool>,
    }

    impl Drop for Live {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Starts the socket; its notices arrive on the returned receiver.
    pub fn start(server: &str, token: &str) -> Option<(Live, UnboundedReceiver<Notice>)> {
        let url = socket_url(server)?;
        let (tx, rx) = unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, bearer) = (stop.clone(), token.to_owned());
        let spawned = std::thread::Builder::new()
            .name("nb-live".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                rt.block_on(run(url, bearer, tx, flag));
            });
        if let Err(e) = spawned {
            log::warn!("live sync socket: {e}");
            return None;
        }
        Some((
            Live {
                server: server.to_owned(),
                token: token.to_owned(),
                stop,
            },
            rx,
        ))
    }

    async fn run(url: String, token: String, tx: UnboundedSender<Notice>, stop: Arc<AtomicBool>) {
        let mut pause = Duration::from_secs(2);
        while !stop.load(Ordering::Relaxed) {
            let up = once(&url, &token, &tx, &stop).await;
            if tx.send(Notice::Down).is_err() {
                return; // the app is gone
            }
            // A socket that worked for a while comes back fast; one that keeps
            // failing (no network, server down) is tried less and less often.
            pause = if up {
                Duration::from_secs(2)
            } else {
                (pause * 2).min(Duration::from_secs(60))
            };
            let mut waited = Duration::ZERO;
            while waited < pause && !stop.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(250)).await;
                waited += Duration::from_millis(250);
            }
        }
    }

    /// One connection, until it ends; whether it got in.
    async fn once(url: &str, token: &str, tx: &UnboundedSender<Notice>, stop: &AtomicBool) -> bool {
        let Ok(mut req) = url.into_client_request() else {
            return false;
        };
        let Ok(auth) = format!("Bearer {token}").parse() else {
            return false;
        };
        req.headers_mut().insert("authorization", auth);
        let connect = tokio_tungstenite::connect_async(req);
        let Ok(Ok((mut socket, _))) = tokio::time::timeout(Duration::from_secs(15), connect).await
        else {
            return false;
        };
        let mut up = false;
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = socket.close(None).await;
                return up;
            }
            // The server pings every 25 s; a minute of silence is a dead link.
            let next = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
            let msg = match next {
                Err(_) => continue, // nothing yet: look at `stop` again
                Ok(None) | Ok(Some(Err(_))) => return up,
                Ok(Some(Ok(m))) => m,
            };
            match msg {
                Message::Text(t) => {
                    let notice = if t.contains("\"ready\"") {
                        up = true;
                        Notice::Up
                    } else if t.contains("\"changed\"") {
                        Notice::Changed
                    } else {
                        continue;
                    };
                    if tx.send(notice).is_err() {
                        return up;
                    }
                }
                Message::Close(_) => return up,
                _ => {}
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    //! The browser's WebSocket: it cannot send headers, so the device's
    //! token is its first message. Cut, it tries again after 2 s, then up
    //! to a minute between tries.
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{CloseEvent, MessageEvent, WebSocket};

    use super::{Notice, UnboundedReceiver, UnboundedSender, socket_url, unbounded_channel};

    struct Inner {
        url: String,
        token: String,
        tx: UnboundedSender<Notice>,
        stop: Cell<bool>,
        pause_ms: Cell<i32>,
        socket: RefCell<Option<WebSocket>>,
    }

    /// A running socket; dropping it closes it and stops the retries.
    pub struct Live {
        pub server: String,
        pub token: String,
        inner: Rc<Inner>,
    }

    impl Drop for Live {
        fn drop(&mut self) {
            self.inner.stop.set(true);
            if let Some(ws) = self.inner.socket.borrow_mut().take() {
                let _ = ws.close();
            }
        }
    }

    pub fn start(server: &str, token: &str) -> Option<(Live, UnboundedReceiver<Notice>)> {
        let url = socket_url(server)?;
        let (tx, rx) = unbounded_channel();
        let inner = Rc::new(Inner {
            url,
            token: token.to_owned(),
            tx,
            stop: Cell::new(false),
            pause_ms: Cell::new(2_000),
            socket: RefCell::new(None),
        });
        connect(inner.clone());
        Some((
            Live {
                server: server.to_owned(),
                token: token.to_owned(),
                inner,
            },
            rx,
        ))
    }

    fn connect(inner: Rc<Inner>) {
        if inner.stop.get() {
            return;
        }
        let Ok(ws) = WebSocket::new(&inner.url) else {
            again(inner);
            return;
        };
        let hello = serde_json::json!({ "token": inner.token }).to_string();
        let opened = ws.clone();
        let on_open = Closure::<dyn FnMut()>::new(move || {
            let _ = opened.send_with_str(&hello);
        });
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        on_open.forget();
        let heard = inner.clone();
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Some(text) = e.data().as_string() else {
                return;
            };
            let notice = if text.contains("\"ready\"") {
                heard.pause_ms.set(2_000);
                Notice::Up
            } else if text.contains("\"changed\"") {
                Notice::Changed
            } else {
                return;
            };
            let _ = heard.tx.send(notice);
        });
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();
        let lost = inner.clone();
        let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |_: CloseEvent| {
            lost.socket.borrow_mut().take();
            let _ = lost.tx.send(Notice::Down);
            again(lost.clone());
        });
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        on_close.forget();
        *inner.socket.borrow_mut() = Some(ws);
    }

    /// Tries again after a pause that grows while it keeps failing.
    fn again(inner: Rc<Inner>) {
        if inner.stop.get() {
            return;
        }
        let pause = inner.pause_ms.get();
        inner.pause_ms.set((pause * 2).min(60_000));
        let Some(window) = web_sys::window() else {
            return;
        };
        let retry = Closure::once_into_js(move || connect(inner));
        let _ = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(retry.unchecked_ref(), pause);
    }
}

#[cfg(test)]
mod tests {
    use super::socket_url;

    #[test]
    fn socket_addresses() {
        assert_eq!(
            socket_url("https://sync.example.dz/").as_deref(),
            Some("wss://sync.example.dz/v1/ws")
        );
        assert_eq!(
            socket_url("http://192.168.1.20:8787").as_deref(),
            Some("ws://192.168.1.20:8787/v1/ws")
        );
        assert_eq!(socket_url("ftp://x"), None);
    }
}

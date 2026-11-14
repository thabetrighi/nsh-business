//! The browser keeps the shop's database in memory; this saves it to the
//! browser's IndexedDB (database `nsh-business`, store `files`, key
//! `shop`) and loads it back when the page opens. The server stays the
//! shop's real copy: this only lets the page reopen without downloading
//! everything, and keep working offline between visits.

use js_sys::{Promise, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbRequest, IdbTransactionMode};

const DB: &str = "nsh-business";
const STORE: &str = "files";
const KEY: &str = "shop";

fn text(e: JsValue) -> String {
    e.as_string()
        .or_else(|| {
            e.dyn_ref::<web_sys::DomException>()
                .map(web_sys::DomException::message)
        })
        .unwrap_or_else(|| "IndexedDB".into())
}

/// A request's result, once it settles.
async fn settled(req: &IdbRequest) -> Result<JsValue, String> {
    let promise = Promise::new(&mut |resolve, reject| {
        let ok = Closure::once_into_js(move |e: web_sys::Event| {
            let value = e
                .target()
                .and_then(|t| t.dyn_into::<IdbRequest>().ok())
                .and_then(|r| r.result().ok())
                .unwrap_or(JsValue::UNDEFINED);
            let _ = resolve.call1(&JsValue::NULL, &value);
        });
        let bad = Closure::once_into_js(move |e: web_sys::Event| {
            let _ = reject.call1(&JsValue::NULL, &e);
        });
        req.set_onsuccess(Some(ok.unchecked_ref()));
        req.set_onerror(Some(bad.unchecked_ref()));
    });
    JsFuture::from(promise).await.map_err(text)
}

async fn database() -> Result<IdbDatabase, String> {
    let window = web_sys::window().ok_or("no window")?;
    let factory = window
        .indexed_db()
        .map_err(text)?
        .ok_or("IndexedDB unavailable")?;
    let open: IdbOpenDbRequest = factory.open_with_u32(DB, 1).map_err(text)?;
    // First visit: the store the file lives in.
    let upgrade = Closure::once_into_js(move |e: web_sys::IdbVersionChangeEvent| {
        if let Some(db) = e
            .target()
            .and_then(|t| t.dyn_into::<IdbOpenDbRequest>().ok())
            .and_then(|r| r.result().ok())
            .and_then(|r| r.dyn_into::<IdbDatabase>().ok())
            && !db.object_store_names().contains(STORE)
        {
            let _ = db.create_object_store(STORE);
        }
    });
    open.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    let db = settled(&open).await?;
    db.dyn_into::<IdbDatabase>().map_err(text)
}

/// The saved database, if this browser has one.
pub async fn load() -> Result<Option<Vec<u8>>, String> {
    let db = database().await?;
    let tx = db.transaction_with_str(STORE).map_err(text)?;
    let req = tx
        .object_store(STORE)
        .map_err(text)?
        .get(&KEY.into())
        .map_err(text)?;
    let value = settled(&req).await?;
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    Ok(Some(Uint8Array::new(&value).to_vec()))
}

/// Saves the database (replacing the saved one).
pub async fn save(bytes: &[u8]) -> Result<(), String> {
    let db = database().await?;
    let tx = db
        .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite)
        .map_err(text)?;
    let data = Uint8Array::from(bytes);
    let req = tx
        .object_store(STORE)
        .map_err(text)?
        .put_with_key(&data, &KEY.into())
        .map_err(text)?;
    settled(&req).await.map(|_| ())
}

/// Forgets the saved database (unlinking a shared computer, for example).
#[allow(dead_code)]
pub async fn clear() -> Result<(), String> {
    let db = database().await?;
    let tx = db
        .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite)
        .map_err(text)?;
    let req = tx
        .object_store(STORE)
        .map_err(text)?
        .delete(&KEY.into())
        .map_err(text)?;
    settled(&req).await.map(|_| ())
}

/// Asks the browser not to clear this site's data when space runs low.
pub async fn keep() {
    if let Some(w) = web_sys::window()
        && let Ok(storage) = w.navigator().storage().persist()
    {
        let _ = JsFuture::from(storage).await;
    }
}

//! NSH Business client: one NSH app for desktop, mobile and web.
//!
//! The UI only composes and displays; business rules live in the `nb-*`
//! crates. Design rules: `docs/design/DESIGN.md`.

// UI geometry is in f32 points by nature. Money and quantities never use floats:
// they go through nb-core (Money, Quantity), where float arithmetic stays denied.
#![allow(clippy::float_arithmetic)]

use nsh_core::prelude::*;

mod command_palette;
pub mod db;
mod live;
mod nav;
pub mod printers;
mod screens;
#[cfg(not(target_arch = "wasm32"))]
mod sheets;
mod shell;
mod state;
mod sync;
mod topbar;
mod ui;
#[cfg(target_arch = "wasm32")]
mod web_store;

#[native_module("nb-client/native")]
fn native(registry: &mut Registry) {
    registry.assets(assets!());
    registry.register::<App>();
}

/// The application's root.
#[component]
pub fn App() -> impl IntoElement {
    let st = state::AppState::new();
    // Development overrides, used for screenshots: NB_LANG=ar|fr|en,
    // NB_APPEARANCE=light|dark, NB_ROUTE=…, NB_OPEN=palette|add_product,
    // NB_ACCENT=…, NB_FONT=…, NB_TEXT=…, NB_DENSITY=….
    let env = |k: &str| std::env::var(k).ok();
    if let Some(lang) = env("NB_LANG").as_deref().and_then(nb_i18n::Lang::from_tag) {
        st.lang.set(lang);
    }
    if let Some(mode) = env("NB_APPEARANCE")
        .as_deref()
        .and_then(state::ThemeMode::from_code)
    {
        st.prefs.theme.set(mode);
    }
    if let Some(a) = env("NB_ACCENT")
        .as_deref()
        .and_then(ui::tokens::Accent::from_code)
    {
        st.prefs.accent.set(a);
    }
    if let Some(f) = env("NB_FONT")
        .as_deref()
        .and_then(ui::tokens::UiFont::from_code)
    {
        st.prefs.font.set(f);
    }
    if let Some(t) = env("NB_TEXT")
        .as_deref()
        .and_then(ui::tokens::TextSize::from_code)
    {
        st.prefs.text.set(t);
    }
    if let Some(d) = env("NB_DENSITY")
        .as_deref()
        .and_then(ui::tokens::Density::from_code)
    {
        st.prefs.density.set(d);
    }
    if let Some(r) = nav::Route::from_env() {
        st.route.set(r);
    }
    if env("NB_OPEN").as_deref() == Some("palette") {
        st.palette.set(true);
    }
    // A device kept for showing prices opens on them.
    if crate::state::stored_flag(crate::screens::price_check::AT_START) {
        st.route.set(nav::Route::PriceCheck);
    }
    // NB_OPEN=sync: settings › sync (screenshots).
    if env("NB_OPEN").as_deref() == Some("sync") {
        st.route.set(nav::Route::Settings);
        st.settings_tab.set(8);
    }
    // NB_OPEN=printing: settings › printing (screenshots).
    if env("NB_OPEN").as_deref() == Some("printing") {
        st.route.set(nav::Route::Settings);
        st.settings_tab.set(5);
    }
    st.persist();
    #[cfg(not(target_arch = "wasm32"))]
    st.connect();
    // The browser: its kept copy of the shop first, then the shop opens.
    #[cfg(target_arch = "wasm32")]
    web_open(st);

    // The look (accent, font, sizes, density, corners) is read by every
    // component as plain values: a change installs the new look and rebuilds
    // the tree. Navigation and data live in `st`, outside, and stay.
    // The automatic lock (M4.7) lives here, at the root, so that no screen
    // change cancels it. Touches and page changes count as activity.
    effect(move || {
        st.route.get();
        st.touch();
    });
    let lock_timer = set_interval(std::time::Duration::from_secs(15), move || {
        let Some(me) = st.user.get_untracked() else {
            return;
        };
        let idle = state::now_ms() - st.last_active.get();
        if me.auto_lock_min > 0 && idle >= i64::from(me.auto_lock_min) * 60_000 {
            st.lock();
        }
    });
    drop(lock_timer); // runs while the root lives: tasks end with their owner

    // Scheduled backups (M14.1), from the root so that no screen ends them:
    // a check soon after the start, then every ten minutes.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let check = move || {
            if st.session.get_untracked().is_none() {
                return;
            }
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(|s| {
                        if nb_store::usecases::backups::backup_due(s)? {
                            nb_store::usecases::backups::backup_now(s, None).map(Some)
                        } else {
                            Ok(None)
                        }
                    })
                    .await;
                match r {
                    Ok(Some(p)) => log::info!("backup: {}", p.display()),
                    Ok(None) => {}
                    Err(e) => log::error!("backup: {}", e.message),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        };
        let first = set_timeout(std::time::Duration::from_secs(60), check);
        drop(first); // runs while the root lives
        let every = set_interval(std::time::Duration::from_secs(600), check);
        drop(every); // runs while the root lives
    }

    // Sync with the server (M11/M1), from the root as well.
    sync::start(st);

    let look = computed(move || st.prefs.look());
    view()
        .flex_1()
        .on_touch(move |_: &nsh_core::TouchEvent| st.touch())
        .child(dynamic(move || {
            let l = look.get();
            ui::tokens::install(l);
            ui::tokens::set_rtl(st.lang().direction() == nb_i18n::Direction::Rtl);
            ui::theme::apply(l);
            view()
                .flex_1()
                .font_family(l.font.family())
                .child(shell::shell(st))
                .into_element()
        }))
}

/// The browser keeps the shop in IndexedDB: loaded before the shop opens,
/// saved a few seconds after it changes. The shop opens anyway if the
/// browser's storage does not answer within 3 seconds.
#[cfg(target_arch = "wasm32")]
fn web_open(st: state::AppState) {
    use std::cell::Cell;
    use std::rc::Rc;
    st.root.run(|| {
        // Asking to be kept is a hint: never waited for (it may never answer).
        let ask = spawn(web_store::keep());
        drop(ask); // runs to completion under the root
        let opened = Rc::new(Cell::new(false));
        let open_once = {
            let opened = opened.clone();
            move || {
                if !opened.replace(true) {
                    st.connect();
                }
            }
        };
        let late = {
            let open_once = open_once.clone();
            set_timeout(std::time::Duration::from_secs(3), move || open_once())
        };
        drop(late); // fires once, under the root
        let was = opened.clone();
        let task = spawn(async move {
            match web_store::load().await {
                Ok(Some(bytes)) if !was.get() => {
                    if let Err(e) = st.db().adopt_saved(bytes).await {
                        log::warn!(
                            "the copy kept in this browser could not open: {}",
                            e.message
                        );
                    }
                }
                Ok(Some(_)) => log::warn!(
                    "browser storage answered late: the kept copy waits for the next visit"
                ),
                Ok(None) => {}
                Err(e) => log::warn!("browser storage: {e}"),
            }
            open_once();
        });
        drop(task); // runs to completion under the root
        let saved = Rc::new(Cell::new(0u64));
        let every = set_interval(std::time::Duration::from_secs(3), move || {
            // Not before the shop opened: an empty start must not replace a
            // copy still being read.
            if !opened.get() {
                return;
            }
            let saved = saved.clone();
            let task = spawn(async move {
                if let Ok(Some((n, bytes))) = st.db().copy_if_changed(saved.get()).await {
                    match web_store::save(&bytes).await {
                        Ok(()) => saved.set(n),
                        Err(e) => log::warn!("browser storage: {e}"),
                    }
                }
            });
            drop(task); // runs to completion under the root
        });
        drop(every); // runs while the root lives
    });
}

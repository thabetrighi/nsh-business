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
mod nav;
pub mod printers;
mod screens;
mod shell;
mod state;
mod ui;

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
    // NB_OPEN=printing: settings › printing (screenshots).
    if env("NB_OPEN").as_deref() == Some("printing") {
        st.route.set(nav::Route::Settings);
        st.settings_tab.set(5);
    }
    st.persist();
    st.connect();

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

    let look = computed(move || st.prefs.look());
    view()
        .flex_1()
        .on_touch(move |_: &nsh_core::TouchEvent| st.touch())
        .child(dynamic(move || {
            let l = look.get();
            ui::tokens::install(l);
            ui::theme::apply(l);
            view()
                .flex_1()
                .font_family(l.font.family())
                .child(shell::shell(st))
                .into_element()
        }))
}

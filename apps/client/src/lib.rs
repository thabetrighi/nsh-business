//! NSH Business client: one NSH app for desktop, mobile and web.
//!
//! The UI only composes and displays; business rules live in the `nb-*`
//! crates. Design rules: `docs/design/DESIGN.md`.

use nsh_core::prelude::*;

mod screens;
mod shell;
mod state;
mod ui;

/// UI family bundled in `assets/fonts` (IBM Plex Sans Arabic, OFL).
const FONT_FAMILY: &str = "IBM Plex Sans Arabic";

#[native_module("nb-client/native")]
fn native(registry: &mut Registry) {
    registry.assets(assets!());
    registry.register::<App>();
}

/// The application's root.
#[component]
pub fn App() -> impl IntoElement {
    let st = state::AppState::new();
    // Development overrides, used for screenshots: NB_LANG=ar|fr|en, NB_APPEARANCE=light|dark.
    if let Some(lang) = std::env::var("NB_LANG")
        .ok()
        .as_deref()
        .and_then(nb_i18n::Lang::from_tag)
    {
        st.lang.set(lang);
    }
    match std::env::var("NB_APPEARANCE").as_deref() {
        Ok("dark") => set_color_scheme(Appearance::Dark),
        Ok("light") => set_color_scheme(Appearance::Light),
        _ => {}
    }
    view()
        .flex_1()
        .font_family(FONT_FAMILY)
        .child(shell::shell(st))
}

//! What every page shares: its header (icon, section, title, subtitle,
//! actions), the tabs to the other pages of its section, and the page shown
//! for modules still being built.

use nsh_core::element::View;
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::state::AppState;
use crate::ui::icon::icon;
use crate::ui::kit::label_line;
use crate::ui::surface::{card, empty_state, page_header_full, page_pad};
use crate::ui::tokens::{Tone, sp, ty};

/// The header of `route`'s page, named from the app's map.
pub fn page_head(
    st: AppState,
    route: Route,
    subtitle: Option<&'static str>,
    actions: View,
    compact: bool,
) -> View {
    let eyebrow = route
        .section()
        .map(|(_, s)| Box::new(st.t(s.key)) as Box<dyn Fn() -> String>);
    page_header_full(
        Some(route.icon()),
        eyebrow,
        st.t(route.key()),
        // Every page says what it is for: its own line, or its route's.
        Some(Box::new(st.t(subtitle.unwrap_or(route.desc_key()))) as Box<dyn Fn() -> String>),
        actions,
        compact,
    )
}

/// Tabs to the pages of `route`'s section, under its header.
pub fn section_tabs(st: AppState, route: Route) -> View {
    let Some((_, section)) = route.section() else {
        return view();
    };
    let mut row = view().flex_row().items_end().gap(px(4.0));
    for &r in section
        .routes
        .iter()
        .filter(|r| **r == route || st.shows(**r))
    {
        let active = r == route;
        let ink = if active { Tone::Ink } else { Tone::Muted };
        row = row.child(
            view()
                .test_id(&format!("tab-{}", r.code()))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .h(px(sp(40.0)))
                .px(px(10.0))
                .cursor_pointer()
                .border_b(px(2.0))
                .border_color(if active {
                    Tone::Primary.color()
                } else {
                    rgb(0x000000).opacity(0.0).into()
                })
                .hover(|s| s.bg(Tone::Subtle.color()))
                .on_press(move || st.route.set(r))
                .child(icon(r.icon(), 16.0, move || {
                    if active { Tone::Primary } else { Tone::Faint }
                }))
                .child(label_line(
                    st.t(r.key()),
                    ty::SM,
                    if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    },
                    ink,
                )),
        );
    }
    view()
        .border_b_1()
        .border_color(Tone::Border.color())
        .child(scroll().horizontal().child(row))
}

/// A module still being built: its header and tabs, and a clear note.
pub fn coming_soon(st: AppState, route: Route, compact: bool) -> View {
    view()
        .w_full()
        .p(px(page_pad(compact)))
        .gap(px(if compact { 16.0 } else { 20.0 }))
        .child(page_head(st, route, None, view(), compact))
        .child(section_tabs(st, route))
        .child(card().child(empty_state(
            route.icon(),
            st.t("page.coming.title"),
            st.t("placeholder.coming"),
            None,
        )))
}

/// A page of a section: header with its actions, the section's tabs, then
/// the body.
pub fn page(
    st: AppState,
    route: Route,
    subtitle: &'static str,
    actions: View,
    body: View,
    compact: bool,
) -> View {
    view()
        .w_full()
        .p(px(page_pad(compact)))
        .gap(px(if compact { 16.0 } else { 20.0 }))
        .child(page_head(st, route, Some(subtitle), actions, compact))
        .child(section_tabs(st, route))
        .child(body)
}

/// A unit's short name in the language shown (`pc` → قطعة, `kg` → كغ):
/// a unit the app does not know shows as typed.
pub fn unit_short(st: AppState, code: &str) -> String {
    if code.is_empty() {
        return String::new();
    }
    let key = format!("unit.short.{code}");
    let t = st.tn(&key);
    if t == key { code.to_owned() } else { t }
}

/// The export button of a list page (one look everywhere).
pub fn export_button(st: AppState, test_id: &'static str, run: impl Fn() + 'static) -> View {
    crate::ui::kit::button(
        crate::ui::kit::Variant::Secondary,
        crate::ui::kit::Size::Md,
        Some(crate::ui::icon::Icon::Download),
        st.t("action.export"),
        run,
    )
    .test_id(test_id)
}

/// Exports `rows` (a header row, then the rows) as `file_key`'s name: one
/// save dialog offering Excel and CSV; nothing to export says so.
pub fn export_rows(st: AppState, file_key: &'static str, rows: Vec<Vec<String>>) {
    if rows.len() <= 1 {
        crate::ui::overlay::show_toast(st, st.tn("export.nothing"));
        return;
    }
    crate::screens::reports::export(st, st.tn(file_key), rows);
}

/// Reads what to export (all pages of a list, not only the one shown),
/// then writes it as rows (on the page's thread: names and formats come
/// from the session) and exports them.
pub fn export_from_db<T: Send + 'static>(
    st: AppState,
    file_key: &'static str,
    read: impl FnOnce(&mut nb_store::Store) -> Result<T, nb_store::Error> + Send + 'static,
    rows: impl FnOnce(T) -> Vec<Vec<String>> + 'static,
) {
    let task = spawn(async move {
        match st.db().call(read).await {
            Ok(v) => export_rows(st, file_key, rows(v)),
            Err(_) => crate::ui::overlay::show_toast(st, st.tn("reports.export_failed")),
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

/// Header actions side by side.
pub fn actions_row(items: Vec<View>) -> View {
    let mut v = view().flex_row().flex_wrap().items_center().gap(px(8.0));
    for i in items {
        v = v.child(i);
    }
    v
}

/// A date and time from milliseconds, in the store's time zone (UTC+1).
pub fn date_time(st: AppState, ms: i64) -> String {
    let offset = 60;
    let (y, m, d) = nb_store::time::civil_date(ms, offset);
    let mins = (ms.div_euclid(60_000) + i64::from(offset)).rem_euclid(24 * 60);
    let f = st.fmt();
    format!(
        "{} {}",
        f.date(y, m, d),
        f.time((mins / 60) as u32, (mins % 60) as u32)
    )
}

//! The import wizard (M14.3): a sheet pasted or chosen, its columns
//! matched to fields (guessed from the header, changed by the person), the
//! first rows shown as they will be read, a check that writes nothing, then
//! the import, all lines or none.

use nb_domain::import::{Kind, Table, guess, parse, rows};
use nb_i18n::tr_args;
use nb_store::usecases::catalog::ImportReport;
use nb_store::usecases::import::import_rows;
use nsh_core::element::{View, input};
use nsh_core::prelude::*;

use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::icon::Icon;
use crate::ui::kit::{Size, Variant, button, label, label_line};
use crate::ui::overlay::{dialog_frame, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::tokens::{Tone, radius, ty};

fn kind_key(kind: Kind, what: &str) -> &'static str {
    match (what, kind) {
        ("done", Kind::Products) => "import.done",
        ("done", Kind::Customers) => "import.done.customers",
        ("done", _) => "import.done.suppliers",
        ("title", Kind::Products) => "import.title",
        ("title", Kind::Customers) => "import.title.customers",
        ("title", _) => "import.title.suppliers",
        (_, Kind::Products) => "import.format",
        (_, Kind::Customers) => "import.format.customers",
        (_, _) => "import.format.suppliers",
    }
}

/// Test ids of the column choices, one per field (13 at most).
const COLUMN_IDS: [&str; 13] = [
    "import-col-0",
    "import-col-1",
    "import-col-2",
    "import-col-3",
    "import-col-4",
    "import-col-5",
    "import-col-6",
    "import-col-7",
    "import-col-8",
    "import-col-9",
    "import-col-10",
    "import-col-11",
    "import-col-12",
];

/// The wizard for `kind`.
pub fn import_dialog(st: AppState, kind: Kind, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let warehouse = session.warehouse();
    let text = signal(String::new());
    let table = computed(move || parse(&text.get()).ok());
    let fields = kind.fields();
    // One choice per field: 0 = no column, i + 1 = column i.
    let picks: Vec<Signal<usize>> = fields.iter().map(|_| signal(0usize)).collect();
    let picks_c = picks.clone();
    effect(move || {
        let headers = table.with(|t| t.as_ref().map(|t| t.headers.clone()));
        let guessed = headers.map_or_else(|| vec![None; fields.len()], |h| guess(kind, &h));
        for (p, g) in picks_c.iter().zip(guessed) {
            p.set(g.map_or(0, |c| c + 1));
        }
    });
    let messages: Signal<Vec<String>> = signal(Vec::new());
    let ready: Signal<Option<u32>> = signal(None);
    let busy = signal(false);
    let picks_m = picks.clone();
    let mapping = move || -> Vec<Option<usize>> {
        picks_m
            .iter()
            .map(|p| p.get_untracked().checked_sub(1))
            .collect()
    };
    let line_errors = move |r: &ImportReport| -> Vec<String> {
        r.errors
            .iter()
            .take(50)
            .map(|e| {
                tr_args(
                    st.lang(),
                    "import.line",
                    &[
                        ("line", &st.fmt().integer(e.line as i64)),
                        ("message", &error_text(st, &e.code)),
                    ],
                )
            })
            .collect()
    };
    // Checks (no meta) or imports.
    let go = {
        let close = close.clone();
        let mapping = mapping.clone();
        move |write: bool| {
            if busy.get_untracked() {
                return;
            }
            messages.set(Vec::new());
            ready.set(None);
            let parsed: Result<Table, _> = parse(&text.get_untracked());
            let rs = match parsed.and_then(|t| rows(kind, &t, &mapping())) {
                Ok(r) => r,
                Err(e) => {
                    messages.set(vec![error_text(st, e.code)]);
                    return;
                }
            };
            let meta = write.then(|| session.meta("import.run"));
            busy.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(move |s| import_rows(s, meta, kind, &rs, warehouse))
                    .await;
                busy.set(false);
                match r {
                    Ok(rep) if rep.errors.is_empty() && write => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                kind_key(kind, "done"),
                                &[("count", &st.fmt().integer(i64::from(rep.created)))],
                            ),
                        );
                    }
                    Ok(rep) if rep.errors.is_empty() => ready.set(Some(rep.created)),
                    Ok(rep) => messages.set(line_errors(&rep)),
                    Err(e) => messages.set(vec![error_text(
                        st,
                        e.codes.first().map_or("internal", |c| c.1),
                    )]),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let check = {
        let go = go.clone();
        move || go(false)
    };
    let run = move || go(true);
    let choose = move || {
        let task = spawn(async move {
            let dialog =
                nsh_core::file_dialog::OpenDialog::files().title(st.tn("import.choose_file"));
            if let Ok(Some(paths)) = pick_paths(dialog).await
                && let Some(p) = paths.first()
            {
                match std::fs::read_to_string(p) {
                    Ok(t) => text.set(t),
                    Err(_) => messages.set(vec![st.tn("import.unreadable")]),
                }
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    // The columns: a field per row, the column it reads.
    let picks_v = picks.clone();
    let columns = dynamic(move || {
        let Some(t) = table.get() else {
            return view().into_element();
        };
        let mut v = view().gap(px(8.0)).child(label_line(
            st.t("import.columns"),
            ty::SM,
            FontWeight::SEMIBOLD,
            Tone::Ink,
        ));
        let headers = t.headers.clone();
        for (i, (field, pick)) in fields.iter().zip(picks_v.iter().copied()).enumerate() {
            let name = format!(
                "{}{}",
                st.tn(&format!("import.field.{}", field.key)),
                if field.required { " *" } else { "" }
            );
            let hs = headers.clone();
            v = v.child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.0))
                    .child(view().w(px(170.0)).child(label_line(
                        move || name.clone(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Body,
                    )))
                    .child(view().flex_1().min_w(px(180.0)).child(dropdown(
                        move || {
                            let mut o = vec![(0usize, st.tn("import.no_column"))];
                            o.extend(hs.iter().enumerate().map(|(c, h)| (c + 1, h.clone())));
                            o
                        },
                        pick,
                        None,
                        st.t("import.no_column"),
                        COLUMN_IDS.get(i).copied().unwrap_or("import-col"),
                    ))),
            );
        }
        // The first rows, as they will be read.
        let mut preview = view()
            .gap(px(4.0))
            .p(px(10.0))
            .rounded(px(radius::md()))
            .bg(Tone::Subtle.color())
            .child(label_line(
                st.t("import.preview"),
                ty::XS,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            ));
        let m: Vec<Option<usize>> = picks_v.iter().map(|p| p.get().checked_sub(1)).collect();
        if let Ok(rs) = rows(kind, &t, &m) {
            for r in rs.iter().take(3) {
                let line = fields
                    .iter()
                    .filter_map(|f| {
                        let v = r.get(f.key);
                        (!v.is_empty()).then(|| v.to_owned())
                    })
                    .collect::<Vec<_>>()
                    .join(" · ");
                preview = preview.child(label_line(
                    move || line.clone(),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Ink,
                ));
            }
            let n = rs.len();
            preview = preview.child(label_line(
                move || {
                    tr_args(
                        st.lang(),
                        "import.rows",
                        &[("count", &st.fmt().integer(i64::try_from(n).unwrap_or(0)))],
                    )
                },
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        v.child(preview).into_element()
    });
    let body = view()
        .gap(px(14.0))
        .child(label(
            st.t(kind_key(kind, "format")),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(
            view()
                .h(px(140.0))
                .p(px(10.0))
                .rounded(px(radius::md()))
                .border_1()
                .border_color(Tone::BorderStrong.color())
                .bg(Tone::Surface.color())
                .child(
                    input()
                        .test_id("import-text")
                        .flex_1()
                        .multiline(true)
                        .value(move || text.get())
                        .on_change(move |t: &str| text.set(t.to_owned()))
                        .placeholder(st.t("import.paste"))
                        .text_size(px(crate::ui::tokens::fs(ty::SM)))
                        .text_color(Tone::Ink.color()),
                ),
        )
        .child(view().flex_row().child(button(
            Variant::Secondary,
            Size::Sm,
            Some(Icon::FileUp),
            st.t("import.choose_file"),
            choose,
        )))
        .child(columns)
        .child(dynamic(move || match ready.get() {
            Some(n) => label(
                move || {
                    tr_args(
                        st.lang(),
                        "import.ready",
                        &[("count", &st.fmt().integer(i64::from(n)))],
                    )
                },
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Success,
            )
            .test_id("import-ready")
            .into_element(),
            None => view().into_element(),
        }))
        .child(dynamic(move || {
            let list = messages.get();
            if list.is_empty() {
                return view().into_element();
            }
            let mut v = view()
                .gap(px(4.0))
                .p(px(12.0))
                .rounded(px(radius::md()))
                .bg(Tone::DangerSoft.color())
                .child(label(
                    st.t("import.nothing_written"),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Danger,
                ));
            for line in list {
                v = v.child(label(
                    move || line.clone(),
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                ));
            }
            v.test_id("import-errors").into_element()
        }));
    let footer = view()
        .child(button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("form.cancel"),
            close.clone(),
        ))
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Check),
                st.t("import.check"),
                check,
            )
            .test_id("import-check"),
        )
        .child(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Upload),
                st.t("import.run"),
                run,
            )
            .test_id("import-run"),
        );
    dialog_frame(
        st,
        st.t(kind_key(kind, "title")),
        st.t("import.description"),
        body,
        footer,
        close,
    )
}

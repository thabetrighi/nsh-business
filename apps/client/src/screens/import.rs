//! The import wizard (M14.3, owner notes): a file dropped or chosen —
//! Excel, OpenDocument or CSV — or a sheet pasted; a template to fill
//! (Excel or CSV) for each kind; its columns matched to fields (guessed
//! from the header in any of the app's languages, changed by the person),
//! with samples of what each column holds; a preview of the rows as they
//! will be read, each checked (the faulty ones marked with why); the
//! choice to import the good rows only; and a progress bar while it reads,
//! checks and writes. Nothing is written by a check; an import is all the
//! rows it takes or none.

use std::collections::BTreeMap;

use nb_domain::import::{Kind, Row, Table, guess, parse, rows};
use nb_i18n::{Lang, tr, tr_args};
use nb_store::usecases::catalog::ImportReport;
use nb_store::usecases::import::import_rows;
use nsh_core::element::{View, input};
use nsh_core::prelude::*;

use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, label, label_line, switch};
use crate::ui::overlay::{dialog_frame, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::tokens::{Status, Tone, radius, ty};

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

/// Rows shown in the preview.
const PREVIEW: usize = 50;

fn field_name(st: AppState, key: &str) -> String {
    st.tn(&format!("import.field.{key}"))
}

/// The guess from the header, then the fields' names in every language of
/// the app (a template filled in Arabic reads in French too).
fn guess_columns(kind: Kind, headers: &[String]) -> Vec<Option<usize>> {
    let mut picks = guess(kind, headers);
    let norm = |s: &str| s.trim().trim_end_matches('*').trim().to_lowercase();
    for (i, field) in kind.fields().iter().enumerate() {
        if picks[i].is_some() {
            continue;
        }
        let names: Vec<String> = [Lang::Ar, Lang::Fr, Lang::En]
            .into_iter()
            .map(|l| norm(&tr(l, &format!("import.field.{}", field.key))))
            .collect();
        let at = headers
            .iter()
            .enumerate()
            .position(|(c, h)| !picks.contains(&Some(c)) && names.contains(&norm(h)));
        picks[i] = at;
    }
    picks
}

/// A line of example for the template.
fn example(kind: Kind, key: &str) -> &'static str {
    match (kind, key) {
        (Kind::Products, "name") => "Café moulu 250 g",
        (Kind::Products, "price") => "450",
        (Kind::Products, "sku") => "CAF-250",
        (Kind::Products, "barcode") => "6130000123455",
        (Kind::Products, "cost") => "380",
        (Kind::Products, "unit") => "pc",
        (Kind::Products, "category") => "Épicerie",
        (Kind::Products, "stock") => "24",
        (Kind::Products, "tax") => "19",
        (_, "name") => "Ahmed Benali",
        (_, "kind") => "person",
        (_, "phone") => "0555 12 34 56",
        (_, "email") => "contact@example.dz",
        (_, "city") => "El Oued",
        (_, "address") => "Cité 400 logements",
        (_, "payment_days") => "30",
        (_, "credit_limit") => "50000",
        (_, "balance") => "0",
        _ => "",
    }
}

/// The template of `kind`: the fields' names, a line of example.
fn template(st: AppState, kind: Kind) -> Vec<Vec<String>> {
    let fields = kind.fields();
    vec![
        fields
            .iter()
            .map(|f| {
                let n = field_name(st, f.key);
                if f.required { format!("{n} *") } else { n }
            })
            .collect(),
        fields
            .iter()
            .map(|f| example(kind, f.key).to_owned())
            .collect(),
    ]
}

/// Where the import is: reading, checking, writing (a share of 100).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Checking,
    Writing,
    Done,
}

/// The wizard for `kind`.
pub fn import_dialog(st: AppState, kind: Kind, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let warehouse = session.warehouse();
    // What is read: pasted text, or a file (which then takes precedence).
    let text = signal(String::new());
    let from_file: Signal<Option<(String, Table)>> = signal(None);
    let read_error: Signal<Option<String>> = signal(None);
    let table = computed(move || match from_file.get() {
        Some((_, t)) => Some(t),
        None => parse(&text.get()).ok(),
    });
    let fields = kind.fields();
    // One choice per field: 0 = no column, i + 1 = column i.
    let picks: Vec<Signal<usize>> = fields.iter().map(|_| signal(0usize)).collect();
    let picks_c = picks.clone();
    effect(move || {
        let headers = table.with(|t| t.as_ref().map(|t| t.headers.clone()));
        let guessed = headers.map_or_else(|| vec![None; fields.len()], |h| guess_columns(kind, &h));
        for (p, g) in picks_c.iter().zip(guessed) {
            p.set(g.map_or(0, |c| c + 1));
        }
    });
    let picks_m = picks.clone();
    let mapping =
        move || -> Vec<Option<usize>> { picks_m.iter().map(|p| p.get().checked_sub(1)).collect() };
    let mapping_now = {
        let picks = picks.clone();
        move || -> Vec<Option<usize>> {
            picks
                .iter()
                .map(|p| p.get_untracked().checked_sub(1))
                .collect()
        }
    };
    // The check's verdict, line by line (empty: not checked or all good).
    let faults: Signal<BTreeMap<usize, Vec<String>>> = signal(BTreeMap::new());
    let checked: Signal<Option<(u32, usize)>> = signal(None);
    let general: Signal<Vec<String>> = signal(Vec::new());
    let skip_bad = signal(false);
    let stage = signal(Stage::Idle);
    // A new file, other columns: the check is to do again.
    let mapping_e = mapping.clone();
    effect(move || {
        table.with(|_| ());
        let _ = mapping_e();
        faults.set(BTreeMap::new());
        checked.set(None);
        general.set(Vec::new());
    });
    let fault_lines = move |r: &ImportReport| {
        let mut m: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for e in &r.errors {
            // The store names some fields its own way (`opening_qty`).
            let field = e.field.as_deref().and_then(|f| {
                let key = match f {
                    "opening_qty" => "stock",
                    "price_minor" => "price",
                    "cost_minor" => "cost",
                    "category_id" => "category",
                    other => other,
                };
                let k = format!("import.field.{key}");
                let n = st.tn(&k);
                (n != k).then_some(n)
            });
            let msg = match field {
                Some(f) => format!("{f}: {}", error_text(st, &e.code)),
                None => error_text(st, &e.code),
            };
            m.entry(e.line).or_default().push(msg);
        }
        m
    };
    // Checks (`write`: then imports; with `skip_bad`, the good rows only).
    let go = {
        let close = close.clone();
        let mapping_now = mapping_now.clone();
        move |write: bool| {
            if matches!(stage.get_untracked(), Stage::Checking | Stage::Writing) {
                return;
            }
            general.set(Vec::new());
            let Some(t) = table.get_untracked() else {
                // Why nothing reads: the file's fault, or the pasted text's.
                let code = read_error.get_untracked().unwrap_or_else(|| {
                    parse(&text.get_untracked())
                        .err()
                        .map_or_else(|| "import.empty".to_owned(), |e| e.code.to_owned())
                });
                general.set(vec![if code.starts_with("import.") {
                    error_text(st, &code)
                } else {
                    st.tn("import.unreadable")
                }]);
                return;
            };
            let all: Vec<Row> = match rows(kind, &t, &mapping_now()) {
                Ok(r) => r,
                Err(e) => {
                    general.set(vec![error_text(st, e.code)]);
                    return;
                }
            };
            let skip = skip_bad.get_untracked();
            let meta = write.then(|| session.meta("import.run"));
            stage.set(Stage::Checking);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                // First the check: nothing written.
                let n = all.len();
                let check = {
                    let rs = all.clone();
                    db.call(move |s| import_rows(s, None, kind, &rs, warehouse))
                        .await
                };
                let report = match check {
                    Ok(r) => r,
                    Err(e) => {
                        stage.set(Stage::Idle);
                        general.set(vec![error_text(
                            st,
                            e.codes.first().map_or("internal", |c| c.1),
                        )]);
                        return;
                    }
                };
                let bad = fault_lines(&report);
                let good = n - bad.len().min(n);
                faults.set(bad.clone());
                checked.set(Some((u32::try_from(good).unwrap_or(0), bad.len())));
                if !write || (!bad.is_empty() && !skip) || good == 0 {
                    stage.set(Stage::Idle);
                    if write && !bad.is_empty() && !skip {
                        general.set(vec![st.tn("import.nothing_written")]);
                    }
                    return;
                }
                // Then the rows that passed, in one transaction.
                stage.set(Stage::Writing);
                let keep: Vec<Row> = all
                    .into_iter()
                    .filter(|r| !bad.contains_key(&r.line))
                    .collect();
                let r = db
                    .call(move |s| import_rows(s, meta, kind, &keep, warehouse))
                    .await;
                match r {
                    Ok(rep) if rep.errors.is_empty() => {
                        stage.set(Stage::Done);
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
                    Ok(rep) => {
                        stage.set(Stage::Idle);
                        faults.set(fault_lines(&rep));
                    }
                    Err(e) => {
                        stage.set(Stage::Idle);
                        general.set(vec![error_text(
                            st,
                            e.codes.first().map_or("internal", |c| c.1),
                        )]);
                    }
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
    let load = move |path: std::path::PathBuf| {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let task = spawn(async move {
            match read(path).await {
                Ok(Ok(t)) => {
                    read_error.set(None);
                    from_file.set(Some((name, t)));
                }
                Ok(Err(code)) => {
                    from_file.set(None);
                    read_error.set(Some(code.clone()));
                    general.set(vec![if code.starts_with("import.") {
                        error_text(st, &code)
                    } else {
                        st.tn("import.unreadable")
                    }]);
                }
                Err(()) => {}
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let choose = move || {
        let task = spawn(async move {
            let dialog =
                nsh_core::file_dialog::OpenDialog::files().title(st.tn("import.choose_file"));
            if let Ok(Some(paths)) = pick_paths(dialog).await
                && let Some(p) = paths.into_iter().next()
            {
                load(p);
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let download = move |excel: bool| {
        let rows = template(st, kind);
        let name = format!("{}-{}", st.tn("import.template_file"), kind.code());
        if excel {
            crate::screens::reports::export_xlsx(st, name, rows);
        } else {
            crate::screens::reports::export_csv(st, format!("{name}.csv"), rows);
        }
    };

    // 1. The source: a file (dropped or chosen), or a sheet pasted.
    let drop_zone = view()
        .test_id("import-drop")
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(12.0))
        .p(px(16.0))
        .rounded(px(radius::lg()))
        .border_2()
        .border_color(Tone::BorderStrong.color())
        .bg(Tone::Subtle.color())
        .on_file_drop(move |e: &FileDropEvent| {
            if let Some(p) = e.uris.first().and_then(|u| file_path(u)) {
                load(p);
            }
        })
        .child(icon(Icon::FileUp, 28.0, || Tone::Primary))
        .child(
            view()
                .flex_1()
                .min_w(px(200.0))
                .gap(px(2.0))
                .child(label(
                    move || match from_file.get() {
                        Some((name, t)) => tr_args(
                            st.lang(),
                            "import.file_read",
                            &[
                                ("name", &name),
                                ("count", &st.fmt().integer(t.rows.len() as i64)),
                            ],
                        ),
                        None => st.tn("import.drop"),
                    },
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(label(
                    st.t("import.formats"),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        )
        .child(
            button(
                Variant::Primary,
                Size::Sm,
                Some(Icon::FileUp),
                st.t("import.choose_file"),
                choose,
            )
            .test_id("import-choose"),
        )
        .child(when(
            move || from_file.get().is_some(),
            move || {
                button(
                    Variant::Ghost,
                    Size::Sm,
                    Some(Icon::Close),
                    st.t("action.clear"),
                    move || from_file.set(None),
                )
                .into_element()
            },
        ));
    let templates = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .child(label_line(
            st.t("import.template"),
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Body,
        ))
        .child(
            button(
                Variant::Secondary,
                Size::Sm,
                Some(Icon::Download),
                st.t("export.xlsx"),
                move || download(true),
            )
            .test_id("import-template-xlsx"),
        )
        .child(
            button(
                Variant::Secondary,
                Size::Sm,
                Some(Icon::Download),
                st.t("export.csv"),
                move || download(false),
            )
            .test_id("import-template-csv"),
        );
    let paste = when(
        move || from_file.get().is_none(),
        move || {
            view()
                .h(px(110.0))
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
                )
                .into_element()
        },
    );
    let source = step(
        st,
        1,
        "import.step.source",
        view()
            .gap(px(12.0))
            .child(label(
                st.t(kind_key(kind, "format")),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .child(drop_zone)
            .child(templates)
            .child(paste),
    );

    // 2. The columns: each field, the column it reads, what that column holds.
    let picks_v = picks.clone();
    let columns = dynamic(move || {
        let Some(t) = table.get() else {
            return label(
                st.t("import.no_table"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        };
        let headers = t.headers.clone();
        let samples = move |c: usize| -> String {
            t.rows
                .iter()
                .filter_map(|(_, r)| r.get(c).filter(|v| !v.is_empty()).cloned())
                .take(3)
                .collect::<Vec<_>>()
                .join(" · ")
        };
        let mut v = view()
            .rounded(px(radius::md()))
            .border_1()
            .border_color(Tone::Border.color());
        v = v.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(8.0))
                .bg(Tone::Subtle.color())
                .child(view().w(px(170.0)).child(label_line(
                    st.t("import.col.field"),
                    ty::XS,
                    FontWeight::SEMIBOLD,
                    Tone::Muted,
                )))
                .child(view().w(px(220.0)).child(label_line(
                    st.t("import.col.column"),
                    ty::XS,
                    FontWeight::SEMIBOLD,
                    Tone::Muted,
                )))
                .child(view().flex_1().child(label_line(
                    st.t("import.col.sample"),
                    ty::XS,
                    FontWeight::SEMIBOLD,
                    Tone::Muted,
                ))),
        );
        for (i, (field, pick)) in fields.iter().zip(picks_v.iter().copied()).enumerate() {
            let name = format!(
                "{}{}",
                field_name(st, field.key),
                if field.required { " *" } else { "" }
            );
            let hs = headers.clone();
            let required = field.required;
            let sample = {
                let samples = samples.clone();
                move || pick.get().checked_sub(1).map(&samples).unwrap_or_default()
            };
            v = v.child(
                view()
                    .test_id(&format!("import-field-{i}"))
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(12.0))
                    .py(px(6.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(
                        view()
                            .w(px(170.0))
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(dynamic(move || {
                                let ok = pick.get() > 0;
                                let (ic, tone) = match (ok, required) {
                                    (true, _) => (Icon::CheckCircle, Tone::Success),
                                    (false, true) => (Icon::Alert, Tone::Danger),
                                    (false, false) => (Icon::Minus, Tone::Faint),
                                };
                                icon(ic, 14.0, move || tone).into_element()
                            }))
                            .child(label_line(
                                move || name.clone(),
                                ty::SM,
                                FontWeight::MEDIUM,
                                Tone::Body,
                            )),
                    )
                    .child(view().w(px(220.0)).child(dropdown(
                        move || {
                            let mut o = vec![(0usize, st.tn("import.no_column"))];
                            o.extend(hs.iter().enumerate().map(|(c, h)| (c + 1, h.clone())));
                            o
                        },
                        pick,
                        None,
                        st.t("import.no_column"),
                        COLUMN_IDS.get(i).copied().unwrap_or("import-col"),
                    )))
                    .child(view().flex_1().min_w(px(120.0)).child(label_line(
                        sample,
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    ))),
            );
        }
        v.into_element()
    });
    let mapping_step = step(st, 2, "import.step.columns", view().child(columns));

    // 3. The rows as they will be read, checked.
    let preview = dynamic(move || {
        let Some(t) = table.get() else {
            return view().into_element();
        };
        let m = mapping();
        let Ok(rs) = rows(kind, &t, &m) else {
            return label(
                st.t("import.map_required"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Warning,
            )
            .into_element();
        };
        let bad = faults.get();
        let shown: Vec<&'static str> = fields
            .iter()
            .zip(&m)
            .filter(|(_, c)| c.is_some())
            .map(|(f, _)| f.key)
            .collect();
        let mut head = view()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(8.0))
            .bg(Tone::Subtle.color())
            .child(view().w(px(48.0)).child(label_line(
                st.t("import.col.line"),
                ty::XS,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            )));
        for k in &shown {
            let n = field_name(st, k);
            head = head.child(view().flex_1().min_w(px(90.0)).child(label_line(
                move || n.clone(),
                ty::XS,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            )));
        }
        let mut body = view()
            .rounded(px(radius::md()))
            .border_1()
            .border_color(Tone::Border.color())
            .child(head);
        for (i, r) in rs.iter().take(PREVIEW).enumerate() {
            let errs = bad.get(&r.line).cloned();
            let wrong = errs.is_some();
            let line = r.line;
            let mut cells =
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(view().w(px(48.0)).child(label_line(
                        move || line.to_string(),
                        ty::XS,
                        FontWeight::MEDIUM,
                        if wrong { Tone::Danger } else { Tone::Muted },
                    )));
            for k in &shown {
                let v = r.get(k).to_owned();
                cells = cells.child(view().flex_1().min_w(px(90.0)).child(label_line(
                    move || v.clone(),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Ink,
                )));
            }
            let mut row = view()
                .test_id(&format!("import-row-{i}"))
                .px(px(10.0))
                .py(px(6.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .bg(if wrong {
                    Tone::DangerSoft.color()
                } else {
                    Tone::Surface.color()
                })
                .child(cells);
            if let Some(e) = errs {
                let msg = e.join(" — ");
                row = row.child(label(
                    move || msg.clone(),
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                ));
            }
            body = body.child(row);
        }
        let n = rs.len();
        let more = n.saturating_sub(PREVIEW);
        let mut v = view().gap(px(8.0)).child(body);
        if more > 0 {
            v = v.child(label(
                move || {
                    tr_args(
                        st.lang(),
                        "import.more_rows",
                        &[("count", &st.fmt().integer(more as i64))],
                    )
                },
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        v.into_element()
    });
    let summary = dynamic(move || {
        let total = table.with(|t| t.as_ref().map_or(0, |t| t.rows.len()));
        if total == 0 {
            return view().into_element();
        }
        let mut v = view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(badge(Status::Neutral, Some(Icon::FileText), move || {
                tr_args(
                    st.lang(),
                    "import.rows",
                    &[("count", &st.fmt().integer(total as i64))],
                )
            }));
        if let Some((good, bad)) = checked.get() {
            v = v.child(
                badge(Status::Success, Some(Icon::CheckCircle), move || {
                    tr_args(
                        st.lang(),
                        "import.good",
                        &[("count", &st.fmt().integer(i64::from(good)))],
                    )
                })
                .test_id("import-ready"),
            );
            if bad > 0 {
                v = v.child(badge(Status::Danger, Some(Icon::Alert), move || {
                    tr_args(
                        st.lang(),
                        "import.bad",
                        &[("count", &st.fmt().integer(bad as i64))],
                    )
                }));
            }
        }
        v.into_element()
    });
    let options = view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(switch(skip_bad, st.t("import.skip_bad")).test_id("import-skip-bad"))
        .child(label(
            st.t("import.skip_bad"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Body,
        ));
    let progress = dynamic(move || {
        let s = stage.get();
        if s == Stage::Idle {
            return view().into_element();
        }
        let (share, key) = match s {
            Stage::Checking => (0.4_f32, "import.stage.checking"),
            Stage::Writing => (0.8, "import.stage.writing"),
            _ => (1.0, "import.stage.done"),
        };
        view()
            .test_id("import-progress")
            .gap(px(6.0))
            .child(label_line(
                st.t(key),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Primary,
            ))
            .child(
                view()
                    .h(px(8.0))
                    .rounded_full()
                    .bg(Tone::Subtle.color())
                    .child(
                        view()
                            .h_full()
                            .w(relative(share))
                            .rounded_full()
                            .bg(Tone::Primary.color()),
                    ),
            )
            .into_element()
    });
    let errors = dynamic(move || {
        let list = general.get();
        if list.is_empty() {
            return view().into_element();
        }
        let mut v = view()
            .gap(px(4.0))
            .p(px(12.0))
            .rounded(px(radius::md()))
            .bg(Tone::DangerSoft.color());
        for line in list {
            v = v.child(label(
                move || line.clone(),
                ty::SM,
                FontWeight::MEDIUM,
                Tone::Danger,
            ));
        }
        v.test_id("import-errors").into_element()
    });
    let preview_step = step(
        st,
        3,
        "import.step.preview",
        view()
            .gap(px(12.0))
            .child(summary)
            .child(preview)
            .child(options)
            .child(progress)
            .child(errors),
    );
    let body = view()
        .gap(px(16.0))
        .child(source)
        .child(mapping_step)
        .child(preview_step);
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

/// A numbered part of the wizard.
fn step(st: AppState, n: u8, key: &'static str, body: View) -> View {
    view()
        .gap(px(12.0))
        .p(px(16.0))
        .rounded(px(radius::lg()))
        .border_1()
        .border_color(Tone::Border.color())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    view()
                        .w(px(26.0))
                        .h(px(26.0))
                        .rounded_full()
                        .items_center()
                        .justify_center()
                        .bg(Tone::Primary.color())
                        .child(label_line(
                            move || n.to_string(),
                            ty::SM,
                            FontWeight::BOLD,
                            Tone::OnPrimary,
                        )),
                )
                .child(label_line(st.t(key), ty::MD, FontWeight::BOLD, Tone::Ink)),
        )
        .child(body)
}

/// Reads a chosen file off the interface's thread (a workbook can take a
/// moment); `Err(())` if the reader went away.
#[cfg(not(target_arch = "wasm32"))]
async fn read(path: std::path::PathBuf) -> Result<Result<Table, String>, ()> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::sheets::read_file(&path));
    });
    rx.await.map_err(|_| ())
}

/// On the web a file is read as text.
#[cfg(target_arch = "wasm32")]
async fn read(path: std::path::PathBuf) -> Result<Result<Table, String>, ()> {
    Ok(std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| parse(&t).map_err(|e| e.code.to_owned())))
}

/// A dropped file's path from its `file:` URL (percent-escapes decoded).
fn file_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///C:/x` on Windows: no leading slash before the drive.
    let rest = match rest.as_bytes() {
        [b'/', _, b':', ..] if cfg!(windows) => &rest[1..],
        _ => rest,
    };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).ok()?);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok().map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_uris_become_paths() {
        assert_eq!(
            file_path("file:///home/a/My%20Sheet%C3%A9.xlsx"),
            Some(std::path::PathBuf::from("/home/a/My Sheeté.xlsx"))
        );
        assert_eq!(file_path("https://x"), None);
    }

    #[test]
    fn headers_in_any_language_are_matched() {
        let h = vec!["سعر البيع *".to_owned(), "Nom".to_owned()];
        let g = guess_columns(Kind::Products, &h);
        assert_eq!(g[0], Some(1));
        assert_eq!(g[1], Some(0));
    }
}

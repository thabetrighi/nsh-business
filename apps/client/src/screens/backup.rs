//! Backups in the settings (M14.1, M14.2): the passphrase, the schedule,
//! how many to keep and where; back up now; the backups found, each
//! checked or restored; restoring from another file. Backups exist on
//! devices with files (not in the browser).

use std::path::PathBuf;

use nb_i18n::tr_args;
use nb_store::usecases::backups::{
    BackupFile, BackupSettings, LogRow, SettingsInput, backup_log, backup_now, backup_settings,
    list_backups, save_backup_settings, verify_backup,
};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::screens::common::date_time;
use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::form::{field, labelled, secret_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, divider, label, label_line, segmented};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::surface::{card, card_header_icon};
use crate::ui::tokens::{Status, Tone, ty};

#[derive(Debug, Clone, PartialEq, Default)]
struct Shelf {
    settings: Option<BackupSettings>,
    files: Vec<BackupFile>,
    log: Vec<LogRow>,
    default_dir: String,
}

fn first_error(st: AppState, e: &crate::db::DbError) -> String {
    error_text(st, e.codes.first().map_or("internal", |c| c.1))
}

fn size_text(st: AppState, bytes: u64) -> String {
    let kb = i64::try_from(bytes / 1024).unwrap_or(i64::MAX).max(1);
    if kb < 1024 {
        format!("{} KB", st.fmt().integer(kb))
    } else {
        format!("{} MB", st.fmt().integer(kb / 1024))
    }
}

fn file_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// The backup card of the settings' data tab.
pub fn backup_card(st: AppState, compact: bool) -> View {
    let data = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    let default_dir = s
                        .path()
                        .and_then(|p| p.parent().map(|d| d.join("backups")))
                        .unwrap_or_else(|| std::env::temp_dir().join("nsh-business-backups"));
                    Ok(Shelf {
                        settings: backup_settings(s)?,
                        files: list_backups(s)?,
                        log: backup_log(s, 6)?,
                        default_dir: default_dir.to_string_lossy().into_owned(),
                    })
                })
                .await
            }
        },
    );
    let shelf = computed(move || data.with(|d: Option<&Shelf>| d.cloned()));
    let pass = signal(String::new());
    let pass2 = signal(String::new());
    // 0: off, 1: daily, 2: weekly.
    let schedule = signal(1usize);
    let keep = signal("14".to_owned());
    let dir = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    // The form follows what is saved once it loads.
    effect(move || {
        if let Some(sh) = shelf.get() {
            match &sh.settings {
                Some(s) => {
                    schedule.set(match s.schedule.as_str() {
                        "off" => 0,
                        "weekly" => 2,
                        _ => 1,
                    });
                    keep.set(s.keep.to_string());
                    dir.set(s.directory.clone());
                }
                None => {
                    if dir.get_untracked().is_empty() {
                        dir.set(sh.default_dir.clone());
                    }
                }
            }
        }
    });
    let save = move || {
        if busy.get_untracked() {
            return;
        }
        let Some(session) = st.session.get_untracked() else {
            return;
        };
        if pass.get_untracked() != pass2.get_untracked() {
            error.set(st.tn("backup.passphrase_mismatch"));
            return;
        }
        let input = SettingsInput {
            passphrase: pass.get_untracked(),
            schedule: ["off", "daily", "weekly"][schedule.get_untracked().min(2)].into(),
            keep: keep.get_untracked().trim().parse().unwrap_or(0),
            directory: dir.get_untracked(),
        };
        let meta = session.meta("backup.configure");
        busy.set(true);
        error.set(String::new());
        let db = st.db();
        let task = spawn(async move {
            let r = db.call(move |s| save_backup_settings(s, meta, input)).await;
            busy.set(false);
            match r {
                Ok(_) => {
                    pass.set(String::new());
                    pass2.set(String::new());
                    st.changed();
                    show_toast(st, st.tn("backup.saved"));
                }
                Err(e) => error.set(first_error(st, &e)),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let run_now = move || {
        if busy.get_untracked() {
            return;
        }
        let actor = st
            .session
            .get_untracked()
            .map(|s| s.meta("backup.run").actor.map(|a| a.to_string()));
        busy.set(true);
        error.set(String::new());
        let db = st.db();
        let task = spawn(async move {
            let r = db
                .call(move |s| backup_now(s, actor.flatten().as_deref()))
                .await;
            busy.set(false);
            match r {
                Ok(p) => {
                    st.changed();
                    show_toast(
                        st,
                        tr_args(
                            st.lang(),
                            "backup.done",
                            &[("name", &file_name(&p.to_string_lossy()))],
                        ),
                    );
                }
                Err(e) => {
                    st.changed();
                    error.set(first_error(st, &e));
                }
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let restoring: Signal<Option<(String, bool)>> = signal(None);
    let restoring_open = signal(false);
    effect(move || {
        let open = restoring.get().is_some();
        if restoring_open.get_untracked() != open {
            restoring_open.set(open);
        }
    });
    effect(move || {
        if !restoring_open.get() {
            restoring.set(None);
        }
    });
    let check = move |path: String| {
        let db = st.db();
        let task = spawn(async move {
            let p = PathBuf::from(&path);
            match db.call(move |s| verify_backup(s, &p, "")).await {
                Ok(_) => show_toast(
                    st,
                    tr_args(st.lang(), "backup.checked", &[("name", &file_name(&path))]),
                ),
                Err(e) => show_toast(st, first_error(st, &e)),
            }
            st.changed();
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let pick = move || {
        let task = spawn(async move {
            let dialog = nsh_core::file_dialog::OpenDialog::files()
                .title(st.tn("backup.restore_file"))
                .filter(nsh_core::file_dialog::FileFilter::new(
                    st.tn("backup.file_kind"),
                    ["nbk"],
                ));
            if let Ok(Some(paths)) = pick_paths(dialog).await
                && let Some(p) = paths.first()
            {
                restoring.set(Some((p.to_string_lossy().into_owned(), true)));
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let status = dynamic(move || {
        let Some(sh) = shelf.get() else {
            return view().into_element();
        };
        let last = sh.log.iter().find(|l| l.kind == "backup" && l.ok).cloned();
        let failed = sh
            .log
            .first()
            .filter(|l| l.kind == "backup" && !l.ok)
            .cloned();
        let mut v = view().flex_row().flex_wrap().items_center().gap(px(10.0));
        v = v.child(match (&sh.settings, &last) {
            (None, _) => badge(Status::Warning, Some(Icon::Alert), st.t("backup.not_set")),
            (Some(_), Some(l)) => {
                let when = date_time(st, l.created_at);
                badge(Status::Success, Some(Icon::Check), move || {
                    tr_args(st.lang(), "backup.last", &[("when", &when)])
                })
            }
            (Some(_), None) => badge(Status::Warning, Some(Icon::Alert), st.t("backup.never")),
        });
        if let Some(f) = failed {
            let why = error_text(st, &f.message);
            v = v.child(label_line(
                move || tr_args(st.lang(), "backup.last_failed", &[("why", &why)]),
                ty::SM,
                FontWeight::MEDIUM,
                Tone::Danger,
            ));
        }
        v.into_element()
    });
    let set_up = computed(move || shelf.get().is_some_and(|s| s.settings.is_some()));
    let form = view()
        .gap(px(14.0))
        .p(px(20.0))
        .child(status)
        .child(label(
            st.t("backup.help"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(12.0))
                .child(view().flex_1().min_w(px(200.0)).child(secret_field(
                    move || {
                        st.tn(if set_up.get() {
                            "backup.new_passphrase"
                        } else {
                            "backup.passphrase"
                        })
                    },
                    pass,
                    String::new,
                    "bk-pass",
                )))
                .child(view().flex_1().min_w(px(200.0)).child(secret_field(
                    st.t("backup.passphrase_again"),
                    pass2,
                    String::new,
                    "bk-pass2",
                ))),
        )
        .child(label(
            st.t("backup.passphrase_warning"),
            ty::XS,
            FontWeight::MEDIUM,
            Tone::Warning,
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap(px(12.0))
                .child(labelled(
                    st.t("backup.schedule"),
                    segmented(
                        vec![
                            (0, Box::new(st.t("backup.schedule.off"))),
                            (1, Box::new(st.t("backup.schedule.daily"))),
                            (2, Box::new(st.t("backup.schedule.weekly"))),
                        ],
                        schedule,
                    ),
                    None,
                ))
                .child(view().w(px(120.0)).child(field(
                    st.t("backup.keep"),
                    keep,
                    String::new,
                    String::new,
                    KeyboardType::Number,
                    "bk-keep",
                ))),
        )
        .child(field(
            st.t("backup.directory"),
            dir,
            String::new,
            String::new,
            KeyboardType::Default,
            "bk-dir",
        ))
        .child(when(
            move || !error.get().is_empty(),
            move || {
                label(
                    move || error.get(),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                )
                .into_element()
            },
        ));
    let actions = view()
        .flex_row()
        .flex_wrap()
        .gap(px(8.0))
        .justify_end()
        .px(px(20.0))
        .py(px(14.0))
        .bg(Tone::Subtle.color())
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Upload),
                st.t("backup.restore_file"),
                pick,
            )
            .test_id("bk-pick"),
        )
        .child(when(
            move || set_up.get(),
            move || {
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::HardDrive),
                    st.t("action.backup"),
                    run_now,
                )
                .test_id("bk-now")
                .into_element()
            },
        ))
        .child(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Check),
                st.t("form.save"),
                save,
            )
            .test_id("bk-save"),
        );
    let files = dynamic(move || {
        let Some(sh) = shelf.get() else {
            return view().into_element();
        };
        if sh.files.is_empty() {
            return view().into_element();
        }
        let mut v = view()
            .child(divider())
            .child(view().px(px(20.0)).pt(px(14.0)).child(label_line(
                st.t("backup.files"),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            )));
        for (i, f) in sh.files.into_iter().enumerate() {
            let name = file_name(&f.path);
            let detail = match &f.header {
                Some(h) => format!(
                    "{} · {} · {}",
                    date_time(st, h.created_at),
                    size_text(st, f.size),
                    h.organization
                ),
                None => st.tn("error.backup.not_a_backup"),
            };
            let (p1, p2) = (f.path.clone(), f.path.clone());
            let mut row = view()
                .test_id(&format!("bk-file-{i}"))
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(10.0))
                .px(px(20.0))
                .py(px(10.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(icon(Icon::HardDrive, 18.0, || Tone::Faint))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(180.0))
                        .child(label_line(
                            move || name.clone(),
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            move || detail.clone(),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                );
            row = row
                .child(
                    button(
                        Variant::Ghost,
                        Size::Sm,
                        Some(Icon::Check),
                        st.t("backup.check"),
                        move || check(p1.clone()),
                    )
                    .test_id(&format!("bk-check-{i}")),
                )
                .child(
                    button(
                        Variant::Secondary,
                        Size::Sm,
                        Some(Icon::Upload),
                        st.t("action.restore"),
                        move || restoring.set(Some((p2.clone(), false))),
                    )
                    .test_id(&format!("bk-restore-{i}")),
                );
            v = v.child(row);
        }
        v.into_element()
    });
    let _ = compact;
    view()
        .child(
            card()
                .child(card_header_icon(
                    Some(Icon::HardDrive),
                    st.t("backup.title"),
                    Some(Box::new(st.t("backup.subtitle"))),
                    view(),
                ))
                .child(divider())
                .child(form)
                .child(files)
                .child(actions),
        )
        .child(modal(restoring_open, compact, move || {
            match restoring.get_untracked() {
                Some((path, foreign)) => {
                    restore_dialog(st, path, foreign, move || restoring_open.set(false))
                        .into_element()
                }
                None => view().into_element(),
            }
        }))
}

/// Confirms a restore: what it replaces, and the passphrase when the file
/// is not one of this store's.
fn restore_dialog(
    st: AppState,
    path: String,
    foreign: bool,
    close: impl Fn() + Clone + 'static,
) -> View {
    let pass = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let p = path.clone();
    let go = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Some(session) = st.session.get_untracked() else {
                return;
            };
            let meta = session.meta("backup.restore");
            let (path, pass_v) = (PathBuf::from(&p), pass.get_untracked());
            busy.set(true);
            error.set(String::new());
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.restore(meta, path, pass_v).await {
                    Ok(r) => {
                        close();
                        show_toast(
                            st,
                            if r.health.is_sound() {
                                st.tn("backup.restored")
                            } else {
                                st.tn("backup.restored_unsound")
                            },
                        );
                        // The data changed under every screen: start again
                        // from the store, as at launch.
                        st.route.set(crate::nav::Route::Home);
                        st.connect();
                        st.changed();
                    }
                    Err(e) => {
                        busy.set(false);
                        error.set(first_error(st, &e));
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let name = file_name(&path);
    let mut body = view()
        .gap(px(14.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .p(px(10.0))
                .rounded(px(8.0))
                .bg(Tone::Subtle.color())
                .child(icon(Icon::HardDrive, 18.0, || Tone::Faint))
                .child(label_line(
                    move || name.clone(),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                )),
        )
        .child(label(
            st.t("backup.restore_warning"),
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Danger,
        ));
    body = body.child(secret_field(
        st.t(if foreign {
            "backup.passphrase"
        } else {
            "backup.passphrase_optional"
        }),
        pass,
        String::new,
        "bk-restore-pass",
    ));
    let body = body.child(when(
        move || !error.get().is_empty(),
        move || {
            label(
                move || error.get(),
                ty::SM,
                FontWeight::MEDIUM,
                Tone::Danger,
            )
            .into_element()
        },
    ));
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
                Variant::Danger,
                Size::Md,
                Some(Icon::Upload),
                st.t("backup.restore_confirm"),
                go,
            )
            .test_id("bk-restore-confirm"),
        );
    dialog_frame(
        st,
        st.t("backup.restore_title"),
        st.t("backup.restore_help"),
        body,
        footer,
        close,
    )
}

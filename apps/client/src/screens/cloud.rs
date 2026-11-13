//! Settings › sync (M11/M1): link this device to a server (its address and
//! the server's enrollment key), then see what waits, when it last went
//! well, sync now, check that the server's copy is this one, or unlink.

use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::screens::common::date_time;
use crate::state::AppState;
use crate::ui::form::field;
use crate::ui::icon::Icon;
use crate::ui::kit::{Size, Variant, badge, button, divider, label};
use crate::ui::overlay::show_toast;
use crate::ui::surface::{card, card_header_icon};
use crate::ui::tokens::{Status, Tone, ty};

fn show_failure(st: AppState, f: &crate::sync::Failed) {
    let mut t = st.tn(f.key);
    if !f.detail.is_empty() {
        t.push_str(" — ");
        t.push_str(&f.detail);
    }
    show_toast(st, t);
}

pub fn cloud_card(st: AppState, compact: bool) -> View {
    let _ = compact;
    let task = spawn(crate::sync::refresh(st));
    drop(task); // reads the state once; the root keeps it fresh
    let v = computed(move || st.sync.get());
    let state_badge = dynamic(move || {
        let s = v.get();
        let (status, which, key) = if !s.linked {
            (Status::Neutral, Icon::Offline, "sync.not_linked")
        } else if s.busy {
            (Status::Info, Icon::Refresh, "sync.busy")
        } else if !s.error.is_empty() {
            (Status::Danger, Icon::Alert, "sync.failing")
        } else if s.pending > 0 {
            (Status::Warning, Icon::CloudUpload, "sync.waiting")
        } else {
            (Status::Success, Icon::CheckCircle, "sync.synced")
        };
        badge(status, Some(which), st.t(key)).into_element()
    });
    let body = dynamic(move || {
        let s = v.get();
        if s.linked {
            linked_body(st, s).into_element()
        } else {
            link_form(st).into_element()
        }
    });
    card()
        .test_id("cloud-card")
        .child(card_header_icon(
            Some(Icon::CloudUpload),
            st.t("sync.status"),
            Some(Box::new(st.t("sync.local_first"))),
            view().child(state_badge),
        ))
        .child(divider())
        .child(body)
}

fn stat(st: AppState, key: &'static str, value: String) -> View {
    view()
        .flex_1()
        .min_w(px(150.0))
        .gap(px(4.0))
        .child(label(st.t(key), ty::XS, FontWeight::MEDIUM, Tone::Muted))
        .child(label(
            move || value.clone(),
            ty::MD,
            FontWeight::SEMIBOLD,
            Tone::Ink,
        ))
}

fn linked_body(st: AppState, s: crate::sync::SyncView) -> View {
    let last = s
        .last_ok_at
        .map(|t| date_time(st, t))
        .unwrap_or_else(|| st.tn("sync.never"));
    let mut v = view().child(
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(20.0))
            .p(px(20.0))
            .child(stat(st, "sync.server", s.server.clone()))
            .child(stat(st, "sync.pending_ops", st.fmt().integer(s.pending)))
            .child(stat(st, "sync.last", last)),
    );
    if !s.error.is_empty() {
        let e = s.error.clone();
        v = v.child(view().px(px(20.0)).pb(px(14.0)).child(label(
            move || e.clone(),
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Danger,
        )));
    }
    v.child(
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(8.0))
            .justify_end()
            .px(px(20.0))
            .py(px(14.0))
            .bg(Tone::Subtle.color())
            .border_t_1()
            .border_color(Tone::Border.color())
            .child(button(
                Variant::Ghost,
                Size::Md,
                Some(Icon::Unlink),
                st.t("sync.unlink"),
                move || {
                    let task = spawn(crate::sync::unlink(st));
                    drop(task); // runs to completion
                },
            ))
            .child(
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Shield),
                    st.t("sync.verify"),
                    move || {
                        let task = st.root.run(|| {
                            spawn(async move {
                                match crate::sync::verify(st).await {
                                    Ok(true) => show_toast(st, st.tn("sync.same")),
                                    Ok(false) => show_toast(st, st.tn("sync.differs")),
                                    Err(f) => show_failure(st, &f),
                                }
                            })
                        });
                        drop(task); // runs to completion
                    },
                )
                .test_id("sync-verify"),
            )
            .child(
                button(
                    Variant::Primary,
                    Size::Md,
                    Some(Icon::Refresh),
                    st.t("action.sync_now"),
                    move || {
                        let task = st.root.run(|| {
                            spawn(async move {
                                match crate::sync::push_now(st).await {
                                    Ok(n) => show_toast(
                                        st,
                                        nb_i18n::tr_args(
                                            st.lang(),
                                            "sync.sent",
                                            &[("n", &st.fmt().integer(n as i64))],
                                        ),
                                    ),
                                    Err(f) => show_failure(st, &f),
                                }
                            })
                        });
                        drop(task); // runs to completion
                    },
                )
                .test_id("sync-now"),
            ),
    )
}

fn link_form(st: AppState) -> View {
    let url = signal(String::new());
    let key = signal(String::new());
    let busy = signal(false);
    let go = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        let (u, k) = (url.get_untracked(), key.get_untracked());
        let task = st.root.run(|| {
            spawn(async move {
                #[cfg(not(target_arch = "wasm32"))]
                match crate::sync::link(st, u, k).await {
                    Ok(l) => show_toast(
                        st,
                        nb_i18n::tr_args(st.lang(), "sync.linked", &[("org", &l.org_name)]),
                    ),
                    Err(f) => show_failure(st, &f),
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (u, k);
                    show_toast(st, st.tn("sync.web_later"));
                }
                busy.set(false);
            })
        });
        drop(task); // runs to completion
    };
    view()
        .p(px(20.0))
        .gap(px(14.0))
        .child(label(
            st.t("sync.link_hint"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Body,
        ))
        .child(field(
            st.t("sync.server"),
            url,
            st.t("sync.server_placeholder"),
            String::new,
            KeyboardType::Url,
            "sync-url",
        ))
        .child(field(
            st.t("sync.key"),
            key,
            String::new,
            String::new,
            KeyboardType::Default,
            "sync-key",
        ))
        .child(
            view().flex_row().justify_end().child(
                button(
                    Variant::Primary,
                    Size::Md,
                    Some(Icon::Link),
                    st.t("sync.link"),
                    go,
                )
                .test_id("sync-link"),
            ),
        )
}

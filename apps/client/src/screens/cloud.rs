//! Settings › sync (M11/M1–M2): link this device to a server (its address
//! and the server's enrollment key), then see what waits, when it last went
//! well, sync now, check that the server's copy is this one, invite another
//! device of the shop (a code it enters on its first screen), or unlink.

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
    // Outside the part drawn again when the state changes.
    let invite = signal(None::<nb_store::sync::Invite>);
    let devices = signal(None::<Vec<nb_store::sync::DeviceInfo>>);
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
            linked_body(st, s, invite, devices).into_element()
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

fn linked_body(
    st: AppState,
    s: crate::sync::SyncView,
    invite: Signal<Option<nb_store::sync::Invite>>,
    devices: Signal<Option<Vec<nb_store::sync::DeviceInfo>>>,
) -> View {
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
            .child(stat(st, "sync.last", last))
            .child(stat(st, "sync.reviews", st.fmt().integer(s.reviews))),
    );
    v = v.child(dynamic(move || match invite.get() {
        None => view().into_element(),
        Some(i) => {
            let hint = nb_i18n::tr_args(
                st.lang(),
                "sync.invite_hint",
                &[("time", &date_time(st, i.expires_at))],
            );
            let code = format!(
                "{} {}",
                &i.code[..4.min(i.code.len())],
                &i.code[4.min(i.code.len())..]
            );
            view()
                .mx(px(20.0))
                .mb(px(14.0))
                .p(px(16.0))
                .gap(px(8.0))
                .rounded(px(crate::ui::tokens::radius::md()))
                .bg(Tone::PrimarySoft.color())
                .items_center()
                .child(
                    label(
                        move || code.clone(),
                        ty::H1,
                        FontWeight::BOLD,
                        Tone::Primary,
                    )
                    .test_id("sync-invite-code"),
                )
                .child(label(
                    move || hint.clone(),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Body,
                ))
                .into_element()
        }
    }));
    if !s.error.is_empty() {
        let e = s.error.clone();
        v = v.child(view().px(px(20.0)).pb(px(14.0)).child(label(
            move || e.clone(),
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Danger,
        )));
    }
    v = v.child(dynamic(move || match devices.get() {
        None => view().into_element(),
        Some(list) => device_list(st, list, devices).into_element(),
    }));
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
                    Variant::Ghost,
                    Size::Md,
                    Some(Icon::Monitor),
                    st.t("sync.devices"),
                    move || {
                        if devices.get_untracked().is_some() {
                            devices.set(None);
                            return;
                        }
                        load_devices(st, devices);
                    },
                )
                .test_id("sync-devices"),
            )
            .child(
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Key),
                    st.t("sync.invite"),
                    move || {
                        let task = st.root.run(|| {
                            spawn(async move {
                                match crate::sync::invite(st).await {
                                    Ok(i) => invite.set(Some(i)),
                                    Err(f) => show_failure(st, &f),
                                }
                            })
                        });
                        drop(task); // runs to completion
                    },
                )
                .test_id("sync-invite"),
            )
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

/// What running other devices' operations here left to look at: the
/// product or sale, how much, and a button to mark it seen (on every copy).
/// Shown only when something waits.
pub fn reviews_card(st: AppState) -> View {
    let items = signal(Vec::<nb_store::usecases::reviews::ReviewItem>::new());
    let load = move || {
        let task = spawn(async move {
            if let Ok(v) = st
                .db()
                .call(|s| nb_store::usecases::reviews::review_items(s))
                .await
            {
                items.set(v);
            }
        });
        drop(task); // runs to completion
    };
    // Again whenever data changed (a pull, a sale here).
    effect(move || {
        let _ = st.data_rev.get();
        let _ = st.sync.get().reviews;
        untracked(load);
    });
    view().child(dynamic(move || {
        let list = items.get();
        if list.is_empty() {
            return view().into_element();
        }
        let mut rows = view();
        for (i, item) in list.into_iter().enumerate() {
            if i > 0 {
                rows = rows.child(divider());
            }
            rows = rows.child(review_row(st, item));
        }
        card()
            .test_id("reviews-card")
            .child(card_header_icon(
                Some(Icon::Alert),
                st.t("review.title"),
                Some(Box::new(st.t("review.help"))),
                view(),
            ))
            .child(divider())
            .child(rows)
            .into_element()
    }))
}

fn review_row(st: AppState, item: nb_store::usecases::reviews::ReviewItem) -> View {
    let (kind_key, qty_key) = match item.kind.as_str() {
        "stock_shortfall" => ("review.stock_shortfall", "review.stock_now"),
        "return_excess" => ("review.return_excess", "review.returned_beyond"),
        "drawer_short" => ("review.drawer_short", ""),
        _ => ("review.other", "review.qty"),
    };
    let qty = st
        .fmt()
        .quantity(nb_core::Quantity::from_milli(item.qty_milli), 3);
    let at = date_time(st, item.created_at);
    let subject = item.subject.clone();
    let key = nb_store::usecases::reviews::ReviewKey {
        kind: item.kind.clone(),
        entity_id: item.entity_id.clone(),
    };
    let busy = signal(false);
    let seen = move || {
        if busy.get_untracked() {
            return;
        }
        let Some(session) = st.session.get_untracked() else {
            return;
        };
        busy.set(true);
        let meta = session.meta("review.resolve");
        let key = key.clone();
        let task = st.root.run(|| {
            spawn(async move {
                match st
                    .db()
                    .call(move |s| nb_store::usecases::reviews::resolve_review(s, meta, key))
                    .await
                {
                    Ok(()) => {
                        st.changed();
                        crate::sync::refresh(st).await;
                    }
                    Err(e) => {
                        busy.set(false);
                        show_toast(st, e.message);
                    }
                }
            })
        });
        drop(task); // runs to completion
    };
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(12.0))
        .px(px(20.0))
        .py(px(12.0))
        .child(
            view()
                .flex_1()
                .min_w(px(180.0))
                .gap(px(2.0))
                .child(label(
                    move || subject.clone(),
                    ty::MD,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(6.0))
                        .child(badge(Status::Warning, None, st.t(kind_key)))
                        .child(label(
                            move || at.clone(),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                ),
        )
        // The number and its word apart (bidi); a drawer has none.
        .child(if qty_key.is_empty() {
            view()
        } else {
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(label(
                    st.t(qty_key),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .child(crate::ui::kit::figure(
                    move || qty.clone(),
                    ty::MD,
                    FontWeight::SEMIBOLD,
                    || Tone::Ink,
                ))
        })
        // Shown once the person is known (read inside, not at build).
        .child(when(
            move || st.can("audit.view"),
            move || {
                let seen = seen.clone();
                button(
                    Variant::Secondary,
                    Size::Sm,
                    Some(Icon::Check),
                    st.t("review.seen"),
                    seen,
                )
                .test_id("review-seen")
                .into_element()
            },
        ))
}

fn load_devices(st: AppState, devices: Signal<Option<Vec<nb_store::sync::DeviceInfo>>>) {
    let task = st.root.run(|| {
        spawn(async move {
            match crate::sync::devices(st).await {
                Ok(list) => devices.set(Some(list)),
                Err(f) => show_failure(st, &f),
            }
        })
    });
    drop(task); // runs to completion
}

/// The organization's devices: letter, name, last seen; a lost one is
/// stopped with two taps (the second confirms).
fn device_list(
    st: AppState,
    list: Vec<nb_store::sync::DeviceInfo>,
    devices: Signal<Option<Vec<nb_store::sync::DeviceInfo>>>,
) -> View {
    let mut rows = view()
        .mx(px(20.0))
        .mb(px(14.0))
        .rounded(px(crate::ui::tokens::radius::md()))
        .border_1()
        .border_color(Tone::Border.color());
    for (i, d) in list.into_iter().enumerate() {
        if i > 0 {
            rows = rows.child(divider());
        }
        let seen = d
            .last_seen_at
            .map(|t| date_time(st, t))
            .unwrap_or_else(|| st.tn("sync.never"));
        let name = d.name.clone();
        let letter = d.prefix.clone();
        let mut tags = view().flex_row().flex_wrap().gap(px(6.0));
        if d.this {
            tags = tags.child(badge(Status::Info, None, st.t("sync.device")));
        }
        if d.revoked_at.is_some() {
            tags = tags.child(badge(Status::Danger, None, st.t("sync.revoked")));
        }
        let mut row = view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(12.0))
            .px(px(14.0))
            .py(px(10.0))
            .child(
                view()
                    .w(px(32.0))
                    .h(px(32.0))
                    .rounded(px(crate::ui::tokens::radius::md()))
                    .bg(Tone::PrimarySoft.color())
                    .items_center()
                    .justify_center()
                    .child(label(
                        move || letter.clone(),
                        ty::MD,
                        FontWeight::BOLD,
                        Tone::Primary,
                    )),
            )
            .child(
                view()
                    .flex_1()
                    .min_w(px(160.0))
                    .gap(px(2.0))
                    .child(label(
                        move || name.clone(),
                        ty::MD,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    ))
                    .child(label(
                        move || seen.clone(),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )),
            )
            .child(tags);
        if !d.this && d.revoked_at.is_none() {
            let armed = signal(false);
            let id = d.id.clone();
            row = row.child(when(
                move || st.can("settings.edit"),
                move || {
                    let id = id.clone();
                    view()
                        .child(dynamic(move || {
                            let id = id.clone();
                            let sure = armed.get();
                            button(
                                if sure {
                                    Variant::Danger
                                } else {
                                    Variant::Ghost
                                },
                                Size::Sm,
                                Some(Icon::Unlink),
                                st.t(if sure {
                                    "sync.revoke_sure"
                                } else {
                                    "sync.revoke"
                                }),
                                move || {
                                    if !armed.get_untracked() {
                                        armed.set(true);
                                        return;
                                    }
                                    let id = id.clone();
                                    let task = st.root.run(|| {
                                        spawn(async move {
                                            match crate::sync::revoke(st, id).await {
                                                Ok(()) => load_devices(st, devices),
                                                Err(f) => show_failure(st, &f),
                                            }
                                        })
                                    });
                                    drop(task); // runs to completion
                                },
                            )
                            .test_id("sync-revoke")
                            .into_element()
                        }))
                        .into_element()
                },
            ));
        }
        rows = rows.child(row);
    }
    rows
}

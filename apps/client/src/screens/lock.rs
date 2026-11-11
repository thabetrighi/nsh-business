//! The lock screen (M4.7): who is working? People pick their name and type
//! their PIN (keypad or keyboard), or sign in with a login and password.
//! Shown on start once someone on the device has a PIN or password, after
//! the automatic lock, and when someone locks the screen by hand.

use nb_store::repo::access::UserRow;
use nb_store::usecases::access::{SignInOutcome, list_users, sign_in_password, sign_in_pin};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::screens::product_form::error_text;
use crate::state::AppState;
use crate::ui::form::{field, secret_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, avatar, button, label, label_line};
use crate::ui::surface::card;
use crate::ui::tokens::{Status, Tone, radius, ty};

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Pick,
    Pin(UserRow),
    Password,
}

/// What a refusal says, with the lock's end time when there is one.
fn refusal(st: AppState, code: &str, locked_until: Option<i64>) -> String {
    match locked_until {
        Some(t) => {
            let offset = 60; // the store's time zone (UTC+1, Algeria)
            let mins = (t / 60_000 + offset).rem_euclid(24 * 60);
            let time = st.fmt().time((mins / 60) as u32, (mins % 60) as u32);
            nb_i18n::tr_args(st.lang(), "lock.locked_until", &[("time", &time)])
        }
        None => error_text(st, code.trim_start_matches("error.")),
    }
}

pub fn lock_screen(st: AppState) -> View {
    let mode = signal(Mode::Pick);
    let users = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    Ok(list_users(s)?
                        .into_iter()
                        .filter(|u| u.active && u.has_pin)
                        .collect::<Vec<_>>())
                })
                .await
            }
        },
    );
    let body = dynamic(move || match mode.get() {
        Mode::Pick => pick(
            st,
            mode,
            users
                .with(|u: Option<&Vec<UserRow>>| u.cloned())
                .unwrap_or_default(),
        )
        .into_element(),
        Mode::Pin(u) => pin_pad(st, mode, u).into_element(),
        Mode::Password => password(st, mode).into_element(),
    });
    view()
        .test_id("lock-screen")
        .flex_1()
        .direction(move || st.direction())
        .bg(Tone::Canvas.color())
        .items_center()
        .justify_center()
        .p(px(16.0))
        .child(
            card()
                .w(relative(1.0))
                .max_w(px(420.0))
                .p(px(28.0))
                .gap(px(20.0))
                .child(
                    view()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            view()
                                .w(px(48.0))
                                .h(px(48.0))
                                .rounded(px(radius::lg()))
                                .bg(Tone::PrimarySoft.color())
                                .items_center()
                                .justify_center()
                                .child(icon(Icon::Shield, 24.0, || Tone::Primary)),
                        )
                        .child(label(
                            st.t("lock.title"),
                            ty::XL,
                            FontWeight::BOLD,
                            Tone::Ink,
                        )),
                )
                .child(body),
        )
}

fn pick(st: AppState, mode: Signal<Mode>, users: Vec<UserRow>) -> View {
    let mut list = view().gap(px(8.0));
    if users.is_empty() {
        list = list.child(
            label(
                st.t("lock.no_pin_users"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .text_center(),
        );
    } else {
        list = list.child(
            label(
                st.t("lock.subtitle"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .text_center(),
        );
    }
    for (i, u) in users.into_iter().enumerate() {
        let name = u.name.clone();
        let shown = u.name.clone();
        list = list.child(
            view()
                .test_id(&format!("lock-user-{i}"))
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .p(px(10.0))
                .rounded(px(radius::md()))
                .border_1()
                .border_color(Tone::Border.color())
                .cursor_pointer()
                .hover(|s| s.bg(Tone::Hover.color()))
                .on_press(move || mode.set(Mode::Pin(u.clone())))
                .child(avatar(move || name.clone(), 36.0, Status::Accent))
                .child(view().flex_1().min_w(px(0.0)).child(label_line(
                    move || shown.clone(),
                    ty::MD,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                )))
                .child(icon(
                    if st.rtl() {
                        Icon::ChevronLeft
                    } else {
                        Icon::ChevronRight
                    },
                    16.0,
                    || Tone::Faint,
                )),
        );
    }
    view().gap(px(16.0)).child(list).child(
        button(
            Variant::Ghost,
            Size::Md,
            Some(Icon::Key),
            st.t("lock.with_password"),
            move || mode.set(Mode::Password),
        )
        .test_id("lock-password-mode"),
    )
}

fn pin_pad(st: AppState, mode: Signal<Mode>, user: UserRow) -> View {
    let pin = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let uid: Option<nb_domain::UserId> = user.id.parse().ok();
    let submit = move || {
        let typed = pin.get_untracked();
        if busy.get_untracked() || typed.len() < 4 {
            return;
        }
        let Some(uid) = uid else { return };
        busy.set(true);
        let db = st.db();
        let task = spawn(async move {
            let r = db.call(move |s| sign_in_pin(s, uid, &typed)).await;
            busy.set(false);
            pin.set(String::new());
            match r {
                Ok(SignInOutcome::SignedIn(me)) => st.signed_in(me),
                Ok(SignInOutcome::Refused { code, locked_until }) => {
                    error.set(refusal(st, &code, locked_until));
                }
                Err(e) => error.set(error_text(st, e.codes.first().map_or("internal", |c| c.1))),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let press = move |d: char| {
        error.set(String::new());
        pin.update(|p| {
            if p.len() < 6 {
                p.push(d);
            }
        });
    };
    let back = move || {
        pin.update(|p| {
            p.pop();
        })
    };
    // Keyboards type the PIN too (desktop), while this pad shows.
    for d in '0'..='9' {
        on_key(&d.to_string(), move |_| press(d));
    }
    on_key("backspace", move |_| back());
    on_key("enter", move |_| submit());
    on_key("escape", move |_| mode.set(Mode::Pick));

    let dots = {
        let mut row = view().flex_row().justify_center().gap(px(12.0)).h(px(16.0));
        for i in 0..6 {
            row = row.child(
                view()
                    .w(px(12.0))
                    .h(px(12.0))
                    .rounded_full()
                    .border_1()
                    .border_color(Tone::BorderStrong.color())
                    .bg(move || {
                        if pin.get().len() > i {
                            Tone::Primary.color()
                        } else {
                            Tone::Surface.color()
                        }
                    })
                    .opacity(move || {
                        if i < 4 || pin.get().len() >= i {
                            1.0_f32
                        } else {
                            0.35
                        }
                    }),
            );
        }
        row
    };
    let key = move |content: View, test: String, on: Box<dyn Fn()>| {
        view()
            .test_id(&test)
            .flex_1()
            .h(px(56.0))
            .rounded(px(radius::md()))
            .bg(Tone::Subtle.color())
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Hover.color()))
            .active(|s| s.bg(Tone::BorderStrong.color()))
            .on_press(on)
            .child(content)
    };
    let digit = move |d: char| {
        key(
            view().child(label(
                move || d.to_string(),
                ty::XL,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            )),
            format!("pin-{d}"),
            Box::new(move || press(d)),
        )
    };
    let mut pad = view().gap(px(8.0));
    for row in [['1', '2', '3'], ['4', '5', '6'], ['7', '8', '9']] {
        let mut r = view().flex_row().gap(px(8.0));
        for d in row {
            r = r.child(digit(d));
        }
        pad = pad.child(r);
    }
    pad = pad.child(
        view()
            .flex_row()
            .gap(px(8.0))
            .child(key(
                view().child(icon(Icon::Backspace, 20.0, || Tone::Muted)),
                "pin-back".into(),
                Box::new(back),
            ))
            .child(digit('0'))
            .child(
                key(
                    view().child(icon(Icon::Check, 22.0, || Tone::OnPrimary)),
                    "pin-ok".into(),
                    Box::new(submit),
                )
                .bg(Tone::Primary.color())
                .hover(|s| s.opacity(0.9)),
            ),
    );
    let name = user.name.clone();
    view()
        .gap(px(18.0))
        .child(
            view()
                .items_center()
                .gap(px(8.0))
                .child(avatar(move || name.clone(), 44.0, Status::Accent))
                .child(label(
                    {
                        let n = user.name.clone();
                        move || nb_i18n::tr_args(st.lang(), "lock.pin_for", &[("name", &n)])
                    },
                    ty::MD,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                )),
        )
        .child(dots)
        .child(when(
            move || !error.get().is_empty(),
            move || {
                label(
                    move || error.get(),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                )
                .text_center()
                .test_id("lock-error")
                .into_element()
            },
        ))
        .child(pad)
        .child(button(
            Variant::Ghost,
            Size::Md,
            Some(if st.rtl() {
                Icon::ChevronRight
            } else {
                Icon::ChevronLeft
            }),
            st.t("lock.back"),
            move || mode.set(Mode::Pick),
        ))
}

fn password(st: AppState, mode: Signal<Mode>) -> View {
    let login = signal(String::new());
    let pw = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let submit = move || {
        if busy.get_untracked() {
            return;
        }
        let (l, p) = (login.get_untracked(), pw.get_untracked());
        busy.set(true);
        let db = st.db();
        let task = spawn(async move {
            let r = db.call(move |s| sign_in_password(s, &l, &p)).await;
            busy.set(false);
            match r {
                Ok(SignInOutcome::SignedIn(me)) => st.signed_in(me),
                Ok(SignInOutcome::Refused { code, locked_until }) => {
                    pw.set(String::new());
                    error.set(refusal(st, &code, locked_until));
                }
                Err(e) => error.set(error_text(st, e.codes.first().map_or("internal", |c| c.1))),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    on_key("enter", move |_| submit());
    view()
        .gap(px(16.0))
        .child(field(
            st.t("lock.login"),
            login,
            String::new,
            String::new,
            KeyboardType::Default,
            "lock-login",
        ))
        .child(secret_field(
            st.t("lock.password"),
            pw,
            move || error.get(),
            "lock-pw",
        ))
        .child(
            button(
                Variant::Primary,
                Size::Lg,
                Some(Icon::Check),
                st.t("lock.sign_in"),
                submit,
            )
            .test_id("lock-submit"),
        )
        .child(button(
            Variant::Ghost,
            Size::Md,
            Some(Icon::Users),
            st.t("lock.with_pin"),
            move || mode.set(Mode::Pick),
        ))
}

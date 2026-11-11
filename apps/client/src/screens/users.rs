//! Users and roles (M4.9), in the settings: the team with each person's
//! roles, scope and sign-in state; a form to add or edit someone (name,
//! login, password, PIN, role and where it applies); the roles with what
//! they hold. Everything goes through the access use cases, which check
//! the rules (no escalation, an owner always remains…).

use nb_i18n::tr_args;
use nb_store::repo::access::{GrantRow, RoleRow, Secret, UserRow};
use nb_store::usecases::access::{
    UserInput, UserUpdate, create_user, list_roles, list_users, session_of, set_secret,
    sign_in_required, update_user,
};
use nb_store::usecases::org::{BranchTree, places};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::screens::forms::error_text;
use crate::state::{AppState, now_ms};
use crate::ui::form::{chips, field, secret_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, avatar, badge, button, label, label_line, switch};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::surface::{card, card_header_icon};
use crate::ui::tokens::{Status, Tone, radius, ty};

/// Everything the tab shows, loaded together.
#[derive(Debug, Clone, Default, PartialEq)]
struct Team {
    users: Vec<UserRow>,
    roles: Vec<RoleRow>,
    places: Vec<BranchTree>,
}

/// A role's display name.
fn role_name(st: AppState, roles: &[RoleRow], id: &str) -> String {
    match roles.iter().find(|r| r.id == id) {
        Some(r) if !r.builtin => r.name.clone(),
        _ => st.tn(&format!("role.{id}")),
    }
}

/// Where a grant applies, in words.
fn scope_name(st: AppState, places: &[BranchTree], g: &GrantRow) -> String {
    let id = g.scope_id.as_deref().unwrap_or("");
    match g.scope_kind.as_str() {
        "branch" => places
            .iter()
            .find(|p| p.branch.id == id)
            .map(|p| tr_args(st.lang(), "users.scope.branch", &[("name", &p.branch.name)]))
            .unwrap_or_default(),
        "register" => places
            .iter()
            .flat_map(|p| p.registers.iter().map(move |r| (p, r)))
            .find(|(_, r)| r.id == id)
            .map(|(p, r)| {
                let name = format!("{} · {}", p.branch.name, r.name);
                tr_args(st.lang(), "users.scope.register", &[("name", &name)])
            })
            .unwrap_or_default(),
        _ => st.tn("users.scope.org"),
    }
}

/// Scope choices: the whole business, each branch, each register.
fn scope_choices(st: AppState, places: &[BranchTree]) -> Vec<(GrantRow, String)> {
    let mut out = vec![(GrantRow::org(""), st.tn("users.scope.org"))];
    for p in places {
        let g = GrantRow {
            role: String::new(),
            scope_kind: "branch".into(),
            scope_id: Some(p.branch.id.clone()),
        };
        out.push((g.clone(), scope_name(st, places, &g)));
        for r in &p.registers {
            let g = GrantRow {
                role: String::new(),
                scope_kind: "register".into(),
                scope_id: Some(r.id.clone()),
            };
            out.push((g.clone(), scope_name(st, places, &g)));
        }
    }
    out
}

pub fn users_tab(st: AppState, compact: bool) -> View {
    let team = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    Ok(Team {
                        users: list_users(s)?,
                        roles: list_roles(s)?,
                        places: places(s)?,
                    })
                })
                .await
            }
        },
    );
    let data = computed(move || team.with(|t: Option<&Team>| t.cloned()).unwrap_or_default());
    // The form: closed, a new user (None), or someone to edit.
    let editing: Signal<Option<Option<UserRow>>> = signal(None);
    let open = signal(false);
    effect(move || open.set(editing.get().is_some()));
    effect(move || {
        if !open.get() {
            editing.set(None);
        }
    });

    let list = dynamic(move || {
        let t = data.get();
        let mut c = card().child(card_header_icon(
            Some(Icon::Users),
            st.t("settings.card.team"),
            None,
            view().child(when(
                move || st.can("users.manage"),
                move || {
                    button(
                        Variant::Primary,
                        Size::Sm,
                        Some(Icon::UserPlus),
                        st.t("users.add"),
                        move || editing.set(Some(None)),
                    )
                    .test_id("user-add")
                    .into_element()
                },
            )),
        ));
        for (i, u) in t.users.iter().enumerate() {
            c = c.child(user_row(st, &t, u.clone(), i, compact, editing));
        }
        c.into_element()
    });
    let roles = dynamic(move || roles_card(st, &data.get()).into_element());
    view()
        .gap(px(20.0))
        .child(list)
        .child(roles)
        .child(modal(open, compact, move || {
            let Some(who) = editing.get_untracked() else {
                return view().into_element();
            };
            user_form(st, data.get_untracked(), who, move || editing.set(None)).into_element()
        }))
}

fn user_row(
    st: AppState,
    t: &Team,
    u: UserRow,
    i: usize,
    compact: bool,
    editing: Signal<Option<Option<UserRow>>>,
) -> View {
    let status = if !u.active {
        Status::Neutral
    } else if u.grants.iter().any(|g| g.role == "owner") {
        Status::Accent
    } else {
        Status::Info
    };
    let name = u.name.clone();
    let line2 = {
        let mut parts = Vec::new();
        if let Some(l) = &u.login {
            parts.push(nb_i18n::bidi::isolate_ltr(l));
        }
        parts.extend(u.grants.iter().map(|g| scope_name(st, &t.places, g)));
        parts.join(" · ")
    };
    let roles: Vec<String> = u
        .grants
        .iter()
        .map(|g| role_name(st, &t.roles, &g.role))
        .collect();
    let locked = u.locked_until.is_some_and(|until| until > now_ms());
    let mut r = view()
        .test_id(&format!("user-{i}"))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(20.0))
        .py(px(12.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(avatar(move || name.clone(), 34.0, status))
        .child({
            let n = u.name.clone();
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(label_line(
                    move || n.clone(),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    move || line2.clone(),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
        });
    for role in roles {
        r = r.child(badge(status, None, move || role.clone()));
    }
    if !compact {
        let (s, which, key) = if !u.active {
            (Status::Neutral, Icon::XCircle, "users.inactive")
        } else if locked {
            (Status::Danger, Icon::Lock, "users.locked")
        } else if u.has_pin {
            (Status::Success, Icon::Key, "users.pin_set")
        } else if u.has_password {
            (Status::Success, Icon::Key, "users.password_set")
        } else {
            (Status::Warning, Icon::Info, "users.no_secret")
        };
        r = r.child(badge(s, Some(which), st.t(key)));
    }
    if st.can("users.manage") {
        r = r
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Hover.color()))
            .on_press(move || editing.set(Some(Some(u.clone()))));
    }
    r
}

fn roles_card(st: AppState, t: &Team) -> View {
    let mut c = card().child(card_header_icon(
        Some(Icon::Shield),
        st.t("users.roles"),
        Some(Box::new(st.t("users.roles.help"))),
        view(),
    ));
    for r in &t.roles {
        let name = role_name(st, &t.roles, &r.id);
        let (users, perms) = (r.users, r.permissions.len());
        let kind = if r.builtin {
            "users.builtin"
        } else {
            "users.custom"
        };
        c = c.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .px(px(20.0))
                .py(px(10.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(icon(Icon::Shield, 16.0, move || {
                    if users > 0 {
                        Tone::Primary
                    } else {
                        Tone::Faint
                    }
                }))
                .child(view().flex_1().min_w(px(0.0)).child(label_line(
                    move || name.clone(),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Ink,
                )))
                .child(label(
                    move || {
                        let f = st.fmt();
                        format!(
                            "{} · {}",
                            tr_args(
                                st.lang(),
                                "users.role_perms",
                                &[("count", &f.integer(perms as i64))]
                            ),
                            tr_args(
                                st.lang(),
                                "users.role_users",
                                &[("count", &f.integer(i64::from(users)))]
                            )
                        )
                    },
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .child(badge(Status::Neutral, None, st.t(kind))),
        );
    }
    c
}

/// Add (`who` = None) or edit someone.
fn user_form(
    st: AppState,
    t: Team,
    who: Option<UserRow>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let first = who.as_ref().and_then(|u| u.grants.first().cloned());
    let name = signal(who.as_ref().map(|u| u.name.clone()).unwrap_or_default());
    let phone = signal(
        who.as_ref()
            .and_then(|u| u.phone.clone())
            .unwrap_or_default(),
    );
    let login = signal(
        who.as_ref()
            .and_then(|u| u.login.clone())
            .unwrap_or_default(),
    );
    let password = signal(String::new());
    let pin = signal(String::new());
    let active = signal(who.as_ref().is_none_or(|u| u.active));
    let roles = t.roles.clone();
    let role_idx = signal(
        first
            .as_ref()
            .and_then(|g| roles.iter().position(|r| r.id == g.role))
            .unwrap_or_else(|| roles.iter().position(|r| r.id == "cashier").unwrap_or(0)),
    );
    let scopes = scope_choices(st, &t.places);
    let scope_idx = signal(
        first
            .as_ref()
            .and_then(|g| {
                scopes
                    .iter()
                    .position(|(s, _)| s.scope_kind == g.scope_kind && s.scope_id == g.scope_id)
            })
            .unwrap_or(0),
    );
    let errors: [Signal<String>; 6] = std::array::from_fn(|_| signal(String::new()));
    let [name_err, login_err, pw_err, pin_err, role_err, form_err] = errors;
    let saving = signal(false);
    let editing = who.is_some();
    let id = who.as_ref().map(|u| u.id.clone());
    // One operation per opening: a double tap saves once.
    let meta = session.meta(if editing {
        "user.update"
    } else {
        "user.create"
    });

    let role_options = roles
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let n = role_name(st, &roles, &r.id);
            (i, Box::new(move || n.clone()) as Box<dyn Fn() -> String>)
        })
        .collect();
    let scope_options = scopes
        .iter()
        .enumerate()
        .map(|(i, (_, n))| {
            let n = n.clone();
            (i, Box::new(move || n.clone()) as Box<dyn Fn() -> String>)
        })
        .collect();

    let save = {
        let close = close.clone();
        let roles = roles.clone();
        let scopes = scopes.clone();
        let id = id.clone();
        move || {
            if saving.get_untracked() {
                return;
            }
            for e in errors {
                e.set(String::new());
            }
            let (scope, _) = scopes[scope_idx.get_untracked().min(scopes.len() - 1)].clone();
            let grant = GrantRow {
                role: roles
                    .get(role_idx.get_untracked())
                    .map(|r| r.id.clone())
                    .unwrap_or_default(),
                ..scope
            };
            let text = |s: Signal<String>| {
                Some(s.get_untracked().trim().to_owned()).filter(|v| !v.is_empty())
            };
            let (pw, code) = (text(password), text(pin));
            let id = id.clone();
            let input_name = name.get_untracked();
            let input_phone = text(phone);
            let input_login = text(login);
            let is_active = active.get_untracked();
            saving.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                let me: Option<nb_domain::UserId> =
                    st.user.get_untracked().and_then(|u| u.user_id.parse().ok());
                let r = db
                    .call(move |s| {
                        let row = match id {
                            None => create_user(
                                s,
                                meta,
                                UserInput {
                                    name: input_name,
                                    phone: input_phone,
                                    login: input_login,
                                    password: pw,
                                    pin: code,
                                    grants: vec![grant],
                                },
                            )?,
                            Some(id) => {
                                let row = update_user(
                                    s,
                                    meta,
                                    UserUpdate {
                                        id,
                                        name: input_name,
                                        phone: input_phone,
                                        login: input_login,
                                        active: is_active,
                                        grants: vec![grant],
                                    },
                                )?;
                                let uid = nb_store::repo::parse_id(&row.id)?;
                                // Typed secrets replace the old ones; empty keeps them.
                                if pw.is_some() {
                                    let m = nb_store::OpMeta {
                                        id: nb_core::OperationId::new(),
                                        kind: "user.secret",
                                        ..meta
                                    };
                                    set_secret(s, m, uid, Secret::Password, pw)?;
                                }
                                if code.is_some() {
                                    let m = nb_store::OpMeta {
                                        id: nb_core::OperationId::new(),
                                        kind: "user.secret",
                                        ..meta
                                    };
                                    set_secret(s, m, uid, Secret::Pin, code)?;
                                }
                                row
                            }
                        };
                        // Read in the same call: the form's tasks end when it closes.
                        let needs = sign_in_required(s)?;
                        let me = me.map(|u| session_of(s, u)).transpose()?;
                        Ok((row, needs, me))
                    })
                    .await;
                saving.set(false);
                match r {
                    Ok((row, needs, me)) => {
                        st.needs_sign_in.set(needs);
                        if let Some(me) = me {
                            st.user.set(Some(me));
                        }
                        st.changed();
                        close();
                        show_toast(st, format!("{} · {}", st.tn("users.saved"), row.name));
                    }
                    Err(e) => {
                        for (field, code) in &e.codes {
                            let target = match field.unwrap_or("") {
                                "name" => name_err,
                                "login" => login_err,
                                "password" => pw_err,
                                "pin" => pin_err,
                                "roles" | "scope" => role_err,
                                _ => form_err,
                            };
                            if target.get_untracked().is_empty() {
                                target.set(error_text(st, code));
                            }
                        }
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let err = |s: Signal<String>| move || s.get();
    let pair = |a: View, b: View| {
        view()
            .flex_row()
            .gap(px(12.0))
            .child(view().flex_1().child(a))
            .child(view().flex_1().child(b))
    };
    let mut body = view()
        .gap(px(16.0))
        .child(when(
            move || !form_err.get().is_empty(),
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::DangerSoft.color())
                    .child(icon(Icon::Info, 16.0, || Tone::Danger))
                    .child(label(
                        move || form_err.get(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Danger,
                    ))
                    .into_element()
            },
        ))
        .child(pair(
            field(
                st.t("users.name"),
                name,
                String::new,
                err(name_err),
                KeyboardType::Default,
                "user-name",
            ),
            field(
                st.t("users.phone"),
                phone,
                String::new,
                String::new,
                KeyboardType::Phone,
                "user-phone",
            ),
        ))
        .child(pair(
            field(
                st.t("users.login"),
                login,
                String::new,
                err(login_err),
                KeyboardType::Default,
                "user-login",
            ),
            secret_field(
                st.t("users.password"),
                password,
                err(pw_err),
                "user-password",
            ),
        ))
        .child(field(
            st.t("users.pin"),
            pin,
            || "••••".to_owned(),
            err(pin_err),
            KeyboardType::Number,
            "user-pin",
        ))
        .child(chips(st.t("users.role"), role_options, role_idx))
        .child(chips(st.t("users.scope"), scope_options, scope_idx))
        .child(when(
            move || !role_err.get().is_empty(),
            move || {
                label(
                    move || role_err.get(),
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                )
                .into_element()
            },
        ));
    if editing {
        body = body.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(switch(active, st.t("users.active")))
                .child(label(
                    st.t("users.active"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Ink,
                )),
        );
    }
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
                Variant::Primary,
                Size::Md,
                Some(Icon::Check),
                st.t("form.save"),
                save,
            )
            .test_id("user-save"),
        );
    dialog_frame(
        st,
        st.t(if editing { "users.edit" } else { "users.add" }),
        st.t("users.form.description"),
        body,
        footer,
        close,
    )
}

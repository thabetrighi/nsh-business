//! A supervisor's approval on the spot (M4.8): when an operation needs a
//! right its user lacks but may borrow (a large discount, credit past the
//! limit, a return…), a supervisor present enters their PIN; the approval
//! is good once, for that action, for a few minutes, and is audited. The
//! operation is then sent again with it.

use nb_domain::access::{Permission, Target};
use nb_i18n::tr_args;
use nb_store::repo::access::UserRow;
use nb_store::usecases::access::{ApprovalOutcome, ApproveInput, approve, supervisors};
use nsh_core::element::{AnyElement, KeyboardType, View};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::form::{field, labelled, secret_field};
use crate::ui::icon::Icon;
use crate::ui::kit::{Size, Variant, button, label};
use crate::ui::overlay::dialog_frame;
use crate::ui::popup::dropdown;
use crate::ui::tokens::{Tone, ty};

/// The right an error asks a supervisor for, if it is one that may be
/// approved (`access.needs_approval`).
pub fn needs_approval(e: &DbError) -> Option<&'static str> {
    e.codes
        .iter()
        .find(|(_, c)| *c == "access.needs_approval")
        .and_then(|(p, _)| *p)
}

/// What the approval is for, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub permission: &'static str,
    pub branch_id: Option<String>,
    pub register_id: Option<String>,
}

fn what(st: AppState, permission: &str) -> String {
    let key = format!("approval.what.{permission}");
    let t = st.tn(&key);
    if t == key {
        st.tn("approval.what.other")
    } else {
        t
    }
}

/// Asks a supervisor; `on_ok` gets the approval's id.
pub fn approval_dialog(
    st: AppState,
    ask: Ask,
    on_ok: impl Fn(String) + Clone + 'static,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let target = Target {
        branch: ask.branch_id.as_deref().and_then(|b| b.parse().ok()),
        warehouse: None,
        register: ask.register_id.as_deref().and_then(|r| r.parse().ok()),
    };
    let permission = ask.permission;
    let found = resource(
        move || Some(()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    let p = Permission::from_code(permission)
                        .ok_or(nb_store::Error::NotFound("permission"))?;
                    supervisors(s, p, target)
                })
                .await
            }
        },
    );
    let me = session.actor.to_string();
    let people = computed(move || {
        found
            .with(|v: Option<&Vec<UserRow>>| v.cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|u| u.id != me)
            .collect::<Vec<_>>()
    });
    let chosen = signal(0usize);
    let pin = signal(String::new());
    let reason = signal(what(st, permission));
    let error = signal(String::new());
    let busy = signal(false);
    let go = {
        let (close, on_ok, ask) = (close.clone(), on_ok.clone(), ask.clone());
        move || {
            if busy.get_untracked() {
                return;
            }
            let Some(sup) = people.with_untracked(|p| p.get(chosen.get_untracked()).cloned())
            else {
                error.set(st.tn("approval.no_supervisor"));
                return;
            };
            let input = ApproveInput {
                supervisor_id: sup.id,
                pin: pin.get_untracked(),
                permission: permission.to_owned(),
                branch_id: ask.branch_id.clone(),
                warehouse_id: None,
                register_id: ask.register_id.clone(),
                reason: reason.get_untracked(),
            };
            // Each attempt is its own operation (a wrong PIN, then the right one).
            let meta = session.meta("access.approve");
            busy.set(true);
            error.set(String::new());
            let (close, on_ok) = (close.clone(), on_ok.clone());
            let db = st.db();
            let task = spawn(async move {
                let r = db.call(move |s| approve(s, meta, input)).await;
                busy.set(false);
                match r {
                    Ok(ApprovalOutcome::Approved { id, .. }) => {
                        close();
                        on_ok(id);
                    }
                    Ok(ApprovalOutcome::Refused { code }) => {
                        pin.set(String::new());
                        error.set(error_text(st, &code));
                    }
                    Err(e) => {
                        error.set(error_text(st, e.codes.first().map_or("internal", |c| c.1)))
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    on_key("enter", {
        let go = go.clone();
        move |_| go()
    });
    let body = view()
        .gap(px(14.0))
        .child(label(
            move || {
                tr_args(
                    st.lang(),
                    "approval.asks",
                    &[("what", &what(st, permission))],
                )
            },
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Body,
        ))
        .child(dynamic(move || {
            if people.with(Vec::is_empty) {
                return label(
                    st.t("approval.no_supervisor"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                )
                .into_element();
            }
            labelled(
                st.t("approval.supervisor"),
                dropdown(
                    move || {
                        people
                            .get()
                            .into_iter()
                            .enumerate()
                            .map(|(i, u)| (i, u.name))
                            .collect()
                    },
                    chosen,
                    Some(Icon::User),
                    st.t("approval.supervisor"),
                    "approval-who",
                ),
                None,
            )
            .into_element()
        }))
        .child(secret_field(
            st.t("approval.pin"),
            pin,
            String::new,
            "approval-pin",
        ))
        .child(field(
            st.t("approval.reason"),
            reason,
            String::new,
            String::new,
            KeyboardType::Default,
            "approval-reason",
        ))
        .child(error_line(error));
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
                st.t("approval.confirm"),
                go,
            )
            .test_id("approval-confirm"),
        );
    dialog_frame(
        st,
        st.t("approval.title"),
        st.t("approval.help"),
        body,
        footer,
        close,
    )
}

fn error_line(error: Signal<String>) -> AnyElement {
    when(
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
    )
    .into_element()
}

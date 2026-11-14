//! The first run (ADR-0009), as the well-known shop apps do it: an account
//! first, and everything else follows from it.
//!
//! - **Create an account** (name, phone, password) → its recovery codes,
//!   kept once → the shop (name, activity, town) → a 4-digit PIN for this
//!   device → the shop opens, already saved on the server.
//! - **Sign in** (phone, password, or a recovery code) → the account's
//!   shops (none: make one) → this device joins the one picked (a reinstall
//!   takes back its old place) → the person's PIN if they have none → the
//!   shop opens, as its owner or, for an employee's account, as that
//!   employee.
//! - **Other options**: the demo shop, a code from another device, this
//!   device alone without an account, a shop's own server.
//!
//! No server address, link or sync to understand: the account holds the
//! shops; the device holds a token of its own, never the password.

use nb_domain::features::Profiles;
use nb_i18n::Lang;
use nb_store::repo::access::Secret;
use nb_store::sync::{DeviceInfo, OrgInfo};
use nb_store::usecases::access::{list_users, set_secret};
use nb_store::usecases::setup::SetupInput;
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::sync::Failed;
use crate::ui::form::{field, labelled, password_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, button, checkbox, divider, figure, label, label_line};
use crate::ui::popup::dropdown;
use crate::ui::surface::card;
use crate::ui::tokens::{Tone, radius, ty};

/// Which user an employee's own phone is for (its lock screen offers that
/// person only), kept per device identity: another shop's copy on the
/// same computer, or a new one after a reinstall, starts without it.
pub fn personal_user(device: &str) -> String {
    crate::state::stored_text(&format!("nb.personal_user.{device}"))
}

fn set_personal_user(device: &str, user: &str) {
    crate::state::store_text(&format!("nb.personal_user.{device}"), user);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Welcome,
    More,
    SignUp,
    Codes,
    SignIn,
    Recover,
    Shops,
    Device,
    Shop,
    Pin,
    Code,
    Working,
}

/// The account signed in or just made.
#[derive(Clone, Debug, Default, PartialEq)]
struct Account {
    token: String,
    name: String,
}

/// What the PIN step finishes.
#[derive(Clone, Debug, PartialEq)]
enum PinFor {
    /// The new shop's owner (the shop is made with it).
    NewShop,
    /// A person of a shop just joined, who has no PIN yet (`""`: owner).
    Joined(String),
}

pub fn setup_wizard(st: AppState) -> View {
    let step = signal(Step::Welcome);
    let error = signal(String::new());
    let busy = signal(false);
    let working = signal(String::new());
    let s = |v: &str| signal(v.to_owned());

    // Where accounts live, and a private server's key (other options).
    let server = signal(crate::sync::default_server());
    let server_key = s("");
    // The account.
    let (name, phone, password) = (s(""), s(""), s(""));
    let (recovery, new_password) = (s(""), s(""));
    let account = signal(None::<Account>);
    let codes = signal(Vec::<String>::new());
    let codes_kept = signal(false);
    let shops = signal(Vec::<OrgInfo>::new());
    let chosen = signal(None::<String>);
    let olds = signal(Vec::<DeviceInfo>::new());
    // The new shop.
    let activities: Vec<String> = Profiles::builtin()
        .activities()
        .map(str::to_owned)
        .collect();
    let grocery = activities.iter().position(|a| a == "grocery").unwrap_or(0);
    let (shop_name, town, owner) = (s(""), s(""), s(""));
    let activity = signal(grocery);
    // This device alone, without an account.
    let alone = signal(false);
    // The shop is made here already (its server copy may still be missing).
    let made = signal(false);
    // The PIN.
    let (pin, pin_again) = (s(""), s(""));
    let pin_for = signal(PinFor::NewShop);
    // A code from another device.
    let invite = s("");

    let go_to = move |to: Step| {
        error.set(String::new());
        step.set(to);
    };
    let fail = move |back: Step, f: &Failed| {
        busy.set(false);
        error.set(crate::screens::cloud::failure_text(st, f));
        step.set(back);
    };
    let start = move |message: &str| {
        if busy.get_untracked() {
            return false;
        }
        busy.set(true);
        error.set(String::new());
        working.set(st.tn(message));
        true
    };
    // The shop opens, as `who` (`""`: the owner; `None`: whoever the lock
    // screen asks for).
    let enter = move |who: Option<String>| {
        busy.set(false);
        if let Some(who) = who {
            st.enter_as.replace(Some(who));
        }
        st.needs_setup.set(false);
        st.connect();
    };

    // ---------------------------------------------------------- actions

    // A device of a shop the account belongs to.
    let join = move |org: String, replaces: Option<String>| {
        if !start("onboard.working.join") {
            return;
        }
        step.set(Step::Working);
        let (url, token) = (
            server.get_untracked(),
            account.get_untracked().map(|a| a.token).unwrap_or_default(),
        );
        let task = st.root.run(|| {
            spawn(async move {
                let joined = match crate::sync::join_mine(st, url, token, org, replaces).await {
                    Ok(j) => j,
                    Err(f) => return fail(Step::Shops, &f),
                };
                // An employee's own phone shows that person only.
                let who = joined.user_id.clone().unwrap_or_default();
                set_personal_user(&joined.device_id, &who);
                let needs_pin = st
                    .db()
                    .call({
                        let who = who.clone();
                        move |s| {
                            let users = list_users(s)?;
                            let me = if who.is_empty() {
                                let owner = crate::db::owner_of(s)?;
                                users.into_iter().find(|u| u.id == owner)
                            } else {
                                users.into_iter().find(|u| u.id == who)
                            };
                            Ok(me.is_some_and(|u| !u.has_pin))
                        }
                    })
                    .await
                    .unwrap_or(false);
                if needs_pin {
                    busy.set(false);
                    pin.set(String::new());
                    pin_again.set(String::new());
                    pin_for.set(PinFor::Joined(who));
                    step.set(Step::Pin);
                } else {
                    enter(Some(who));
                }
            })
        });
        drop(task); // runs to completion under the root
    };
    // A shop picked: a reinstall may take back the place of one of its
    // devices (the account's own, for an employee).
    let pick = move |org: String| {
        if !start("onboard.working.wait") {
            return;
        }
        step.set(Step::Working);
        let (url, token) = (
            server.get_untracked(),
            account.get_untracked().map(|a| a.token).unwrap_or_default(),
        );
        let task = st.root.run(|| {
            spawn(async move {
                match crate::sync::shop_devices(&url, &token, &org).await {
                    Ok(list) if list.iter().all(|d| d.revoked_at.is_some()) => {
                        busy.set(false);
                        join(org, None);
                    }
                    Ok(list) => {
                        busy.set(false);
                        olds.set(
                            list.into_iter()
                                .filter(|d| d.revoked_at.is_none())
                                .collect(),
                        );
                        chosen.set(Some(org));
                        step.set(Step::Device);
                    }
                    Err(f) => fail(Step::Shops, &f),
                }
            })
        });
        drop(task); // runs to completion under the root
    };
    // Signed in (or recovered): to the account's shops.
    let signed_in = move |session: nb_store::sync::Session| {
        let token = session.token.clone();
        crate::sync::set_setting_up(&session.name);
        account.set(Some(Account {
            token: session.token,
            name: session.name,
        }));
        password.set(String::new());
        new_password.set(String::new());
        recovery.set(String::new());
        let url = server.get_untracked();
        let task = st.root.run(|| {
            spawn(async move {
                match crate::sync::my_orgs(&url, &token).await {
                    Ok(list) if list.is_empty() => {
                        busy.set(false);
                        error.set(st.tn("onboard.no_shops"));
                        step.set(Step::Shop);
                    }
                    Ok(list) if list.len() == 1 => {
                        busy.set(false);
                        let org = list[0].org_id.clone();
                        shops.set(list);
                        pick(org);
                    }
                    Ok(list) => {
                        busy.set(false);
                        shops.set(list);
                        step.set(Step::Shops);
                    }
                    Err(f) => fail(Step::SignIn, &f),
                }
            })
        });
        drop(task); // runs to completion under the root
    };
    let sign_up = move || {
        let missing = name.get_untracked().trim().is_empty()
            || phone.get_untracked().trim().is_empty()
            || password.get_untracked().is_empty();
        if missing {
            error.set(st.tn("onboard.err.required"));
            return;
        }
        if password.get_untracked().chars().count() < 8 {
            error.set(st.tn("account.err.short"));
            return;
        }
        if !start("onboard.working.wait") {
            return;
        }
        step.set(Step::Working);
        let (url, n, p, w, k) = (
            server.get_untracked(),
            name.get_untracked(),
            phone.get_untracked(),
            password.get_untracked(),
            server_key.get_untracked(),
        );
        let task = st.root.run(|| {
            spawn(async move {
                match crate::sync::sign_up(&url, &n, &p, &w, &k).await {
                    Ok(session) => {
                        busy.set(false);
                        password.set(String::new());
                        codes.set(session.recovery_codes.clone());
                        codes_kept.set(false);
                        crate::sync::set_setting_up(&session.name);
                        account.set(Some(Account {
                            token: session.token,
                            name: session.name,
                        }));
                        step.set(Step::Codes);
                    }
                    // A private server asks for its key.
                    Err(f) if f.key == "account.err.wrong" => {
                        busy.set(false);
                        error.set(st.tn("onboard.err.key"));
                        step.set(Step::SignUp);
                    }
                    Err(f) => fail(Step::SignUp, &f),
                }
            })
        });
        drop(task); // runs to completion under the root
    };
    let sign_in = move || {
        if phone.get_untracked().trim().is_empty() || password.get_untracked().is_empty() {
            error.set(st.tn("onboard.err.required"));
            return;
        }
        if !start("onboard.working.wait") {
            return;
        }
        step.set(Step::Working);
        let (url, p, w) = (
            server.get_untracked(),
            phone.get_untracked(),
            password.get_untracked(),
        );
        let task = st.root.run(|| {
            spawn(async move {
                match crate::sync::sign_in(&url, &p, &w).await {
                    Ok(session) => signed_in(session),
                    Err(f) => fail(Step::SignIn, &f),
                }
            })
        });
        drop(task); // runs to completion under the root
    };
    let recover = move || {
        if phone.get_untracked().trim().is_empty()
            || recovery.get_untracked().trim().is_empty()
            || new_password.get_untracked().is_empty()
        {
            error.set(st.tn("onboard.err.required"));
            return;
        }
        if new_password.get_untracked().chars().count() < 8 {
            error.set(st.tn("account.err.short"));
            return;
        }
        if !start("onboard.working.wait") {
            return;
        }
        step.set(Step::Working);
        let (url, p, c, w) = (
            server.get_untracked(),
            phone.get_untracked(),
            recovery.get_untracked(),
            new_password.get_untracked(),
        );
        let task = st.root.run(|| {
            spawn(async move {
                match crate::sync::recover(&url, &p, &c, &w).await {
                    Ok(session) => signed_in(session),
                    Err(f) => fail(Step::Recover, &f),
                }
            })
        });
        drop(task); // runs to completion under the root
    };
    let shop_next = move || {
        let need_owner = alone.get_untracked();
        if shop_name.get_untracked().trim().is_empty()
            || (need_owner && owner.get_untracked().trim().is_empty())
        {
            error.set(st.tn("onboard.err.required"));
            return;
        }
        pin.set(String::new());
        pin_again.set(String::new());
        pin_for.set(PinFor::NewShop);
        go_to(Step::Pin);
    };
    // The PIN chosen: the shop is made (and saved on the server), or the
    // person who just joined gets it; then the shop opens.
    let pin_done = move || {
        let (p, again) = (pin.get_untracked(), pin_again.get_untracked());
        if let Err(e) = nb_domain::access::check_pin(&p) {
            error.set(error_text(st, e.code));
            return;
        }
        if p != again {
            error.set(st.tn("onboard.pin.mismatch"));
            return;
        }
        match pin_for.get_untracked() {
            PinFor::Joined(who) => {
                if !start("onboard.working.wait") {
                    return;
                }
                step.set(Step::Working);
                let task = st.root.run(|| {
                    spawn(async move {
                        let r = st
                            .db()
                            .call({
                                let who = who.clone();
                                move |s| {
                                    let owner = crate::db::owner_of(s)?;
                                    let id = if who.is_empty() { owner } else { who };
                                    let user = nb_store::repo::parse_id(&id)?;
                                    let org = crate::db::org_of(s)?;
                                    let meta =
                                        nb_store::OpMeta::new("user.pin", Some(org), Some(user));
                                    set_secret(s, meta, user, Secret::Pin, Some(p))
                                }
                            })
                            .await;
                        match r {
                            Ok(()) => enter(Some(who)),
                            Err(e) => {
                                busy.set(false);
                                let code = e.codes.first().map_or("internal", |c| c.1);
                                error.set(error_text(st, code));
                                step.set(Step::Pin);
                            }
                        }
                    })
                });
                drop(task); // runs to completion under the root
            }
            PinFor::NewShop => {
                if !start("onboard.working.create") {
                    return;
                }
                step.set(Step::Working);
                let lang = st.lang.get_untracked();
                let acct = account.get_untracked();
                let owner_name = match &acct {
                    Some(a) if !alone.get_untracked() => a.name.clone(),
                    _ => owner.get_untracked().trim().to_owned(),
                };
                let input = SetupInput {
                    org_name: shop_name.get_untracked().trim().to_owned(),
                    activity: activities_at(grocery, activity.get_untracked()),
                    country: "DZ".into(),
                    currency: "DZD".into(),
                    currency_exp: 2,
                    branch_name: {
                        let t = town.get_untracked().trim().to_owned();
                        if t.is_empty() {
                            st.tn("setup.main_branch")
                        } else {
                            t
                        }
                    },
                    owner_name,
                    language: lang.code().to_owned(),
                    fiscal_year_start: 1,
                };
                let url = server.get_untracked();
                let task = st.root.run(|| {
                    spawn(async move {
                        // Made once: a retry after a failed server copy
                        // only sends it.
                        if !made.get_untracked() {
                            let db = st.db();
                            let r = async {
                                let session = db
                                    .setup(Some((input, Default::default())), st.demo_lang())
                                    .await?;
                                let owner = session.owner();
                                db.call(move |s| {
                                    set_secret(
                                        s,
                                        session.meta("user.pin"),
                                        owner,
                                        Secret::Pin,
                                        Some(p),
                                    )
                                })
                                .await
                            }
                            .await;
                            if let Err(e) = r {
                                busy.set(false);
                                let code = e.codes.first().map_or("internal", |c| c.1);
                                error.set(error_text(st, code));
                                step.set(Step::Shop);
                                return;
                            }
                            made.set(true);
                        }
                        match acct.filter(|_| !alone.get_untracked()) {
                            // Saved on the server from the first minute.
                            Some(a) => match crate::sync::link(st, url, a.token).await {
                                Ok(_) => enter(Some(String::new())),
                                Err(f) => {
                                    busy.set(false);
                                    error.set(nb_i18n::tr_args(
                                        st.lang(),
                                        "onboard.link_failed",
                                        &[("reason", &crate::screens::cloud::failure_text(st, &f))],
                                    ));
                                    step.set(Step::Pin);
                                }
                            },
                            None => enter(Some(String::new())),
                        }
                    })
                });
                drop(task); // runs to completion under the root
            }
        }
    };
    let demo = move || {
        if !start("onboard.working.wait") {
            return;
        }
        step.set(Step::Working);
        let db = st.db();
        let lang = st.demo_lang();
        let task = st.root.run(|| {
            spawn(async move {
                match db.setup(None, lang).await {
                    Ok(_) => enter(None),
                    Err(e) => {
                        busy.set(false);
                        error.set(error_text(st, e.codes.first().map_or("internal", |c| c.1)));
                        step.set(Step::More);
                    }
                }
            })
        });
        drop(task); // runs to completion under the root
    };
    let join_by_code = move || {
        if invite.get_untracked().trim().is_empty() {
            error.set(st.tn("onboard.err.required"));
            return;
        }
        if !start("onboard.working.join") {
            return;
        }
        step.set(Step::Working);
        let (u, c) = (server.get_untracked(), invite.get_untracked());
        let task = st.root.run(|| {
            spawn(async move {
                match crate::sync::join(st, u, c).await {
                    // Whoever works here signs in on the lock screen.
                    Ok(_) => enter(None),
                    Err(f) => fail(Step::Code, &f),
                }
            })
        });
        drop(task); // runs to completion under the root
    };

    // ---------------------------------------------------------- screens

    let primary = move |caption: &'static str, id: &'static str, on: Box<dyn Fn()>| {
        button(Variant::Primary, Size::Lg, None, st.t(caption), on)
            .w_full()
            .test_id(id)
    };
    let link_row =
        move |question: &'static str, action: &'static str, to: Step, id: &'static str| {
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_center()
                .gap(px(4.0))
                .child(label_line(
                    st.t(question),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .child(
                    button(Variant::Ghost, Size::Sm, None, st.t(action), move || {
                        go_to(to)
                    })
                    .test_id(id),
                )
        };
    let back = move |to: Step| {
        button(
            Variant::Ghost,
            Size::Md,
            Some(if st.rtl() {
                Icon::ArrowForward
            } else {
                Icon::ArrowBack
            }),
            st.t("onboard.back"),
            move || go_to(to),
        )
        .test_id("onboard-back")
    };
    let phone_field =
        move || crate::ui::form::phone_field(st.t("onboard.phone"), phone, "onboard-phone");
    let password_input = move |value: Signal<String>, caption: &'static str, id: &'static str| {
        password_field(
            st.t(caption),
            value,
            String::new,
            KeyboardType::Default,
            id,
            st.t("onboard.show_password"),
        )
    };

    let welcome = move || {
        view()
            .gap(px(12.0))
            .child(primary(
                "onboard.create",
                "onboard-create",
                Box::new(move || go_to(Step::SignUp)),
            ))
            .child(
                button(
                    Variant::Secondary,
                    Size::Lg,
                    None,
                    st.t("onboard.sign_in"),
                    move || go_to(Step::SignIn),
                )
                .w_full()
                .test_id("onboard-sign-in"),
            )
            .child(view().h(px(4.0)))
            .child(
                button(
                    Variant::Ghost,
                    Size::Md,
                    Some(Icon::More),
                    st.t("onboard.more"),
                    move || go_to(Step::More),
                )
                .self_center()
                .test_id("onboard-more"),
            )
            .into_element()
    };

    let more = move || {
        let option = |i: Icon,
                      caption: &'static str,
                      help: &'static str,
                      id: &'static str,
                      on: Box<dyn Fn()>| {
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .p(px(14.0))
                .rounded(px(radius::lg()))
                .border_1()
                .border_color(Tone::Border.color())
                .bg(Tone::Surface.color())
                .cursor_pointer()
                .on_press(on)
                .test_id(id)
                .child(icon(i, 20.0, || Tone::Primary))
                .child(
                    view()
                        .flex_1()
                        .gap(px(2.0))
                        .child(label_line(
                            st.t(caption),
                            ty::MD,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label(st.t(help), ty::SM, FontWeight::NORMAL, Tone::Muted)),
                )
        };
        view()
            .gap(px(10.0))
            .child(option(
                Icon::Wand,
                "onboard.demo",
                "setup.demo.help",
                "onboard-demo",
                Box::new(demo),
            ))
            .child(option(
                Icon::Link,
                "onboard.by_code",
                "onboard.by_code.help",
                "onboard-by-code",
                Box::new(move || go_to(Step::Code)),
            ))
            .child(option(
                Icon::HardDrive,
                "onboard.alone",
                "onboard.alone.help",
                "onboard-alone",
                Box::new(move || {
                    alone.set(true);
                    go_to(Step::Shop);
                }),
            ))
            .child(divider())
            .child(field(
                st.t("onboard.server"),
                server,
                String::new,
                String::new,
                KeyboardType::Url,
                "onboard-server",
            ))
            .child(label(
                st.t("onboard.server.help"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .child(field(
                st.t("onboard.key"),
                server_key,
                st.t("onboard.key.hint"),
                String::new,
                KeyboardType::Default,
                "onboard-key",
            ))
            .child(view().flex_row().child(back(Step::Welcome)))
            .into_element()
    };

    let signup_view = move || {
        view()
            .gap(px(14.0))
            .child(field(
                st.t("onboard.name"),
                name,
                st.t("setup.owner.hint"),
                String::new,
                KeyboardType::Default,
                "onboard-name",
            ))
            .child(phone_field())
            .child(password_input(
                password,
                "onboard.password",
                "onboard-password",
            ))
            .child(label(
                st.t("onboard.password.hint"),
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .child(primary(
                "onboard.signup.go",
                "onboard-signup-go",
                Box::new(sign_up),
            ))
            .child(label(
                st.t("onboard.terms"),
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .child(link_row(
                "onboard.have_account",
                "onboard.sign_in",
                Step::SignIn,
                "onboard-to-sign-in",
            ))
            .child(view().flex_row().child(back(Step::Welcome)))
            .into_element()
    };

    let codes_view = move || {
        let mut grid = view().flex_row().flex_wrap().gap(px(8.0));
        for (i, c) in codes.get_untracked().into_iter().enumerate() {
            grid = grid.child(
                view()
                    .flex_1()
                    .min_w(px(150.0))
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .py(px(10.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::Subtle.color())
                    .direction(LayoutDirection::Ltr)
                    .child(figure(
                        move || format!("{}.", i + 1),
                        ty::SM,
                        FontWeight::NORMAL,
                        || Tone::Muted,
                    ))
                    .child(figure(
                        move || c.clone(),
                        ty::MD,
                        FontWeight::SEMIBOLD,
                        || Tone::Ink,
                    )),
            );
        }
        view()
            .gap(px(14.0))
            .child(label(
                st.t("onboard.codes.help"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Body,
            ))
            .child(grid)
            .child(
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Copy),
                    st.t("onboard.codes.copy"),
                    move || {
                        set_clipboard(codes.get_untracked().join("\n"));
                        crate::ui::overlay::show_toast(st, st.tn("onboard.codes.copied"));
                    },
                )
                .self_start(),
            )
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .on_press(move || codes_kept.update(|k| *k = !*k))
                    .test_id("onboard-codes-kept")
                    .child(checkbox(
                        move || Some(codes_kept.get()),
                        move || codes_kept.update(|k| *k = !*k),
                    ))
                    .child(label(
                        st.t("onboard.codes.kept"),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Ink,
                    )),
            )
            .child(
                button(
                    Variant::Primary,
                    Size::Lg,
                    None,
                    st.t("onboard.continue"),
                    move || {
                        if !codes_kept.get_untracked() {
                            error.set(st.tn("onboard.codes.must_keep"));
                            return;
                        }
                        alone.set(false);
                        go_to(Step::Shop);
                    },
                )
                .w_full()
                .opacity(move || if codes_kept.get() { 1.0_f32 } else { 0.55 })
                .test_id("onboard-codes-next"),
            )
            .into_element()
    };

    let signin_view = move || {
        view()
            .gap(px(14.0))
            .child(phone_field())
            .child(password_input(
                password,
                "onboard.password",
                "onboard-password",
            ))
            .child(
                button(
                    Variant::Ghost,
                    Size::Sm,
                    None,
                    st.t("onboard.forgot"),
                    move || go_to(Step::Recover),
                )
                .self_end()
                .test_id("onboard-forgot"),
            )
            .child(primary(
                "onboard.signin.go",
                "onboard-signin-go",
                Box::new(sign_in),
            ))
            .child(link_row(
                "onboard.no_account",
                "onboard.create",
                Step::SignUp,
                "onboard-to-sign-up",
            ))
            .child(view().flex_row().child(back(Step::Welcome)))
            .into_element()
    };

    let recover_view = move || {
        view()
            .gap(px(14.0))
            .child(phone_field())
            .child(field(
                st.t("onboard.recover.code"),
                recovery,
                || "ABCD-EFGH".to_owned(),
                String::new,
                KeyboardType::Default,
                "onboard-recovery",
            ))
            .child(password_input(
                new_password,
                "onboard.recover.new",
                "onboard-new-password",
            ))
            .child(primary(
                "onboard.recover.go",
                "onboard-recover-go",
                Box::new(recover),
            ))
            .child(view().flex_row().child(back(Step::SignIn)))
            .into_element()
    };

    let shops_view = move || {
        let mut list = view().gap(px(8.0));
        for o in shops.get() {
            let org = o.org_id.clone();
            let role = if o.role == "owner" {
                "onboard.role.owner"
            } else {
                "onboard.role.staff"
            };
            list = list.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .p(px(14.0))
                    .rounded(px(radius::lg()))
                    .border_1()
                    .border_color(Tone::Border.color())
                    .bg(Tone::Surface.color())
                    .cursor_pointer()
                    .on_press(move || pick(org.clone()))
                    .test_id("onboard-shop")
                    .child(
                        view()
                            .w(px(40.0))
                            .h(px(40.0))
                            .rounded(px(radius::md()))
                            .bg(Tone::PrimarySoft.color())
                            .items_center()
                            .justify_center()
                            .child(icon(Icon::Store, 20.0, || Tone::Primary)),
                    )
                    .child(
                        view()
                            .flex_1()
                            .gap(px(2.0))
                            .child(label_line(
                                {
                                    let n = o.name.clone();
                                    move || n.clone()
                                },
                                ty::MD,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            ))
                            .child(label_line(
                                st.t(role),
                                ty::SM,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(icon(
                        if st.rtl() {
                            Icon::ChevronLeft
                        } else {
                            Icon::ChevronRight
                        },
                        18.0,
                        || Tone::Muted,
                    )),
            );
        }
        view()
            .gap(px(14.0))
            .child(list)
            .child(
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Plus),
                    st.t("onboard.shops.new"),
                    move || {
                        alone.set(false);
                        go_to(Step::Shop);
                    },
                )
                .self_start()
                .test_id("onboard-new-shop"),
            )
            .into_element()
    };

    let device_view = move || {
        let Some(org) = chosen.get() else {
            return view().into_element();
        };
        let mut list = view().gap(px(8.0));
        for d in olds.get() {
            let (org, id) = (org.clone(), d.id.clone());
            let seen = d
                .last_seen_at
                .map(|t| crate::screens::common::date_time(st, t))
                .unwrap_or_default();
            let caption = format!("\u{2068}{}\u{2069}", d.name);
            let when = nb_i18n::tr_args(st.lang(), "onboard.device.last_seen", &[("time", &seen)]);
            list = list.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .p(px(14.0))
                    .rounded(px(radius::lg()))
                    .border_1()
                    .border_color(Tone::Border.color())
                    .bg(Tone::Surface.color())
                    .cursor_pointer()
                    .on_press(move || join(org.clone(), Some(id.clone())))
                    .test_id("onboard-old-device")
                    .child(icon(Icon::Monitor, 20.0, || Tone::Muted))
                    .child(
                        view()
                            .flex_1()
                            .gap(px(2.0))
                            .child(label_line(
                                move || caption.clone(),
                                ty::MD,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            ))
                            .child(label_line(
                                move || when.clone(),
                                ty::SM,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    ),
            );
        }
        let org2 = org.clone();
        view()
            .gap(px(14.0))
            .child(list)
            .child(
                button(
                    Variant::Primary,
                    Size::Lg,
                    None,
                    st.t("onboard.device.new"),
                    move || join(org2.clone(), None),
                )
                .w_full()
                .test_id("onboard-new-device"),
            )
            .child(
                view()
                    .flex_row()
                    .child(back(if shops.with_untracked(|s| s.len() > 1) {
                        Step::Shops
                    } else {
                        Step::SignIn
                    })),
            )
            .into_element()
    };

    let shop_view = {
        let activities = activities.clone();
        move || {
            let activities = activities.clone();
            view()
                .gap(px(14.0))
                .child(field(
                    st.t("setup.shop_name"),
                    shop_name,
                    st.t("setup.shop_name.hint"),
                    String::new,
                    KeyboardType::Default,
                    "onboard-shop-name",
                ))
                .child(labelled(
                    st.t("store.activity"),
                    dropdown(
                        move || {
                            activities
                                .iter()
                                .enumerate()
                                .map(|(i, a)| (i, st.tn(&format!("activity.{a}"))))
                                .collect()
                        },
                        activity,
                        Some(Icon::Store),
                        st.t("store.activity"),
                        "onboard-activity",
                    )
                    .w_full(),
                    None,
                ))
                .child(field(
                    st.t("setup.town"),
                    town,
                    st.t("setup.town.hint"),
                    String::new,
                    KeyboardType::Default,
                    "onboard-town",
                ))
                .child(when(
                    move || alone.get(),
                    move || {
                        field(
                            st.t("setup.owner"),
                            owner,
                            st.t("setup.owner.hint"),
                            String::new,
                            KeyboardType::Default,
                            "onboard-owner",
                        )
                        .into_element()
                    },
                ))
                .child(primary(
                    "onboard.continue",
                    "onboard-shop-next",
                    Box::new(shop_next),
                ))
                .child(view().flex_row().child(back(if alone.get_untracked() {
                    Step::More
                } else if account.get_untracked().is_some()
                    && codes.with_untracked(|c| !c.is_empty())
                {
                    Step::Codes
                } else {
                    Step::Welcome
                })))
                .into_element()
        }
    };

    let pin_view = move || {
        let new_shop = pin_for.get() == PinFor::NewShop;
        view()
            .gap(px(14.0))
            .child(password_field(
                st.t("onboard.pin"),
                pin,
                String::new,
                KeyboardType::Number,
                "onboard-pin",
                st.t("onboard.show_pin"),
            ))
            .child(password_field(
                st.t("onboard.pin.again"),
                pin_again,
                String::new,
                KeyboardType::Number,
                "onboard-pin-again",
                st.t("onboard.show_pin"),
            ))
            .child(label(
                st.t("onboard.pin.help"),
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .child(primary(
                if new_shop {
                    "onboard.pin.create"
                } else {
                    "onboard.pin.enter"
                },
                "onboard-pin-go",
                Box::new(pin_done),
            ))
            .child(when(
                move || pin_for.get() == PinFor::NewShop && !made.get(),
                move || view().flex_row().child(back(Step::Shop)).into_element(),
            ))
            .into_element()
    };

    let code_view = move || {
        view()
            .gap(px(14.0))
            .child(field(
                st.t("sync.code"),
                invite,
                || "ABCD EFGH".to_owned(),
                String::new,
                KeyboardType::Default,
                "onboard-invite",
            ))
            .child(primary(
                "sync.join",
                "onboard-invite-go",
                Box::new(join_by_code),
            ))
            .child(view().flex_row().child(back(Step::More)))
            .into_element()
    };

    let working_view = move || {
        view()
            .gap(px(16.0))
            .items_center()
            .py(px(24.0))
            .child(crate::ui::kit::spinner(28.0))
            .child(label(
                move || working.get(),
                ty::MD,
                FontWeight::MEDIUM,
                Tone::Muted,
            ))
            .into_element()
    };

    // Each step in its own place: an input never carries over what was
    // typed in another step's input.
    let body = view()
        .child(when(move || step.get() == Step::Welcome, welcome))
        .child(when(move || step.get() == Step::More, more))
        .child(when(move || step.get() == Step::SignUp, signup_view))
        .child(when(move || step.get() == Step::Codes, codes_view))
        .child(when(move || step.get() == Step::SignIn, signin_view))
        .child(when(move || step.get() == Step::Recover, recover_view))
        .child(when(move || step.get() == Step::Shops, shops_view))
        .child(when(move || step.get() == Step::Device, device_view))
        .child(when(move || step.get() == Step::Shop, shop_view))
        .child(when(move || step.get() == Step::Pin, pin_view))
        .child(when(move || step.get() == Step::Code, code_view))
        .child(when(move || step.get() == Step::Working, working_view));
    let titles = move || {
        let k = match step.get() {
            Step::Welcome => ("onboard.welcome.title", "onboard.welcome.subtitle"),
            Step::More => ("onboard.more", "onboard.more.subtitle"),
            Step::SignUp => ("onboard.signup.title", "onboard.signup.subtitle"),
            Step::Codes => ("onboard.codes.title", "onboard.codes.subtitle"),
            Step::SignIn => ("onboard.signin.title", "onboard.signin.subtitle"),
            Step::Recover => ("onboard.recover.title", "onboard.recover.subtitle"),
            Step::Shops => ("onboard.shops.title", "onboard.shops.subtitle"),
            Step::Device => ("onboard.device.title", "onboard.device.subtitle"),
            Step::Shop => ("onboard.shop.title", "onboard.shop.subtitle"),
            Step::Pin => ("onboard.pin.title", "onboard.pin.subtitle"),
            Step::Code => ("onboard.code.title", "onboard.code.subtitle"),
            Step::Working => ("onboard.working.title", "onboard.working.subtitle"),
        };
        (st.tn(k.0), st.tn(k.1))
    };
    let languages = dropdown(
        || {
            Lang::ALL
                .iter()
                .map(|&l| (l, l.native_name().to_owned()))
                .collect()
        },
        st.lang,
        Some(Icon::Languages),
        st.t("settings.language"),
        "onboard-language",
    );
    view()
        .test_id("setup-wizard")
        .flex_1()
        .direction(move || st.direction())
        .bg(Tone::Canvas.color())
        .child(
            scroll().flex_1().child(
                view()
                    .w_full()
                    .items_center()
                    .px(px(16.0))
                    .pt(move || px(if window().width.0 < 700.0 { 16.0 } else { 56.0 }))
                    .pb(px(24.0))
                    .gap(px(16.0))
                    .child(
                        view()
                            .w(relative(1.0))
                            .max_w(px(460.0))
                            .flex_row()
                            .items_center()
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(10.0))
                                    .flex_1()
                                    .child(
                                        view()
                                            .w(px(36.0))
                                            .h(px(36.0))
                                            .rounded(px(radius::md()))
                                            .bg(Tone::Primary.color())
                                            .items_center()
                                            .justify_center()
                                            .child(icon(Icon::Store, 20.0, || Tone::OnPrimary)),
                                    )
                                    .child(label_line(
                                        || "NSH Business".to_owned(),
                                        ty::LG,
                                        FontWeight::BOLD,
                                        Tone::Ink,
                                    )),
                            )
                            .child(view().w(px(170.0)).child(languages)),
                    )
                    .child(
                        card()
                            .w(relative(1.0))
                            .max_w(px(460.0))
                            .p(px(28.0))
                            .gap(px(22.0))
                            .child(
                                view()
                                    .gap(px(6.0))
                                    .child(label(
                                        move || titles().0,
                                        ty::XL,
                                        FontWeight::BOLD,
                                        Tone::Ink,
                                    ))
                                    .child(label(
                                        move || titles().1,
                                        ty::SM,
                                        FontWeight::NORMAL,
                                        Tone::Muted,
                                    )),
                            )
                            .child(when(
                                move || !error.get().is_empty(),
                                move || {
                                    let e = error.get();
                                    view()
                                        .flex_row()
                                        .gap(px(8.0))
                                        .p(px(12.0))
                                        .rounded(px(radius::md()))
                                        .bg(Tone::DangerSoft.color())
                                        .child(icon(Icon::Alert, 16.0, || Tone::Danger))
                                        .child(
                                            view().flex_1().child(
                                                label(
                                                    move || e.clone(),
                                                    ty::SM,
                                                    FontWeight::MEDIUM,
                                                    Tone::Danger,
                                                )
                                                .test_id("setup-error"),
                                            ),
                                        )
                                        .into_element()
                                },
                            ))
                            .child(body),
                    ),
            ),
        )
}

fn activities_at(fallback: usize, i: usize) -> String {
    let all: Vec<String> = Profiles::builtin()
        .activities()
        .map(str::to_owned)
        .collect();
    all.get(i)
        .or_else(|| all.get(fallback))
        .cloned()
        .unwrap_or_else(|| "grocery".into())
}

//! The first run (M4.2): a device with no shop yet asks for one — the
//! language, the shop (name, activity, town, owner, fiscal year), its
//! legal identity (optional, also in settings later) — joins a shop that
//! already syncs to a server (a code from one of its devices), or opens the
//! demo shop to look around. Then the app opens on it.

use nb_domain::features::Profiles;
use nb_i18n::Lang;
use nb_store::usecases::org::BusinessProfile;
use nb_store::usecases::setup::SetupInput;
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::form::{field, labelled, secret_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, button, label, label_line, segmented};
use crate::ui::popup::dropdown;
use crate::ui::surface::card;
use crate::ui::tokens::{Tone, radius, ty};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Shop,
    Legal,
    Join,
}

/// How a new device finds its shop.
#[derive(Clone, Copy, PartialEq, Eq)]
enum JoinBy {
    Account,
    Code,
}

pub fn setup_wizard(st: AppState) -> View {
    let step = signal(Step::Welcome);
    let activities: Vec<String> = Profiles::builtin()
        .activities()
        .map(str::to_owned)
        .collect();
    let grocery = activities.iter().position(|a| a == "grocery").unwrap_or(0);
    let s = |v: &str| signal(v.to_owned());
    let (name, town, owner) = (s(""), s(""), s(""));
    let activity = signal(grocery);
    let fiscal = signal(1u32);
    let (nif, nis, rc, ai, address, phone) = (s(""), s(""), s(""), s(""), s(""), s(""));
    let error = signal(String::new());
    let busy = signal(false);

    let finish = move |demo: bool| {
        if busy.get_untracked() {
            return;
        }
        let lang = st.lang.get_untracked();
        let input = (!demo).then(|| {
            (
                SetupInput {
                    org_name: name.get_untracked().trim().to_owned(),
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
                    owner_name: owner.get_untracked().trim().to_owned(),
                    language: lang.code().to_owned(),
                    fiscal_year_start: u8::try_from(fiscal.get_untracked()).unwrap_or(1),
                },
                BusinessProfile {
                    legal_name: String::new(),
                    nif: nif.get_untracked().trim().into(),
                    nis: nis.get_untracked().trim().into(),
                    rc: rc.get_untracked().trim().into(),
                    ai: ai.get_untracked().trim().into(),
                    address: address.get_untracked().trim().into(),
                    phone: phone.get_untracked().trim().into(),
                    email: String::new(),
                },
            )
        });
        busy.set(true);
        error.set(String::new());
        let db = st.db();
        let demo_lang = st.demo_lang();
        let task = spawn(async move {
            match db.setup(input, demo_lang).await {
                Ok(_) => {
                    st.needs_setup.set(false);
                    st.connect();
                }
                Err(e) => {
                    busy.set(false);
                    let code = e.codes.first().map_or("internal", |c| c.1);
                    error.set(error_text(st, code));
                    // A mistake in the shop's form: back to it.
                    if e.codes
                        .iter()
                        .any(|c| matches!(c.0, Some("org_name" | "owner_name")))
                    {
                        step.set(Step::Shop);
                    }
                }
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };

    let activity_options = {
        let activities = activities.clone();
        move || {
            activities
                .iter()
                .enumerate()
                .map(|(i, a)| (i, st.tn(&format!("activity.{a}"))))
                .collect()
        }
    };
    let months = move || {
        (1..=12)
            .map(|m| (m, st.fmt().month_short(m).to_owned()))
            .collect::<Vec<(u32, String)>>()
    };

    let welcome = move || {
        view()
            .gap(px(16.0))
            .child(labelled(
                st.t("settings.language"),
                dropdown(
                    || {
                        Lang::ALL
                            .iter()
                            .map(|&l| (l, l.native_name().to_owned()))
                            .collect()
                    },
                    st.lang,
                    Some(Icon::Languages),
                    st.t("settings.language"),
                    "setup-language",
                )
                .w_full(),
                None,
            ))
            .child(
                button(
                    Variant::Primary,
                    Size::Lg,
                    Some(Icon::Store),
                    st.t("setup.create"),
                    move || step.set(Step::Shop),
                )
                .w_full()
                .test_id("setup-create"),
            )
            .child(
                button(
                    Variant::Secondary,
                    Size::Lg,
                    Some(Icon::Link),
                    st.t("setup.join"),
                    move || {
                        error.set(String::new());
                        step.set(Step::Join);
                    },
                )
                .w_full()
                .test_id("setup-join"),
            )
            .child(
                button(
                    Variant::Secondary,
                    Size::Lg,
                    Some(Icon::Wand),
                    st.t("setup.demo"),
                    move || finish(true),
                )
                .w_full()
                .test_id("setup-demo"),
            )
            .child(label(
                st.t("setup.demo.help"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .into_element()
    };
    let shop = move || {
        let activity_options = activity_options.clone();
        view()
            .gap(px(14.0))
            .child(field(
                st.t("setup.shop_name"),
                name,
                st.t("setup.shop_name.hint"),
                String::new,
                KeyboardType::Default,
                "setup-name",
            ))
            .child(labelled(
                st.t("store.activity"),
                dropdown(
                    activity_options,
                    activity,
                    Some(Icon::Store),
                    st.t("store.activity"),
                    "setup-activity",
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
                "setup-town",
            ))
            .child(field(
                st.t("setup.owner"),
                owner,
                st.t("setup.owner.hint"),
                String::new,
                KeyboardType::Default,
                "setup-owner",
            ))
            .child(labelled(
                st.t("store.fiscal_start"),
                dropdown(
                    months,
                    fiscal,
                    Some(Icon::Calendar),
                    st.t("store.fiscal_start"),
                    "setup-fiscal",
                )
                .w_full(),
                None,
            ))
            .child(
                view()
                    .flex_row()
                    .gap(px(10.0))
                    .child(button(
                        Variant::Secondary,
                        Size::Md,
                        None,
                        st.t("setup.back"),
                        move || step.set(Step::Welcome),
                    ))
                    .child(view().flex_1())
                    .child(
                        button(
                            Variant::Primary,
                            Size::Md,
                            None,
                            st.t("setup.next"),
                            move || {
                                if name.get_untracked().trim().is_empty()
                                    || owner.get_untracked().trim().is_empty()
                                {
                                    error.set(st.tn("setup.required"));
                                    return;
                                }
                                error.set(String::new());
                                step.set(Step::Legal);
                            },
                        )
                        .test_id("setup-next"),
                    ),
            )
            .into_element()
    };
    let legal = move || {
        let small = |caption: &'static str, value: Signal<String>, id: &'static str| {
            view().flex_1().min_w(px(150.0)).child(field(
                st.t(caption),
                value,
                String::new,
                String::new,
                KeyboardType::Default,
                id,
            ))
        };
        view()
            .gap(px(14.0))
            .child(label(
                st.t("setup.legal.help"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(12.0))
                    .child(small("store.nif", nif, "setup-nif"))
                    .child(small("store.nis", nis, "setup-nis")),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(12.0))
                    .child(small("store.rc", rc, "setup-rc"))
                    .child(small("store.ai", ai, "setup-ai")),
            )
            .child(field(
                st.t("store.address"),
                address,
                String::new,
                String::new,
                KeyboardType::Default,
                "setup-address",
            ))
            .child(field(
                st.t("store.phone"),
                phone,
                || "0550 00 00 00".to_owned(),
                String::new,
                KeyboardType::Phone,
                "setup-phone",
            ))
            .child(
                view()
                    .flex_row()
                    .gap(px(10.0))
                    .child(button(
                        Variant::Secondary,
                        Size::Md,
                        None,
                        st.t("setup.back"),
                        move || step.set(Step::Shop),
                    ))
                    .child(view().flex_1())
                    .child(
                        button(
                            Variant::Primary,
                            Size::Md,
                            Some(Icon::Check),
                            st.t("setup.finish"),
                            move || finish(false),
                        )
                        .test_id("setup-finish"),
                    ),
            )
            .into_element()
    };
    let (server, code) = (s(""), s(""));
    let (phone, password) = (s(""), s(""));
    // By a code from a device of the shop, or by the owner's account.
    let by = signal(JoinBy::Account);
    let session = signal(String::new());
    let shops = signal(Vec::<nb_store::sync::OrgInfo>::new());
    let fail = move |f: crate::sync::Failed| {
        busy.set(false);
        error.set(crate::screens::cloud::failure_text(st, &f));
    };
    let opened = move |j: nb_store::sync::Joined| {
        crate::ui::overlay::show_toast(
            st,
            nb_i18n::tr_args(st.lang(), "sync.joined", &[("org", &j.org_name)]),
        );
        st.needs_setup.set(false);
        st.connect();
    };
    let start = move || {
        if busy.get_untracked() {
            return false;
        }
        busy.set(true);
        error.set(String::new());
        true
    };
    let join = move || {
        if !start() {
            return;
        }
        let (u, c) = (server.get_untracked(), code.get_untracked());
        // Under the root: this screen goes away once the shop opens, and
        // what it started would end with it.
        let task = st.root.run(|| {
            spawn(async move {
                match crate::sync::join(st, u, c).await {
                    Ok(j) => opened(j),
                    Err(f) => fail(f),
                }
            })
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let sign_in = move || {
        if !start() {
            return;
        }
        let (u, p, w) = (
            server.get_untracked(),
            phone.get_untracked(),
            password.get_untracked(),
        );
        let task = st.root.run(|| {
            spawn(async move {
                let found = async {
                    let s = crate::sync::sign_in(&u, &p, &w).await?;
                    let list = crate::sync::my_orgs(&u, &s.token).await?;
                    Ok::<_, crate::sync::Failed>((s.token, list))
                }
                .await;
                match found {
                    Ok((token, list)) if list.is_empty() => {
                        let _ = token;
                        busy.set(false);
                        error.set(st.tn("account.no_shops"));
                    }
                    Ok((token, list)) => {
                        busy.set(false);
                        session.set(token);
                        shops.set(list);
                    }
                    Err(f) => fail(f),
                }
            })
        });
        drop(task); // runs to completion
    };
    // The shop picked, and its devices: this one may take back the place
    // of one of them (a reinstall) or be new.
    let chosen = signal(None::<String>);
    let olds = signal(Vec::<nb_store::sync::DeviceInfo>::new());
    let go = move |org: String, replaces: Option<String>| {
        if !start() {
            return;
        }
        let (u, t) = (server.get_untracked(), session.get_untracked());
        let task = st.root.run(|| {
            spawn(async move {
                match crate::sync::join_mine(st, u, t, org, replaces).await {
                    Ok(j) => opened(j),
                    Err(f) => fail(f),
                }
            })
        });
        drop(task); // runs to completion
    };
    let pick = move |org: String| {
        if !start() {
            return;
        }
        let (u, t) = (server.get_untracked(), session.get_untracked());
        let task = st.root.run(|| {
            spawn(async move {
                match crate::sync::shop_devices(&u, &t, &org).await {
                    // A shop with no device yet: nothing to take back.
                    Ok(list) if list.is_empty() => {
                        busy.set(false);
                        go(org, None);
                    }
                    Ok(list) => {
                        busy.set(false);
                        olds.set(list);
                        chosen.set(Some(org));
                    }
                    Err(f) => fail(f),
                }
            })
        });
        drop(task); // runs to completion
    };
    let back = move || {
        button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("setup.back"),
            move || {
                if chosen.get_untracked().is_some() {
                    chosen.set(None);
                } else if shops.with_untracked(|v| v.is_empty()) {
                    step.set(Step::Welcome);
                } else {
                    shops.set(Vec::new());
                }
            },
        )
    };
    let joining = move || {
        view()
            .gap(px(14.0))
            .child(label(
                st.t("setup.join.help"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .child(segmented(
                vec![
                    (JoinBy::Account, Box::new(st.t("setup.join.by_account"))),
                    (JoinBy::Code, Box::new(st.t("setup.join.by_code"))),
                ],
                by,
            ))
            .child(field(
                st.t("sync.server"),
                server,
                st.t("sync.server_placeholder"),
                String::new,
                KeyboardType::Url,
                "join-url",
            ))
            // Each way in its own place (inputs are not shared by position).
            .child(when(
                move || by.get() == JoinBy::Code,
                move || {
                    view()
                        .gap(px(14.0))
                        .child(field(
                            st.t("sync.code"),
                            code,
                            || "ABCD EFGH".to_owned(),
                            String::new,
                            KeyboardType::Default,
                            "join-code",
                        ))
                        .child(
                            view()
                                .flex_row()
                                .gap(px(10.0))
                                .child(back())
                                .child(view().flex_1())
                                .child(
                                    button(
                                        Variant::Primary,
                                        Size::Md,
                                        Some(Icon::Link),
                                        st.t("sync.join"),
                                        join,
                                    )
                                    .test_id("join-go"),
                                ),
                        )
                        .into_element()
                },
            ))
            .child(when(
                move || by.get() == JoinBy::Account && shops.with(|v| v.is_empty()),
                move || {
                    view()
                        .gap(px(14.0))
                        .child(field(
                            st.t("account.phone"),
                            phone,
                            || "\u{2066}0550 12 34 56\u{2069}".to_owned(),
                            String::new,
                            KeyboardType::Phone,
                            "join-phone",
                        ))
                        .child(secret_field(
                            st.t("account.password"),
                            password,
                            String::new,
                            "join-password",
                        ))
                        .child(
                            view()
                                .flex_row()
                                .gap(px(10.0))
                                .child(back())
                                .child(view().flex_1())
                                .child(
                                    button(
                                        Variant::Primary,
                                        Size::Md,
                                        Some(Icon::User),
                                        st.t("account.sign_in"),
                                        sign_in,
                                    )
                                    .test_id("join-sign-in"),
                                ),
                        )
                        .into_element()
                },
            ))
            // The account's shops: one tap makes this device one of its.
            .child(when(
                move || {
                    by.get() == JoinBy::Account
                        && shops.with(|v| !v.is_empty())
                        && chosen.get().is_none()
                },
                move || {
                    view()
                        .gap(px(10.0))
                        .child(label(
                            st.t("setup.join.pick"),
                            ty::MD,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(dynamic(move || {
                            let mut list = view().gap(px(8.0));
                            for o in shops.get() {
                                let org = o.org_id.clone();
                                let name = o.name.clone();
                                list = list.child(
                                    button(
                                        Variant::Secondary,
                                        Size::Lg,
                                        Some(Icon::Store),
                                        move || name.clone(),
                                        move || pick(org.clone()),
                                    )
                                    .w_full()
                                    .test_id("join-shop"),
                                );
                            }
                            list.into_element()
                        }))
                        .child(view().flex_row().child(back()))
                        .into_element()
                },
            ))
            // A reinstall takes back its place: its letter, its numbers,
            // its till (the old one stops). Otherwise a new device.
            .child(when(
                move || by.get() == JoinBy::Account && chosen.get().is_some(),
                move || {
                    view()
                        .gap(px(10.0))
                        .child(label(
                            st.t("setup.join.replace"),
                            ty::MD,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label(
                            st.t("setup.join.replace_help"),
                            ty::SM,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        ))
                        .child(dynamic(move || {
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
                                let caption = format!("{} · {} · {seen}", d.prefix, d.name);
                                list = list.child(
                                    button(
                                        Variant::Secondary,
                                        Size::Lg,
                                        Some(Icon::Refresh),
                                        move || caption.clone(),
                                        move || go(org.clone(), Some(id.clone())),
                                    )
                                    .w_full()
                                    .test_id("join-replace"),
                                );
                            }
                            let org2 = org.clone();
                            list.child(
                                button(
                                    Variant::Primary,
                                    Size::Lg,
                                    Some(Icon::Plus),
                                    st.t("setup.join.new_device"),
                                    move || go(org2.clone(), None),
                                )
                                .w_full()
                                .test_id("join-new"),
                            )
                            .into_element()
                        }))
                        .child(view().flex_row().child(back()))
                        .into_element()
                },
            ))
            .into_element()
    };
    // Each step in its own place: an input never carries over what was
    // typed in another step's input.
    let body = view()
        .child(when(move || step.get() == Step::Welcome, welcome))
        .child(when(move || step.get() == Step::Shop, shop))
        .child(when(move || step.get() == Step::Legal, legal))
        .child(when(move || step.get() == Step::Join, joining));
    let titles = move || match step.get() {
        Step::Welcome => (st.tn("setup.title"), st.tn("setup.subtitle")),
        Step::Shop => (st.tn("setup.shop"), st.tn("setup.shop.help")),
        Step::Legal => (st.tn("setup.legal"), st.tn("setup.legal.optional")),
        Step::Join => (st.tn("setup.join.title"), st.tn("sync.local_first")),
    };
    view()
        .test_id("setup-wizard")
        .flex_1()
        .direction(move || st.direction())
        .bg(Tone::Canvas.color())
        .items_center()
        .justify_center()
        .p(px(16.0))
        .child(
            scroll().w(relative(1.0)).max_w(px(520.0)).child(
                card()
                    .p(px(28.0))
                    .gap(px(20.0))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(14.0))
                            .child(
                                view()
                                    .w(px(48.0))
                                    .h(px(48.0))
                                    .rounded(px(radius::lg()))
                                    .bg(Tone::PrimarySoft.color())
                                    .items_center()
                                    .justify_center()
                                    .child(icon(Icon::Store, 24.0, || Tone::Primary)),
                            )
                            .child(
                                view()
                                    .flex_1()
                                    .gap(px(2.0))
                                    .child(label_line(
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
                            ),
                    )
                    .child(body)
                    .child(dynamic(move || {
                        let e = error.get();
                        if e.is_empty() {
                            view().into_element()
                        } else {
                            label(move || e.clone(), ty::SM, FontWeight::MEDIUM, Tone::Danger)
                                .test_id("setup-error")
                                .into_element()
                        }
                    })),
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

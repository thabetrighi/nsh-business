//! The first run (M4.2): a device with no shop yet asks for one — the
//! language, the shop (name, activity, town, owner, fiscal year), its
//! legal identity (optional, also in settings later) — or opens the demo
//! shop to look around. Then the app opens on it.

use nb_domain::features::Profiles;
use nb_i18n::Lang;
use nb_store::usecases::org::BusinessProfile;
use nb_store::usecases::setup::SetupInput;
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::form::{field, labelled};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, button, label, label_line};
use crate::ui::popup::dropdown;
use crate::ui::surface::card;
use crate::ui::tokens::{Tone, radius, ty};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Shop,
    Legal,
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
    // Each step in its own place: an input never carries over what was
    // typed in another step's input.
    let body = view()
        .child(when(move || step.get() == Step::Welcome, welcome))
        .child(when(move || step.get() == Step::Shop, shop))
        .child(when(move || step.get() == Step::Legal, legal));
    let titles = move || match step.get() {
        Step::Welcome => (st.tn("setup.title"), st.tn("setup.subtitle")),
        Step::Shop => (st.tn("setup.shop"), st.tn("setup.shop.help")),
        Step::Legal => (st.tn("setup.legal"), st.tn("setup.legal.optional")),
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

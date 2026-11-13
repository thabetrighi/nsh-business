//! Settings, in tabs grouped like a control panel: preferences (general,
//! appearance, numbers), business (store, taxes), hardware (printing,
//! devices) and administration (users, sync, about). Every change applies at
//! once and is saved on the device; appearance has a live preview.

use nb_core::{Currency, Money};
use nb_i18n::{Digits, Lang};
use nsh_core::element::{SharedString, View};
use nsh_core::prelude::*;

use nb_domain::features::{Feature, Profiles};
use nb_store::usecases::org::{self as org_usecases, BusinessProfile, OrgSettings};

use crate::shell::user_name;
use crate::state::{AppState, ThemeMode};
use crate::ui::form::field;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::switch;
use crate::ui::kit::{
    Size, Variant, avatar, badge, button, divider, figure, label, label_line, segmented,
};
use crate::ui::overlay::show_toast;
use crate::ui::popup::dropdown;
use crate::ui::surface::card_header_icon;
use crate::ui::surface::{card, card_header, delta, page_header_full, page_pad, setting_row};
use crate::ui::tokens::{Accent, Corners, Density, Status, TextSize, Tone, UiFont, radius, sp, ty};
use nsh_core::element::KeyboardType;

fn options<T: Copy + 'static>(
    st: AppState,
    all: &[T],
    key: impl Fn(T) -> String + 'static + Copy,
) -> Vec<(T, Box<dyn Fn() -> String>)> {
    let _ = st;
    all.iter()
        .map(|&v| (v, Box::new(move || key(v)) as Box<dyn Fn() -> String>))
        .collect()
}

fn accent_swatches(st: AppState) -> View {
    let accent = st.prefs.accent;
    let mut row = view().flex_row().flex_wrap().gap(px(10.0));
    for a in Accent::ALL {
        let on = computed(move || accent.get() == a);
        row = row.child(
            view()
                .w(px(28.0))
                .h(px(28.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .border(px(2.0))
                .border_color(move || {
                    if on.get() {
                        Tone::Ink.color()
                    } else {
                        Tone::Canvas.color()
                    }
                })
                .label(move || SharedString::from(st.tn(&format!("accent.{}", a.code()))))
                .on_press(move || accent.set(a))
                .child(
                    view()
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_full()
                        .items_center()
                        .justify_center()
                        .bg(a.swatch())
                        .child(when(on, || {
                            icon(Icon::Check, 12.0, || Tone::OnPrimary).into_element()
                        })),
                ),
        );
    }
    view().gap(px(8.0)).items_end().child(row).child(label(
        move || st.tn(&format!("accent.{}", accent.get().code())),
        ty::XS,
        FontWeight::MEDIUM,
        Tone::Muted,
    ))
}

fn preview(st: AppState) -> View {
    card()
        .child(card_header(st.t("settings.preview"), None, view()))
        .child(
            view()
                .px(px(20.0))
                .pb(px(20.0))
                .gap(px(16.0))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(view().flex_1().child(label_line(
                            st.t("kpi.revenue"),
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::Muted,
                        )))
                        .child(delta(1_240, move || st.fmt().percent_signed(1_240))),
                )
                .child(figure(
                    move || {
                        st.fmt()
                            .money_whole(Money::from_minor(128_450_000, Currency::DZD))
                    },
                    ty::H1,
                    FontWeight::BOLD,
                    || Tone::Ink,
                ))
                .child(label(
                    st.t("design.sample_text"),
                    ty::MD,
                    FontWeight::NORMAL,
                    Tone::Body,
                ))
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(badge(
                            Status::Success,
                            Some(Icon::Check),
                            st.t("status.paid"),
                        ))
                        .child(badge(Status::Warning, None, st.t("stock.low")))
                        .child(badge(Status::Danger, None, st.t("status.overdue"))),
                )
                .child(divider())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(avatar(user_name(st), 32.0, Status::Accent))
                        .child(
                            view()
                                .flex_1()
                                .child(label_line(
                                    user_name(st),
                                    ty::SM,
                                    FontWeight::SEMIBOLD,
                                    Tone::Ink,
                                ))
                                .child(label_line(
                                    st.t("user.role.owner"),
                                    ty::XS,
                                    FontWeight::NORMAL,
                                    Tone::Muted,
                                )),
                        )
                        .child(figure(
                            move || st.fmt().money(Money::from_minor(1_845_000, Currency::DZD)),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            || Tone::Ink,
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .gap(px(8.0))
                        .child(button(
                            Variant::Secondary,
                            Size::Md,
                            None,
                            st.t("form.cancel"),
                            || {},
                        ))
                        .child(button(
                            Variant::Primary,
                            Size::Md,
                            Some(Icon::Check),
                            st.t("form.save"),
                            || {},
                        )),
                ),
        )
}

/// The tabs: (code, icon, group).
const TABS: [(&str, Icon, usize); 10] = [
    ("general", Icon::Sliders, 0),
    ("appearance", Icon::Brush, 0),
    ("formats", Icon::Dollar, 0),
    ("store", Icon::Store, 1),
    ("taxes", Icon::PercentPlain, 1),
    ("printing", Icon::Printer, 2),
    ("devices", Icon::Scale, 2),
    ("users", Icon::Shield, 3),
    ("sync", Icon::CloudUpload, 3),
    ("about", Icon::Info, 3),
];
const GROUPS: [&str; 4] = [
    "settings.group.prefs",
    "settings.group.business",
    "settings.group.hardware",
    "settings.group.admin",
];

fn tab_key(code: &str, suffix: &str) -> String {
    format!("settings.tab.{code}{suffix}")
}

/// A card of setting rows, separated by hairlines.
/// A card of setting rows; titled when the tab has several cards.
pub(crate) fn group_card(
    st: AppState,
    head: Option<(Icon, &'static str)>,
    rows: Vec<View>,
) -> View {
    let mut c = card();
    if let Some((which, title)) = head {
        c = c.child(card_header_icon(Some(which), st.t(title), None, view()));
    }
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 || head.is_some() {
            c = c.child(divider());
        }
        c = c.child(r);
    }
    c
}

pub(crate) fn row(
    st: AppState,
    name: &'static str,
    help: &'static str,
    control: View,
    compact: bool,
) -> View {
    setting_row(st.t(name), st.t(help), control, compact)
}

fn choices(st: AppState, keys: &'static [&'static str]) -> Vec<(usize, Box<dyn Fn() -> String>)> {
    keys.iter()
        .enumerate()
        .map(|(i, &k)| (i, Box::new(st.t(k)) as Box<dyn Fn() -> String>))
        .collect()
}

fn plain_choices(items: &'static [&'static str]) -> Vec<(usize, Box<dyn Fn() -> String>)> {
    items
        .iter()
        .enumerate()
        .map(|(i, &k)| (i, Box::new(move || k.to_owned()) as Box<dyn Fn() -> String>))
        .collect()
}

fn general(st: AppState, compact: bool) -> View {
    let b = st.biz;
    view().gap(px(20.0)).child(group_card(
        st,
        None,
        vec![
            row(
                st,
                "settings.language",
                "settings.language.help",
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
                    "language-select",
                )
                .w_full(),
                compact,
            ),
            row(
                st,
                "settings.start_page",
                "settings.start_page.help",
                segmented(choices(st, &["nav.home", "nav.sell"]), b.start_page),
                compact,
            ),
            row(
                st,
                "settings.confirm_delete",
                "settings.confirm_delete.help",
                switch(b.confirm_delete, st.t("settings.confirm_delete")),
                compact,
            ),
        ],
    ))
}

fn appearance(st: AppState, compact: bool) -> View {
    let p = st.prefs;
    let main = view()
        .gap(px(20.0))
        .child(group_card(
            st,
            Some((Icon::Palette, "settings.card.color")),
            vec![
                row(
                    st,
                    "settings.theme",
                    "settings.theme.help",
                    segmented(
                        options(st, &ThemeMode::ALL, move |m| {
                            st.tn(match m {
                                ThemeMode::Light => "settings.theme.light",
                                ThemeMode::Dark => "settings.theme.dark",
                                ThemeMode::System => "settings.theme.system",
                            })
                        }),
                        p.theme,
                    ),
                    compact,
                ),
                row(
                    st,
                    "settings.accent",
                    "settings.accent.help",
                    accent_swatches(st),
                    compact,
                ),
            ],
        ))
        .child(group_card(
            st,
            Some((Icon::Type, "settings.card.type")),
            vec![
                row(
                    st,
                    "settings.font",
                    "settings.font.help",
                    dropdown(
                        || {
                            UiFont::ALL
                                .iter()
                                .map(|&f| (f, f.family().to_owned()))
                                .collect()
                        },
                        p.font,
                        Some(Icon::Type),
                        st.t("settings.font"),
                        "font-select",
                    )
                    .w_full(),
                    compact,
                ),
                row(
                    st,
                    "settings.text_size",
                    "settings.text_size.help",
                    segmented(
                        options(st, &TextSize::ALL, move |t| {
                            st.tn(&format!("size.{}", t.code()))
                        }),
                        p.text,
                    ),
                    compact,
                ),
            ],
        ))
        .child(group_card(
            st,
            Some((Icon::Columns, "settings.card.layout")),
            vec![
                row(
                    st,
                    "settings.density",
                    "settings.density.help",
                    segmented(
                        options(st, &Density::ALL, move |d| {
                            st.tn(&format!("density.{}", d.code()))
                        }),
                        p.density,
                    ),
                    compact,
                ),
                row(
                    st,
                    "settings.corners",
                    "settings.corners.help",
                    segmented(
                        options(st, &Corners::ALL, move |c| {
                            st.tn(&format!("corners.{}", c.code()))
                        }),
                        p.corners,
                    ),
                    compact,
                ),
            ],
        ));
    // The preview first: changes below show in it at once.
    let _ = compact;
    view().gap(px(20.0)).child(preview(st)).child(main)
}

fn formats(st: AppState, compact: bool) -> View {
    let p = st.prefs;
    let b = st.biz;
    let currency = signal(0usize);
    let date = signal(0usize);
    view().child(group_card(
        st,
        None,
        vec![
            row(
                st,
                "settings.digits",
                "settings.digits.help",
                segmented(
                    vec![
                        (
                            Digits::Latin,
                            Box::new(st.t("digits.latin")) as Box<dyn Fn() -> String>,
                        ),
                        (Digits::ArabicIndic, Box::new(st.t("digits.arabic"))),
                    ],
                    p.digits,
                ),
                compact,
            ),
            row(
                st,
                "settings.currency",
                "settings.currency.help",
                dropdown(
                    || vec![(0usize, "DZD — Dinar algérien / دينار جزائري".to_owned())],
                    currency,
                    Some(Icon::Banknote),
                    st.t("settings.currency"),
                    "currency-select",
                )
                .w_full(),
                compact,
            ),
            row(
                st,
                "settings.date_format",
                "settings.date_format.help",
                segmented(plain_choices(&["31/12/2026"]), date),
                compact,
            ),
            row(
                st,
                "settings.cents",
                "settings.cents.help",
                switch(b.show_cents, st.t("settings.cents")),
                compact,
            ),
        ],
    ))
}

/// Auto-lock choices, in minutes (0 = never).
const AUTO_LOCK: [u32; 8] = [0, 1, 2, 5, 10, 15, 30, 60];

/// The store's settings and legal identity, from the organization's
/// database (M4.1, M4.9), and the modules its activity turns on (M4.3–M4.4).
fn store(st: AppState, compact: bool) -> View {
    let profiles = Profiles::builtin();
    let activities: Vec<String> = profiles.activities().map(str::to_owned).collect();
    let s = |v: &str| signal(v.to_owned());
    let (name, legal, phone, address, email) = (s(""), s(""), s(""), s(""), s(""));
    let (nif, nis, rc, ai) = (s(""), s(""), s(""), s(""));
    let activity = signal(0usize);
    let language = signal(0usize);
    let fiscal = signal(1u32);
    let auto_lock = signal(3usize);
    let errs: [Signal<String>; 5] = std::array::from_fn(|_| signal(String::new()));
    let [name_err, nif_err, nis_err, email_err, form_err] = errs;
    let saving = signal(false);

    let loaded = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    Ok((
                        org_usecases::settings(s)?,
                        org_usecases::business_profile(s)?,
                    ))
                })
                .await
            }
        },
    );
    {
        let activities = activities.clone();
        effect(move || {
            let Some((o, p)) = loaded.with(|v: Option<&(OrgSettings, BusinessProfile)>| v.cloned())
            else {
                return;
            };
            name.set(o.name);
            activity.set(
                activities
                    .iter()
                    .position(|a| *a == o.activity)
                    .unwrap_or(0),
            );
            language.set(
                org_usecases::LANGUAGES
                    .iter()
                    .position(|l| *l == o.language)
                    .unwrap_or(0),
            );
            fiscal.set(u32::from(o.fiscal_year_start));
            auto_lock.set(
                AUTO_LOCK
                    .iter()
                    .position(|m| *m == o.auto_lock_min)
                    .unwrap_or(3),
            );
            legal.set(p.legal_name);
            phone.set(p.phone);
            address.set(p.address);
            email.set(p.email);
            nif.set(p.nif);
            nis.set(p.nis);
            rc.set(p.rc);
            ai.set(p.ai);
        });
    }

    let save = {
        let activities = activities.clone();
        move || {
            let Some(session) = st.session.get_untracked() else {
                return;
            };
            if saving.get_untracked() {
                return;
            }
            for e in errs {
                e.set(String::new());
            }
            let settings = OrgSettings {
                name: name.get_untracked(),
                activity: activities
                    .get(activity.get_untracked())
                    .cloned()
                    .unwrap_or_default(),
                language: org_usecases::LANGUAGES[language.get_untracked().min(2)].to_owned(),
                fiscal_year_start: u8::try_from(fiscal.get_untracked()).unwrap_or(1),
                auto_lock_min: AUTO_LOCK[auto_lock.get_untracked().min(AUTO_LOCK.len() - 1)],
            };
            let profile = BusinessProfile {
                legal_name: legal.get_untracked(),
                nif: nif.get_untracked(),
                nis: nis.get_untracked(),
                rc: rc.get_untracked(),
                ai: ai.get_untracked(),
                address: address.get_untracked(),
                phone: phone.get_untracked(),
                email: email.get_untracked(),
            };
            saving.set(true);
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(move |s| {
                        let o =
                            org_usecases::update_settings(s, session.meta("org.update"), settings)?;
                        org_usecases::update_business_profile(
                            s,
                            session.meta("org.profile"),
                            profile,
                        )?;
                        Ok((o, org_usecases::features(s)?))
                    })
                    .await;
                saving.set(false);
                match r {
                    Ok((o, features)) => {
                        st.changed();
                        st.features.set(Some(features));
                        st.shop.update(|s| {
                            if let Some(s) = s {
                                s.0 = o.name.clone();
                            }
                        });
                        st.user.update(|u| {
                            if let Some(u) = u {
                                u.auto_lock_min = o.auto_lock_min;
                            }
                        });
                        show_toast(st, st.tn("settings.saved"));
                    }
                    Err(e) => {
                        for (field, code) in &e.codes {
                            let target = match field.unwrap_or("") {
                                "name" => name_err,
                                "nif" => nif_err,
                                "nis" => nis_err,
                                "email" => email_err,
                                _ => form_err,
                            };
                            if target.get_untracked().is_empty() {
                                target.set(crate::screens::forms::error_text(st, code));
                            }
                        }
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };

    let pair = move |a: View, c: View| {
        if compact {
            view().gap(px(16.0)).child(a).child(c)
        } else {
            view()
                .flex_row()
                .gap(px(16.0))
                .child(a.flex_1())
                .child(c.flex_1())
        }
    };
    let f = move |key: &'static str,
                  sig: Signal<String>,
                  err: Signal<String>,
                  kb: KeyboardType,
                  id: &'static str| {
        field(st.t(key), sig, String::new, move || err.get(), kb, id)
    };
    let none = signal(String::new());
    let caption =
        move |key: &'static str| label(st.t(key), ty::SM, FontWeight::SEMIBOLD, Tone::Ink);
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
    let profile = card()
        .child(card_header_icon(
            Some(Icon::Store),
            st.t("settings.card.identity"),
            None,
            view(),
        ))
        .child(divider())
        .child(
            view()
                .p(px(20.0))
                .gap(px(16.0))
                .child(pair(
                    f(
                        "store.name",
                        name,
                        name_err,
                        KeyboardType::Default,
                        "store-name",
                    ),
                    f(
                        "store.phone",
                        phone,
                        none,
                        KeyboardType::Phone,
                        "store-phone",
                    ),
                ))
                .child(pair(
                    f(
                        "store.address",
                        address,
                        none,
                        KeyboardType::Default,
                        "store-address",
                    ),
                    f(
                        "store.email",
                        email,
                        email_err,
                        KeyboardType::Email,
                        "store-email",
                    ),
                ))
                .child(pair(
                    view()
                        .gap(px(6.0))
                        .child(caption("store.activity"))
                        .child(dropdown(
                            activity_options,
                            activity,
                            Some(Icon::Tag),
                            st.t("store.activity"),
                            "activity-select",
                        ))
                        .child(label(
                            st.t("store.activity.help"),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                    view()
                        .gap(px(6.0))
                        .child(caption("store.language"))
                        .child(dropdown(
                            || {
                                org_usecases::LANGUAGES
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(i, l)| {
                                        Lang::from_tag(l).map(|l| (i, l.native_name().to_owned()))
                                    })
                                    .collect()
                            },
                            language,
                            Some(Icon::Languages),
                            st.t("store.language"),
                            "org-language",
                        )),
                ))
                .child(pair(
                    view()
                        .gap(px(6.0))
                        .child(caption("store.fiscal_start"))
                        .child(dropdown(
                            move || {
                                (1..=12)
                                    .map(|m| (m, st.fmt().month_short(m).to_owned()))
                                    .collect()
                            },
                            fiscal,
                            Some(Icon::Calendar),
                            st.t("store.fiscal_start"),
                            "org-fiscal",
                        )),
                    view()
                        .gap(px(6.0))
                        .child(caption("store.auto_lock"))
                        .child(dropdown(
                            move || {
                                AUTO_LOCK
                                    .iter()
                                    .enumerate()
                                    .map(|(i, m)| {
                                        let t = if *m == 0 {
                                            st.tn("store.auto_lock.never")
                                        } else {
                                            nb_i18n::tr_args(
                                                st.lang(),
                                                "store.auto_lock.min",
                                                &[("count", &st.fmt().integer(i64::from(*m)))],
                                            )
                                        };
                                        (i, t)
                                    })
                                    .collect()
                            },
                            auto_lock,
                            Some(Icon::Lock),
                            st.t("store.auto_lock"),
                            "org-autolock",
                        ))
                        .child(label(
                            st.t("store.auto_lock.help"),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )),
        );
    let legal_card = card()
        .child(card_header_icon(
            Some(Icon::FileText),
            st.t("store.legal"),
            Some(Box::new(st.t("store.legal.help"))),
            view(),
        ))
        .child(divider())
        .child(
            view()
                .p(px(20.0))
                .gap(px(16.0))
                .child(f(
                    "store.legal_name",
                    legal,
                    none,
                    KeyboardType::Default,
                    "store-legal",
                ))
                .child(pair(
                    f("store.nif", nif, nif_err, KeyboardType::Number, "store-nif"),
                    f("store.nis", nis, nis_err, KeyboardType::Number, "store-nis"),
                ))
                .child(pair(
                    f("store.rc", rc, none, KeyboardType::Default, "store-rc"),
                    f("store.ai", ai, none, KeyboardType::Number, "store-ai"),
                ))
                .child(when(
                    move || !form_err.get().is_empty(),
                    move || {
                        label(
                            move || form_err.get(),
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::Danger,
                        )
                        .into_element()
                    },
                )),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .px(px(20.0))
                .py(px(14.0))
                .bg(Tone::Subtle.color())
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Check),
                        st.t("action.save_changes"),
                        save,
                    )
                    .test_id("store-save"),
                ),
        );
    view()
        .gap(px(20.0))
        .child(profile)
        .child(legal_card)
        .child(modules(st, compact))
        .child(valuation_card(st, compact))
}

/// How stock going out is costed (M6.10): saved at once.
fn valuation_card(st: AppState, compact: bool) -> View {
    // 0 = weighted average, 1 = FIFO.
    let method = signal(0usize);
    let loaded = signal(false);
    let task = spawn(async move {
        if let Ok(code) = st.db().call(|s| org_usecases::valuation(s)).await {
            method.set(usize::from(code == "fifo"));
        }
        loaded.set(true);
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
    effect(move || {
        let m = method.get();
        if !loaded.get_untracked() {
            return;
        }
        let Some(session) = st.session.get_untracked() else {
            return;
        };
        let code = if m == 1 { "fifo" } else { "average" };
        let task = spawn(async move {
            match st
                .db()
                .call(move |s| org_usecases::set_valuation(s, session.meta("org.valuation"), code))
                .await
            {
                Ok(_) => show_toast(st, st.tn("settings.saved")),
                Err(e) => show_toast(
                    st,
                    crate::screens::forms::error_text(
                        st,
                        e.codes.first().map_or("internal", |c| c.1),
                    ),
                ),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    });
    group_card(
        st,
        Some((Icon::Package, "valuation.title")),
        vec![row(
            st,
            "valuation.method",
            "valuation.method.help",
            segmented(
                vec![
                    (
                        0,
                        Box::new(st.t("valuation.average")) as Box<dyn Fn() -> String>,
                    ),
                    (1, Box::new(st.t("valuation.fifo"))),
                ],
                method,
            )
            .test_id("valuation-method"),
            compact,
        )],
    )
}

/// The optional modules, switched on by the activity; each can be changed.
fn modules(st: AppState, compact: bool) -> View {
    let profiles = Profiles::builtin();
    let mut rows = Vec::new();
    for &f in Feature::ALL.iter().filter(|f| !profiles.is_core(**f)) {
        let on = signal(st.has(f));
        // Follow the organization's features as they load or change…
        effect(move || {
            let now = st.has(f);
            if on.get_untracked() != now {
                on.set(now);
            }
        });
        // …and save a switch the person flips.
        effect(move || {
            let want = on.get();
            if want == st.features.get_untracked().is_none_or(|s| s.has(f)) {
                return;
            }
            let Some(session) = st.session.get_untracked() else {
                return;
            };
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(move |s| {
                        org_usecases::set_feature(
                            s,
                            session.meta("org.feature"),
                            f.code(),
                            Some(want),
                        )
                    })
                    .await;
                match r {
                    Ok(set) => st.features.set(Some(set)),
                    Err(e) => {
                        on.set(!want);
                        show_toast(
                            st,
                            crate::screens::forms::error_text(
                                st,
                                e.codes.first().map_or("internal", |c| c.1),
                            ),
                        );
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        });
        let key = format!("feature.{}", f.code());
        let label_key = key.clone();
        rows.push(setting_row(
            move || st.tn(&label_key),
            String::new,
            view().child(switch(on, move || st.tn(&key))),
            compact,
        ));
    }
    let mut c = card().child(card_header_icon(
        Some(Icon::Layers),
        st.t("settings.card.modules"),
        Some(Box::new(st.t("settings.card.modules.help"))),
        view(),
    ));
    for r in rows {
        c = c.child(divider()).child(r);
    }
    c
}

fn taxes(st: AppState, compact: bool) -> View {
    let b = st.biz;
    view()
        .gap(px(20.0))
        .child(crate::screens::settings_catalog::tax_classes_card(
            st, compact,
        ))
        .child(group_card(
            st,
            None,
            vec![
                row(
                    st,
                    "tax.included",
                    "tax.included.help",
                    switch(b.prices_incl_tax, st.t("tax.included")),
                    compact,
                ),
                row(
                    st,
                    "tax.stamp",
                    "tax.stamp.help",
                    switch(b.stamp_duty, st.t("tax.stamp")),
                    compact,
                ),
            ],
        ))
}

fn printing(st: AppState, compact: bool) -> View {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::screens::settings_printing::printing_settings(st, compact)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = compact;
        view().child(label(
            st.t("print.unavailable"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
    }
}

fn devices(st: AppState, compact: bool) -> View {
    let b = st.biz;
    view()
        .gap(px(20.0))
        .child(group_card(
            st,
            None,
            vec![
                row(
                    st,
                    "device.scale",
                    "device.scale.help",
                    dropdown(
                        move || {
                            vec![
                                (0usize, st.tn("device.scale.none")),
                                (1, st.tn("device.scale.barcode")),
                                (2, st.tn("device.scale.serial")),
                            ]
                        },
                        b.scale,
                        Some(Icon::Scale),
                        st.t("device.scale"),
                        "scale-select",
                    )
                    .w_full(),
                    compact,
                ),
                row(
                    st,
                    "device.scanner",
                    "device.scanner.help",
                    switch(b.scanner, st.t("device.scanner")),
                    compact,
                ),
            ],
        ))
        .child(crate::screens::settings_catalog::scale_formats_card(
            st, compact,
        ))
}

fn sync(st: AppState, compact: bool) -> View {
    let status = crate::screens::cloud::cloud_card(st, compact);
    let mut v = view().gap(px(20.0)).child(status);
    #[cfg(not(target_arch = "wasm32"))]
    {
        v = v.child(crate::screens::backup::backup_card(st, compact));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = compact;
    }
    v
}

fn about(st: AppState, compact: bool) -> View {
    let line = move |key: &'static str, value: &'static str| {
        view()
            .flex_row()
            .items_center()
            .px(px(20.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(Tone::Border.color())
            .child(
                view()
                    .flex_1()
                    .child(label(st.t(key), ty::SM, FontWeight::MEDIUM, Tone::Muted)),
            )
            .child(figure(
                move || value.to_owned(),
                ty::SM,
                FontWeight::SEMIBOLD,
                || Tone::Ink,
            ))
    };
    let _ = compact;
    view()
        .gap(px(20.0))
        .child(
            card()
                .child(card_header_icon(
                    Some(Icon::Info),
                    st.t("app.name"),
                    None,
                    view(),
                ))
                .child(line("about.version", "0.1.0"))
                .child(line("about.engine", "NSH 0.4.0")),
        )
        .child(
            card()
                .child(card_header_icon(
                    Some(Icon::Book),
                    st.t("about.licenses"),
                    None,
                    view(),
                ))
                .child(divider())
                .child(view().p(px(20.0)).child(label(
                    st.t("about.licenses.body"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Body,
                ))),
        )
}

fn panel(st: AppState, tab: usize, compact: bool) -> View {
    match TABS.get(tab).map(|t| t.0) {
        Some("appearance") => appearance(st, compact),
        Some("formats") => formats(st, compact),
        Some("store") => store(st, compact),
        Some("taxes") => taxes(st, compact),
        Some("printing") => printing(st, compact),
        Some("devices") => devices(st, compact),
        Some("users") => crate::screens::users::users_tab(st, compact),
        Some("sync") => sync(st, compact),
        Some("about") => about(st, compact),
        _ => general(st, compact),
    }
}

/// The settings list at the reading start (desktop).
fn side_nav(st: AppState) -> View {
    let mut nav = view().w(px(240.0)).gap(px(2.0));
    for (g, gkey) in GROUPS.iter().enumerate() {
        nav = nav.child(
            view()
                .px(px(10.0))
                .pt(px(if g == 0 { 0.0 } else { 14.0 }))
                .pb(px(4.0))
                .child(label(st.t(gkey), ty::XS, FontWeight::SEMIBOLD, Tone::Faint)),
        );
        for (i, (code, which, group)) in TABS.iter().copied().enumerate() {
            if group != g {
                continue;
            }
            let active = computed(move || st.settings_tab.get() == i);
            let name = tab_key(code, "");
            nav = nav.child(
                view()
                    .test_id(&format!("settings-{code}"))
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .h(px(sp(36.0)))
                    .px(px(10.0))
                    .rounded(px(radius::md()))
                    .cursor_pointer()
                    .style_when(active, |s| s.bg(Tone::Surface.color()).shadow_xs())
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .on_press(move || st.settings_tab.set(i))
                    .child(dynamic(move || {
                        icon(which, 16.0, move || {
                            if active.get() {
                                Tone::Primary
                            } else {
                                Tone::Muted
                            }
                        })
                        .into_element()
                    }))
                    .child(
                        label_line(move || st.tn(&name), ty::SM, FontWeight::MEDIUM, Tone::Body)
                            .style_when(active, |s| s.font_weight(FontWeight::SEMIBOLD))
                            .text_color(move || {
                                if active.get() {
                                    Tone::Ink.color()
                                } else {
                                    Tone::Body.color()
                                }
                            }),
                    ),
            );
        }
    }
    nav
}

/// The settings as chips in a scrolling row (phones).
fn top_tabs(st: AppState) -> View {
    let mut row = view().flex_row().gap(px(6.0));
    for (i, (code, which, _)) in TABS.iter().copied().enumerate() {
        let active = computed(move || st.settings_tab.get() == i);
        let name = tab_key(code, "");
        row = row.child(
            view()
                .test_id(&format!("settings-{code}"))
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(34.0))
                .px(px(12.0))
                .rounded_full()
                .border_1()
                .border_color(move || {
                    if active.get() {
                        Tone::Primary.color()
                    } else {
                        Tone::Border.color()
                    }
                })
                .bg(move || {
                    if active.get() {
                        Tone::PrimarySoft.color()
                    } else {
                        Tone::Surface.color()
                    }
                })
                .on_press(move || st.settings_tab.set(i))
                .child(icon(which, 14.0, move || {
                    if active.get() {
                        Tone::Primary
                    } else {
                        Tone::Muted
                    }
                }))
                .child(
                    label_line(move || st.tn(&name), ty::SM, FontWeight::MEDIUM, Tone::Body)
                        .text_color(move || {
                            if active.get() {
                                Tone::Primary.color()
                            } else {
                                Tone::Body.color()
                            }
                        }),
                ),
        );
    }
    row
}

pub fn settings(st: AppState, compact: bool) -> View {
    let p = st.prefs;
    let pad = page_pad(compact);
    let header = page_header_full(
        Some(Icon::Settings),
        None,
        st.t("settings.title"),
        Some(Box::new(st.t("settings.subtitle"))),
        view().child(button(
            Variant::Secondary,
            Size::Md,
            Some(Icon::Refresh),
            st.t("settings.reset"),
            move || p.reset(),
        )),
        compact,
    );
    // The open tab's title and description, then its panel.
    let current = move || {
        let i = st.settings_tab.get();
        let (code, which, _) = TABS.get(i).copied().unwrap_or(TABS[0]);
        let title = tab_key(code, "");
        let desc = tab_key(code, ".desc");
        view()
            .gap(px(16.0))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(icon(which, 20.0, || Tone::Primary))
                    .child(
                        view()
                            .child(label(
                                move || st.tn(&title),
                                ty::XL,
                                FontWeight::BOLD,
                                Tone::Ink,
                            ))
                            .child(label(
                                move || st.tn(&desc),
                                ty::SM,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    ),
            )
            .child(panel(st, i, compact))
            .into_element()
    };
    if compact {
        view().flex_1().child(
            scroll().flex_1().child(
                view()
                    .w_full()
                    .p(px(pad))
                    .gap(px(16.0))
                    .child(header)
                    .child(scroll().horizontal().child(top_tabs(st)))
                    .child(dynamic(current)),
            ),
        )
    } else {
        view()
            .flex_1()
            .min_h(px(0.0))
            .child(view().px(px(pad)).pt(px(pad)).pb(px(20.0)).child(header))
            .child(
                view()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex_row()
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(
                        scroll().w(px(240.0 + pad + 16.0)).child(
                            view()
                                .ps(px(pad))
                                .pe(px(16.0))
                                .py(px(20.0))
                                .child(side_nav(st)),
                        ),
                    )
                    .child(
                        scroll().flex_1().child(
                            view()
                                .pe(px(pad))
                                .ps(px(8.0))
                                .py(px(20.0))
                                .max_w(px(980.0))
                                .child(dynamic(current)),
                        ),
                    ),
            )
    }
}

//! Settings, in tabs grouped like a control panel: preferences (general,
//! appearance, numbers), business (store, taxes), hardware (printing,
//! devices) and administration (users, sync, about). Every change applies at
//! once and is saved on the device; appearance has a live preview.

use nb_core::{Currency, Money};
use nb_i18n::{Digits, Lang};
use nsh_core::element::{SharedString, View};
use nsh_core::prelude::*;

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
fn group_card(st: AppState, head: Option<(Icon, &'static str)>, rows: Vec<View>) -> View {
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

fn row(st: AppState, name: &'static str, help: &'static str, control: View, compact: bool) -> View {
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

fn store(st: AppState, compact: bool) -> View {
    let b = st.biz;
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
    let f = move |key: &'static str, sig: Signal<String>, kb: KeyboardType, id: &'static str| {
        field(st.t(key), sig, String::new, String::new, kb, id)
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
                        b.store_name,
                        KeyboardType::Default,
                        "store-name",
                    ),
                    f("store.phone", b.phone, KeyboardType::Phone, "store-phone"),
                ))
                .child(f(
                    "store.address",
                    b.address,
                    KeyboardType::Default,
                    "store-address",
                ))
                .child(
                    view()
                        .gap(px(6.0))
                        .child(label(
                            st.t("store.activity"),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(dropdown(
                            move || {
                                [
                                    "activity.grocery",
                                    "activity.wholesale",
                                    "activity.distribution",
                                    "activity.restaurant",
                                    "activity.clothing",
                                    "activity.pharmacy",
                                ]
                                .iter()
                                .enumerate()
                                .map(|(i, k)| (i, st.tn(k)))
                                .collect()
                            },
                            b.activity,
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
                ),
        );
    let legal = card()
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
                .child(pair(
                    f("store.nif", b.nif, KeyboardType::Number, "store-nif"),
                    f("store.nis", b.nis, KeyboardType::Number, "store-nis"),
                ))
                .child(pair(
                    f("store.rc", b.rc, KeyboardType::Default, "store-rc"),
                    f("store.ai", b.ai, KeyboardType::Number, "store-ai"),
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
                .child(button(
                    Variant::Primary,
                    Size::Md,
                    Some(Icon::Check),
                    st.t("action.save_changes"),
                    move || show_toast(st, st.tn("settings.saved")),
                )),
        );
    view().gap(px(20.0)).child(profile).child(legal)
}

fn taxes(st: AppState, compact: bool) -> View {
    let b = st.biz;
    view().child(group_card(
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
                "tax.default_rate",
                "tax.default_rate.help",
                segmented(
                    vec![
                        (
                            0usize,
                            Box::new(|| "19 %".to_owned()) as Box<dyn Fn() -> String>,
                        ),
                        (1, Box::new(|| "9 %".to_owned())),
                        (2, Box::new(st.t("tax.exempt"))),
                    ],
                    b.tva_rate,
                ),
                compact,
            ),
            row(
                st,
                "tax.stamp",
                "tax.stamp.help",
                switch(b.stamp_duty, st.t("tax.stamp")),
                compact,
            ),
            row(
                st,
                "tax.on_receipt",
                "tax.on_receipt.help",
                switch(b.tax_on_receipt, st.t("tax.on_receipt")),
                compact,
            ),
        ],
    ))
}

fn printing(st: AppState, compact: bool) -> View {
    let b = st.biz;
    view().gap(px(20.0)).child(group_card(
        st,
        None,
        vec![
            row(
                st,
                "print.printer",
                "print.printer.help",
                dropdown(
                    move || {
                        vec![
                            (0usize, st.tn("print.none")),
                            (1, "XPrinter XP-58 · USB".to_owned()),
                            (2, "Sunmi V2 · Bluetooth".to_owned()),
                            (3, "Epson TM-T20 · 192.168.1.50".to_owned()),
                        ]
                    },
                    b.printer,
                    Some(Icon::Printer),
                    st.t("print.printer"),
                    "printer-select",
                )
                .w_full(),
                compact,
            ),
            row(
                st,
                "print.width",
                "print.width.help",
                segmented(plain_choices(&["58 mm", "80 mm"]), b.receipt_width),
                compact,
            ),
            row(
                st,
                "print.copies",
                "print.copies.help",
                segmented(plain_choices(&["1", "2"]), b.copies),
                compact,
            ),
            row(
                st,
                "print.logo",
                "print.logo.help",
                switch(b.print_logo, st.t("print.logo")),
                compact,
            ),
            row(
                st,
                "print.auto",
                "print.auto.help",
                switch(b.auto_print, st.t("print.auto")),
                compact,
            ),
            view()
                .px(px(20.0))
                .py(px(14.0))
                .flex_row()
                .justify_end()
                .child(button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Printer),
                    st.t("action.test_print"),
                    || {},
                )),
        ],
    ))
}

fn devices(st: AppState, compact: bool) -> View {
    let b = st.biz;
    view().child(group_card(
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
            row(
                st,
                "device.drawer",
                "device.drawer.help",
                switch(b.drawer, st.t("device.drawer")),
                compact,
            ),
        ],
    ))
}

fn users(st: AppState, compact: bool) -> View {
    let people: [(&str, &str, &str, Status, &str); 4] = [
        (
            "أحمد بن علي",
            "Ahmed Benali",
            "role.owner",
            Status::Accent,
            "10/11 · 22:40",
        ),
        (
            "سارة بلقاسم",
            "Sara Belkacem",
            "role.cashier",
            Status::Info,
            "10/11 · 19:05",
        ),
        (
            "يوسف مداني",
            "Youcef Madani",
            "role.rep",
            Status::Success,
            "09/11 · 17:32",
        ),
        (
            "ليلى قاسمي",
            "Leila Kacemi",
            "role.accountant",
            Status::Neutral,
            "07/11 · 11:10",
        ),
    ];
    let mut list = card().child(card_header_icon(
        Some(Icon::Users),
        st.t("settings.card.team"),
        None,
        view().child(button(
            Variant::Primary,
            Size::Sm,
            Some(Icon::Plus),
            st.t("action.invite"),
            || {},
        )),
    ));
    for (ar, fr, role, status, seen) in people {
        let name = move || {
            if st.lang() == Lang::Ar {
                ar.to_owned()
            } else {
                fr.to_owned()
            }
        };
        let mut r = view()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .px(px(20.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(Tone::Border.color())
            .child(avatar(name, 34.0, status))
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(label_line(name, ty::SM, FontWeight::SEMIBOLD, Tone::Ink))
                    .child(label_line(
                        st.t_with("users.last_seen", move |_| {
                            vec![("when", nb_i18n::bidi::isolate_ltr(seen))]
                        }),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )),
            )
            .child(badge(status, None, st.t(role)));
        if !compact {
            r = r.child(badge(
                Status::Success,
                Some(Icon::Check),
                st.t("users.active"),
            ));
        }
        list = list.child(r);
    }
    list
}

fn sync(st: AppState, compact: bool) -> View {
    let stat = move |key: &'static str, value: Box<dyn Fn() -> String>| {
        view()
            .flex_1()
            .min_w(px(150.0))
            .gap(px(4.0))
            .child(label(st.t(key), ty::XS, FontWeight::MEDIUM, Tone::Muted))
            .child(label(value, ty::MD, FontWeight::SEMIBOLD, Tone::Ink))
    };
    let status = card()
        .child(card_header_icon(
            Some(Icon::CloudUpload),
            st.t("sync.status"),
            Some(Box::new(st.t("sync.local_first"))),
            view().child(badge(
                Status::Warning,
                Some(Icon::Offline),
                st.t("sync.offline"),
            )),
        ))
        .child(divider())
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(20.0))
                .p(px(20.0))
                .child(stat(
                    "sync.pending_ops",
                    Box::new(move || st.fmt().integer(0)),
                ))
                .child(stat("sync.last", Box::new(st.t("sync.never"))))
                .child(stat(
                    "sync.device",
                    Box::new(|| "Linux · NB-7F3A".to_owned()),
                )),
        )
        .child(
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
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Upload),
                    st.t("action.restore"),
                    || {},
                ))
                .child(button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::HardDrive),
                    st.t("action.backup"),
                    || {},
                ))
                .child(button(
                    Variant::Primary,
                    Size::Md,
                    Some(Icon::Refresh),
                    st.t("action.sync_now"),
                    || {},
                )),
        );
    let _ = compact;
    status
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
        Some("users") => users(st, compact),
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

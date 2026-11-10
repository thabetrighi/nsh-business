//! Settings: language, appearance (theme, accent, font, text size, density,
//! corners) and number format. Every change applies at once and is saved
//! on the device; a live preview shows the result.

use nb_core::{Currency, Money};
use nb_i18n::{Digits, Lang};
use nsh_core::element::{SharedString, View};
use nsh_core::prelude::*;

use crate::shell::user_name;
use crate::state::{AppState, ThemeMode};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, avatar, badge, button, divider, figure, label, label_line, segmented,
};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header, delta, page_header, page_pad, setting_row};
use crate::ui::tokens::{Accent, Corners, Density, Status, TextSize, Tone, UiFont, ty};

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

pub fn settings(st: AppState, compact: bool) -> View {
    let p = st.prefs;
    let tn = move |k: String| move || st.tn(&k);

    let general = card()
        .child(card_header(st.t("settings.section.general"), None, view()))
        .child(divider())
        .child(setting_row(
            st.t("settings.language"),
            st.t("settings.language.help"),
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
        ))
        .child(divider())
        .child(setting_row(
            st.t("settings.theme"),
            st.t("settings.theme.help"),
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
        ));

    let appearance = card()
        .child(card_header(
            st.t("settings.section.appearance"),
            None,
            view(),
        ))
        .child(divider())
        .child(setting_row(
            st.t("settings.accent"),
            st.t("settings.accent.help"),
            accent_swatches(st),
            compact,
        ))
        .child(divider())
        .child(setting_row(
            st.t("settings.font"),
            st.t("settings.font.help"),
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
        ))
        .child(divider())
        .child(setting_row(
            st.t("settings.text_size"),
            st.t("settings.text_size.help"),
            segmented(
                options(st, &TextSize::ALL, move |t| {
                    st.tn(&format!("size.{}", t.code()))
                }),
                p.text,
            ),
            compact,
        ))
        .child(divider())
        .child(setting_row(
            st.t("settings.density"),
            st.t("settings.density.help"),
            segmented(
                options(st, &Density::ALL, move |d| {
                    st.tn(&format!("density.{}", d.code()))
                }),
                p.density,
            ),
            compact,
        ))
        .child(divider())
        .child(setting_row(
            st.t("settings.corners"),
            st.t("settings.corners.help"),
            segmented(
                options(st, &Corners::ALL, move |c| {
                    st.tn(&format!("corners.{}", c.code()))
                }),
                p.corners,
            ),
            compact,
        ));

    let formats = card()
        .child(card_header(st.t("settings.section.formats"), None, view()))
        .child(divider())
        .child(setting_row(
            st.t("settings.digits"),
            st.t("settings.digits.help"),
            segmented(
                vec![
                    (
                        Digits::Latin,
                        Box::new(tn("digits.latin".into())) as Box<dyn Fn() -> String>,
                    ),
                    (Digits::ArabicIndic, Box::new(tn("digits.arabic".into()))),
                ],
                p.digits,
            ),
            compact,
        ));

    let gap = if compact { 16.0 } else { 24.0 };
    let main = view()
        .gap(px(gap))
        .child(general)
        .child(appearance)
        .child(formats);
    let body = if compact {
        main.child(preview(st))
    } else {
        view()
            .flex_row()
            .items_start()
            .gap(px(gap))
            .child(main.flex_grow(2.0).basis(px(0.0)).min_w(px(0.0)))
            .child(
                view()
                    .flex_grow(1.0)
                    .basis(px(0.0))
                    .min_w(px(280.0))
                    .child(preview(st)),
            )
    };
    view()
        .w_full()
        .p(px(page_pad(compact)))
        .gap(px(gap))
        .child(page_header(
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
        ))
        .child(body)
}

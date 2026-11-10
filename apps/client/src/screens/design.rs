//! The design system, live: colors, type scale, buttons, badges, fields and
//! choices, in the current theme, language and settings. Used to review
//! the system (gate G1) and to check a change everywhere at once.

use nb_core::{Currency, Money};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::state::AppState;
use crate::ui::chart::sparkline;
use crate::ui::form::{field, search_field};
use crate::ui::icon::Icon;
use crate::ui::kit::{
    Size, Variant, avatar, badge, button, caption, checkbox, figure, icon_button, kbd, label,
    menu_button, segmented, switch,
};
use crate::ui::popup::{Action, dropdown, row_menu};
use crate::ui::surface::{
    card, card_body, card_header, delta, empty_state, kpi, page_header_full, page_pad,
};
use crate::ui::tokens::{Status, Tone, radius, ty};

fn section(title: impl Fn() -> String + 'static, body: View) -> View {
    card()
        .child(card_header(title, None, view()))
        .child(card_body().child(body))
}

fn swatch(name: &'static str, tone: Tone) -> View {
    view()
        .w(px(132.0))
        .gap(px(6.0))
        .child(
            view()
                .h(px(48.0))
                .rounded(px(radius::md()))
                .bg(tone.color())
                .border_1()
                .border_color(Tone::Border.color()),
        )
        .child(caption(name, ty::XS, FontWeight::SEMIBOLD, Tone::Ink))
        .child(label(
            move || tone.hex(),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
}

pub fn design(st: AppState, compact: bool) -> View {
    let gap = if compact { 16.0 } else { 24.0 };

    let mut colors = view().flex_row().flex_wrap().gap(px(14.0));
    for (n, t) in [
        ("Canvas", Tone::Canvas),
        ("Sidebar", Tone::Sidebar),
        ("Subtle", Tone::Subtle),
        ("Border", Tone::Border),
        ("Ink", Tone::Ink),
        ("Body", Tone::Body),
        ("Muted", Tone::Muted),
        ("Primary", Tone::Primary),
        ("Primary soft", Tone::PrimarySoft),
        ("Success", Tone::Success),
        ("Warning", Tone::Warning),
        ("Danger", Tone::Danger),
        ("Info", Tone::Info),
    ] {
        colors = colors.child(swatch(n, t));
    }

    let mut type_scale = view().gap(px(12.0));
    for (n, size, w) in [
        ("Display 34", ty::DISPLAY, FontWeight::BOLD),
        ("H1 28", ty::H1, FontWeight::BOLD),
        ("H2 24", ty::H2, FontWeight::BOLD),
        ("XL 18", ty::XL, FontWeight::SEMIBOLD),
        ("LG 16", ty::LG, FontWeight::SEMIBOLD),
        ("MD 14", ty::MD, FontWeight::NORMAL),
        ("SM 13", ty::SM, FontWeight::NORMAL),
        ("XS 12", ty::XS, FontWeight::NORMAL),
    ] {
        type_scale = type_scale.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(16.0))
                .child(view().w(px(90.0)).child(caption(
                    n,
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )))
                .child(view().flex_1().min_w(px(0.0)).child(
                    label(st.t("design.sample_text"), size, w, Tone::Ink).number_of_lines(1),
                )),
        );
    }
    type_scale = type_scale.child(
        view()
            .flex_row()
            .items_center()
            .gap(px(16.0))
            .child(view().w(px(90.0)).child(caption(
                "Figures",
                ty::XS,
                FontWeight::MEDIUM,
                Tone::Muted,
            )))
            .child(figure(
                move || {
                    st.fmt()
                        .money(Money::from_minor(123_456_789, Currency::DZD))
                },
                ty::XL,
                FontWeight::SEMIBOLD,
                || Tone::Ink,
            )),
    );

    let mut buttons = view().gap(px(14.0));
    for size in [Size::Sm, Size::Md, Size::Lg] {
        buttons = buttons.child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(10.0))
                .child(button(
                    Variant::Primary,
                    size,
                    Some(Icon::Plus),
                    st.t("design.primary"),
                    || {},
                ))
                .child(button(
                    Variant::Secondary,
                    size,
                    Some(Icon::Download),
                    st.t("design.secondary"),
                    || {},
                ))
                .child(button(
                    Variant::Ghost,
                    size,
                    None,
                    st.t("design.ghost"),
                    || {},
                ))
                .child(button(
                    Variant::Danger,
                    size,
                    Some(Icon::Trash),
                    st.t("design.danger"),
                    || {},
                )),
        );
    }
    buttons = buttons.child(
        view()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(icon_button(
                || Icon::Pencil,
                st.t("action.edit"),
                Variant::Secondary,
                || {},
            ))
            .child(icon_button(
                || Icon::Bell,
                st.t("action.notifications"),
                Variant::Ghost,
                || {},
            ))
            .child(menu_button(Some(Icon::Filter), st.t("action.filters")))
            .child(row_menu(
                move || {
                    vec![
                        Action::new(Icon::Pencil, st.tn("action.edit"), || {}),
                        Action::new(Icon::Copy, st.tn("action.duplicate"), || {}),
                        Action::new(Icon::Trash, st.tn("action.delete"), || {}).danger(),
                    ]
                },
                st.t("action.row_actions"),
            ))
            .child(kbd(st.t("search.shortcut"))),
    );

    let badges = view()
        .flex_row()
        .flex_wrap()
        .gap(px(8.0))
        .child(badge(
            Status::Success,
            Some(Icon::Check),
            st.t("status.paid"),
        ))
        .child(badge(
            Status::Info,
            Some(Icon::Clock),
            st.t("status.partial"),
        ))
        .child(badge(Status::Neutral, None, st.t("status.unpaid")))
        .child(badge(
            Status::Danger,
            Some(Icon::Alert),
            st.t("status.overdue"),
        ))
        .child(badge(Status::Warning, None, st.t("stock.low")))
        .child(badge(Status::Accent, None, st.t("design.primary")))
        .child(delta(1_240, move || st.fmt().percent_signed(1_240)))
        .child(delta(-310, move || st.fmt().percent_signed(-310)))
        .child(avatar(|| "Karim Bouzid".to_owned(), 32.0, Status::Accent))
        .child(avatar(|| "مخبزة الأمل".to_owned(), 32.0, Status::Success));

    let text = signal(String::new());
    let q = signal(String::new());
    let on = signal(true);
    let checked = signal(true);
    let choice = signal(1usize);
    let pick = signal(0usize);
    let inputs = view()
        .gap(px(16.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(16.0))
                .child(view().w(px(280.0)).child(field(
                    st.t("products.col.name"),
                    text,
                    st.t("products.form.name_hint"),
                    String::new,
                    KeyboardType::Default,
                    "design-field",
                )))
                .child(view().w(px(280.0)).child(field(
                    st.t("products.col.price"),
                    signal("12a".to_owned()),
                    String::new,
                    move || st.tn("products.form.price_invalid"),
                    KeyboardType::Decimal,
                    "design-field-error",
                ))),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(16.0))
                .child(view().w(px(280.0)).child(search_field(
                    q,
                    st.t("products.search"),
                    "design-search",
                )))
                .child(view().w(px(220.0)).child(dropdown(
                    move || {
                        vec![
                            (0usize, st.tn("period.7d")),
                            (1, st.tn("period.30d")),
                            (2, st.tn("period.12m")),
                        ]
                    },
                    pick,
                    Some(Icon::Calendar),
                    st.t("period.label"),
                    "design-dropdown",
                ))),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(20.0))
                .child(segmented(
                    vec![
                        (
                            0usize,
                            Box::new(st.t("period.today")) as Box<dyn Fn() -> String>,
                        ),
                        (1, Box::new(st.t("period.7d"))),
                        (2, Box::new(st.t("period.30d"))),
                    ],
                    choice,
                ))
                .child(switch(on, st.t("settings.theme")))
                .child(checkbox(
                    move || Some(checked.get()),
                    move || checked.update(|c| *c = !*c),
                ))
                .child(checkbox(|| None, || {})),
        );

    let kpis = view()
        .flex_row()
        .flex_wrap()
        .gap(px(16.0))
        .child(kpi(
            st.t("kpi.revenue"),
            move || {
                st.fmt()
                    .money_whole(Money::from_minor(128_450_000, Currency::DZD))
            },
            Some(delta(1_240, move || st.fmt().percent_signed(1_240))),
            sparkline(
                vec![3, 5, 4, 6, 8, 7, 9, 12, 10, 13],
                160.0,
                36.0,
                Tone::Primary,
                st.rtl(),
            ),
            compact,
        ))
        .child(card().flex_1().min_w(px(260.0)).child(empty_state(
            Icon::Search,
            st.t("table.empty.title"),
            st.t("table.empty.body"),
            None,
        )));

    view()
        .w_full()
        .p(px(page_pad(compact)))
        .gap(px(gap))
        .child(page_header_full(
            Some(Icon::Palette),
            None,
            st.t("nav.design"),
            Some(Box::new(st.t("design.subtitle"))),
            view(),
            compact,
        ))
        .child(section(st.t("design.colors"), colors))
        .child(section(st.t("design.type"), type_scale))
        .child(section(st.t("design.buttons"), buttons))
        .child(section(st.t("design.badges"), badges))
        .child(section(st.t("design.inputs"), inputs))
        .child(kpis)
}

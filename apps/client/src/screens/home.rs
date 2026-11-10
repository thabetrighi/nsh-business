//! Home: today's money at a glance, who owes you, what runs out.
//!
//! The figures are SAMPLE DATA until the sales and finance modules (M8, M9)
//! feed real projections. Amounts are integer centimes (`Money`).

use nb_core::{Currency, Money, Quantity};
use nb_i18n::Lang;
use nsh_core::prelude::*;

use crate::state::AppState;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{figure, initials, label, label_line, quiet_button, rule, size};
use crate::ui::palette::Tone;

const DZD: Currency = Currency::DZD;

struct Debtor {
    ar: &'static str,
    fr: &'static str,
    owed_minor: i64,
    overdue_days: i64,
}

struct LowStock {
    ar: &'static str,
    fr: &'static str,
    left: i64,
}

const DEBTORS: [Debtor; 4] = [
    Debtor {
        ar: "كريم بوزيد",
        fr: "Karim Bouzid",
        owed_minor: 1_250_000,
        overdue_days: 45,
    },
    Debtor {
        ar: "مخبزة الأمل",
        fr: "Boulangerie El Amel",
        owed_minor: 860_000,
        overdue_days: 12,
    },
    Debtor {
        ar: "سمير حداد",
        fr: "Samir Haddad",
        owed_minor: 340_050,
        overdue_days: 3,
    },
    Debtor {
        ar: "محل الوفاء",
        fr: "Magasin El Wafa",
        owed_minor: 215_000,
        overdue_days: 0,
    },
];

const LOW_STOCK: [LowStock; 3] = [
    LowStock {
        ar: "حليب Soummam 1L",
        fr: "Lait Soummam 1L",
        left: 6,
    },
    LowStock {
        ar: "زيت مائدة Elio 5L",
        fr: "Huile Elio 5L",
        left: 2,
    },
    LowStock {
        ar: "سكر أبيض 1kg",
        fr: "Sucre blanc 1kg",
        left: 9,
    },
];

fn pick(st: AppState, ar: &'static str, fr: &'static str) -> impl Fn() -> String + 'static {
    move || {
        let s = if st.lang() == Lang::Ar { ar } else { fr };
        nb_i18n::bidi::isolate_runs(s, st.lang().direction())
    }
}

pub fn home(st: AppState, compact: bool) -> impl IntoElement {
    let pad = if compact { 16.0 } else { 32.0 };
    let lists = if compact {
        view().gap(px(16.0))
    } else {
        view().flex_row().gap(px(24.0)).items_start()
    };
    view()
        .w_full()
        .max_w(px(1120.0))
        .p(px(pad))
        .gap(px(if compact { 16.0 } else { 24.0 }))
        .child(header(st, compact))
        .child(today(st, compact))
        .child(lists.child(owed(st, compact)).child(low_stock(st, compact)))
}

fn header(st: AppState, compact: bool) -> impl IntoElement {
    let greeting = st.t_with("dashboard.greeting", |s| {
        vec![(
            "name",
            if s.lang() == Lang::Ar {
                "أحمد".into()
            } else {
                "Ahmed".into()
            },
        )]
    });
    let date = move || format!("{} {}", tr_today(st), st.fmt().date(2026, 11, 10));
    let row = view().flex_row().items_center().gap(px(16.0)).child(
        view()
            .flex_1()
            .gap(px(2.0))
            .child(label(
                greeting,
                if compact { size::TITLE } else { size::HEADLINE },
                FontWeight::BOLD,
                Tone::Ink,
            ))
            .child(label(date, size::BODY, FontWeight::NORMAL, Tone::Muted)),
    );
    // On wide screens the sidebar holds the one "start a sale" action; phones use the centre tab.
    let _ = compact;
    row
}

fn tr_today(st: AppState) -> String {
    nb_i18n::tr(st.lang(), "dashboard.today").into_owned()
}

/// The memorable element: today's takings as a large ledger figure, then
/// the four money movements that make it up, as ledger lines.
fn today(st: AppState, compact: bool) -> impl IntoElement {
    let total = Money::from_minor(4_875_000, DZD);
    let summary = st.t_with("dashboard.invoices_count", |s| {
        vec![("count", s.fmt().integer(37))]
    });
    let delta = st.t_with("dashboard.vs_yesterday", |_| vec![("delta", "+12%".into())]);
    // `signed`: cash movements show + / −; credit sales are not cash, so no sign.
    let line = move |key: &'static str, ico: Icon, minor: i64, tone: Tone, signed: bool| {
        view()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .py(px(12.0))
            .child(
                view()
                    .w(px(32.0))
                    .h(px(32.0))
                    .rounded(px(8.0))
                    .items_center()
                    .justify_center()
                    .bg(Tone::Paper.color())
                    .child(icon(ico, 18.0, move || tone)),
            )
            .child(view().flex_1().child(label(
                st.t(key),
                size::BODY,
                FontWeight::NORMAL,
                Tone::Ink,
            )))
            .child(figure(
                move || {
                    let m = Money::from_minor(minor, DZD);
                    if signed {
                        st.fmt().money_signed(m)
                    } else {
                        st.fmt().money(m)
                    }
                },
                size::LEAD,
                FontWeight::SEMIBOLD,
                move || tone,
            ))
    };
    view()
        .p(px(if compact { 16.0 } else { 24.0 }))
        .rounded(px(12.0))
        .bg(Tone::Surface.color())
        .border_1()
        .border_color(Tone::Line.color())
        .child(label(
            st.t("dashboard.sales_today"),
            size::BODY,
            FontWeight::MEDIUM,
            Tone::Muted,
        ))
        .child(figure(
            move || st.fmt().money(total),
            if compact { 32.0 } else { size::FIGURE },
            FontWeight::BOLD,
            || Tone::Ink,
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(16.0))
                .pb(px(12.0))
                .child(label(summary, size::BODY, FontWeight::NORMAL, Tone::Muted))
                .child(label(delta, size::BODY, FontWeight::MEDIUM, Tone::Paid)),
        )
        .child(rule())
        .child(line(
            "dashboard.cash_in",
            Icon::Banknote,
            3_915_000,
            Tone::Paid,
            true,
        ))
        .child(rule())
        .child(line(
            "dashboard.credit_given",
            Icon::Out,
            960_000,
            Tone::Owed,
            false,
        ))
        .child(rule())
        .child(line(
            "dashboard.collected",
            Icon::Coins,
            420_000,
            Tone::Paid,
            true,
        ))
        .child(rule())
        .child(line(
            "dashboard.expenses",
            Icon::Wallet,
            -185_000,
            Tone::Owed,
            true,
        ))
}

fn panel(title: impl IntoElement, body: impl IntoElement) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .rounded(px(12.0))
        .bg(Tone::Surface.color())
        .border_1()
        .border_color(Tone::Line.color())
        .child(view().px_5().pt_5().pb_3().child(title))
        .child(body)
}

fn owed(st: AppState, compact: bool) -> impl IntoElement {
    let total: i64 = DEBTORS.iter().map(|d| d.owed_minor).sum();
    let mut rows = view().px_5().pb_2();
    for (i, d) in DEBTORS.iter().enumerate() {
        if i > 0 {
            rows = rows.child(rule());
        }
        let overdue = d.overdue_days;
        let due = st.t_with("dashboard.overdue", move |s| {
            vec![("days", s.fmt().integer(overdue))]
        });
        let late = move || if overdue > 0 { due() } else { String::new() };
        rows = rows.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .py(px(12.0))
                .child(initials(
                    pick(st, d.ar, d.fr),
                    if overdue > 30 {
                        Tone::Owed
                    } else {
                        Tone::Primary
                    },
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label_line(
                            pick(st, d.ar, d.fr),
                            size::BODY,
                            FontWeight::MEDIUM,
                            Tone::Ink,
                        ))
                        .child(label(
                            late,
                            size::CAPTION,
                            FontWeight::NORMAL,
                            if overdue > 30 { Tone::Owed } else { Tone::Warn },
                        )),
                )
                .child(figure(
                    {
                        let minor = d.owed_minor;
                        move || st.fmt().money(Money::from_minor(minor, DZD))
                    },
                    size::BODY,
                    FontWeight::SEMIBOLD,
                    || Tone::Owed,
                ))
                .child(action(st, compact, st.t("dashboard.collect"))),
        );
    }
    panel(
        view()
            .gap(px(4.0))
            .child(label(
                st.t("dashboard.owed_title"),
                size::LEAD,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            ))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(label(
                        st.t("dashboard.owed_total"),
                        size::BODY,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    ))
                    .child(figure(
                        move || st.fmt().money(Money::from_minor(total, DZD)),
                        size::TITLE,
                        FontWeight::BOLD,
                        || Tone::Owed,
                    )),
            ),
        rows,
    )
}

fn low_stock(st: AppState, compact: bool) -> impl IntoElement {
    let mut rows = view().px_5().pb_2();
    for (i, p) in LOW_STOCK.iter().enumerate() {
        if i > 0 {
            rows = rows.child(rule());
        }
        let left = p.left;
        let tone = if left <= 3 { Tone::Owed } else { Tone::Warn };
        let remaining = st.t_with("dashboard.left", move |s| {
            vec![("qty", s.fmt().quantity(Quantity::whole(left), 0))]
        });
        rows = rows.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .py(px(12.0))
                .child(icon(Icon::Alert, 18.0, move || tone))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(label_line(
                            pick(st, p.ar, p.fr),
                            size::BODY,
                            FontWeight::MEDIUM,
                            Tone::Ink,
                        ))
                        .child(label(remaining, size::CAPTION, FontWeight::NORMAL, tone)),
                )
                .child(action(st, compact, st.t("dashboard.reorder"))),
        );
    }
    panel(
        label(
            st.t("dashboard.low_stock_title"),
            size::LEAD,
            FontWeight::SEMIBOLD,
            Tone::Ink,
        ),
        rows,
    )
}

/// Row action: a quiet button on wide screens; on phones the row itself is
/// the target (space goes to the name and the amount), shown by a chevron
/// pointing to the reading end.
fn action(st: AppState, compact: bool, caption: impl Fn() -> String + 'static) -> AnyElement {
    if compact {
        dynamic(move || {
            let chevron = if st.rtl() {
                Icon::ChevronLeft
            } else {
                Icon::ChevronRight
            };
            icon(chevron, 18.0, || Tone::Muted).into_element()
        })
        .into_element()
    } else {
        quiet_button(caption, || {}).into_element()
    }
}

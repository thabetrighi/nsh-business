//! The price display (owner notes): a screen for customers on a device of
//! its own — scan a product, or type its name, and its price shows large;
//! after a while it goes back to the welcome. It takes the whole screen
//! (no sidebar, no top bar) and is left with a button or Escape, behind a
//! sign-in when the device asks for one. It can open by itself when the
//! device starts.

use std::time::Duration;

use nb_core::{Money, Quantity};
use nb_i18n::{Lang, tr_args};
use nb_store::repo::catalog::{ProductPage, ProductQuery, ProductRow, ProductSort};
use nb_store::usecases::catalog::{Scan, scan};
use nb_store::usecases::printing::print_shop;
use nb_store::usecases::products::list_products;
use nsh_core::element::{View, element_ref, input};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::state::{AppState, persisted_flag};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, button, figure, icon_button, label, label_line, switch};
use crate::ui::overlay::{dialog_frame, modal};
use crate::ui::tokens::{Tone, radius, ty};

/// Seconds without a touch before the welcome shows again.
const IDLE_SECS: u64 = 25;

/// The storage key of "open at start".
pub const AT_START: &str = "nb.price_check.at_start";

/// What the screen shows.
#[derive(Debug, Clone, PartialEq)]
enum Shown {
    Welcome,
    One(Box<ProductRow>, Option<i64>),
    Many(Vec<ProductRow>),
    NotFound(String),
}

pub fn price_check(st: AppState, compact: bool) -> View {
    // The whole screen: no sidebar, no bar at the top.
    st.focus.set(true);
    st.bare.set(true);
    cleanup(move || {
        st.focus.set(false);
        st.bare.set(false);
    });
    let text = signal(String::new());
    let shown = signal(Shown::Welcome);
    let busy = signal(false);
    let leaving = signal(false);
    let last = signal(now_secs());
    let field = element_ref();
    let shop = resource(
        move || st.session.get().map(|_| ()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| print_shop(s).map(|p| p.name)).await }
        },
    );
    // Back to the welcome after a while; the field keeps the keyboard.
    let tick = set_interval(Duration::from_secs(1), move || {
        if !leaving.get_untracked() {
            field.focus();
        }
        if shown.with_untracked(|s| *s != Shown::Welcome)
            && now_secs().saturating_sub(last.get_untracked()) >= IDLE_SECS
        {
            shown.set(Shown::Welcome);
            text.set(String::new());
        }
    });
    cleanup(move || drop(tick));
    let look = move |code: String| {
        let code = code.trim().to_owned();
        if code.is_empty() || busy.get_untracked() {
            return;
        }
        busy.set(true);
        last.set(now_secs());
        let db = st.db();
        let task = spawn(async move {
            let q = code.clone();
            let r = db
                .call(move |s| {
                    Ok(match scan(s, &q)? {
                        Scan::Item { product, .. } | Scan::Weighed { product, .. } => {
                            Shown::One(Box::new(product), None)
                        }
                        Scan::Priced {
                            product,
                            amount_minor,
                            ..
                        } => Shown::One(Box::new(product), Some(amount_minor)),
                        Scan::Unknown { .. } => {
                            let page: ProductPage = list_products(
                                s,
                                &ProductQuery {
                                    text: q.clone(),
                                    sort: ProductSort::Name,
                                    active: Some(true),
                                    limit: 12,
                                    with_counts: false,
                                    ..Default::default()
                                },
                            )?;
                            match page.rows.len() {
                                0 => Shown::NotFound(q),
                                1 => Shown::One(Box::new(page.rows[0].clone()), None),
                                _ => Shown::Many(page.rows),
                            }
                        }
                    })
                })
                .await;
            busy.set(false);
            text.set(String::new());
            shown.set(r.unwrap_or(Shown::NotFound(code)));
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    on_key("escape", move |_| leaving.set(true));

    // The head: the shop, the time, the languages, the way out.
    let langs = {
        let mut row = view().flex_row().gap(px(6.0));
        for l in Lang::ALL {
            row = row.child(
                view()
                    .test_id(&format!("price-lang-{}", l.code()))
                    .px(px(12.0))
                    .h(px(36.0))
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .border_1()
                    .border_color(move || {
                        if st.lang.get() == l {
                            Tone::Primary.color()
                        } else {
                            Tone::Border.color()
                        }
                    })
                    .on_press(move || {
                        last.set(now_secs());
                        st.lang.set(l)
                    })
                    .child(label_line(
                        move || l.native_name().to_owned(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Body,
                    )),
            );
        }
        row
    };
    let head = view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .child(
            view()
                .w(px(44.0))
                .h(px(44.0))
                .rounded(px(radius::lg()))
                .items_center()
                .justify_center()
                .bg(Tone::Primary.color())
                .child(icon(Icon::Tag, 22.0, || Tone::OnPrimary)),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(label_line(
                    move || shop.get().unwrap_or_default(),
                    ty::XL,
                    FontWeight::BOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    st.t("price.subtitle"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        )
        .child(if compact { view() } else { langs })
        .child(
            icon_button(
                || Icon::LogOut,
                st.t("price.exit"),
                Variant::Ghost,
                move || leaving.set(true),
            )
            .test_id("price-exit"),
        );

    // The field: scans arrive here as typed text ending with Return.
    let search = view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .h(px(64.0))
        .px(px(20.0))
        .rounded(px(radius::lg()))
        .border_2()
        .border_color(Tone::Primary.color())
        .bg(Tone::Surface.color())
        .child(icon(Icon::Barcode, 26.0, || Tone::Primary))
        .child(
            input()
                .test_id("price-search")
                .element_ref(&field)
                .auto_focus(true)
                .flex_1()
                .placeholder(st.t("price.search"))
                .value(move || text.get())
                .on_change(move |s: &str| {
                    last.set(now_secs());
                    text.set(s.to_owned())
                })
                .on_submit(move |s: &str| look(s.to_owned()))
                .text_size(px(22.0))
                .text_color(Tone::Ink.color()),
        )
        .child(
            button(
                Variant::Primary,
                Size::Lg,
                Some(Icon::Search),
                st.t("price.find"),
                move || look(text.get_untracked()),
            )
            .test_id("price-find"),
        );

    let body = dynamic(move || match shown.get() {
        Shown::Welcome => welcome(st).into_element(),
        Shown::One(p, amount) => one(st, &p, amount, compact).into_element(),
        Shown::Many(rows) => {
            let mut g = view()
                .grid()
                .grid_template_columns(SharedString::from("repeat(auto-fill, minmax(240px, 1fr))"))
                .gap(px(14.0));
            for (i, p) in rows.into_iter().enumerate() {
                let name = p.name.clone();
                let price = p.price_minor;
                g = g.child(
                    view()
                        .test_id(&format!("price-pick-{i}"))
                        .p(px(18.0))
                        .gap(px(8.0))
                        .rounded(px(radius::lg()))
                        .border_1()
                        .border_color(Tone::Border.color())
                        .bg(Tone::Surface.color())
                        .cursor_pointer()
                        .hover(|s| s.border_color(Tone::Primary.color()))
                        .on_press(move || {
                            last.set(now_secs());
                            shown.set(Shown::One(Box::new(p.clone()), None))
                        })
                        .child(label(
                            move || name.clone(),
                            ty::MD,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(figure(
                            move || money(st, price),
                            ty::XL,
                            FontWeight::BOLD,
                            || Tone::Primary,
                        )),
                );
            }
            view()
                .gap(px(14.0))
                .child(label(
                    st.t("price.choose"),
                    ty::LG,
                    FontWeight::SEMIBOLD,
                    Tone::Body,
                ))
                .child(g)
                .into_element()
        }
        Shown::NotFound(code) => view()
            .test_id("price-not-found")
            .items_center()
            .gap(px(14.0))
            .child(icon(Icon::Search, 64.0, || Tone::Warning))
            .child(
                label(st.t("price.not_found"), ty::H2, FontWeight::BOLD, Tone::Ink).text_center(),
            )
            .child(label_line(
                move || code.clone(),
                ty::MD,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .into_element(),
    });

    view()
        .test_id("price-check")
        .flex_1()
        .min_h(px(0.0))
        .bg(Tone::Canvas.color())
        .p(px(if compact { 16.0 } else { 32.0 }))
        .gap(px(24.0))
        .on_press(move || last.set(now_secs()))
        .child(head)
        .child(search)
        .child(
            scroll().flex_1().min_h(px(0.0)).child(
                view()
                    .min_h(px(360.0))
                    .justify_center()
                    .py(px(24.0))
                    .child(body),
            ),
        )
        .child(modal(leaving, compact, move || {
            exit_dialog(st, move || leaving.set(false)).into_element()
        }))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn money(st: AppState, minor: i64) -> String {
    let c = st
        .session
        .get_untracked()
        .map_or(nb_core::Currency::DZD, |s| s.currency);
    st.fmt().money(Money::from_minor(minor, c))
}

fn welcome(st: AppState) -> View {
    view()
        .test_id("price-welcome")
        .items_center()
        .gap(px(18.0))
        .child(
            view()
                .w(px(120.0))
                .h(px(120.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(Tone::PrimarySoft.color())
                .child(icon(Icon::Barcode, 60.0, || Tone::Primary)),
        )
        .child(label(st.t("price.welcome"), ty::H1, FontWeight::BOLD, Tone::Ink).text_center())
        .child(
            label(
                st.t("price.welcome_hint"),
                ty::LG,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .text_center(),
        )
}

/// One product: its name, its price as large as the screen allows, its
/// unit and other packs, whether it is there.
fn one(st: AppState, p: &ProductRow, amount: Option<i64>, compact: bool) -> View {
    let name = p.name.clone();
    let alt = p.alt_name.clone().unwrap_or_default();
    let price = amount.unwrap_or(p.price_minor);
    let unit = p
        .units
        .first()
        .map(|u| crate::screens::common::unit_short(st, &u.code))
        .unwrap_or_default();
    let packs: Vec<String> = p
        .units
        .iter()
        .skip(1)
        .filter_map(|u| {
            let f: Quantity = u.factor.parse().ok()?;
            let milli = nb_store::repo::to_milli(f).ok()?;
            let total = (i128::from(p.price_minor) * i128::from(milli) / 1000) as i64;
            Some(format!(
                "{} {} · {}",
                st.fmt().quantity(f, 3),
                crate::screens::common::unit_short(st, &p.units[0].code),
                money(st, total)
            ))
            .map(|line| {
                format!(
                    "{}: {line}",
                    crate::screens::common::unit_short(st, &u.code)
                )
            })
        })
        .collect();
    let stocked = p.kind == "service" || p.on_hand_milli > 0;
    let sku = p.sku.clone();
    let mut v = view()
        .test_id("price-result")
        .items_center()
        .gap(px(14.0))
        .child(label(move || name.clone(), ty::H1, FontWeight::BOLD, Tone::Ink).text_center());
    if !alt.is_empty() {
        v = v.child(label_line(
            move || alt.clone(),
            ty::LG,
            FontWeight::MEDIUM,
            Tone::Muted,
        ));
    }
    v = v
        .child(
            view()
                .px(px(36.0))
                .py(px(18.0))
                .rounded(px(24.0))
                .bg(Tone::PrimarySoft.color())
                .child(
                    figure(
                        move || money(st, price),
                        if compact { 48.0 } else { 96.0 },
                        FontWeight::BOLD,
                        || Tone::Primary,
                    )
                    .test_id("price-amount"),
                ),
        )
        .child(label_line(
            move || {
                if amount.is_some() {
                    st.tn("price.label_amount")
                } else if unit.is_empty() {
                    String::new()
                } else {
                    tr_args(st.lang(), "price.per_unit", &[("unit", &unit)])
                }
            },
            ty::LG,
            FontWeight::MEDIUM,
            Tone::Body,
        ));
    for line in packs {
        v = v.child(label_line(
            move || line.clone(),
            ty::MD,
            FontWeight::NORMAL,
            Tone::Muted,
        ));
    }
    if st.biz.price_stock.get_untracked() {
        v = v.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(14.0))
                .py(px(6.0))
                .rounded_full()
                .bg(if stocked {
                    Tone::SuccessSoft.color()
                } else {
                    Tone::DangerSoft.color()
                })
                .child(icon(
                    if stocked {
                        Icon::CheckCircle
                    } else {
                        Icon::Alert
                    },
                    16.0,
                    move || if stocked { Tone::Success } else { Tone::Danger },
                ))
                .child(label_line(
                    st.t(if stocked {
                        "price.in_stock"
                    } else {
                        "price.out_of_stock"
                    }),
                    ty::MD,
                    FontWeight::SEMIBOLD,
                    if stocked { Tone::Success } else { Tone::Danger },
                )),
        );
    }
    v.child(label_line(
        move || sku.clone(),
        ty::SM,
        FontWeight::NORMAL,
        Tone::Muted,
    ))
}

/// Leaving the price display: behind the sign-in when the device asks for
/// one; opening it by itself at start is chosen here.
fn exit_dialog(st: AppState, close: impl Fn() + Clone + 'static) -> View {
    let at_start = persisted_flag(AT_START, false);
    let leave = move || {
        st.route.set(Route::Home);
        // The device asks who is there: the one leaving signs in again.
        if st.needs_sign_in.get_untracked() {
            st.user.set(None);
        }
    };
    let body = view()
        .gap(px(14.0))
        .child(label(
            move || {
                st.tn(if st.needs_sign_in.get() {
                    "price.exit_signed"
                } else {
                    "price.exit_hint"
                })
            },
            ty::SM,
            FontWeight::NORMAL,
            Tone::Body,
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(switch(at_start, st.t("price.at_start")))
                .child(view().flex_1().child(label(
                    st.t("price.at_start"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Ink,
                ))),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(switch(st.biz.price_stock, st.t("price.show_stock")))
                .child(view().flex_1().child(label(
                    st.t("price.show_stock"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Ink,
                ))),
        );
    let footer = view()
        .child(button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("price.stay"),
            close.clone(),
        ))
        .child(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::LogOut),
                st.t("price.exit"),
                leave,
            )
            .test_id("price-exit-confirm"),
        );
    dialog_frame(
        st,
        st.t("price.exit_title"),
        st.t("price.subtitle"),
        body,
        footer,
        close,
    )
}

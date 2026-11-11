//! The point of sale (M8.9, M8.10): find or scan products, a cart whose
//! totals come from the domain's cart, and payment. The till must be open:
//! the screen opens it with its float and closes it with the cash counted.
//! The payment dialog makes its operation when it opens, so a double tap
//! on "pay" sells once (M8.5).

use nb_core::{Money, Quantity, Rate};
use nb_domain::sale::{Cart, CartLine, Discount};
use nb_domain::tax::TaxMode;
use nb_i18n::tr_args;
use nb_store::repo::catalog::{ProductPage, ProductQuery, ProductRow, ProductSort};
use nb_store::usecases::catalog::{Scan, scan};
use nb_store::usecases::org::{Register, registers};
use nb_store::usecases::products::list_products;
use nb_store::usecases::sales::{
    PaymentInput, SaleInput, SaleLineInput, ShiftRow, close_shift, complete_sale, current_shift,
    open_shift,
};
use nsh_core::element::{AnyElement, KeyboardType, View, input};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::nav::Route;
use crate::screens::common::page;
use crate::screens::forms::{error_text, parse_price};
use crate::state::AppState;
use crate::ui::form::{field, labelled};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, figure, icon_button, label, label_line, segmented,
};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::surface::{card, card_header_icon, empty_state};
use crate::ui::tokens::{Status, Tone, radius, ty};

fn first_error(st: AppState, e: &DbError) -> String {
    error_text(st, e.codes.first().map_or("internal", |c| c.1))
}

fn currency(st: AppState) -> nb_core::Currency {
    st.session
        .get_untracked()
        .map_or(nb_core::Currency::DZD, |s| s.currency)
}

fn money(st: AppState, minor: i64) -> String {
    st.fmt().money(Money::from_minor(minor, currency(st)))
}

/// One line of the cart: the product as listed, and how much of it (in
/// thousandths of its base unit).
#[derive(Debug, Clone, PartialEq)]
struct Item {
    product: ProductRow,
    qty_milli: i64,
}

impl Item {
    fn unit(&self) -> String {
        self.product
            .units
            .first()
            .map(|u| u.code.clone())
            .unwrap_or_default()
    }
}

/// The cart as the domain prices it (prices with tax, base units).
fn cart_of(items: &[Item], c: nb_core::Currency) -> Cart {
    Cart {
        mode: TaxMode::Inclusive,
        lines: items
            .iter()
            .map(|i| CartLine {
                qty: Quantity::from_milli(i.qty_milli),
                unit_price: Money::from_minor(i.product.price_minor, c),
                min_price: None,
                tax_rate: Rate::from_bps(i.product.tax_bps),
                discount: Discount::None,
            })
            .collect(),
        discount: Discount::None,
    }
}

/// Adds `qty_milli` of `p` (to its line if already in the cart).
fn add(cart: Signal<Vec<Item>>, p: ProductRow, qty_milli: i64) {
    cart.update(|v| match v.iter_mut().find(|i| i.product.id == p.id) {
        Some(i) => i.qty_milli += qty_milli,
        None => v.push(Item {
            product: p,
            qty_milli,
        }),
    });
}

pub fn pos_page(st: AppState, compact: bool) -> View {
    // The till of this device: the first of the branch (a choice comes
    // with the device settings).
    let till = resource(
        move || {
            st.session
                .get()
                .map(|s| (s.branch().to_string(), st.data_rev.get()))
        },
        move |(branch, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    let all = s.read(|r| {
                        let o = nb_store::repo::org::the_org(r.conn())?
                            .ok_or(nb_store::Error::NotFound("organization"))?;
                        registers(r.conn(), o.org_id()?)
                    })?;
                    let Some(reg) = all.into_iter().find(|r| r.branch_id == branch) else {
                        return Ok(None);
                    };
                    let shift = current_shift(s, &reg.id)?;
                    Ok(Some((reg, shift)))
                })
                .await
            }
        },
    );
    let state = computed(move || {
        till.with(|t: Option<&Option<(Register, Option<ShiftRow>)>>| t.cloned().flatten())
    });
    let cart: Signal<Vec<Item>> = signal(Vec::new());
    let paying = signal(false);
    let closing = signal(false);

    let actions = view().child(dynamic(move || match state.get() {
        Some((_, Some(shift))) => view()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(badge(Status::Success, Some(Icon::Wallet), move || {
                tr_args(st.lang(), "pos.till_open", &[("number", &shift.number)])
            }))
            .child(
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Lock),
                    st.t("pos.close_till"),
                    move || closing.set(true),
                )
                .test_id("till-close"),
            )
            .into_element(),
        _ => view().into_element(),
    }));

    let body = dynamic(move || match state.get() {
        None => card()
            .child(empty_state(
                Icon::Wallet,
                st.t("pos.no_till"),
                st.t("pos.no_till.body"),
                None,
            ))
            .into_element(),
        Some((reg, None)) => open_till(st, reg).into_element(),
        Some((reg, Some(shift))) => selling(st, reg, shift, cart, paying, compact).into_element(),
    });
    view()
        .child(page(
            st,
            Route::Sell,
            "pos.subtitle",
            actions,
            view().child(body),
            compact,
        ))
        .child(modal(paying, compact, move || {
            match state.get_untracked() {
                Some((reg, Some(_))) => {
                    pay_dialog(st, reg, cart, move || paying.set(false)).into_element()
                }
                _ => view().into_element(),
            }
        }))
        .child(modal(closing, compact, move || {
            match state.get_untracked() {
                Some((_, Some(shift))) => {
                    close_dialog(st, shift, move || closing.set(false)).into_element()
                }
                _ => view().into_element(),
            }
        }))
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

/// The till is closed: open it with the cash in the drawer.
fn open_till(st: AppState, reg: Register) -> View {
    let float = signal(String::new());
    let error = signal(String::new());
    let id = reg.id.clone();
    let start = move || {
        let Some(session) = st.session.get_untracked() else {
            return;
        };
        let text = float.get_untracked();
        let opening = if text.trim().is_empty() {
            0
        } else {
            match parse_price(&text, session.currency) {
                Ok(m) => m.minor(),
                Err(_) => {
                    error.set(error_text(st, "invalid_amount"));
                    return;
                }
            }
        };
        let (id, meta) = (id.clone(), session.meta("shift.open"));
        let db = st.db();
        let task = spawn(async move {
            match db.call(move |s| open_shift(s, meta, &id, opening)).await {
                Ok(_) => st.changed(),
                Err(e) => error.set(first_error(st, &e)),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    // NB_OPEN=till (development, screenshots): open it at once.
    if std::env::var("NB_OPEN").as_deref() == Ok("till") {
        start();
    }
    let name = reg.name.clone();
    card().child(
        view()
            .items_center()
            .gap(px(14.0))
            .p(px(32.0))
            .child(
                view()
                    .w(px(56.0))
                    .h(px(56.0))
                    .rounded(px(radius::lg()))
                    .items_center()
                    .justify_center()
                    .bg(Tone::PrimarySoft.color())
                    .child(icon(Icon::Wallet, 26.0, || Tone::Primary)),
            )
            .child(label_line(
                move || tr_args(st.lang(), "pos.open_till.title", &[("till", &name)]),
                ty::LG,
                FontWeight::BOLD,
                Tone::Ink,
            ))
            .child(label(
                st.t("pos.open_till.body"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .child(view().w(px(280.0)).child(field(
                st.t("pos.float"),
                float,
                || "0,00".to_owned(),
                String::new,
                KeyboardType::Decimal,
                "till-float",
            )))
            .child(error_line(error))
            .child(
                button(
                    Variant::Primary,
                    Size::Lg,
                    Some(Icon::Check),
                    st.t("pos.open_till"),
                    start,
                )
                .test_id("till-open"),
            ),
    )
}

/// Products to pick (search, or a scanned barcode), and the cart.
fn selling(
    st: AppState,
    _reg: Register,
    _shift: ShiftRow,
    cart: Signal<Vec<Item>>,
    paying: Signal<bool>,
    compact: bool,
) -> View {
    let text = signal(String::new());
    let found = resource(
        move || {
            st.session.get()?;
            Some((text.get(), st.data_rev.get()))
        },
        move |(t, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    list_products(
                        s,
                        &ProductQuery {
                            text: t,
                            sort: ProductSort::Name,
                            limit: if compact { 8 } else { 24 },
                            ..Default::default()
                        },
                    )
                })
                .await
            }
        },
    );
    let rows = computed(move || {
        found
            .with(|p: Option<&ProductPage>| p.map(|p| p.rows.clone()))
            .unwrap_or_default()
    });
    // Return in the field: a barcode (or scale label) adds its product.
    let submit = move |code: &str| {
        let code = code.trim().to_owned();
        if code.is_empty() {
            return;
        }
        let db = st.db();
        let task = spawn(async move {
            match db.call(move |s| scan(s, &code)).await {
                Ok(Scan::Item { product, .. }) => {
                    add(cart, product, 1_000);
                    text.set(String::new());
                }
                Ok(Scan::Weighed { product, qty } | Scan::Priced { product, qty, .. }) => {
                    let milli = qty
                        .parse::<Quantity>()
                        .ok()
                        .and_then(|q| nb_store::repo::to_milli(q).ok())
                        .unwrap_or(0);
                    add(cart, product, milli);
                    text.set(String::new());
                }
                _ => {}
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let search = view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(44.0))
        .px(px(12.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::BorderStrong.color())
        .bg(Tone::Surface.color())
        .child(icon(Icon::Barcode, 18.0, || Tone::Muted))
        .child(
            input()
                .test_id("pos-search")
                .flex_1()
                .placeholder(st.t("pos.search"))
                .value(move || text.get())
                .on_change(move |t: &str| text.set(t.to_owned()))
                .on_submit(submit)
                .text_color(Tone::Ink.color()),
        );
    let grid = dynamic(move || {
        let all = rows.get();
        if all.is_empty() {
            return empty_state(
                Icon::Search,
                st.t("table.empty.title"),
                st.t("table.empty.body"),
                None,
            )
            .into_element();
        }
        let mut g = view().flex_row().flex_wrap().gap(px(10.0));
        for (i, p) in all.into_iter().enumerate() {
            let (name, price, stock) = (p.name.clone(), p.price_minor, p.on_hand_milli);
            let low = stock <= 0;
            g = g.child(
                view()
                    .test_id(&format!("pos-item-{i}"))
                    // Cards grow to fill each row (a gap-free grid at any width).
                    .flex_grow(1.0)
                    .basis(px(if compact { 140.0 } else { 168.0 }))
                    .max_w(px(260.0))
                    .min_h(px(96.0))
                    .p(px(12.0))
                    .gap(px(6.0))
                    .rounded(px(radius::lg()))
                    .border_1()
                    .border_color(Tone::Border.color())
                    .bg(Tone::Surface.color())
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(Tone::Subtle.color())
                            .border_color(Tone::Primary.color())
                    })
                    .on_press(move || add(cart, p.clone(), 1_000))
                    .child(label(
                        move || name.clone(),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    ))
                    .child(view().flex_1())
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .child(view().flex_1().child(figure(
                                move || money(st, price),
                                ty::SM,
                                FontWeight::BOLD,
                                || Tone::Primary,
                            )))
                            .child(label_line(
                                move || st.fmt().quantity(Quantity::from_milli(stock), 3),
                                ty::XS,
                                FontWeight::MEDIUM,
                                if low { Tone::Danger } else { Tone::Muted },
                            )),
                    ),
            );
        }
        g.into_element()
    });
    let products = card().p(px(16.0)).gap(px(14.0)).child(search).child(grid);
    let side = cart_panel(st, cart, paying);
    if compact {
        view().gap(px(16.0)).child(products).child(side)
    } else {
        view()
            .flex_row()
            .items_start()
            .gap(px(20.0))
            .child(
                view()
                    .flex_grow(3.0)
                    .basis(px(0.0))
                    .min_w(px(360.0))
                    .child(products),
            )
            .child(view().w(px(380.0)).flex_shrink_0().child(side))
    }
}

fn cart_panel(st: AppState, cart: Signal<Vec<Item>>, paying: Signal<bool>) -> View {
    let totals = computed(move || {
        let items = cart.get();
        if items.is_empty() {
            return None;
        }
        cart_of(&items, currency(st)).totals().ok()
    });
    let lines = dynamic(move || {
        let items = cart.get();
        if items.is_empty() {
            return view()
                .p(px(24.0))
                .items_center()
                .child(label(
                    st.t("pos.cart.empty"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .into_element();
        }
        let mut v = view();
        for (i, it) in items.into_iter().enumerate() {
            let name = it.product.name.clone();
            let unit = it.unit();
            let qty = it.qty_milli;
            let line_total = Quantity::from_milli(qty)
                .times_price(Money::from_minor(it.product.price_minor, currency(st)))
                .map_or(0, |m| m.minor());
            let step = move |delta: i64| {
                move || {
                    cart.update(|v| {
                        if let Some(l) = v.get_mut(i) {
                            l.qty_milli += delta;
                            if l.qty_milli <= 0 {
                                v.remove(i);
                            }
                        }
                    })
                }
            };
            v = v.child(
                view()
                    .test_id(&format!("cart-line-{i}"))
                    .gap(px(6.0))
                    .px(px(16.0))
                    .py(px(10.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(view().flex_1().min_w(px(0.0)).child(label_line(
                                move || name.clone(),
                                ty::SM,
                                FontWeight::MEDIUM,
                                Tone::Ink,
                            )))
                            .child(figure(
                                move || money(st, line_total),
                                ty::SM,
                                FontWeight::SEMIBOLD,
                                || Tone::Ink,
                            )),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                icon_button(
                                    || Icon::ArrowDown,
                                    st.t("pos.less"),
                                    Variant::Secondary,
                                    step(-1_000),
                                )
                                .test_id(&format!("cart-less-{i}")),
                            )
                            .child(view().min_w(px(44.0)).items_center().child(label_line(
                                move || {
                                    format!(
                                        "{} {unit}",
                                        st.fmt().quantity(Quantity::from_milli(qty), 3)
                                    )
                                },
                                ty::SM,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            )))
                            .child(
                                icon_button(
                                    || Icon::ArrowUp,
                                    st.t("pos.more"),
                                    Variant::Secondary,
                                    step(1_000),
                                )
                                .test_id(&format!("cart-more-{i}")),
                            )
                            .child(view().flex_1())
                            .child(icon_button(
                                || Icon::Trash,
                                st.t("action.delete"),
                                Variant::Ghost,
                                move || {
                                    cart.update(|v| {
                                        if i < v.len() {
                                            v.remove(i);
                                        }
                                    })
                                },
                            )),
                    ),
            );
        }
        v.into_element()
    });
    let row = |key: &'static str, value: Box<dyn Fn() -> String>, strong: bool| {
        view()
            .flex_row()
            .items_center()
            .child(view().flex_1().child(label_line(
                st.t(key),
                if strong { ty::LG } else { ty::SM },
                if strong {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                },
                if strong { Tone::Ink } else { Tone::Muted },
            )))
            .child(figure(
                value,
                if strong { ty::H2 } else { ty::SM },
                if strong {
                    FontWeight::BOLD
                } else {
                    FontWeight::MEDIUM
                },
                || Tone::Ink,
            ))
    };
    let pick = move |f: fn(&nb_domain::sale::Totals) -> Money| {
        move || {
            totals
                .get()
                .map_or_else(|| money(st, 0), |t| money(st, f(&t).minor()))
        }
    };
    let sums = view()
        .gap(px(8.0))
        .p(px(16.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(row("pos.net", Box::new(pick(|t| t.net)), false))
        .child(row("pos.tax", Box::new(pick(|t| t.tax)), false))
        .child(row("pos.total", Box::new(pick(|t| t.total)), true));
    let pay = view().p(px(16.0)).pt(px(0.0)).child(
        button(
            Variant::Primary,
            Size::Lg,
            Some(Icon::Banknote),
            st.t("pos.pay"),
            move || {
                if !cart.get_untracked().is_empty() {
                    paying.set(true);
                }
            },
        )
        .test_id("pos-pay")
        .w_full(),
    );
    card()
        .child(card_header_icon(
            Some(Icon::Cart),
            st.t("pos.cart"),
            None,
            view().child(when(
                move || !cart.get().is_empty(),
                move || {
                    button(
                        Variant::Ghost,
                        Size::Sm,
                        None,
                        st.t("pos.cart.clear"),
                        move || cart.set(Vec::new()),
                    )
                    .test_id("cart-clear")
                    .into_element()
                },
            )),
        ))
        .child(lines)
        .child(sums)
        .child(pay)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Cash,
    Card,
    Transfer,
}

fn pay_dialog(
    st: AppState,
    reg: Register,
    cart: Signal<Vec<Item>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    // One operation for this payment screen: a second tap returns the sale
    // already made.
    let meta = session.meta("sale.complete");
    let c = session.currency;
    let due = cart_of(&cart.get_untracked(), c)
        .totals()
        .map_or(0, |t| t.total.minor());
    let way = signal(Way::Cash);
    let given = signal(Money::from_minor(due, c).to_plain_string());
    let error = signal(String::new());
    let given_minor = move || parse_price(&given.get(), c).ok().map(|m| m.minor());
    let change = computed(move || match (way.get(), given_minor()) {
        (Way::Cash, Some(g)) if g > due => g - due,
        _ => 0,
    });
    let save = {
        let close = close.clone();
        let warehouse = session.warehouse().to_string();
        let register = reg.id.clone();
        move || {
            let amount = match way.get_untracked() {
                Way::Cash => match given_minor() {
                    Some(g) => g,
                    None => {
                        error.set(error_text(st, "invalid_amount"));
                        return;
                    }
                },
                _ => due,
            };
            let method = match way.get_untracked() {
                Way::Cash => "cash",
                Way::Card => "card",
                Way::Transfer => "transfer",
            };
            let input = SaleInput {
                register_id: register.clone(),
                warehouse_id: warehouse.clone(),
                lines: cart
                    .get_untracked()
                    .iter()
                    .map(|i| SaleLineInput {
                        product_id: i.product.id.clone(),
                        qty: Quantity::from_milli(i.qty_milli).to_string(),
                        ..Default::default()
                    })
                    .collect(),
                payments: vec![PaymentInput {
                    method: method.into(),
                    amount_minor: amount,
                }],
                ..Default::default()
            };
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| complete_sale(s, meta, input)).await {
                    Ok(r) => {
                        cart.set(Vec::new());
                        st.changed();
                        close();
                        let text = if r.change_minor > 0 {
                            tr_args(
                                st.lang(),
                                "pos.sold_change",
                                &[
                                    ("number", &r.number),
                                    ("change", &money(st, r.change_minor)),
                                ],
                            )
                        } else {
                            tr_args(st.lang(), "pos.sold", &[("number", &r.number)])
                        };
                        show_toast(st, text);
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let quick = {
        let mut v = view().flex_row().flex_wrap().gap(px(8.0));
        let mut amounts = vec![due];
        for step in [10_000, 50_000, 100_000, 200_000] {
            let up = (due + step - 1) / step * step;
            if up > due && !amounts.contains(&up) {
                amounts.push(up);
            }
        }
        for (i, a) in amounts.into_iter().take(4).enumerate() {
            v = v.child(
                button(
                    Variant::Secondary,
                    Size::Sm,
                    None,
                    move || money(st, a),
                    move || given.set(Money::from_minor(a, c).to_plain_string()),
                )
                .test_id(&format!("pay-quick-{i}")),
            );
        }
        v
    };
    let cash_part = view()
        .gap(px(12.0))
        .child(field(
            st.t("pos.given"),
            given,
            String::new,
            String::new,
            KeyboardType::Decimal,
            "pay-given",
        ))
        .child(quick)
        .child(
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(label_line(
                    st.t("pos.change"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )))
                .child(figure(
                    move || money(st, change.get()),
                    ty::H2,
                    FontWeight::BOLD,
                    || Tone::Success,
                )),
        );
    let body = view()
        .gap(px(16.0))
        .child(
            view()
                .items_center()
                .gap(px(4.0))
                .p(px(16.0))
                .rounded(px(radius::lg()))
                .bg(Tone::Subtle.color())
                .child(label_line(
                    st.t("pos.due"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                ))
                .child(figure(
                    move || money(st, due),
                    ty::H1,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        )
        .child(labelled(
            st.t("pos.method"),
            segmented(
                vec![
                    (
                        Way::Cash,
                        Box::new(st.t("pos.method.cash")) as Box<dyn Fn() -> String>,
                    ),
                    (Way::Card, Box::new(st.t("pos.method.card"))),
                    (Way::Transfer, Box::new(st.t("pos.method.transfer"))),
                ],
                way,
            ),
            None,
        ))
        .child(cash_part.style_when(move || way.get() != Way::Cash, |s| s.hidden()))
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
                st.t("pos.confirm"),
                save,
            )
            .test_id("pay-confirm"),
        );
    dialog_frame(
        st,
        st.t("pos.pay"),
        st.t("pos.pay.help"),
        body,
        footer,
        close,
    )
}

fn close_dialog(st: AppState, shift: ShiftRow, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let c = session.currency;
    let counted = signal(String::new());
    let note = signal(String::new());
    let error = signal(String::new());
    let meta = session.meta("shift.close");
    let expected = shift.expected_minor;
    let diff = computed(move || {
        parse_price(&counted.get(), c)
            .ok()
            .map(|m| m.minor() - expected)
    });
    let id = shift.id.clone();
    let save = {
        let close = close.clone();
        move || {
            let Ok(n) = parse_price(&counted.get_untracked(), c) else {
                error.set(error_text(st, "invalid_amount"));
                return;
            };
            let (id, text, close) = (id.clone(), note.get_untracked(), close.clone());
            let db = st.db();
            let task = spawn(async move {
                match db
                    .call(move |s| close_shift(s, meta, &id, n.minor(), &text))
                    .await
                {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "pos.closed", &[("number", &r.number)]),
                        );
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let figures = [
        ("pos.float", shift.opening_minor),
        ("pos.cash_sales", shift.cash_sales_minor),
        ("pos.cash_in", shift.cash_in_minor),
        ("pos.cash_out", -shift.cash_out_minor),
        ("pos.expected", shift.expected_minor),
    ];
    let mut summary = view().gap(px(6.0));
    for (key, v) in figures {
        let strong = key == "pos.expected";
        summary = summary.child(
            view()
                .flex_row()
                .child(view().flex_1().child(label_line(
                    st.t(key),
                    ty::SM,
                    if strong {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    },
                    Tone::Body,
                )))
                .child(figure(
                    move || money(st, v),
                    ty::SM,
                    if strong {
                        FontWeight::BOLD
                    } else {
                        FontWeight::MEDIUM
                    },
                    || Tone::Ink,
                )),
        );
    }
    let body = view()
        .gap(px(16.0))
        .child(summary)
        .child(field(
            st.t("pos.counted"),
            counted,
            String::new,
            String::new,
            KeyboardType::Decimal,
            "till-counted",
        ))
        .child(dynamic(move || match diff.get() {
            Some(0) => {
                badge(Status::Success, Some(Icon::Check), st.t("pos.diff.none")).into_element()
            }
            Some(d) if d < 0 => badge(Status::Danger, None, move || {
                tr_args(st.lang(), "pos.diff.short", &[("amount", &money(st, -d))])
            })
            .into_element(),
            Some(d) => badge(Status::Warning, None, move || {
                tr_args(st.lang(), "pos.diff.over", &[("amount", &money(st, d))])
            })
            .into_element(),
            None => view().into_element(),
        }))
        .child(field(
            st.t("pos.close_note"),
            note,
            String::new,
            String::new,
            KeyboardType::Default,
            "till-note",
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
                Some(Icon::Lock),
                st.t("pos.close_till"),
                save,
            )
            .test_id("till-close-confirm"),
        );
    let number = shift.number.clone();
    dialog_frame(
        st,
        move || tr_args(st.lang(), "pos.close_till.title", &[("number", &number)]),
        st.t("pos.close_till.help"),
        body,
        footer,
        close,
    )
}

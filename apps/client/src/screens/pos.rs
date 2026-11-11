//! The point of sale (M8.9–M8.12): find or scan products, a cart that stays
//! in place while the products scroll, lines edited in place (quantity,
//! price, name, discount), items not in the catalog, and payment — with
//! keyboard shortcuts for the counter. Totals come from the domain's cart.
//! The till must be open: the screen opens it with its float and closes it
//! with the cash counted. The payment dialog makes its operation when it
//! opens, so a double tap on "pay" sells once (M8.5).

use nb_core::{Money, Quantity, Rate};
use nb_domain::sale::{Cart, CartLine, Discount, Totals};
use nb_domain::tax::TaxMode;
use nb_i18n::tr_args;
use nb_store::repo::catalog::{ProductPage, ProductQuery, ProductRow, ProductSort};
use nb_store::usecases::catalog::{Scan, TaxClass, list_tax_classes, scan};
use nb_store::usecases::org::{Register, registers};
use nb_store::usecases::products::list_products;
use nb_store::usecases::sales::{
    PaymentInput, SaleInput, SaleLineInput, ShiftRow, close_shift, complete_sale, current_shift,
    open_shift,
};
use nsh_core::element::{AnyElement, ElementRef, KeyboardType, View, element_ref, input};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::nav::Route;
use crate::screens::common::page_head;
use crate::screens::forms::{error_text, parse_price};
use crate::state::AppState;
use crate::ui::form::{field, labelled};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, figure, icon_button, kbd, label, label_line, segmented,
};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon, empty_state, page_pad};
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

fn qty_text(st: AppState, milli: i64) -> String {
    st.fmt().quantity(Quantity::from_milli(milli), 3)
}

/// Parses a quantity typed by a person (`2`, `1,5`) into thousandths.
fn parse_qty(s: &str) -> Option<i64> {
    let q: Quantity = s.trim().replace(',', ".").parse().ok()?;
    let milli = nb_store::repo::to_milli(q).ok()?;
    (milli > 0).then_some(milli)
}

/// One line of the cart: a product of the catalog (or an item typed at the
/// till), how much of it (thousandths of its unit), and its price, name and
/// discount as the seller may have changed them.
#[derive(Debug, Clone, PartialEq)]
struct Item {
    product: Option<ProductRow>,
    name: String,
    unit: String,
    qty_milli: i64,
    price_minor: i64,
    tax_bps: i64,
    discount_bps: i64,
}

impl Item {
    fn of(p: ProductRow, qty_milli: i64) -> Self {
        Self {
            name: p.name.clone(),
            unit: p.units.first().map(|u| u.code.clone()).unwrap_or_default(),
            price_minor: p.price_minor,
            tax_bps: p.tax_bps,
            product: Some(p),
            qty_milli,
            discount_bps: 0,
        }
    }
    fn catalog_price(&self) -> Option<i64> {
        self.product.as_ref().map(|p| p.price_minor)
    }
    fn edited(&self) -> bool {
        self.product.as_ref().is_none_or(|p| {
            p.name != self.name || p.price_minor != self.price_minor || self.discount_bps != 0
        })
    }
}

/// The cart as the domain prices it (prices with tax).
fn cart_of(items: &[Item], c: nb_core::Currency) -> Cart {
    Cart {
        mode: TaxMode::Inclusive,
        lines: items
            .iter()
            .map(|i| CartLine {
                qty: Quantity::from_milli(i.qty_milli),
                unit_price: Money::from_minor(i.price_minor, c),
                min_price: None,
                tax_rate: Rate::from_bps(i.tax_bps),
                discount: if i.discount_bps > 0 {
                    Discount::Percent(i.discount_bps)
                } else {
                    Discount::None
                },
            })
            .collect(),
        discount: Discount::None,
    }
}

/// The cart's state: its lines and the one selected (the last added, or
/// the one tapped): the keyboard acts on it.
#[derive(Clone, Copy)]
struct Till {
    cart: Signal<Vec<Item>>,
    selected: Signal<Option<usize>>,
    /// The line being edited (its dialog is open).
    editing: Signal<Option<usize>>,
    editing_open: Signal<bool>,
    free_open: Signal<bool>,
    paying: Signal<bool>,
    cart_open: Signal<bool>,
    search: ElementRef,
}

impl Till {
    /// Adds `qty_milli` of `p`: to its line when it is there unchanged.
    fn add(self, p: ProductRow, qty_milli: i64) {
        let mut at = 0;
        self.cart.update(|v| {
            match v
                .iter()
                .position(|i| i.product.as_ref().is_some_and(|x| x.id == p.id) && !i.edited())
            {
                Some(i) => {
                    v[i].qty_milli += qty_milli;
                    at = i;
                }
                None => {
                    v.push(Item::of(p, qty_milli));
                    at = v.len() - 1;
                }
            }
        });
        self.selected.set(Some(at));
    }
    fn edit(self, i: usize) {
        self.selected.set(Some(i));
        self.editing.set(Some(i));
        self.editing_open.set(true);
    }
    fn remove(self, i: usize) {
        self.cart.update(|v| {
            if i < v.len() {
                v.remove(i);
            }
        });
        let n = self.cart.with_untracked(Vec::len);
        self.selected.set((n > 0).then(|| i.min(n - 1)));
    }
    fn step(self, i: usize, delta: i64) {
        let mut gone = false;
        self.cart.update(|v| {
            if let Some(l) = v.get_mut(i) {
                l.qty_milli += delta;
                gone = l.qty_milli <= 0;
            }
        });
        if gone {
            self.remove(i);
        }
    }
    fn clear(self) {
        self.cart.set(Vec::new());
        self.selected.set(None);
    }
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
    let t = Till {
        cart: signal(Vec::new()),
        selected: signal(None),
        editing: signal(None),
        editing_open: signal(false),
        free_open: signal(false),
        paying: signal(false),
        cart_open: signal(false),
        search: element_ref(),
    };
    effect(move || {
        if !t.editing_open.get() {
            t.editing.set(None);
        }
    });
    let closing = signal(false);
    let selling = move || matches!(state.get_untracked(), Some((_, Some(_))));
    let no_dialog = move || {
        !t.paying.get_untracked()
            && !t.editing_open.get_untracked()
            && !t.free_open.get_untracked()
            && !closing.get_untracked()
    };

    // Shortcuts at the counter (while this page is shown).
    on_key("f2", move |_| t.search.focus());
    on_key("f4", move |_| {
        if selling() && no_dialog() {
            t.free_open.set(true);
        }
    });
    on_key("f6", move |_| {
        if let (true, Some(i)) = (no_dialog(), t.selected.get_untracked()) {
            t.edit(i);
        }
    });
    on_key("f9", move |_| {
        if selling() && no_dialog() && !t.cart.with_untracked(Vec::is_empty) {
            t.paying.set(true);
        }
    });
    on_key("f8", move |_| {
        if no_dialog() {
            t.clear();
        }
    });
    on_key("alt+up", move |_| {
        let n = t.cart.with_untracked(Vec::len);
        if n > 0 {
            t.selected
                .update(|s| *s = Some(s.map_or(n - 1, |i| i.saturating_sub(1))));
        }
    });
    on_key("alt+down", move |_| {
        let n = t.cart.with_untracked(Vec::len);
        if n > 0 {
            t.selected
                .update(|s| *s = Some(s.map_or(0, |i| (i + 1).min(n - 1))));
        }
    });
    on_key("alt+=", move |_| {
        if let Some(i) = t.selected.get_untracked() {
            t.step(i, 1_000);
        }
    });
    on_key("alt+-", move |_| {
        if let Some(i) = t.selected.get_untracked() {
            t.step(i, -1_000);
        }
    });
    on_key("alt+delete", move |_| {
        if let Some(i) = t.selected.get_untracked() {
            t.remove(i);
        }
    });

    let actions = view().child(dynamic(move || match state.get() {
        Some((_, Some(shift))) => view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(badge(Status::Success, Some(Icon::Wallet), move || {
                tr_args(st.lang(), "pos.till_open", &[("number", &shift.number)])
            }))
            // Phones: the actions as icons, beside the till's badge.
            .child(if compact {
                icon_button(
                    || Icon::PackagePlus,
                    st.t("pos.free"),
                    Variant::Secondary,
                    move || t.free_open.set(true),
                )
                .test_id("pos-free")
            } else {
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::PackagePlus),
                    st.t("pos.free"),
                    move || t.free_open.set(true),
                )
                .test_id("pos-free")
            })
            .child(if compact {
                icon_button(
                    || Icon::Lock,
                    st.t("pos.close_till"),
                    Variant::Secondary,
                    move || closing.set(true),
                )
                .test_id("till-close")
            } else {
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::Lock),
                    st.t("pos.close_till"),
                    move || closing.set(true),
                )
                .test_id("till-close")
            })
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
        Some((reg, None)) => view().child(open_till(st, reg)).into_element(),
        Some((_, Some(_))) => selling_view(st, t, compact).into_element(),
    });
    // The page fills the window: the products and the cart scroll in
    // their own areas, so the cart and its "pay" stay in place.
    view()
        .flex_1()
        .min_h(px(0.0))
        .p(px(page_pad(compact)))
        .gap(px(if compact { 12.0 } else { 16.0 }))
        .child(page_head(
            st,
            Route::Sell,
            Some("pos.subtitle"),
            actions,
            compact,
        ))
        .child(view().flex_1().min_h(px(0.0)).child(body))
        .child(modal(t.paying, compact, move || {
            match state.get_untracked() {
                Some((reg, Some(_))) => {
                    pay_dialog(st, reg, t, move || t.paying.set(false)).into_element()
                }
                _ => view().into_element(),
            }
        }))
        .child(modal(t.editing_open, compact, move || {
            match t.editing.get_untracked() {
                Some(i) => line_dialog(st, t, i, move || t.editing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
        .child(modal(t.free_open, compact, move || {
            free_dialog(st, t, move || t.free_open.set(false)).into_element()
        }))
        .child(modal(t.cart_open, compact, move || {
            view()
                .flex_1()
                .min_h(px(0.0))
                .child(cart_panel(st, t, true))
                .into_element()
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

/// The products (search or scan, then a grid) beside the cart; on phones
/// the cart sits behind a bar at the bottom.
fn selling_view(st: AppState, t: Till, compact: bool) -> View {
    let text = signal(String::new());
    let found = resource(
        move || {
            st.session.get()?;
            Some((text.get(), st.data_rev.get()))
        },
        move |(q, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    list_products(
                        s,
                        &ProductQuery {
                            text: q,
                            sort: ProductSort::Name,
                            limit: 60,
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
    // Return in the field: a barcode (or scale label) adds its product; a
    // search with one result adds it.
    let submit = move |code: &str| {
        let code = code.trim().to_owned();
        if code.is_empty() {
            return;
        }
        let only = rows.with_untracked(|r| (r.len() == 1).then(|| r[0].clone()));
        let db = st.db();
        let task = spawn(async move {
            match db.call(move |s| scan(s, &code)).await {
                Ok(Scan::Item { product, .. }) => {
                    t.add(product, 1_000);
                    text.set(String::new());
                }
                Ok(Scan::Weighed { product, qty } | Scan::Priced { product, qty, .. }) => {
                    t.add(product, parse_qty(&qty).unwrap_or(0));
                    text.set(String::new());
                }
                _ => {
                    if let Some(p) = only {
                        t.add(p, 1_000);
                        text.set(String::new());
                    } else {
                        show_toast(st, st.tn("pos.not_found"));
                    }
                }
            }
            t.search.focus();
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let search = view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(48.0))
        .px(px(14.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::BorderStrong.color())
        .bg(Tone::Surface.color())
        .child(icon(Icon::Barcode, 20.0, || Tone::Muted))
        .child(
            input()
                .test_id("pos-search")
                .element_ref(&t.search)
                .auto_focus(!compact)
                .flex_1()
                .placeholder(st.t("pos.search"))
                .value(move || text.get())
                .on_change(move |s: &str| text.set(s.to_owned()))
                .on_submit(submit)
                .text_color(Tone::Ink.color()),
        )
        .child(when(
            move || !text.get().is_empty(),
            move || {
                icon_button(
                    || Icon::Close,
                    st.t("action.clear"),
                    Variant::Ghost,
                    move || text.set(String::new()),
                )
                .into_element()
            },
        ))
        .child(when(
            move || !compact,
            move || kbd(|| "F2".to_owned()).into_element(),
        ));
    let grid = dynamic(move || {
        let all = rows.get();
        if all.is_empty() {
            return empty_state(
                Icon::Search,
                st.t("table.empty.title"),
                st.t("pos.empty.body"),
                Some(button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::PackagePlus),
                    st.t("pos.free"),
                    move || t.free_open.set(true),
                )),
            )
            .into_element();
        }
        let mut g = view().flex_row().flex_wrap().gap(px(10.0));
        for (i, p) in all.into_iter().enumerate() {
            g = g.child(product_card(st, t, i, p, compact));
        }
        g.into_element()
    });
    let mut products = card()
        .flex_1()
        .min_h(px(0.0))
        .p(px(if compact { 12.0 } else { 16.0 }))
        .gap(px(12.0))
        .child(search)
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .child(view().pb(px(8.0)).child(grid)),
        );
    if !compact {
        products = products.child(shortcut_bar(st));
    }
    if compact {
        view()
            .flex_1()
            .min_h(px(0.0))
            .gap(px(12.0))
            .child(products)
            .child(cart_bar(st, t))
    } else {
        view()
            .flex_1()
            .min_h(px(0.0))
            .flex_row()
            .gap(px(20.0))
            .child(products)
            .child(
                view()
                    .w(px(400.0))
                    .flex_shrink_0()
                    .min_h(px(0.0))
                    .child(cart_panel(st, t, false)),
            )
    }
}

fn product_card(st: AppState, t: Till, i: usize, p: ProductRow, compact: bool) -> View {
    let (name, price, stock) = (p.name.clone(), p.price_minor, p.on_hand_milli);
    let service = p.kind == "service";
    let in_cart = {
        let id = p.id.clone();
        computed(move || {
            t.cart.with(|v| {
                v.iter()
                    .filter(|l| l.product.as_ref().is_some_and(|x| x.id == id))
                    .map(|l| l.qty_milli)
                    .sum::<i64>()
            })
        })
    };
    view()
        .test_id(&format!("pos-item-{i}"))
        // Cards grow to fill each row (a gap-free grid at any width).
        .flex_grow(1.0)
        .basis(px(if compact { 140.0 } else { 168.0 }))
        .max_w(px(260.0))
        .min_h(px(100.0))
        .p(px(12.0))
        .gap(px(6.0))
        .rounded(px(radius::lg()))
        .border_1()
        .border_color(move || {
            if in_cart.get() > 0 {
                Tone::Primary.color()
            } else {
                Tone::Border.color()
            }
        })
        .bg(move || {
            if in_cart.get() > 0 {
                Tone::PrimarySoft.color()
            } else {
                Tone::Surface.color()
            }
        })
        .cursor_pointer()
        .hover(|s| s.border_color(Tone::Primary.color()))
        .on_press(move || t.add(p.clone(), 1_000))
        .child(
            view()
                .flex_row()
                .items_start()
                .gap(px(6.0))
                .child(view().flex_1().min_w(px(0.0)).child(label(
                    move || name.clone(),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                )))
                .child(when(
                    move || in_cart.get() > 0,
                    move || {
                        view()
                            .min_w(px(22.0))
                            .h(px(22.0))
                            .px(px(6.0))
                            .rounded_full()
                            .items_center()
                            .justify_center()
                            .bg(Tone::Primary.color())
                            .child(label_line(
                                move || qty_text(st, in_cart.get()),
                                ty::XS,
                                FontWeight::BOLD,
                                Tone::OnPrimary,
                            ))
                            .into_element()
                    },
                )),
        )
        .child(view().flex_1())
        .child(
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(figure(
                    move || money(st, price),
                    ty::MD,
                    FontWeight::BOLD,
                    || Tone::Primary,
                )))
                .child(if service {
                    view()
                } else {
                    view().child(label_line(
                        move || qty_text(st, stock),
                        ty::XS,
                        FontWeight::MEDIUM,
                        if stock <= 0 {
                            Tone::Danger
                        } else {
                            Tone::Muted
                        },
                    ))
                }),
        )
}

/// What each key does, under the products.
fn shortcut_bar(st: AppState) -> View {
    let mut v = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(14.0))
        .pt(px(10.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(icon(Icon::Keyboard, 16.0, || Tone::Faint));
    for (key, what) in [
        ("F2", "pos.key.search"),
        ("F4", "pos.key.free"),
        ("F6", "pos.key.edit"),
        ("Alt ↑↓", "pos.key.select"),
        ("Alt +/−", "pos.key.qty"),
        ("F8", "pos.key.clear"),
        ("F9", "pos.key.pay"),
    ] {
        v = v.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(kbd(move || key.to_owned()))
                .child(label_line(
                    st.t(what),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        );
    }
    v
}

/// Phones: the cart's count and total, and "pay", at the bottom.
fn cart_bar(st: AppState, t: Till) -> View {
    let total = computed(move || {
        let items = t.cart.get();
        cart_of(&items, currency(st))
            .totals()
            .map_or(0, |x| x.total.minor())
    });
    view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .p(px(10.0))
        .rounded(px(radius::lg()))
        .bg(Tone::Surface.color())
        .border_1()
        .border_color(Tone::Border.color())
        .shadow_md()
        .child(
            view()
                .test_id("cart-open")
                .flex_1()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .cursor_pointer()
                .on_press(move || t.cart_open.set(true))
                .child(icon(Icon::Cart, 20.0, || Tone::Ink))
                .child(
                    view()
                        .child(label_line(
                            move || {
                                tr_args(
                                    st.lang(),
                                    "pos.cart.count",
                                    &[("count", &st.fmt().integer(t.cart.with(Vec::len) as i64))],
                                )
                            },
                            ty::XS,
                            FontWeight::MEDIUM,
                            Tone::Muted,
                        ))
                        .child(figure(
                            move || money(st, total.get()),
                            ty::LG,
                            FontWeight::BOLD,
                            || Tone::Ink,
                        )),
                ),
        )
        .child(
            button(
                Variant::Primary,
                Size::Lg,
                Some(Icon::Banknote),
                st.t("pos.pay"),
                move || {
                    if !t.cart.with_untracked(Vec::is_empty) {
                        t.paying.set(true);
                    }
                },
            )
            .test_id("pos-pay"),
        )
}

/// The cart: its lines scroll; the totals and "pay" stay at the bottom.
fn cart_panel(st: AppState, t: Till, in_sheet: bool) -> View {
    let totals = computed(move || {
        let items = t.cart.get();
        if items.is_empty() {
            return None;
        }
        cart_of(&items, currency(st)).totals().ok()
    });
    let lines = dynamic(move || {
        let items = t.cart.get();
        if items.is_empty() {
            return view()
                .flex_1()
                .p(px(28.0))
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .child(icon(Icon::Barcode, 32.0, || Tone::Faint))
                .child(label(
                    st.t("pos.cart.empty"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .into_element();
        }
        let figures = totals.get_untracked();
        let mut v = view();
        for (i, it) in items.into_iter().enumerate() {
            let line_total = figures
                .as_ref()
                .and_then(|f| f.lines.get(i))
                .map_or(0, |l| l.taxed.gross.minor());
            v = v.child(cart_line(st, t, i, it, line_total));
        }
        v.into_element()
    });
    let row = move |key: &'static str, value: Box<dyn Fn() -> String>| {
        view()
            .flex_row()
            .items_center()
            .child(view().flex_1().child(label_line(
                st.t(key),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )))
            .child(figure(value, ty::SM, FontWeight::MEDIUM, || Tone::Body))
    };
    let pick = move |f: fn(&Totals) -> i64| {
        move || {
            totals
                .get()
                .map_or_else(|| money(st, 0), |t| money(st, f(&t)))
        }
    };
    let sums = view()
        .gap(px(6.0))
        .px(px(16.0))
        .pt(px(12.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(row("pos.net", Box::new(pick(|t| t.net.minor()))))
        .child(row("pos.tax", Box::new(pick(|t| t.tax.minor()))))
        .child(when(
            move || totals.get().is_some_and(|t| t.discounts.minor() > 0),
            move || row("pos.discounts", Box::new(pick(|t| -t.discounts.minor()))).into_element(),
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .pt(px(6.0))
                .child(view().flex_1().child(label_line(
                    st.t("pos.total"),
                    ty::LG,
                    FontWeight::BOLD,
                    Tone::Ink,
                )))
                .child(figure(
                    pick(|t| t.total.minor()),
                    ty::H2,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        );
    let pay = view().p(px(16.0)).child(
        button(
            Variant::Primary,
            Size::Lg,
            Some(Icon::Banknote),
            st.t("pos.pay"),
            move || {
                if !t.cart.with_untracked(Vec::is_empty) {
                    t.cart_open.set(false);
                    t.paying.set(true);
                }
            },
        )
        .test_id(if in_sheet { "sheet-pay" } else { "pos-pay" })
        .w_full(),
    );
    let count = move || {
        tr_args(
            st.lang(),
            "pos.cart.count",
            &[("count", &st.fmt().integer(t.cart.with(Vec::len) as i64))],
        )
    };
    card()
        .flex_1()
        .h_full()
        .min_h(px(0.0))
        .child(card_header_icon(
            Some(Icon::Cart),
            st.t("pos.cart"),
            Some(Box::new(count)),
            view().child(when(
                move || !t.cart.get().is_empty(),
                move || {
                    button(
                        Variant::Ghost,
                        Size::Sm,
                        Some(Icon::Trash),
                        st.t("pos.cart.clear"),
                        move || t.clear(),
                    )
                    .test_id("cart-clear")
                    .into_element()
                },
            )),
        ))
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(lines),
        )
        .child(sums)
        .child(pay)
}

fn stepper(
    _st: AppState,
    which: Icon,
    _a11y: &'static str,
    on: impl Fn() + 'static,
    id: String,
) -> View {
    view()
        .test_id(&id)
        .w(px(32.0))
        .h(px(32.0))
        .items_center()
        .justify_center()
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::BorderStrong.color())
        .bg(Tone::Surface.color())
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Subtle.color()))
        .on_press(on)
        .child(icon(which, 16.0, || Tone::Ink))
}

fn cart_line(st: AppState, t: Till, i: usize, it: Item, line_total: i64) -> View {
    let selected = computed(move || t.selected.get() == Some(i));
    let name = it.name.clone();
    let unit = it.unit.clone();
    let (qty, price, discount, edited, free) = (
        it.qty_milli,
        it.price_minor,
        it.discount_bps,
        it.edited(),
        it.product.is_none(),
    );
    let detail = move || {
        let mut s = format!("{} × {}", qty_text(st, qty), money(st, price));
        if discount > 0 {
            s.push_str(&format!(
                " · −{}%",
                st.fmt().quantity(Quantity::from_milli(discount * 10), 2)
            ));
        }
        s
    };
    view()
        .test_id(&format!("cart-line-{i}"))
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(Tone::Border.color())
        // The selected line (the keyboard acts on it) is tinted.
        .bg(move || {
            if selected.get() {
                Tone::PrimarySoft.color()
            } else {
                Tone::Surface.color()
            }
        })
        .cursor_pointer()
        .on_press(move || t.selected.set(Some(i)))
        .child(
            view()
                .flex_row()
                .items_start()
                .gap(px(8.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(2.0))
                        .child(label(
                            move || name.clone(),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(detail, ty::XS, FontWeight::NORMAL, Tone::Muted)),
                )
                .child(figure(
                    move || money(st, line_total),
                    ty::SM,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(stepper(
                    st,
                    Icon::Minus,
                    "pos.less",
                    move || t.step(i, -1_000),
                    format!("cart-less-{i}"),
                ))
                // The quantity: tap it to type another.
                .child(
                    view()
                        .test_id(&format!("cart-qty-{i}"))
                        .min_w(px(64.0))
                        .h(px(32.0))
                        .px(px(8.0))
                        .items_center()
                        .justify_center()
                        .rounded(px(radius::md()))
                        .border_1()
                        .border_color(Tone::Border.color())
                        .bg(Tone::Surface.color())
                        .cursor_pointer()
                        .hover(|s| s.border_color(Tone::Primary.color()))
                        .on_press(move || t.edit(i))
                        .child(label_line(
                            move || format!("{} {unit}", qty_text(st, qty)),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        )),
                )
                .child(stepper(
                    st,
                    Icon::Plus,
                    "pos.more",
                    move || t.step(i, 1_000),
                    format!("cart-more-{i}"),
                ))
                .child(view().flex_1())
                .child(when(
                    move || edited || free,
                    move || {
                        badge(
                            Status::Info,
                            None,
                            st.t(if free { "pos.free.badge" } else { "pos.edited" }),
                        )
                        .into_element()
                    },
                ))
                .child(
                    icon_button(
                        || Icon::Pencil,
                        st.t("pos.edit_line"),
                        Variant::Ghost,
                        move || t.edit(i),
                    )
                    .test_id(&format!("cart-edit-{i}")),
                )
                .child(
                    icon_button(
                        || Icon::Trash,
                        st.t("action.delete"),
                        Variant::Ghost,
                        move || t.remove(i),
                    )
                    .test_id(&format!("cart-remove-{i}")),
                ),
        )
}

/// A line edited in place: its name, quantity, unit price and discount.
fn line_dialog(st: AppState, t: Till, i: usize, close: impl Fn() + Clone + 'static) -> View {
    let Some(it) = t.cart.with_untracked(|v| v.get(i).cloned()) else {
        return view();
    };
    let c = currency(st);
    let name = signal(it.name.clone());
    let qty = signal(Quantity::from_milli(it.qty_milli).to_string());
    let price = signal(Money::from_minor(it.price_minor, c).to_plain_string());
    let discount = signal(if it.discount_bps > 0 {
        Quantity::from_milli(it.discount_bps * 10).to_string()
    } else {
        String::new()
    });
    let error = signal(String::new());
    let parse_discount = move || -> Option<i64> {
        let s = discount.get();
        if s.trim().is_empty() {
            return Some(0);
        }
        let q: Quantity = s.trim().replace(',', ".").parse().ok()?;
        let bps = nb_store::repo::to_milli(q).ok()? / 10;
        (0..=10_000).contains(&bps).then_some(bps)
    };
    let catalog = it.catalog_price();
    let base = it.clone();
    let preview = computed(move || {
        let (Some(q), Ok(p), Some(d)) = (
            parse_qty(&qty.get()),
            parse_price(&price.get(), c),
            parse_discount(),
        ) else {
            return None;
        };
        let line = Item {
            qty_milli: q,
            price_minor: p.minor(),
            discount_bps: d,
            ..base.clone()
        };
        cart_of(&[line], c).totals().ok().map(|x| x.total.minor())
    });
    let apply = {
        let close = close.clone();
        move || {
            let (Some(q), Ok(p), Some(d)) = (
                parse_qty(&qty.get_untracked()),
                parse_price(&price.get_untracked(), c),
                parse_discount(),
            ) else {
                error.set(error_text(st, "invalid_value"));
                return;
            };
            let n = name.get_untracked().trim().to_owned();
            if n.is_empty() {
                error.set(error_text(st, "required"));
                return;
            }
            t.cart.update(|v| {
                if let Some(l) = v.get_mut(i) {
                    l.qty_milli = q;
                    l.price_minor = p.minor();
                    l.discount_bps = d;
                    l.name = n.clone();
                }
            });
            close();
        }
    };
    on_key("enter", {
        let apply = apply.clone();
        move |_| apply()
    });
    let step = move |delta: i64| {
        move || {
            let q = parse_qty(&qty.get_untracked()).unwrap_or(0) + delta;
            if q > 0 {
                qty.set(Quantity::from_milli(q).to_string());
            }
        }
    };
    let qty_row = view()
        .flex_row()
        .items_end()
        .gap(px(8.0))
        .child(
            stepper(
                st,
                Icon::Minus,
                "pos.less",
                step(-1_000),
                "line-less".into(),
            )
            .w(px(44.0))
            .h(px(44.0)),
        )
        .child(view().flex_1().child(field(
            st.t("pos.qty"),
            qty,
            String::new,
            String::new,
            KeyboardType::Decimal,
            "line-qty",
        )))
        .child(
            stepper(st, Icon::Plus, "pos.more", step(1_000), "line-more".into())
                .w(px(44.0))
                .h(px(44.0)),
        );
    let mut quick = view().flex_row().flex_wrap().gap(px(6.0));
    for n in [1, 2, 3, 5, 6, 10, 12, 24] {
        quick = quick.child(
            button(
                Variant::Secondary,
                Size::Sm,
                None,
                move || n.to_string(),
                move || qty.set(n.to_string()),
            )
            .test_id(&format!("line-qty-{n}")),
        );
    }
    let price_hint = move || match catalog {
        Some(p) => tr_args(st.lang(), "pos.catalog_price", &[("price", &money(st, p))]),
        None => String::new(),
    };
    let body = view()
        .gap(px(14.0))
        .child(field(
            st.t("pos.line_name"),
            name,
            String::new,
            String::new,
            KeyboardType::Default,
            "line-name",
        ))
        .child(qty_row)
        .child(quick)
        .child(
            view()
                .flex_row()
                .gap(px(12.0))
                .child(view().flex_1().child(labelled(
                    st.t("pos.unit_price"),
                    view().child(field(
                        String::new,
                        price,
                        String::new,
                        String::new,
                        KeyboardType::Decimal,
                        "line-price",
                    )),
                    Some(Box::new(price_hint)),
                )))
                .child(view().w(px(120.0)).child(field(
                    st.t("pos.discount_pct"),
                    discount,
                    || "0".to_owned(),
                    String::new,
                    KeyboardType::Decimal,
                    "line-discount",
                ))),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .p(px(12.0))
                .rounded(px(radius::md()))
                .bg(Tone::Subtle.color())
                .child(view().flex_1().child(label_line(
                    st.t("pos.line_total"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )))
                .child(figure(
                    move || {
                        preview
                            .get()
                            .map_or_else(|| "—".to_owned(), |m| money(st, m))
                    },
                    ty::LG,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        )
        .child(error_line(error));
    let remove = {
        let close = close.clone();
        move || {
            t.remove(i);
            close();
        }
    };
    let footer = view()
        .child(
            button(
                Variant::Ghost,
                Size::Md,
                Some(Icon::Trash),
                st.t("action.delete"),
                remove,
            )
            .test_id("line-remove"),
        )
        .child(view().flex_1())
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
                st.t("pos.apply"),
                apply,
            )
            .test_id("line-apply"),
        );
    dialog_frame(
        st,
        st.t("pos.edit_line"),
        st.t("pos.edit_line.help"),
        body,
        footer,
        close,
    )
}

/// An item that is not in the catalog: its name, price, VAT and quantity.
fn free_dialog(st: AppState, t: Till, close: impl Fn() + Clone + 'static) -> View {
    let c = currency(st);
    let taxes = resource(
        move || st.session.get().map(|_| ()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_tax_classes(s)).await }
        },
    );
    let rates = computed(move || {
        taxes
            .with(|v: Option<&Vec<TaxClass>>| v.cloned())
            .unwrap_or_default()
    });
    let rate = signal(usize::MAX);
    effect(move || {
        let all = rates.get();
        if rate.get_untracked() == usize::MAX && !all.is_empty() {
            rate.set(all.iter().position(|r| r.is_default).unwrap_or(0));
        }
    });
    let name = signal(String::new());
    let price = signal(String::new());
    let qty = signal("1".to_owned());
    let error = signal(String::new());
    let add = {
        let close = close.clone();
        move || {
            let n = name.get_untracked().trim().to_owned();
            if n.is_empty() {
                error.set(error_text(st, "sale.free_name"));
                return;
            }
            let Ok(p) = parse_price(&price.get_untracked(), c) else {
                error.set(error_text(st, "sale.free_price"));
                return;
            };
            let Some(q) = parse_qty(&qty.get_untracked()) else {
                error.set(error_text(st, "sale.qty_positive"));
                return;
            };
            let tax_bps = rates
                .get_untracked()
                .get(rate.get_untracked())
                .map_or(0, |r| r.rate_bps);
            t.cart.update(|v| {
                v.push(Item {
                    product: None,
                    name: n.clone(),
                    unit: String::new(),
                    qty_milli: q,
                    price_minor: p.minor(),
                    tax_bps,
                    discount_bps: 0,
                })
            });
            let n = t.cart.with_untracked(Vec::len);
            t.selected.set(Some(n - 1));
            close();
        }
    };
    on_key("enter", {
        let add = add.clone();
        move |_| add()
    });
    let body = view()
        .gap(px(14.0))
        .child(field(
            st.t("pos.line_name"),
            name,
            st.t("pos.free.name_hint"),
            String::new,
            KeyboardType::Default,
            "free-name",
        ))
        .child(
            view()
                .flex_row()
                .gap(px(12.0))
                .child(view().flex_1().child(field(
                    st.t("pos.unit_price"),
                    price,
                    || "0,00".to_owned(),
                    String::new,
                    KeyboardType::Decimal,
                    "free-price",
                )))
                .child(view().w(px(110.0)).child(field(
                    st.t("pos.qty"),
                    qty,
                    String::new,
                    String::new,
                    KeyboardType::Decimal,
                    "free-qty",
                ))),
        )
        .child(labelled(
            st.t("pos.vat"),
            dropdown(
                move || {
                    rates
                        .get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, r)| (i, r.name))
                        .collect()
                },
                rate,
                Some(Icon::Percent),
                st.t("pos.vat"),
                "free-vat",
            ),
            None,
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
                Some(Icon::Plus),
                st.t("pos.free.add"),
                add,
            )
            .test_id("free-add"),
        );
    dialog_frame(
        st,
        st.t("pos.free"),
        st.t("pos.free.help"),
        body,
        footer,
        close,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Cash,
    Card,
    Transfer,
}

fn pay_dialog(st: AppState, reg: Register, t: Till, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    // One operation for this payment screen: a second tap returns the sale
    // already made.
    let meta = session.meta("sale.complete");
    let c = session.currency;
    let items = t.cart.get_untracked();
    let due = cart_of(&items, c).totals().map_or(0, |x| x.total.minor());
    let way = signal(Way::Cash);
    let given = signal(Money::from_minor(due, c).to_plain_string());
    let error = signal(String::new());
    let busy = signal(false);
    let given_minor = move || parse_price(&given.get(), c).ok().map(|m| m.minor());
    let change = computed(move || match (way.get(), given_minor()) {
        (Way::Cash, Some(g)) if g > due => g - due,
        _ => 0,
    });
    let short = computed(move || match (way.get(), given_minor()) {
        (Way::Cash, Some(g)) if g < due => due - g,
        _ => 0,
    });
    let save = {
        let close = close.clone();
        let warehouse = session.warehouse().to_string();
        let register = reg.id.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
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
                lines: t
                    .cart
                    .get_untracked()
                    .iter()
                    .map(|i| SaleLineInput {
                        product_id: i.product.as_ref().map(|p| p.id.clone()).unwrap_or_default(),
                        qty: Quantity::from_milli(i.qty_milli).to_string(),
                        unit_price_minor: (i.catalog_price() != Some(i.price_minor))
                            .then_some(i.price_minor),
                        discount_bps: (i.discount_bps > 0).then_some(i.discount_bps),
                        name: (i.product.as_ref().map(|p| &p.name) != Some(&i.name))
                            .then(|| i.name.clone()),
                        tax_bps: i.product.is_none().then_some(i.tax_bps),
                        ..Default::default()
                    })
                    .collect(),
                payments: vec![PaymentInput {
                    method: method.into(),
                    amount_minor: amount,
                }],
                ..Default::default()
            };
            busy.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                let r = db.call(move |s| complete_sale(s, meta, input)).await;
                busy.set(false);
                match r {
                    Ok(r) => {
                        t.clear();
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
                        t.search.focus();
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    on_key("f9", {
        let save = save.clone();
        move |_| save()
    });
    on_key("enter", {
        let save = save.clone();
        move |_| save()
    });
    let quick = {
        let mut v = view().flex_row().flex_wrap().gap(px(8.0));
        let mut amounts = vec![due];
        for step in [10_000, 50_000, 100_000, 200_000, 500_000] {
            let up = (due + step - 1) / step * step;
            if up > due && !amounts.contains(&up) {
                amounts.push(up);
            }
        }
        for (i, a) in amounts.into_iter().take(5).enumerate() {
            v = v.child(
                button(
                    Variant::Secondary,
                    Size::Md,
                    None,
                    move || {
                        if i == 0 {
                            st.tn("pos.exact")
                        } else {
                            money(st, a)
                        }
                    },
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
                .p(px(12.0))
                .rounded(px(radius::md()))
                .bg(move || {
                    if short.get() > 0 {
                        Tone::DangerSoft.color()
                    } else {
                        Tone::SuccessSoft.color()
                    }
                })
                .child(view().flex_1().child(label_line(
                    move || {
                        st.tn(if short.get() > 0 {
                            "pos.missing"
                        } else {
                            "pos.change"
                        })
                    },
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                )))
                .child(figure(
                    move || {
                        money(
                            st,
                            if short.get() > 0 {
                                short.get()
                            } else {
                                change.get()
                            },
                        )
                    },
                    ty::H2,
                    FontWeight::BOLD,
                    move || {
                        if short.get() > 0 {
                            Tone::Danger
                        } else {
                            Tone::Success
                        }
                    },
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
                ))
                .child(label_line(
                    move || {
                        tr_args(
                            st.lang(),
                            "pos.cart.count",
                            &[("count", &st.fmt().integer(items.len() as i64))],
                        )
                    },
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
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
                Size::Lg,
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

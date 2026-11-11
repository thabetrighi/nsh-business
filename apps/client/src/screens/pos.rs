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
use nb_store::usecases::customers::{CustomerQuery, CustomerRow, list_customers};
use nb_store::usecases::documents::get_document;
use nb_store::usecases::org::{Register, registers};
use nb_store::usecases::products::list_products;
use nb_store::usecases::returns::{
    HeldCart, ReturnInput, ReturnLineInput, SaleDetail, held_carts, hold_cart, return_sale,
    sale_for_return, take_held,
};
use nb_store::usecases::sales::{
    PaymentInput, SaleInput, SaleLineInput, ShiftRow, close_shift, complete_sale, current_shift,
    open_shift,
};
use nsh_core::element::{AnyElement, ElementRef, KeyboardType, Owner, View, element_ref, input};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::nav::Route;
use crate::screens::approval::{Ask, approval_dialog, needs_approval};
use crate::screens::common::page_head;
use crate::screens::forms::{error_text, parse_price};
use crate::state::AppState;
use crate::ui::form::{field, labelled, row_input};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, checkbox, figure, icon_button, kbd, label, label_line, segmented,
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// A discount on the whole sale: a percentage (bps) or an amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
struct SaleDiscount {
    bps: i64,
    minor: i64,
}

impl SaleDiscount {
    fn is_none(self) -> bool {
        self.bps == 0 && self.minor == 0
    }
    fn of(self, c: nb_core::Currency) -> Discount {
        if self.bps > 0 {
            Discount::Percent(self.bps)
        } else if self.minor > 0 {
            Discount::Amount(Money::from_minor(self.minor, c))
        } else {
            Discount::None
        }
    }
}

/// A cart put aside, as the till saves it.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
struct Held {
    items: Vec<Item>,
    #[serde(default)]
    discount: SaleDiscount,
    /// The quote or order being invoiced (id, number).
    #[serde(default)]
    document: Option<(String, String)>,
    #[serde(default)]
    customer: Option<CustomerRow>,
}

/// The cart as the domain prices it (prices with tax).
fn cart_of(items: &[Item], sale: SaleDiscount, c: nb_core::Currency) -> Cart {
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
        discount: sale.of(c),
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
    /// The discount on the whole sale, and its dialog.
    discount: Signal<SaleDiscount>,
    discount_open: Signal<bool>,
    /// The quote or order this sale invoices (id, number).
    document: Signal<Option<(String, String)>>,
    /// Who buys (none: a counter customer), and its picker.
    customer: Signal<Option<CustomerRow>>,
    customer_open: Signal<bool>,
    search: ElementRef,
    /// This device's till (once known).
    register: Signal<Option<String>>,
    /// The carts put aside at it, reloaded when `held_rev` moves.
    held: Memo<Vec<HeldCart>>,
    held_rev: Signal<u32>,
    recalling: Signal<bool>,
    returning: Signal<bool>,
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
        self.discount.set(SaleDiscount::default());
        self.document.set(None);
        self.customer.set(None);
        self.selected.set(None);
    }
    /// The cart's figures as the domain prices them.
    fn totals(self, c: nb_core::Currency) -> Option<Totals> {
        let items = self.cart.get();
        if items.is_empty() {
            return None;
        }
        cart_of(&items, self.discount.get(), c).totals().ok()
    }
    /// The cart as it is put aside: what it saves, its lines and total.
    fn snapshot(self, c: nb_core::Currency) -> Option<(String, u32, i64)> {
        let held = Held {
            items: self.cart.get_untracked(),
            discount: self.discount.get_untracked(),
            document: self.document.get_untracked(),
            customer: self.customer.get_untracked(),
        };
        if held.items.is_empty() {
            return None;
        }
        let total = cart_of(&held.items, held.discount, c)
            .totals()
            .map_or(0, |x| x.total.minor());
        let lines = u32::try_from(held.items.len()).unwrap_or(u32::MAX);
        serde_json::to_string(&held).ok().map(|p| (p, lines, total))
    }
    /// Puts back a cart saved by [`Till::snapshot`].
    fn restore(self, payload: &str) {
        let held: Held = serde_json::from_str(payload).unwrap_or_default();
        let n = held.items.len();
        self.cart.set(held.items);
        self.discount.set(held.discount);
        self.document.set(held.document);
        self.customer.set(held.customer);
        self.selected.set(n.checked_sub(1));
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
    let register: Signal<Option<String>> = signal(None);
    let held_rev = signal(0u32);
    effect(move || {
        let id = state.with(|s| s.as_ref().map(|(r, _)| r.id.clone()));
        if register.get_untracked() != id {
            register.set(id);
        }
    });
    let held_list = resource(
        move || register.get().map(|r| (r, held_rev.get())),
        move |(reg, _): (String, u32), _| {
            let db = st.db();
            async move { db.call(move |s| held_carts(s, &reg)).await }
        },
    );
    let held = computed(move || {
        held_list.with(|v: Option<&Vec<HeldCart>>| v.cloned().unwrap_or_default())
    });
    let t = Till {
        register,
        held,
        held_rev,
        recalling: signal(false),
        returning: signal(false),
        cart: signal(Vec::new()),
        selected: signal(None),
        editing: signal(None),
        editing_open: signal(false),
        free_open: signal(false),
        paying: signal(false),
        cart_open: signal(false),
        discount: signal(SaleDiscount::default()),
        discount_open: signal(false),
        document: signal(None),
        customer: signal(None),
        customer_open: signal(false),
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
            && !t.recalling.get_untracked()
            && !t.returning.get_untracked()
            && !t.discount_open.get_untracked()
            && !t.customer_open.get_untracked()
    };

    // A quote or order sent to the till (from the sales documents): its
    // lines fill the cart once the till is open. The loading runs under
    // the page (the effect runs again when it clears the request, which
    // would end a task of its own).
    let page_owner = Owner::current();
    effect(move || {
        let Some(crate::state::Intent::Invoice(id)) = st.intent.get() else {
            return;
        };
        if !matches!(state.get(), Some((_, Some(_)))) {
            return;
        }
        st.intent.set(None);
        let db = st.db();
        let task = page_owner.run(|| {
            spawn(async move {
                let loaded = db
                    .call(move |s| {
                        let doc = get_document(s, &id)?;
                        let mut items = Vec::with_capacity(doc.lines.len());
                        for l in &doc.lines {
                            let product = match l.product_id.as_deref() {
                                Some(p) => Some(s.read(|r| {
                                    nb_store::repo::catalog::get(
                                        r.conn(),
                                        nb_store::repo::parse_id(p)?,
                                    )
                                })?),
                                None => None,
                            };
                            items.push(Item {
                                product,
                                name: l.name.clone(),
                                unit: l.unit.clone(),
                                qty_milli: l.qty_milli,
                                price_minor: l.unit_price_minor,
                                tax_bps: l.tax_bps,
                                discount_bps: l.discount_bps,
                            });
                        }
                        Ok((doc, items))
                    })
                    .await;
                match loaded {
                    Ok((doc, items)) => {
                        let n = items.len();
                        t.cart.set(items);
                        t.discount.set(SaleDiscount {
                            bps: doc.discount_bps,
                            minor: doc.discount_minor,
                        });
                        t.document
                            .set(Some((doc.row.id.clone(), doc.row.number.clone())));
                        t.selected.set(n.checked_sub(1));
                    }
                    Err(e) => show_toast(st, first_error(st, &e)),
                }
            })
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    });

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
    on_key("f5", move |_| {
        if selling() && no_dialog() && !t.cart.with_untracked(Vec::is_empty) {
            t.discount_open.set(true);
        }
    });
    on_key("f10", move |_| {
        if selling() && no_dialog() {
            t.customer_open.set(true);
        }
    });
    on_key("f7", move |_| {
        if selling() && no_dialog() {
            hold(st, t);
        }
    });
    on_key("f3", move |_| {
        if selling() && no_dialog() {
            t.recalling.set(true);
        }
    });
    on_key("f12", move |_| {
        if selling() && no_dialog() {
            t.returning.set(true);
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
            .child(dynamic(move || {
                let n = t.held.get().len();
                if n == 0 {
                    return view().into_element();
                }
                let caption = move || {
                    tr_args(
                        st.lang(),
                        "pos.held.count",
                        &[("count", &st.fmt().integer(n as i64))],
                    )
                };
                if compact {
                    icon_button(
                        || Icon::Clock,
                        caption,
                        Variant::Secondary,
                        move || t.recalling.set(true),
                    )
                } else {
                    button(
                        Variant::Secondary,
                        Size::Md,
                        Some(Icon::Clock),
                        caption,
                        move || t.recalling.set(true),
                    )
                }
                .test_id("pos-held")
                .into_element()
            }))
            // A cashier may ask a supervisor for a return.
            .child(when(
                move || st.can("sales.return") || st.can("sales.sell"),
                move || {
                    if compact {
                        icon_button(
                            || Icon::Undo,
                            st.t("pos.return"),
                            Variant::Secondary,
                            move || t.returning.set(true),
                        )
                    } else {
                        button(
                            Variant::Secondary,
                            Size::Md,
                            Some(Icon::Undo),
                            st.t("pos.return"),
                            move || t.returning.set(true),
                        )
                    }
                    .test_id("pos-return")
                    .into_element()
                },
            ))
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
        .child(modal(t.customer_open, compact, move || {
            customer_dialog(st, t, move || t.customer_open.set(false)).into_element()
        }))
        .child(modal(t.discount_open, compact, move || {
            discount_dialog(st, t, move || t.discount_open.set(false)).into_element()
        }))
        .child(modal(t.recalling, compact, move || {
            held_dialog(st, t, move || t.recalling.set(false)).into_element()
        }))
        .child(modal(t.returning, compact, move || {
            return_dialog(st, t, move || t.returning.set(false)).into_element()
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
        ("F10", "pos.key.customer"),
        ("F5", "pos.key.discount"),
        ("F7", "pos.key.hold"),
        ("F3", "pos.key.held"),
        ("F12", "pos.key.return"),
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
    let total = computed(move || t.totals(currency(st)).map_or(0, |x| x.total.minor()));
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
    let totals = computed(move || t.totals(currency(st)));
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
        .child(when(
            move || !t.cart.get().is_empty(),
            move || {
                view()
                    .flex_row()
                    .child(
                        button(
                            Variant::Ghost,
                            Size::Sm,
                            Some(Icon::Percent),
                            move || {
                                let d = t.discount.get();
                                if d.is_none() {
                                    st.t("pos.discount.add")()
                                } else if d.bps > 0 {
                                    tr_args(
                                        st.lang(),
                                        "pos.discount.percent",
                                        &[(
                                            "rate",
                                            &st.fmt().quantity(Quantity::from_milli(d.bps * 10), 2),
                                        )],
                                    )
                                } else {
                                    tr_args(
                                        st.lang(),
                                        "pos.discount.amount",
                                        &[("amount", &money(st, d.minor))],
                                    )
                                }
                            },
                            move || {
                                t.cart_open.set(false);
                                t.discount_open.set(true);
                            },
                        )
                        .test_id(if in_sheet {
                            "sheet-discount"
                        } else {
                            "pos-discount"
                        }),
                    )
                    .into_element()
            },
        ))
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
                    view()
                        .flex_row()
                        .gap(px(4.0))
                        .child(
                            button(
                                Variant::Ghost,
                                Size::Sm,
                                Some(Icon::Clock),
                                st.t("pos.hold"),
                                move || hold(st, t),
                            )
                            .test_id(if in_sheet {
                                "sheet-hold"
                            } else {
                                "cart-hold"
                            }),
                        )
                        .child(
                            button(
                                Variant::Ghost,
                                Size::Sm,
                                Some(Icon::Trash),
                                st.t("pos.cart.clear"),
                                move || t.clear(),
                            )
                            .test_id("cart-clear"),
                        )
                        .into_element()
                },
            )),
        ))
        .child(customer_chip(st, t, in_sheet))
        .child(dynamic(move || match t.document.get() {
            Some((_, number)) => view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(8.0))
                .bg(Tone::PrimarySoft.color())
                .child(icon(Icon::FileText, 16.0, || Tone::Primary))
                .child(view().flex_1().min_w(px(0.0)).child(label_line(
                    move || tr_args(st.lang(), "pos.from_document", &[("number", &number)]),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Ink,
                )))
                .child(
                    icon_button(
                        || Icon::Close,
                        st.t("pos.detach_document"),
                        Variant::Ghost,
                        move || t.document.set(None),
                    )
                    .test_id("pos-detach"),
                )
                .into_element(),
            None => view().into_element(),
        }))
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
        cart_of(&[line], SaleDiscount::default(), c)
            .totals()
            .ok()
            .map(|x| x.total.minor())
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
    /// All of it on the customer's account.
    Credit,
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
    let sale_discount = t.discount.get_untracked();
    let due = cart_of(&items, sale_discount, c)
        .totals()
        .map_or(0, |x| x.total.minor());
    let customer = t.customer.get_untracked();
    let has_customer = customer.is_some();
    let way = signal(Way::Cash);
    let given = signal(Money::from_minor(due, c).to_plain_string());
    let error = signal(String::new());
    let busy = signal(false);
    // A supervisor's approval, when the sale needs one (a large discount,
    // credit past the limit…): asked on the spot, then the sale is sent again.
    let owner = Owner::current();
    let approval: Signal<Option<String>> = signal(None);
    let asking: Signal<Option<Ask>> = signal(None);
    let asking_open = signal(false);
    effect(move || {
        let open = asking.get().is_some();
        if asking_open.get_untracked() != open {
            asking_open.set(open);
        }
    });
    effect(move || {
        if !asking_open.get() {
            asking.set(None);
        }
    });
    let branch_id = session.branch().to_string();
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
                Way::Credit => "credit",
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
                discount_bps: (sale_discount.bps > 0).then_some(sale_discount.bps),
                discount_minor: (sale_discount.minor > 0).then_some(sale_discount.minor),
                document_id: t.document.get_untracked().map(|d| d.0),
                customer_id: t.customer.get_untracked().map(|c| c.id),
                approval: approval.get_untracked(),
                ..Default::default()
            };
            busy.set(true);
            let close = close.clone();
            let ask_at = (branch_id.clone(), register.clone());
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
                    Err(e) => {
                        if let Some(p) = needs_approval(&e) {
                            asking.set(Some(Ask {
                                permission: p,
                                branch_id: Some(ask_at.0),
                                register_id: Some(ask_at.1),
                            }));
                        }
                        error.set(first_error(st, &e));
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let retry = save.clone();
    // While a supervisor is asked, the keys are theirs.
    on_key("f9", {
        let save = save.clone();
        move |_| {
            if !asking_open.get_untracked() {
                save()
            }
        }
    });
    on_key("enter", {
        let save = save.clone();
        move |_| {
            if !asking_open.get_untracked() {
                save()
            }
        }
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
                    if short.get() > 0 && !has_customer {
                        Tone::DangerSoft.color()
                    } else if short.get() > 0 {
                        Tone::WarningSoft.color()
                    } else {
                        Tone::SuccessSoft.color()
                    }
                })
                .child(view().flex_1().child(label_line(
                    move || {
                        st.tn(if short.get() > 0 && has_customer {
                            "pos.on_account"
                        } else if short.get() > 0 {
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
                        if short.get() > 0 && has_customer {
                            Tone::Warning
                        } else if short.get() > 0 {
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
                {
                    let mut ways = vec![
                        (
                            Way::Cash,
                            Box::new(st.t("pos.method.cash")) as Box<dyn Fn() -> String>,
                        ),
                        (Way::Card, Box::new(st.t("pos.method.card"))),
                        (Way::Transfer, Box::new(st.t("pos.method.transfer"))),
                    ];
                    if has_customer {
                        ways.push((Way::Credit, Box::new(st.t("pos.method.credit"))));
                    }
                    ways
                },
                way,
            ),
            None,
        ))
        .child(match customer {
            Some(cu) => {
                let after = cu.balance_minor;
                let limit = cu.credit_limit_minor;
                let name = cu.name.clone();
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(Icon::User, 16.0, || Tone::Muted))
                    .child(label_line(
                        move || name.clone(),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    ))
                    .child(view().flex_1())
                    .child(label_line(
                        st.t("pos.customer.owes"),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    ))
                    .child(figure(
                        move || {
                            let more = match way.get() {
                                Way::Credit => due,
                                Way::Cash => short.get(),
                                _ => 0,
                            };
                            money(st, after + more)
                        },
                        ty::SM,
                        FontWeight::BOLD,
                        move || {
                            let more = match way.get() {
                                Way::Credit => due,
                                Way::Cash => short.get(),
                                _ => 0,
                            };
                            if after + more > limit {
                                Tone::Danger
                            } else {
                                Tone::Ink
                            }
                        },
                    ))
                    .into_element()
            }
            None => view().into_element(),
        })
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
    view()
        .flex_shrink(1.0)
        .min_h(px(0.0))
        .child(dialog_frame(
            st,
            st.t("pos.pay"),
            st.t("pos.pay.help"),
            body,
            footer,
            close,
        ))
        .child(modal(asking_open, false, move || {
            match asking.get_untracked() {
                Some(ask) => {
                    let retry = retry.clone();
                    approval_dialog(
                        st,
                        ask,
                        move |id| {
                            approval.set(Some(id));
                            // Under this dialog: the approval's own closes now.
                            owner.run(&retry);
                        },
                        move || asking_open.set(false),
                    )
                    .into_element()
                }
                None => view().into_element(),
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
        ("pos.collections", shift.collections_minor),
        ("pos.other_income", shift.other_income_minor),
        ("pos.expenses", -shift.expenses_minor),
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

// ---------------------------------------------------------------------------
// Carts put aside (M8.7)

/// Puts the cart aside (the next customer is served meanwhile).
fn hold(st: AppState, t: Till) {
    let (Some(session), Some(reg)) = (st.session.get_untracked(), t.register.get_untracked())
    else {
        return;
    };
    let Some((payload, lines, total)) = t.snapshot(session.currency) else {
        return;
    };
    let meta = session.meta("cart.hold");
    let db = st.db();
    let task = spawn(async move {
        match db
            .call(move |s| hold_cart(s, meta, &reg, "", lines, total, &payload))
            .await
        {
            Ok(h) => {
                t.clear();
                t.cart_open.set(false);
                t.held_rev.update(|n| *n += 1);
                show_toast(st, tr_args(st.lang(), "pos.held", &[("label", &h.label)]));
            }
            Err(e) => show_toast(st, first_error(st, &e)),
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

/// Takes a cart back; the cart being served, if any, is put aside first.
fn recall(st: AppState, t: Till, id: String, close: impl Fn() + 'static) {
    let (Some(session), Some(reg)) = (st.session.get_untracked(), t.register.get_untracked())
    else {
        return;
    };
    let current = t
        .snapshot(session.currency)
        .map(|(p, lines, total)| (p, lines, total, session.meta("cart.hold")));
    let meta = session.meta("cart.take");
    let db = st.db();
    let task = spawn(async move {
        let r = db
            .call(move |s| {
                if let Some((payload, lines, total, m)) = current {
                    hold_cart(s, m, &reg, "", lines, total, &payload)?;
                }
                take_held(s, meta, &id)
            })
            .await;
        match r {
            Ok(h) => {
                t.restore(&h.payload);
                t.held_rev.update(|n| *n += 1);
                close();
            }
            Err(e) => show_toast(st, first_error(st, &e)),
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

/// The carts put aside at this till: tap one to take it back.
fn held_dialog(st: AppState, t: Till, close: impl Fn() + Clone + 'static) -> View {
    let list = {
        let close = close.clone();
        dynamic(move || {
            let held = t.held.get();
            if held.is_empty() {
                return view()
                    .p(px(20.0))
                    .items_center()
                    .child(label(
                        st.t("pos.held.none"),
                        ty::SM,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    ))
                    .into_element();
            }
            let mut v = view().gap(px(8.0));
            for (i, h) in held.into_iter().enumerate() {
                let close = close.clone();
                let id = h.id.clone();
                let lines = i64::from(h.lines);
                v = v.child(
                    view()
                        .test_id(&format!("held-{i}"))
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .p(px(12.0))
                        .rounded(px(radius::md()))
                        .border_1()
                        .border_color(Tone::Border.color())
                        .cursor_pointer()
                        .on_press(move || recall(st, t, id.clone(), close.clone()))
                        .child(icon(Icon::Clock, 18.0, || Tone::Primary))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(label_line(
                                    move || h.label.clone(),
                                    ty::MD,
                                    FontWeight::SEMIBOLD,
                                    Tone::Ink,
                                ))
                                .child(label_line(
                                    move || {
                                        tr_args(
                                            st.lang(),
                                            "pos.cart.count",
                                            &[("count", &st.fmt().integer(lines))],
                                        )
                                    },
                                    ty::XS,
                                    FontWeight::NORMAL,
                                    Tone::Muted,
                                )),
                        )
                        .child(figure(
                            move || money(st, h.total_minor),
                            ty::MD,
                            FontWeight::BOLD,
                            || Tone::Ink,
                        )),
                );
            }
            v.into_element()
        })
    };
    let body = view()
        .gap(px(12.0))
        .child(when(
            move || !t.cart.get().is_empty(),
            move || {
                label(
                    st.t("pos.held.swap"),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )
                .into_element()
            },
        ))
        .child(list);
    let footer = view().child(button(
        Variant::Secondary,
        Size::Md,
        None,
        st.t("action.close"),
        close.clone(),
    ));
    dialog_frame(
        st,
        st.t("pos.held.title"),
        st.t("pos.held.help"),
        body,
        footer,
        close,
    )
}

// ---------------------------------------------------------------------------
// Returns (M8.8)

/// A refund way and its caption.
type Choice = (&'static str, Box<dyn Fn() -> String>);

/// Goods brought back: find the sale by its number, choose what comes back
/// (and whether it goes back on the shelf), how it is refunded, and why.
fn return_dialog(st: AppState, t: Till, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    // Made when the dialog opens: a double tap returns once.
    let meta = session.meta("sale.return");
    let number = signal(String::new());
    let found: Signal<Option<SaleDetail>> = signal(None);
    let qtys: Signal<Vec<String>> = signal(Vec::new());
    let restock: Signal<Vec<bool>> = signal(Vec::new());
    let method = signal("cash");
    let reason = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let owner = Owner::current();
    let approval: Signal<Option<String>> = signal(None);
    let asking: Signal<Option<Ask>> = signal(None);
    let asking_open = signal(false);
    effect(move || {
        let open = asking.get().is_some();
        if asking_open.get_untracked() != open {
            asking_open.set(open);
        }
    });
    effect(move || {
        if !asking_open.get() {
            asking.set(None);
        }
    });
    let branch_id = session.branch().to_string();

    let find = move || {
        let key = number.get_untracked();
        if key.trim().is_empty() {
            return;
        }
        error.set(String::new());
        let db = st.db();
        let task = spawn(async move {
            match db.call(move |s| sale_for_return(s, &key)).await {
                Ok(d) => {
                    qtys.set(vec![String::new(); d.lines.len()]);
                    restock.set(d.lines.iter().map(|l| l.product_id.is_some()).collect());
                    if d.customer_id.is_none() && method.get_untracked() == "credit" {
                        method.set("cash");
                    }
                    found.set(Some(d));
                }
                Err(e) => {
                    found.set(None);
                    error.set(first_error(st, &e));
                }
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    on_key("enter", move |_| {
        if found.get_untracked().is_none() {
            find();
        }
    });

    // What goes back, as the sale priced it (the store computes the exact
    // shares; this is the preview).
    let total = computed(move || {
        let q = qtys.get();
        found.with(|f| {
            f.as_ref().map_or(0, |d| {
                d.lines
                    .iter()
                    .zip(q.iter())
                    .map(|(l, q)| {
                        let back = parse_qty(q).unwrap_or(0).min(l.returnable_milli());
                        if l.qty_milli == 0 {
                            0
                        } else {
                            i64::try_from(
                                i128::from(l.gross_minor) * i128::from(back)
                                    / i128::from(l.qty_milli),
                            )
                            .unwrap_or(0)
                        }
                    })
                    .sum()
            })
        })
    });

    let save = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let (Some(d), Some(reg)) = (found.get_untracked(), t.register.get_untracked()) else {
                return;
            };
            let q = qtys.get_untracked();
            let r = restock.get_untracked();
            let lines: Vec<ReturnLineInput> = d
                .lines
                .iter()
                .enumerate()
                .filter_map(|(i, l)| {
                    let text = q.get(i)?.trim().to_owned();
                    (!text.is_empty()).then(|| ReturnLineInput {
                        position: l.position,
                        qty: text,
                        restock: r.get(i).copied().unwrap_or(true),
                    })
                })
                .collect();
            let input = ReturnInput {
                sale: d.id.clone(),
                register_id: reg,
                lines,
                refund: method.get_untracked().to_owned(),
                reason: reason.get_untracked(),
                approval: approval.get_untracked(),
            };
            let ask_at = (branch_id.clone(), input.register_id.clone());
            busy.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| return_sale(s, meta, input)).await {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "pos.returned",
                                &[("number", &r.number), ("amount", &money(st, r.total_minor))],
                            ),
                        );
                    }
                    Err(e) => {
                        busy.set(false);
                        if let Some(p) = needs_approval(&e) {
                            asking.set(Some(Ask {
                                permission: p,
                                branch_id: Some(ask_at.0),
                                register_id: Some(ask_at.1),
                            }));
                        }
                        error.set(first_error(st, &e));
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };

    let search = view()
        .flex_row()
        .items_end()
        .gap(px(8.0))
        .child(view().flex_1().child(field(
            st.t("pos.return.sale"),
            number,
            || "FV-01-…".to_owned(),
            String::new,
            KeyboardType::Default,
            "ret-sale",
        )))
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Search),
                st.t("pos.return.find"),
                find,
            )
            .test_id("ret-find"),
        );

    let lines = dynamic(move || {
        let Some(d) = found.get() else {
            return view().into_element();
        };
        let mut v = view().gap(px(8.0)).child(label_line(
            {
                let n = d.number.clone();
                move || n.clone()
            },
            ty::MD,
            FontWeight::BOLD,
            Tone::Ink,
        ));
        for (i, l) in d.lines.into_iter().enumerate() {
            let left = l.returnable_milli();
            let q = signal(String::new());
            effect(move || {
                let text = q.get();
                qtys.update(|v| {
                    if let Some(s) = v.get_mut(i) {
                        *s = text;
                    }
                });
            });
            let unit = l.unit.clone();
            let sold = l.qty_milli;
            let info = move || {
                tr_args(
                    st.lang(),
                    "pos.return.left",
                    &[
                        ("sold", &qty_text(st, sold)),
                        ("left", &qty_text(st, left)),
                        ("unit", &unit),
                    ],
                )
            };
            let mut row = view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(10.0))
                .py(px(8.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(
                    view()
                        .flex_1()
                        .min_w(px(140.0))
                        .child(label_line(
                            move || l.name.clone(),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(info, ty::XS, FontWeight::NORMAL, Tone::Muted)),
                );
            if left <= 0 {
                row = row.child(badge(Status::Neutral, None, st.t("pos.return.done")));
            } else {
                row = row
                    .child(view().w(px(90.0)).child(row_input(
                        q,
                        || "0".to_owned(),
                        KeyboardType::Decimal,
                        format!("ret-qty-{i}"),
                        move || parse_qty(&q.get()).is_some_and(|x| x > left),
                    )))
                    .child(
                        button(
                            Variant::Ghost,
                            Size::Sm,
                            None,
                            st.t("pos.return.all"),
                            move || q.set(Quantity::from_milli(left).to_string()),
                        )
                        .test_id(&format!("ret-all-{i}")),
                    );
                if l.product_id.is_some() {
                    row = row.child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                checkbox(
                                    move || {
                                        Some(restock.with(|r| r.get(i).copied().unwrap_or(true)))
                                    },
                                    move || {
                                        restock.update(|r| {
                                            if let Some(x) = r.get_mut(i) {
                                                *x = !*x;
                                            }
                                        })
                                    },
                                )
                                .test_id(&format!("ret-restock-{i}")),
                            )
                            .child(label_line(
                                st.t("pos.return.restock"),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Body,
                            )),
                    );
                }
            }
            v = v.child(row);
        }
        v.into_element()
    });

    let credit_ok =
        computed(move || found.with(|f| f.as_ref().is_some_and(|d| d.customer_id.is_some())));
    let refund = move || {
        dynamic(move || {
            let mut options: Vec<Choice> = vec![
                ("cash", Box::new(st.t("pos.method.cash"))),
                ("card", Box::new(st.t("pos.method.card"))),
                ("transfer", Box::new(st.t("pos.method.transfer"))),
            ];
            if credit_ok.get() {
                options.push(("credit", Box::new(st.t("pos.method.credit"))));
            }
            labelled(st.t("pos.return.refund"), segmented(options, method), None).into_element()
        })
    };

    let body = view()
        .gap(px(14.0))
        .child(search)
        .child(lines)
        .child(when(
            move || found.get().is_some(),
            move || {
                view()
                    .gap(px(14.0))
                    .child(refund())
                    .child(field(
                        st.t("pos.return.reason"),
                        reason,
                        String::new,
                        String::new,
                        KeyboardType::Default,
                        "ret-reason",
                    ))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .child(view().flex_1().child(label_line(
                                st.t("pos.return.total"),
                                ty::MD,
                                FontWeight::BOLD,
                                Tone::Ink,
                            )))
                            .child(figure(
                                move || money(st, total.get()),
                                ty::H2,
                                FontWeight::BOLD,
                                || Tone::Ink,
                            )),
                    )
                    .into_element()
            },
        ))
        .child(error_line(error));
    let save_again = save.clone();
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
                Some(Icon::Undo),
                st.t("pos.return.confirm"),
                save.clone(),
            )
            .test_id("ret-confirm"),
        );
    let retry = save_again;
    view()
        .flex_shrink(1.0)
        .min_h(px(0.0))
        .child(dialog_frame(
            st,
            st.t("pos.return.title"),
            st.t("pos.return.help"),
            body,
            footer,
            close,
        ))
        .child(modal(asking_open, false, move || {
            match asking.get_untracked() {
                Some(ask) => {
                    let retry = retry.clone();
                    approval_dialog(
                        st,
                        ask,
                        move |id| {
                            approval.set(Some(id));
                            // Under this dialog: the approval's own closes now.
                            owner.run(&retry);
                        },
                        move || asking_open.set(false),
                    )
                    .into_element()
                }
                None => view().into_element(),
            }
        }))
}

// ---------------------------------------------------------------------------
// Discount on the whole sale (M8.2)

/// A percentage typed by a person (`10`, `12,5`) in basis points.
fn parse_percent(s: &str) -> Option<i64> {
    let q: Quantity = s.trim().replace(',', ".").parse().ok()?;
    let bps = nb_store::repo::to_milli(q).ok()? / 10;
    (1..=10_000).contains(&bps).then_some(bps)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscountKind {
    Percent,
    Amount,
}

fn discount_dialog(st: AppState, t: Till, close: impl Fn() + Clone + 'static) -> View {
    let c = currency(st);
    let now = t.discount.get_untracked();
    let kind = signal(if now.minor > 0 {
        DiscountKind::Amount
    } else {
        DiscountKind::Percent
    });
    let value = signal(if now.bps > 0 {
        Quantity::from_milli(now.bps * 10).to_string()
    } else if now.minor > 0 {
        Money::from_minor(now.minor, c).to_plain_string()
    } else {
        String::new()
    });
    let error = signal(String::new());
    let typed = move || -> Option<SaleDiscount> {
        let v = value.get();
        match kind.get() {
            DiscountKind::Percent => parse_percent(&v).map(|bps| SaleDiscount { bps, minor: 0 }),
            DiscountKind::Amount => {
                parse_price(&v, c)
                    .ok()
                    .filter(|m| m.minor() > 0)
                    .map(|m| SaleDiscount {
                        bps: 0,
                        minor: m.minor(),
                    })
            }
        }
    };
    let preview = computed(move || {
        let d = typed()?;
        let items = t.cart.get();
        cart_of(&items, d, c).totals().ok().map(|x| x.total.minor())
    });
    let apply = {
        let close = close.clone();
        move || match untracked(typed) {
            Some(d) if untracked(move || preview.get()).is_some() => {
                t.discount.set(d);
                close();
            }
            _ => error.set(error_text(st, "pos.discount_invalid")),
        }
    };
    on_key("enter", {
        let apply = apply.clone();
        move |_| apply()
    });
    let mut quick = view().flex_row().flex_wrap().gap(px(6.0));
    for n in [5, 10, 15, 20] {
        quick = quick.child(
            button(
                Variant::Secondary,
                Size::Sm,
                None,
                move || format!("{n} %"),
                move || {
                    kind.set(DiscountKind::Percent);
                    value.set(n.to_string());
                },
            )
            .test_id(&format!("disc-{n}")),
        );
    }
    let body = view()
        .gap(px(14.0))
        .child(segmented(
            vec![
                (
                    DiscountKind::Percent,
                    Box::new(st.t("pos.discount.kind.percent")),
                ),
                (
                    DiscountKind::Amount,
                    Box::new(st.t("pos.discount.kind.amount")),
                ),
            ],
            kind,
        ))
        .child(field(
            move || {
                if kind.get() == DiscountKind::Percent {
                    st.t("pos.discount.rate")()
                } else {
                    st.t("pos.discount.value")()
                }
            },
            value,
            String::new,
            String::new,
            KeyboardType::Decimal,
            "disc-value",
        ))
        .child(quick)
        .child(
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(label_line(
                    st.t("pos.discount.new_total"),
                    ty::MD,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                )))
                .child(figure(
                    move || {
                        preview
                            .get()
                            .map_or_else(|| "—".to_owned(), |v| money(st, v))
                    },
                    ty::XL,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        )
        .child(error_line(error));
    let footer = view()
        .child(when(move || !t.discount.get().is_none(), {
            let close = close.clone();
            move || {
                let close = close.clone();
                button(
                    Variant::Ghost,
                    Size::Md,
                    Some(Icon::Trash),
                    st.t("pos.discount.remove"),
                    move || {
                        t.discount.set(SaleDiscount::default());
                        close();
                    },
                )
                .test_id("disc-remove")
                .into_element()
            }
        }))
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
                st.t("pos.discount.apply"),
                apply,
            )
            .test_id("disc-apply"),
        );
    dialog_frame(
        st,
        st.t("pos.discount.title"),
        st.t("pos.discount.help"),
        body,
        footer,
        close,
    )
}

// ---------------------------------------------------------------------------
// The customer of the sale (M9.3)

/// Who buys: a counter customer by default; tapped, the customer picker.
fn customer_chip(st: AppState, t: Till, in_sheet: bool) -> View {
    view()
        .test_id(if in_sheet {
            "sheet-customer"
        } else {
            "pos-customer"
        })
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(16.0))
        .py(px(8.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
        .on_press(move || {
            t.cart_open.set(false);
            t.customer_open.set(true);
        })
        .child(icon(Icon::User, 18.0, move || {
            if t.customer.get().is_some() {
                Tone::Primary
            } else {
                Tone::Faint
            }
        }))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(label_line(
                    move || {
                        t.customer
                            .get()
                            .map_or_else(|| st.tn("pos.customer.counter"), |c| c.name)
                    },
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(dynamic(move || match t.customer.get() {
                    Some(c) => view()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(label_line(
                            st.t("pos.customer.owes"),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        ))
                        .child(figure(
                            move || money(st, c.balance_minor),
                            ty::XS,
                            FontWeight::MEDIUM,
                            move || {
                                if c.over_limit() {
                                    Tone::Danger
                                } else {
                                    Tone::Body
                                }
                            },
                        ))
                        .into_element(),
                    None => label_line(
                        st.t("pos.customer.choose"),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )
                    .into_element(),
                })),
        )
        .child(when(
            move || t.customer.get().is_some(),
            move || {
                icon_button(
                    || Icon::Close,
                    st.t("pos.customer.remove"),
                    Variant::Ghost,
                    move || t.customer.set(None),
                )
                .test_id("pos-customer-remove")
                .into_element()
            },
        ))
}

/// Finds the customer by name, code, phone or city.
fn customer_dialog(st: AppState, t: Till, close: impl Fn() + Clone + 'static) -> View {
    let text = signal(String::new());
    let found = resource(
        move || Some(text.get()),
        move |q, _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    list_customers(
                        s,
                        &CustomerQuery {
                            search: q,
                            limit: 8,
                            ..Default::default()
                        },
                    )
                })
                .await
            }
        },
    );
    let list = {
        let close = close.clone();
        dynamic(move || {
            let rows = found
                .with(|p: Option<&nb_store::usecases::customers::CustomerPage>| {
                    p.map(|p| p.rows.clone())
                })
                .unwrap_or_default();
            if rows.is_empty() {
                return label(
                    st.t("pos.customer.none"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )
                .into_element();
            }
            let mut v = view().gap(px(6.0));
            for (i, c) in rows.into_iter().enumerate() {
                let close = close.clone();
                let detail = [c.code.clone(), c.phone.clone(), c.city.clone()]
                    .into_iter()
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                let (name, balance, blocked, over) = (
                    c.name.clone(),
                    c.balance_minor,
                    c.status == "blocked",
                    c.over_limit(),
                );
                let mut tail = view().flex_row().items_center().gap(px(8.0));
                if blocked {
                    tail = tail.child(badge(Status::Danger, None, st.t("customers.blocked")));
                }
                if balance != 0 {
                    tail = tail.child(figure(
                        move || money(st, balance),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        move || if over { Tone::Danger } else { Tone::Ink },
                    ));
                }
                v = v.child(
                    view()
                        .test_id(&format!("cust-pick-{i}"))
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .p(px(10.0))
                        .rounded(px(radius::md()))
                        .border_1()
                        .border_color(Tone::Border.color())
                        .cursor_pointer()
                        .hover(|s| s.bg(Tone::Hover.color()))
                        .on_press(move || {
                            t.customer.set(Some(c.clone()));
                            close();
                        })
                        .child(icon(Icon::User, 18.0, || Tone::Faint))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(label_line(
                                    move || name.clone(),
                                    ty::SM,
                                    FontWeight::SEMIBOLD,
                                    Tone::Ink,
                                ))
                                .child(label_line(
                                    move || detail.clone(),
                                    ty::XS,
                                    FontWeight::NORMAL,
                                    Tone::Muted,
                                )),
                        )
                        .child(tail),
                );
            }
            v.into_element()
        })
    };
    let body = view()
        .gap(px(12.0))
        .child(crate::ui::form::search_field(
            text,
            st.t("pos.customer.search"),
            "cust-search",
        ))
        .child(list);
    let footer = view().child(
        button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("pos.customer.counter"),
            {
                let close = close.clone();
                move || {
                    t.customer.set(None);
                    close();
                }
            },
        )
        .test_id("cust-counter"),
    );
    dialog_frame(
        st,
        st.t("pos.customer.title"),
        st.t("pos.customer.help"),
        body,
        footer,
        close,
    )
}

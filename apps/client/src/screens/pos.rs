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
use nb_store::usecases::customers::{
    CustomerInput, CustomerQuery, CustomerRow, list_customers, save_customer,
};
use nb_store::usecases::documents::get_document;
use nb_store::usecases::org::{Register, registers};
use nb_store::usecases::products::list_products;
use nb_store::usecases::returns::{
    EditInput, ExchangeInput, HeldCart, ReturnInput, ReturnLineInput, SaleDetail, edit_sale,
    exchange, held_carts, hold_cart, return_sale, sale_for_return, sale_to_edit, take_held,
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
use crate::ui::form::{field, focused_field, labelled, row_input};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, checkbox, figure, icon_button, kbd, label, label_line, segmented,
    switch,
};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon, empty_state, page_pad};
use crate::ui::tokens::{Status, Tone, radius, ty};

#[path = "pos_parts.rs"]
mod parts;
use parts::*;
#[path = "pos_board.rs"]
mod board;
use board::*;
#[path = "pos_widgets.rs"]
mod widgets;
use widgets::*;

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
    /// Goods brought back to pay this cart (M8.8): the return, the sale it
    /// comes from, and the value it should come to.
    exchange: Signal<Option<Exchange>>,
    /// The sales made, at hand (F1), to reprint or reopen one.
    sales_open: Signal<bool>,
    /// A sale reopened to be corrected: paying makes the correction.
    correcting: Signal<Option<Correcting>>,
    /// The sale whose print dialog shows (after a sale, or from the list).
    printing: Signal<Option<String>>,
    printing_open: Signal<bool>,
    /// The receipt prints after this payment (pay and print).
    print_after: Signal<bool>,
    /// The one-number dialog: a line and what it changes.
    quick: Signal<Option<(usize, Quick)>>,
    quick_open: Signal<bool>,
    keys_open: Signal<bool>,
    /// The cart printed before paying.
    prebill_open: Signal<bool>,
    /// The category the products are filtered on.
    category: Signal<Option<String>>,
    /// The screen's layout, and its editing.
    board: Board,
    /// A product of the cart edited in the catalog (its id).
    product_edit: Signal<Option<String>>,
    product_edit_open: Signal<bool>,
    /// The till and its shift (once known).
    till: Memo<Option<(Register, Option<ShiftRow>)>>,
    /// The closing of the shift is asked.
    closing: Signal<bool>,
    /// Bumped for each payment made at once (cash, the exact amount).
    direct: Signal<u32>,
    /// The window is in full screen (the sidebar goes).
    full: Signal<bool>,
    /// The last sale made here (its number, total, change).
    last: Signal<Option<nb_store::usecases::sales::SaleResult>>,
    /// The vehicle whose tour the person signed in drives (name,
    /// warehouse): the till sells from its stock.
    van: Memo<Option<(String, String)>>,
}

/// A sale reopened in the cart: its number, what is left of it, and
/// whether it was sold on account (its value then comes off the account
/// instead of paying the new sale).
#[derive(Debug, Clone, PartialEq)]
struct Correcting {
    number: String,
    value_minor: i64,
    on_account: bool,
}

/// Goods brought back toward the cart: returned when the cart is paid, in
/// the same operation.
#[derive(Debug, Clone, PartialEq)]
struct Exchange {
    back: ReturnInput,
    sale_number: String,
    value_minor: i64,
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
        self.exchange.set(None);
        self.correcting.set(None);
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
    let closing = signal(false);
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
    let van_of = resource(
        move || st.session.get().map(|s| (st.data_rev.get(), s.actor)),
        move |(_, me), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    nb_store::usecases::distribution::driving_vehicle(s, &me.to_string())
                        .map(|v| v.map(|v| (v.name, v.warehouse_id)))
                })
                .await
            }
        },
    );
    let van =
        computed(move || van_of.with(|v: Option<&Option<(String, String)>>| v.cloned().flatten()));
    let t = Till {
        van,
        register,
        held,
        held_rev,
        recalling: signal(false),
        returning: signal(false),
        exchange: signal(None),
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
        sales_open: signal(false),
        correcting: signal(None),
        printing: signal(None),
        printing_open: signal(false),
        print_after: signal(false),
        quick: signal(None),
        quick_open: signal(false),
        keys_open: signal(false),
        prebill_open: signal(false),
        category: signal(None),
        board: Board::new(),
        product_edit: signal(None),
        product_edit_open: signal(false),
        till: state,
        closing,
        direct: signal(0),
        full: signal(false),
        last: signal(None),
    };
    // Leaving the till gives the sidebar and the app's bar back.
    cleanup(move || {
        st.focus.set(false);
        st.bare.set(false);
    });
    effect(move || {
        if !t.printing_open.get() {
            t.printing.set(None);
        }
    });
    effect(move || {
        if !t.editing_open.get() {
            t.editing.set(None);
        }
    });
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
            && !t.sales_open.get_untracked()
            && !t.printing_open.get_untracked()
            && !t.quick_open.get_untracked()
            && !t.keys_open.get_untracked()
            && !t.prebill_open.get_untracked()
            && !t.board.layouts_open.get_untracked()
            && !t.product_edit_open.get_untracked()
    };

    // A quote or order sent to the till (from the sales documents): its
    // lines fill the cart once the till is open. The loading runs under
    // the page (the effect runs again when it clears the request, which
    // would end a task of its own).
    let page_owner = Owner::current();
    // A customer sent from a stop of a tour: the sale is theirs.
    effect(move || {
        let Some(crate::state::Intent::SellTo(id)) = st.intent.get() else {
            return;
        };
        st.intent.set(None);
        let db = st.db();
        let task = page_owner.run(|| {
            spawn(async move {
                if let Ok(c) = db
                    .call(move |s| nb_store::usecases::customers::get_customer(s, &id))
                    .await
                {
                    t.customer.set(Some(c));
                }
            })
        });
        drop(task); // runs to completion under the page
    });
    // A sale sent from the sales list to be corrected at the till.
    effect(move || {
        let Some(crate::state::Intent::Reopen(number)) = st.intent.get() else {
            return;
        };
        if !matches!(state.get(), Some((_, Some(_)))) {
            return;
        }
        st.intent.set(None);
        page_owner.run(|| reopen(st, t, number));
    });
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
    on_key("f1", move |_| {
        if selling() && no_dialog() {
            t.sales_open.set(true);
        }
    });
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
        if selling() && no_dialog() {
            t.pay(st, false);
        }
    });
    on_key("mod+enter", move |_| {
        if selling() && no_dialog() {
            t.pay_now(st, false);
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

    let actions = move || {
        view().child(dynamic(move || match state.get() {
            Some((_, Some(_))) => view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(6.0))
                // Which till, which shift, since when and who; its menu.
                .child(till_chip(st, t, compact))
                // The buttons the layout chooses, in its order.
                .child(head_buttons(st, t, compact))
                .into_element(),
            _ => view().into_element(),
        }))
    };

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
        // The space around the till: the layout's own, or the app's.
        .p(move || {
            let pad = if compact { None } else { t.board.shown(st).pad };
            px(pad.map_or(page_pad(compact), f32::from))
        })
        .gap(px(if compact { 12.0 } else { 16.0 }))
        // A layout may leave the title out (more room to sell): the
        // actions stay, on one line.
        .child(dynamic(move || {
            if compact || st.biz.pos.head.get() {
                page_head(st, Route::Sell, Some("pos.subtitle"), actions(), compact).into_element()
            } else {
                view()
                    .test_id("pos-slim-head")
                    .flex_row()
                    .justify_end()
                    .child(actions())
                    .into_element()
            }
        }))
        .child(view().flex_1().min_h(px(0.0)).child(body))
        .child(modal(t.paying, compact, move || {
            match state.get_untracked() {
                Some((reg, Some(_))) => {
                    pay_dialog(st, reg, t, move || t.paying.set(false), false).into_element()
                }
                _ => view().into_element(),
            }
        }))
        // Each payment made at once: the payment's work, with no screen.
        .child(dynamic(move || {
            if t.direct.get() == 0 {
                return view().into_element();
            }
            untracked(move || match state.get_untracked() {
                Some((reg, Some(_))) => pay_dialog(st, reg, t, || {}, true).into_element(),
                _ => view().into_element(),
            })
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
                .child(cart_panel(st, t, true, false))
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
        .child(modal(t.sales_open, compact, move || {
            sales_dialog(st, t, move || t.sales_open.set(false)).into_element()
        }))
        .child(modal(t.printing_open, compact, move || {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(number) = t.printing.get_untracked() {
                return crate::screens::printing::print_dialog(st, number, move || {
                    t.printing_open.set(false)
                })
                .into_element();
            }
            view().into_element()
        }))
        .child(modal(t.quick_open, compact, move || {
            match t.quick.get_untracked() {
                Some((i, q)) => {
                    quick_dialog(st, t, i, q, move || t.quick_open.set(false)).into_element()
                }
                None => view().into_element(),
            }
        }))
        .child(modal(t.board.layouts_open, compact, move || {
            layouts_dialog(st, t, move || t.board.layouts_open.set(false)).into_element()
        }))
        .child(crate::ui::overlay::wide_modal(
            t.product_edit_open,
            compact,
            move || match t.product_edit.get_untracked() {
                Some(id) => scroll()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(crate::screens::product_editor::product_editor(
                        st,
                        crate::screens::product_editor::Target::Edit(id),
                        compact,
                        move || {
                            t.product_edit_open.set(false);
                            st.changed();
                        },
                    ))
                    .into_element(),
                None => view().into_element(),
            },
        ))
        .child(modal(t.keys_open, compact, move || {
            keys_dialog(st, move || t.keys_open.set(false)).into_element()
        }))
        .child(modal(t.prebill_open, compact, move || {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(sale) = prebill_sale(st, t) {
                return crate::screens::printing::document_dialog(
                    st,
                    st.tn("pos.prebill"),
                    "pre-bill.pdf".into(),
                    crate::printers::Role::Receipt,
                    move || crate::screens::printing::prebill_maker(st, sale.clone()),
                    move || t.prebill_open.set(false),
                )
                .into_element();
            }
            view().into_element()
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
            Some((text.get(), t.category.get(), st.data_rev.get()))
        },
        move |(q, cat, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    list_products(
                        s,
                        &ProductQuery {
                            text: q,
                            category_id: cat,
                            sort: ProductSort::Name,
                            limit: 60,
                            with_counts: false,
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
    let make_search = move || {
        view()
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
            ))
    };
    // What is found, under the search, when no products panel shows it.
    let results = move || {
        dynamic(move || {
            if text.get().trim().is_empty() {
                return view().into_element();
            }
            let all = rows.get();
            let mut list = view()
                .rounded(px(radius::md()))
                .border_1()
                .border_color(Tone::Border.color())
                .bg(Tone::Surface.color())
                .shadow_md();
            for (i, p) in all.into_iter().take(8).enumerate() {
                list = list.child(product_row(st, t, i, p, Some(text)));
            }
            list.absolute()
                .top(px(52.0))
                .left_0()
                .right_0()
                .z_index(20)
                .into_element()
        })
    };
    // The favourites (reloaded when data changes).
    let favs = resource(
        move || Some(st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    list_products(
                        s,
                        &ProductQuery {
                            favorites: true,
                            sort: ProductSort::Name,
                            limit: 120,
                            ..Default::default()
                        },
                    )
                })
                .await
            }
        },
    );
    let fav_rows = computed(move || {
        favs.with(|p: Option<&ProductPage>| p.map(|p| p.rows.clone()))
            .unwrap_or_default()
    });
    // Products in a style; `from`: where their test ids start (the
    // favourites' after the products').
    let grid_of = move |rows: Memo<Vec<ProductRow>>, style: u8, from: usize| {
        dynamic(move || {
            let all = rows.get();
            if all.is_empty() && from > 0 {
                return empty_state(
                    Icon::Star,
                    st.t("pos.favorites.none"),
                    st.t("pos.favorites.help"),
                    None,
                )
                .into_element();
            }
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
            // A right click: a favourite or not, the product's card.
            let menu = move |v: View, p: &ProductRow| {
                let (id, fav) = (p.id.clone(), p.favorite);
                crate::ui::menu::with_menu(v, move || product_acts(st, t, id.clone(), fav))
            };
            // Sizes and what shows follow the layout.
            let pr = st.biz.pos;
            let _ = (pr.tile.get(), pr.stock.get(), pr.sku.get());
            match style {
                1 => {
                    let mut g = view()
                        .rounded(px(radius::md()))
                        .border_1()
                        .border_color(Tone::Border.color());
                    for (i, p) in all.into_iter().enumerate() {
                        let m = p.clone();
                        g = g.child(menu(product_row(st, t, from + i, p, None), &m));
                    }
                    g.into_element()
                }
                2 => {
                    let _ = pr.cols.get();
                    let mut g = card_grid(st, true, compact);
                    for (i, p) in all.into_iter().enumerate() {
                        let m = p.clone();
                        g = g.child(menu(product_tile(st, t, from + i, p), &m));
                    }
                    g.into_element()
                }
                _ => {
                    let _ = pr.cols.get();
                    let mut g = card_grid(st, false, compact);
                    for (i, p) in all.into_iter().enumerate() {
                        let m = p.clone();
                        g = g.child(menu(product_card(st, t, from + i, p, compact), &m));
                    }
                    g.into_element()
                }
            }
        })
    };
    let grid = move |style: u8| grid_of(rows, style, 0);
    if compact {
        let products = card()
            .flex_1()
            .min_h(px(0.0))
            .p(px(12.0))
            .gap(px(12.0))
            .child(make_search())
            .child(category_chips(st, t))
            .child(
                scroll()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(view().pb(px(8.0)).child(grid(0))),
            );
        return view()
            .flex_1()
            .min_h(px(0.0))
            .gap(px(12.0))
            .child(products)
            .child(cart_bar(st, t));
    }
    board_view(
        st,
        t,
        BoardParts {
            search: std::rc::Rc::new(move |with_results: bool| {
                let v = view().relative().z_index(10).child(make_search());
                if with_results { v.child(results()) } else { v }
            }),
            products: std::rc::Rc::new(move |style: u8| {
                card().flex_1().min_h(px(0.0)).p(px(12.0)).child(
                    scroll()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(view().pb(px(8.0)).child(grid(style))),
                )
            }),
            favorites: std::rc::Rc::new(move |style: u8| {
                card()
                    .flex_1()
                    .min_h(px(0.0))
                    .p(px(12.0))
                    .gap(px(8.0))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(icon(Icon::Star, 16.0, || Tone::Warning))
                            .child(label_line(
                                st.t("pos.widget.favorites"),
                                ty::SM,
                                FontWeight::BOLD,
                                Tone::Ink,
                            )),
                    )
                    .child(
                        scroll()
                            .flex_1()
                            .min_h(px(0.0))
                            .child(view().pb(px(8.0)).child(grid_of(fav_rows, style, 500))),
                    )
            }),
        },
    )
}

/// What a product at the till offers on a right click: a favourite or
/// not, its card in the catalog.
fn product_acts(st: AppState, t: Till, id: String, fav: bool) -> Vec<crate::ui::menu::Act> {
    use crate::ui::menu::Act;
    let pid = id.clone();
    vec![
        Act::with(
            Icon::Star,
            st.tn(if fav {
                "products.unfavorite"
            } else {
                "products.favorite"
            }),
            move || toggle_favorite(st, pid.clone(), !fav),
        )
        .when(st.can("products.edit")),
        Act::with(Icon::Package, st.tn("pos.menu.edit_product"), move || {
            t.product_edit.set(Some(id.clone()));
            t.product_edit_open.set(true);
        })
        .when(st.can("products.edit")),
    ]
}

/// Marks a product as a favourite or not.
pub(crate) fn toggle_favorite(st: AppState, id: String, on: bool) {
    let Some(session) = st.session.get_untracked() else {
        return;
    };
    let Ok(pid) = id.parse() else {
        return;
    };
    let meta = session.meta("product.favorite");
    let db = st.db();
    let task = spawn(async move {
        match db
            .call(move |s| nb_store::usecases::products::set_favorite(s, meta, &[pid], on))
            .await
        {
            Ok(_) => {
                st.changed();
                show_toast(
                    st,
                    st.tn(if on {
                        "products.favorited"
                    } else {
                        "products.unfavorited"
                    }),
                );
            }
            Err(e) => show_toast(st, first_error(st, &e)),
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

fn product_card(st: AppState, t: Till, i: usize, p: ProductRow, compact: bool) -> View {
    let (name, price, stock) = (p.name.clone(), p.price_minor, p.on_hand_milli);
    let sku = p.sku.clone();
    let service = p.kind == "service";
    let n = in_cart(t, p.id.clone());
    let tile = if compact {
        0
    } else {
        st.biz.pos.tile.get_untracked()
    };
    let (_, height) = card_size(tile, false);
    let (name_size, price_size) = match tile {
        4 => (ty::XS, ty::SM),
        3 => (ty::MD, ty::LG),
        2 => (ty::SM, ty::LG),
        _ => (ty::SM, ty::MD),
    };
    let pr = st.biz.pos;
    view()
        .test_id(&format!("pos-item-{i}"))
        .relative()
        .min_w(px(0.0))
        .h(px(height))
        .p(px(if tile == 4 { 8.0 } else { 12.0 }))
        .gap(px(4.0))
        .overflow_hidden()
        .rounded(px(radius::lg()))
        .border_1()
        .border_color(move || {
            if n.get() > 0 {
                Tone::Primary.color()
            } else {
                Tone::Border.color()
            }
        })
        .bg(move || {
            if n.get() > 0 {
                Tone::PrimarySoft.color()
            } else {
                Tone::Surface.color()
            }
        })
        .cursor_pointer()
        .hover(|s| s.border_color(Tone::Primary.color()))
        .on_press(move || t.add(p.clone(), 1_000))
        // The name keeps clear of the badge's corner.
        // Two lines at most: the price always shows.
        .child(
            view().pe(px(26.0)).min_w(px(0.0)).child(
                crate::ui::kit::caption(name.clone(), name_size, FontWeight::SEMIBOLD, Tone::Ink)
                    .number_of_lines(if tile == 4 { 2 } else { 3 }),
            ),
        )
        .child(if pr.sku.get_untracked() {
            view().child(label_line(
                move || sku.clone(),
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
        } else {
            view()
        })
        .child(view().flex_1())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(view().flex_1().min_w(px(0.0)).child(figure(
                    move || money(st, price),
                    price_size,
                    FontWeight::BOLD,
                    || Tone::Primary,
                )))
                .child(if service || !pr.stock.get_untracked() {
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
        .child(cart_badge(st, n))
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
                move || t.pay(st, false),
            )
            .test_id("pos-pay"),
        )
}

/// The cart: its lines scroll; the totals and "pay" stay at the bottom.
/// What the cart panel holds: the phone's sheet holds everything; a
/// layout's panels split it.
#[derive(Clone, Copy)]
struct CartParts {
    /// The lines: as a list, or as a table.
    lines: Option<bool>,
    customer: bool,
    sums: bool,
    pay: bool,
    /// Its title and count.
    head: bool,
    /// The title's buttons (hold, print before paying, clear).
    actions: bool,
    /// The table's columns.
    cols: Cols,
    /// The table's header stays while its lines scroll.
    sticky: bool,
}

impl CartParts {
    fn of(lines: Option<bool>, customer: bool, sums: bool, pay: bool) -> Self {
        CartParts {
            lines,
            customer,
            sums,
            pay,
            head: true,
            actions: true,
            cols: Cols::all(),
            sticky: false,
        }
    }
}

/// The cart's lines as a list.
fn cart_lines(st: AppState, t: Till) -> View {
    let totals = computed(move || t.totals(currency(st)));
    view().child(dynamic(move || {
        let items = t.cart.get();
        let _ = st.biz.pos.big_lines.get();
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
    }))
}

/// The cart's figures: discount, net, VAT, discounts, total (`band`: the
/// total large).
fn cart_sums(st: AppState, t: Till, in_sheet: bool, band: bool, total: bool) -> View {
    let totals = computed(move || t.totals(currency(st)));
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
    view()
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
        .child(dynamic(move || {
            // The total shows elsewhere (a large total panel).
            if !total {
                return view().into_element();
            }
            if band {
                return total_band(st, t).mt(px(6.0)).into_element();
            }
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
                ))
                .into_element()
        }))
}

fn cart_panel(st: AppState, t: Till, in_sheet: bool, side_only: bool) -> View {
    let parts = if side_only {
        CartParts::of(None, true, true, true)
    } else {
        CartParts::of(Some(false), true, true, true)
    };
    cart_box(st, t, in_sheet, parts, side_only)
}

/// The cart as one panel: its header, chips, lines, figures and payment as
/// `parts` says.
fn cart_box(st: AppState, t: Till, in_sheet: bool, parts: CartParts, side_only: bool) -> View {
    let count = move || {
        tr_args(
            st.lang(),
            "pos.cart.count",
            &[("count", &st.fmt().integer(t.cart.with(Vec::len) as i64))],
        )
    };
    let (cols, sticky, with_actions) = (parts.cols, parts.sticky, parts.actions);
    card()
        .flex_1()
        .h_full()
        .min_h(px(0.0))
        .child(if !parts.head {
            view()
        } else {
            card_header_icon(
                Some(Icon::Cart),
                st.t("pos.cart"),
                Some(Box::new(count)),
                view().child(when(
                    move || with_actions && !t.cart.get().is_empty(),
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
                                icon_button(
                                    || Icon::Printer,
                                    st.t("pos.prebill"),
                                    Variant::Ghost,
                                    move || {
                                        t.cart_open.set(false);
                                        t.prebill_open.set(true);
                                    },
                                )
                                .test_id("cart-prebill"),
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
            )
        })
        .child(if parts.customer {
            customer_chip(st, t, in_sheet)
        } else {
            view()
        })
        .child(exchange_chip(st, t))
        .child(correcting_chip(st, t))
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
        .child(match (side_only, parts.lines) {
            // The lines are the table beside: here what acts on the cart.
            (true, _) => scroll()
                .flex_1()
                .min_h(px(0.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(actions_panel(
                    st,
                    t,
                    &crate::screens::pos_layout::Panel::new(
                        crate::screens::pos_layout::Widget::Actions,
                    ),
                    0,
                ))
                .into_element(),
            // A table whose header stays: the header above the scroll.
            (false, Some(true)) if sticky => view()
                .flex_1()
                .min_h(px(0.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(cart_table_head(st, cols))
                .child(
                    scroll()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(cart_table(st, t, cols, false)),
                )
                .into_element(),
            (false, Some(table)) => scroll()
                .flex_1()
                .min_h(px(0.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(if table {
                    cart_table(st, t, cols, true)
                } else {
                    cart_lines(st, t)
                })
                .into_element(),
            (false, None) => view().into_element(),
        })
        .child(if parts.sums {
            cart_sums(st, t, in_sheet, st.biz.pos.big_total.get_untracked(), true)
        } else {
            view()
        })
        .child(if parts.pay {
            view().p(px(16.0)).child(pay_buttons(st, t, in_sheet))
        } else {
            view()
        })
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
    // A layout may show the lines large (touch screens).
    let (main, small) = if st.biz.pos.big_lines.get_untracked() {
        (ty::XL, ty::MD)
    } else {
        (ty::SM, ty::XS)
    };
    let name = it.name.clone();
    let unit = crate::screens::common::unit_short(st, &it.unit);
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
    let row = view()
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
        .on_long_press(move || t.edit(i))
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
                            main,
                            FontWeight::SEMIBOLD,
                            Tone::Ink,
                        ))
                        .child(label_line(detail, small, FontWeight::NORMAL, Tone::Muted)),
                )
                .child(figure(
                    move || money(st, line_total),
                    main,
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
                        .on_press(move || t.quick(i, Quick::Qty))
                        .child(label_line(
                            move || format!("{} {unit}", qty_text(st, qty)),
                            main,
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
        );
    crate::ui::menu::with_menu(row, move || line_acts(st, t, i))
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
    // A name of its own already (owner notes: the price is what matters;
    // the name can be changed): the next of the items typed at the till.
    let typed = t
        .cart
        .with_untracked(|v| v.iter().filter(|i| i.product.is_none()).count());
    let name = signal(tr_args(
        st.lang(),
        "pos.free.default_name",
        &[("n", &(typed + 1).to_string())],
    ));
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
    // The price first, with the keyboard in it; the name is filled in.
    let body = view()
        .gap(px(14.0))
        .child(
            view()
                .flex_row()
                .gap(px(12.0))
                .child(view().flex_1().child(focused_field(
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
        .child(field(
            st.t("pos.line_name"),
            name,
            st.t("pos.free.name_hint"),
            String::new,
            KeyboardType::Default,
            "free-name",
        ))
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

fn pay_dialog(
    st: AppState,
    reg: Register,
    t: Till,
    close: impl Fn() + Clone + 'static,
    direct: bool,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    // One operation for this payment screen: a second tap returns the sale
    // already made.
    let swap = t.exchange.get_untracked();
    let correcting = t.correcting.get_untracked();
    let meta = session.meta(if correcting.is_some() {
        "sale.edit"
    } else if swap.is_some() {
        "sale.exchange"
    } else {
        "sale.complete"
    });
    let c = session.currency;
    let items = t.cart.get_untracked();
    let sale_discount = t.discount.get_untracked();
    // Goods brought back pay part of it (or all, and some comes back).
    let brought = swap.as_ref().map_or(0, |x| x.value_minor)
        + correcting
            .as_ref()
            .filter(|c| !c.on_account)
            .map_or(0, |c| c.value_minor);
    let due = cart_of(&items, sale_discount, c)
        .totals()
        .map_or(0, |x| x.total.minor())
        - brought;
    let customer = t.customer.get_untracked();
    let has_customer = customer.is_some();
    // What the customer owes before this sale (a corrected sale on account
    // comes off it first).
    let owed = customer.as_ref().map_or(0, |c| c.balance_minor)
        - correcting
            .as_ref()
            .filter(|c| c.on_account)
            .map_or(0, |c| c.value_minor);
    let way = signal(Way::Cash);
    let given = signal(Money::from_minor(due.max(0), c).to_plain_string());
    // By card or transfer, the older debt may be paid in the same payment.
    let settle_too = signal(false);
    let error = signal(String::new());
    let busy = signal(false);
    // A supervisor's approval, when the sale needs one (a large discount,
    // credit past the limit…): asked on the spot, then the sale is sent again.
    let owner = Owner::current();
    let approval: Signal<Option<String>> = signal(None);
    // The return in an exchange has its own approval.
    let approval_back: Signal<Option<String>> = signal(None);
    let asked_for: Signal<&'static str> = signal("");
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
    // Bon par bon: with a customer, cash beyond the sale pays older debt
    // first; without one, it is all change.
    let bon = computed(move || {
        let g = match (way.get(), given_minor()) {
            (Way::Cash, Some(g)) => g,
            (Way::Credit, _) => 0,
            _ if settle_too.get() && has_customer => due.max(0) + owed.max(0),
            _ => due.max(0),
        };
        nb_domain::sale::bon_par_bon(due, g, if has_customer { owed } else { 0 })
    });
    let change = computed(move || bon.get().change);
    let short = computed(move || match (way.get(), given_minor()) {
        (Way::Cash, Some(g)) if g < due => due - g,
        _ => 0,
    });
    let save = {
        let close = close.clone();
        // On a tour, from the vehicle's stock; otherwise the shop's.
        let warehouse = t
            .van
            .get_untracked()
            .map_or_else(|| session.warehouse().to_string(), |v| v.1);
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
                Way::Credit => due.max(0),
                // The old balance too, when asked: it is the settlement.
                _ => due.max(0) + bon.get_untracked().settle,
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
                payments: if amount > 0 {
                    vec![PaymentInput {
                        method: method.into(),
                        amount_minor: amount,
                    }]
                } else {
                    vec![]
                },
                discount_bps: (sale_discount.bps > 0).then_some(sale_discount.bps),
                discount_minor: (sale_discount.minor > 0).then_some(sale_discount.minor),
                document_id: t.document.get_untracked().map(|d| d.0),
                customer_id: t.customer.get_untracked().map(|c| c.id),
                approval: approval.get_untracked(),
                settle_minor: if way.get_untracked() != Way::Credit && has_customer {
                    bon.get_untracked().settle
                } else {
                    0
                },
                ..Default::default()
            };
            let correcting = t.correcting.get_untracked().map(|c| c.number);
            busy.set(true);
            let close = close.clone();
            let ask_at = (branch_id.clone(), register.clone());
            let swap = t.exchange.get_untracked().map(|x| {
                let mut back = x.back;
                back.approval = approval_back.get_untracked();
                back
            });
            let approval_back_for_edit = approval_back.get_untracked();
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(move |s| match (correcting, swap) {
                        (Some(original), _) => edit_sale(
                            s,
                            meta,
                            EditInput {
                                original,
                                sale: input,
                                reason: String::new(),
                                approval_back: approval_back_for_edit,
                            },
                        )
                        .map(|x| (x.sale, None)),
                        (None, Some(back)) => {
                            exchange(s, meta, ExchangeInput { back, sale: input })
                                .map(|x| (x.sale, Some(x.back.number)))
                        }
                        (None, None) => complete_sale(s, meta, input).map(|r| (r, None)),
                    })
                    .await;
                busy.set(false);
                match r {
                    Ok((r, back)) => {
                        t.last.set(Some(r.clone()));
                        t.clear();
                        st.changed();
                        close();
                        let text = if let Some(back) = back {
                            tr_args(
                                st.lang(),
                                "pos.exchanged",
                                &[
                                    ("back", &back),
                                    ("number", &r.number),
                                    ("change", &money(st, r.change_minor)),
                                ],
                            )
                        } else if r.change_minor > 0 {
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
                        #[cfg(not(target_arch = "wasm32"))]
                        if t.print_after.get_untracked() {
                            // No receipt printer: the dialog (preview, PDF).
                            let printer = crate::printers::for_role(
                                &st.biz.printers.get_untracked(),
                                crate::printers::Role::Receipt,
                            );
                            if st.biz.preview_first.get_untracked() || printer.is_none() {
                                t.printing.set(Some(r.number.clone()));
                                t.printing_open.set(true);
                            } else {
                                // The drawer opens when cash came in.
                                crate::screens::printing::print_receipt(
                                    st,
                                    r.number.clone(),
                                    r.cash_minor > 0,
                                );
                            }
                        }
                        t.search.focus();
                    }
                    Err(e) => {
                        // Paid at once but it cannot go through as it is
                        // (an approval, a limit): the payment screen shows.
                        if direct {
                            show_toast(st, first_error(st, &e));
                            t.paying.set(true);
                            return;
                        }
                        if let Some(p) = needs_approval(&e) {
                            asked_for.set(p);
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
    // Paid at once: cash, the exact amount, no screen.
    if direct {
        save();
        return view();
    }
    let retry = save.clone();
    let swap_open = swap.is_some() || t.correcting.with_untracked(Option::is_some);
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
        let mut amounts = vec![due.max(0)];
        // Bon par bon: the sale and the older debt at once.
        if has_customer && owed > 0 {
            amounts.push(due.max(0) + owed);
        }
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
                        } else if has_customer && owed > 0 && i == 1 {
                            tr_args(st.lang(), "pos.pay_all", &[("amount", &money(st, a))])
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
                    move || money(st, due.max(0)),
                    ty::H1,
                    FontWeight::BOLD,
                    || Tone::Ink,
                ))
                .child(match &swap {
                    Some(x) => {
                        let number = x.sale_number.clone();
                        label_line(
                            move || {
                                tr_args(
                                    st.lang(),
                                    "pos.exchange.paid_by",
                                    &[("number", &number), ("amount", &money(st, brought))],
                                )
                            },
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::Success,
                        )
                        .test_id("pay-exchange")
                        .into_element()
                    }
                    None => view().into_element(),
                })
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
                let limit = cu.credit_limit_minor;
                let name = cu.name.clone();
                let line = move |key: &'static str,
                                 value: Box<dyn Fn() -> i64>,
                                 strong: bool,
                                 id: &'static str| {
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(view().flex_1().child(label_line(
                            st.t(key),
                            ty::SM,
                            if strong {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            },
                            if strong { Tone::Ink } else { Tone::Muted },
                        )))
                        .child(
                            figure(
                                move || money(st, value()),
                                if strong { ty::MD } else { ty::SM },
                                if strong {
                                    FontWeight::BOLD
                                } else {
                                    FontWeight::SEMIBOLD
                                },
                                || Tone::Ink,
                            )
                            .test_id(id),
                        )
                };
                let paid = move || match way.get() {
                    Way::Cash => given_minor().unwrap_or(0),
                    Way::Credit => 0,
                    _ => due.max(0) + bon.get().settle,
                };
                view()
                    .gap(px(6.0))
                    .p(px(12.0))
                    .rounded(px(radius::md()))
                    .border_1()
                    .border_color(Tone::Border.color())
                    .test_id("pay-balance")
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(icon(Icon::User, 16.0, || Tone::Muted))
                            .child(label_line(
                                move || name.clone(),
                                ty::SM,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            )),
                    )
                    .child(line(
                        "print.balance.before",
                        Box::new(move || owed),
                        false,
                        "pay-owed",
                    ))
                    .child(line(
                        "print.balance.this",
                        Box::new(move || due.max(0)),
                        false,
                        "pay-this",
                    ))
                    .child(line(
                        "print.balance.sum",
                        Box::new(move || owed + due.max(0)),
                        false,
                        "pay-sum",
                    ))
                    .child(line(
                        "print.balance.paid",
                        Box::new(paid),
                        false,
                        "pay-paid",
                    ))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .pt(px(6.0))
                            .border_t_1()
                            .border_color(Tone::Border.color())
                            .child(view().flex_1().child(label_line(
                                st.t("print.balance.after"),
                                ty::SM,
                                FontWeight::BOLD,
                                Tone::Ink,
                            )))
                            .child(
                                figure(
                                    move || money(st, bon.get().after),
                                    ty::LG,
                                    FontWeight::BOLD,
                                    move || {
                                        if bon.get().after > limit {
                                            Tone::Danger
                                        } else {
                                            Tone::Ink
                                        }
                                    },
                                )
                                .test_id("pay-after"),
                            ),
                    )
                    .into_element()
            }
            None => view().into_element(),
        })
        .child(cash_part.style_when(move || way.get() != Way::Cash, |s| s.hidden()))
        .child(when(
            move || has_customer && owed > 0 && matches!(way.get(), Way::Card | Way::Transfer),
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        switch(settle_too, move || {
                            tr_args(st.lang(), "pos.settle_too", &[("amount", &money(st, owed))])
                        })
                        .test_id("pay-settle-too"),
                    )
                    .child(label(
                        move || {
                            tr_args(st.lang(), "pos.settle_too", &[("amount", &money(st, owed))])
                        },
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Ink,
                    ))
            },
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
                            if asked_for.get_untracked() == "sales.return" && swap_open {
                                approval_back.set(Some(id));
                            } else {
                                approval.set(Some(id));
                            }
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
        ("pos.suppliers", -shift.suppliers_minor),
        ("pos.cash_out", -shift.cash_out_minor),
        ("pos.refunds", -shift.refunds_minor),
        // Shown, but the drawer never saw it.
        ("pos.exchanges", shift.exchanges_minor),
        ("pos.expected", shift.expected_minor),
    ];
    let mut summary = view().gap(px(6.0));
    for (key, v) in figures {
        if v == 0 && matches!(key, "pos.refunds" | "pos.exchanges") {
            continue;
        }
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
            // An exchange: the goods pay the cart, returned with it.
            if input.refund == "exchange" {
                if input.lines.is_empty() {
                    error.set(error_text(st, "return.empty"));
                    return;
                }
                if input.reason.trim().is_empty() {
                    error.set(error_text(st, "required"));
                    return;
                }
                t.exchange.set(Some(Exchange {
                    back: input,
                    sale_number: d.number.clone(),
                    value_minor: total.get_untracked(),
                }));
                close();
                show_toast(st, st.tn("pos.exchange.started"));
                t.search.focus();
                return;
            }
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
            let unit = crate::screens::common::unit_short(st, &l.unit);
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
            options.push(("exchange", Box::new(st.t("pos.method.exchange"))));
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

/// The goods brought back toward this cart, if any.
fn exchange_chip(st: AppState, t: Till) -> AnyElement {
    dynamic(move || {
        let Some(x) = t.exchange.get() else {
            return view().into_element();
        };
        let (number, value) = (x.sale_number.clone(), x.value_minor);
        view()
            .test_id("pos-exchange")
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .px(px(16.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(Tone::Border.color())
            .bg(Tone::Subtle.color())
            .child(icon(Icon::Undo, 18.0, || Tone::Primary))
            .child(view().flex_1().min_w(px(0.0)).child(label_line(
                move || tr_args(st.lang(), "pos.exchange.from", &[("number", &number)]),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            )))
            .child(figure(
                move || format!("−{}", money(st, value)),
                ty::SM,
                FontWeight::BOLD,
                || Tone::Success,
            ))
            .child(
                icon_button(
                    || Icon::Close,
                    st.t("pos.exchange.remove"),
                    Variant::Ghost,
                    move || t.exchange.set(None),
                )
                .test_id("pos-exchange-remove"),
            )
            .into_element()
    })
    .into_element()
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
                            limit: 20,
                            ..Default::default()
                        },
                    )
                })
                .await
            }
        },
    );
    // The highlighted customer: ↑ ↓ move it, Enter takes it.
    let cursor = signal(0usize);
    effect(move || {
        text.with(|_| ());
        cursor.set(0);
    });
    let rows_now = move || {
        found
            .with(|p: Option<&nb_store::usecases::customers::CustomerPage>| {
                p.map(|p| p.rows.clone())
            })
            .unwrap_or_default()
    };
    on_key("down", move |_| {
        let n = rows_now().len();
        if n > 0 {
            cursor.update(|c| *c = (*c + 1).min(n - 1));
        }
    });
    on_key("up", move |_| cursor.update(|c| *c = c.saturating_sub(1)));
    on_key("enter", {
        let close = close.clone();
        move |_| {
            if let Some(c) = rows_now().get(cursor.get_untracked()).cloned() {
                t.customer.set(Some(c));
                close();
            }
        }
    });
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
                let mut parts: Vec<String> = [c.code.clone(), c.phone.clone(), c.city.clone()]
                    .into_iter()
                    .filter(|x| !x.is_empty())
                    .collect();
                if c.credit_limit_minor > 0 {
                    parts.push(tr_args(
                        st.lang(),
                        "pos.customer.limit",
                        &[
                            ("limit", &money(st, c.credit_limit_minor)),
                            ("left", &money(st, c.credit_left_minor())),
                        ],
                    ));
                }
                let detail = parts.join(" · ");
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
                    tail = tail.child(
                        view()
                            .items_end()
                            .child(figure(
                                move || money(st, balance.abs()),
                                ty::SM,
                                FontWeight::BOLD,
                                move || if over { Tone::Danger } else { Tone::Ink },
                            ))
                            .child(label_line(
                                st.t(if balance > 0 {
                                    "pos.customer.owes_short"
                                } else {
                                    "pos.customer.credit_short"
                                }),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    );
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
                        .bg(move || {
                            if cursor.get() == i {
                                Tone::Hover.color()
                            } else {
                                Tone::Surface.color()
                            }
                        })
                        .border_color(move || {
                            if cursor.get() == i {
                                Tone::Primary.color()
                            } else {
                                Tone::Border.color()
                            }
                        })
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
    // A new customer from the till (owner notes), for those who may add
    // one: what was searched becomes the name, or the phone if digits.
    let adding = signal(false);
    let new_name = signal(String::new());
    let new_phone = signal(String::new());
    let new_city = signal(String::new());
    let new_error = signal(String::new());
    let busy = signal(false);
    let start_adding = move || {
        let q = text.get_untracked();
        let q = q.trim();
        if !q.is_empty()
            && q.chars()
                .all(|c| c.is_ascii_digit() || c == ' ' || c == '+')
        {
            new_phone.set(q.to_owned());
            new_name.set(String::new());
        } else {
            new_name.set(q.to_owned());
        }
        new_error.set(String::new());
        adding.set(true);
    };
    let save_new = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Some(session) = st.session.get_untracked() else {
                return;
            };
            let name = new_name.get_untracked().trim().to_owned();
            if name.is_empty() {
                new_error.set(error_text(st, "required"));
                return;
            }
            let input = CustomerInput {
                kind: "person".into(),
                name,
                phone: new_phone.get_untracked().trim().to_owned(),
                city: new_city.get_untracked().trim().to_owned(),
                status: "active".into(),
                ..Default::default()
            };
            busy.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(move |s| save_customer(s, session.meta("customer.create"), input))
                    .await;
                busy.set(false);
                match r {
                    Ok(c) => {
                        st.changed();
                        t.customer.set(Some(c));
                        show_toast(st, st.tn("pos.customer.created"));
                        close();
                    }
                    Err(e) => new_error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let new_form = view()
        .test_id("cust-new-form")
        .gap(px(12.0))
        .p(px(14.0))
        .rounded(px(radius::md()))
        .border_1()
        .border_color(Tone::Primary.color())
        .bg(Tone::PrimarySoft.color())
        .child(label_line(
            st.t("pos.customer.new"),
            ty::SM,
            FontWeight::BOLD,
            Tone::Primary,
        ))
        .child(crate::ui::form::focused_field(
            st.t("customers.name"),
            new_name,
            String::new,
            move || new_error.get(),
            KeyboardType::Default,
            "cust-new-name",
        ))
        .child(
            view()
                .flex_row()
                .gap(px(10.0))
                .child(view().flex_1().child(field(
                    st.t("customers.phone"),
                    new_phone,
                    String::new,
                    String::new,
                    KeyboardType::Phone,
                    "cust-new-phone",
                )))
                .child(view().flex_1().child(field(
                    st.t("customers.city"),
                    new_city,
                    String::new,
                    String::new,
                    KeyboardType::Default,
                    "cust-new-city",
                ))),
        )
        .child(
            view()
                .flex_row()
                .gap(px(8.0))
                .child(view().flex_1())
                .child(button(
                    Variant::Secondary,
                    Size::Md,
                    None,
                    st.t("form.cancel"),
                    move || adding.set(false),
                ))
                .child(
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Check),
                        st.t("pos.customer.create_use"),
                        save_new,
                    )
                    .test_id("cust-new-save"),
                ),
        );
    let body = view()
        .gap(px(12.0))
        .child(crate::ui::form::search_field(
            text,
            st.t("pos.customer.search"),
            "cust-search",
        ))
        .child(dynamic(move || {
            if adding.get() {
                new_form.clone().into_element()
            } else {
                view().into_element()
            }
        }))
        .child(list);
    let footer = view()
        .child(when(
            move || st.can("customers.edit") && !adding.get(),
            move || {
                button(
                    Variant::Secondary,
                    Size::Md,
                    Some(Icon::UserPlus),
                    st.t("pos.customer.new"),
                    start_adding,
                )
                .test_id("cust-new")
                .into_element()
            },
        ))
        .child(
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

/// The sale being corrected, above the cart: what it was worth, and a way
/// out.
fn correcting_chip(st: AppState, t: Till) -> AnyElement {
    dynamic(move || {
        let Some(x) = t.correcting.get() else {
            return view().into_element();
        };
        let (number, value, on_account) = (x.number.clone(), x.value_minor, x.on_account);
        view()
            .test_id("pos-correcting")
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .px(px(16.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(Tone::Border.color())
            .bg(Tone::WarningSoft.color())
            .child(icon(Icon::Pencil, 18.0, || Tone::Warning))
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(label_line(
                        move || tr_args(st.lang(), "pos.correcting", &[("number", &number)]),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    ))
                    .child(label_line(
                        st.t(if on_account {
                            "pos.correcting.account"
                        } else {
                            "pos.correcting.paid"
                        }),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )),
            )
            .child(figure(
                move || format!("−{}", money(st, value)),
                ty::SM,
                FontWeight::BOLD,
                || Tone::Success,
            ))
            .child(
                icon_button(
                    || Icon::Close,
                    st.t("pos.correcting.cancel"),
                    Variant::Ghost,
                    move || t.clear(),
                )
                .test_id("pos-correcting-cancel"),
            )
            .into_element()
    })
    .into_element()
}

/// Reopens sale `number` in the cart to correct it: its lines at the
/// prices sold, its customer; paying makes the correction.
fn reopen(st: AppState, t: Till, number: String) {
    let db = st.db();
    let task = spawn(async move {
        let loaded = db
            .call(move |s| {
                let sale = sale_to_edit(s, &number)?;
                let mut items = Vec::with_capacity(sale.lines.len());
                for l in &sale.lines {
                    let product = match l.product_id.as_deref() {
                        Some(p) => Some(s.read(|r| {
                            nb_store::repo::catalog::get(r.conn(), nb_store::repo::parse_id(p)?)
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
                let customer = match sale.customer_id.as_deref() {
                    Some(c) => Some(nb_store::usecases::customers::get_customer(s, c)?),
                    None => None,
                };
                Ok((sale, items, customer))
            })
            .await;
        match loaded {
            Ok((sale, _, _)) if sale.edited_into.is_some() => show_toast(
                st,
                tr_args(
                    st.lang(),
                    "pos.correct.already",
                    &[("number", sale.edited_into.as_deref().unwrap_or_default())],
                ),
            ),
            Ok((sale, _, _)) if sale.lines.is_empty() => {
                show_toast(st, st.tn("pos.correct.nothing_left"));
            }
            Ok((sale, items, customer)) => {
                let n = items.len();
                t.clear();
                t.cart.set(items);
                t.customer.set(customer);
                t.correcting.set(Some(Correcting {
                    on_account: sale.payments.iter().any(|p| p.0 == "credit" && p.1 > 0),
                    number: sale.number.clone(),
                    value_minor: sale.value_minor,
                }));
                t.selected.set(n.checked_sub(1));
                t.sales_open.set(false);
                t.search.focus();
            }
            Err(e) => show_toast(st, first_error(st, &e)),
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

/// The sales made, newest first, found by number, customer or phone: each
/// with its customer, what was paid and what was left on account, and the
/// time; reprint it, or reopen it to correct it.
fn sales_dialog(st: AppState, t: Till, close: impl Fn() + Clone + 'static) -> View {
    use nb_store::usecases::sales::{SalesPage, SalesQuery, list_sales};
    let text = signal(String::new());
    let found = resource(
        move || Some((text.get(), st.data_rev.get())),
        move |(q, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    list_sales(
                        s,
                        &SalesQuery {
                            search: q,
                            limit: 30,
                            offset: 0,
                        },
                    )
                })
                .await
            }
        },
    );
    let list = dynamic(move || {
        let rows = found
            .with(|p: Option<&SalesPage>| p.map(|p| p.rows.clone()))
            .unwrap_or_default();
        if rows.is_empty() {
            return label(st.t("sales.empty"), ty::SM, FontWeight::NORMAL, Tone::Muted)
                .into_element();
        }
        let mut v = view().gap(px(6.0));
        for (i, r) in rows.into_iter().enumerate() {
            let number = r.number.clone();
            let when = crate::screens::common::date_time(st, r.created_at);
            let who = r
                .customer_name
                .clone()
                .unwrap_or_else(|| st.tn("pos.counter_customer"));
            let detail = format!("{when} · {who}");
            let (total, paid, credit) = (r.total_minor, r.paid_minor, r.credit_minor);
            let mut tail = view().flex_row().flex_wrap().items_center().gap(px(8.0));
            if let Some(n) = r.edited_into.clone() {
                tail = tail.child(badge(Status::Warning, Some(Icon::Pencil), move || {
                    tr_args(st.lang(), "pos.sale.corrected", &[("number", &n)])
                }));
            }
            if credit > 0 {
                tail = tail.child(badge(Status::Danger, None, move || {
                    tr_args(
                        st.lang(),
                        "pos.sale.on_account",
                        &[("amount", &money(st, credit))],
                    )
                }));
            }
            tail = tail
                .child(
                    view()
                        .items_end()
                        .child(figure(
                            move || money(st, total),
                            ty::SM,
                            FontWeight::BOLD,
                            || Tone::Ink,
                        ))
                        .child(label_line(
                            move || {
                                tr_args(st.lang(), "pos.sale.paid", &[("amount", &money(st, paid))])
                            },
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .child({
                    let number = number.clone();
                    icon_button(
                        || Icon::Printer,
                        st.t("action.print"),
                        Variant::Ghost,
                        move || {
                            t.printing.set(Some(number.clone()));
                            t.sales_open.set(false);
                            t.printing_open.set(true);
                        },
                    )
                    .test_id(&format!("pos-sale-print-{i}"))
                });
            let can_correct = r.edited_into.is_none();
            if can_correct {
                let number = number.clone();
                tail = tail.child(
                    button(
                        Variant::Secondary,
                        Size::Sm,
                        Some(Icon::Pencil),
                        st.t("pos.sale.correct"),
                        move || {
                            reopen(st, t, number.clone());
                        },
                    )
                    .test_id(&format!("pos-sale-correct-{i}")),
                );
            }
            v = v.child(
                view()
                    .test_id(&format!("pos-sale-{i}"))
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.0))
                    .p(px(10.0))
                    .rounded(px(radius::md()))
                    .border_1()
                    .border_color(Tone::Border.color())
                    .child(icon(Icon::Receipt, 18.0, || Tone::Faint))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(160.0))
                            .child(label_line(
                                move || number.clone(),
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
    });
    let body = view()
        .gap(px(12.0))
        .child(crate::ui::form::search_field(
            text,
            st.t("pos.sales.search"),
            "pos-sales-search",
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
        st.t("pos.sales"),
        st.t("pos.sales.help"),
        body,
        footer,
        close,
    )
}

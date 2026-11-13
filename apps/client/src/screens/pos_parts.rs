//! The till's screen as the cashier arranges it (owner notes): products as
//! cards, a list or large tiles, or the cart as a table for scanning
//! first; the total in a large band; one press to change a quantity or a
//! price (the number selected, a keypad beside it); a right click on a
//! cart line for what may be done to it; "pay" and "pay and print" side
//! by side; a receipt printed before paying; full screen; the shortcuts
//! behind a help icon.

use nsh_core::element::desktop::toggle_full_screen;

use crate::ui::menu::{Act, with_menu};

use super::*;

/// What the one-field dialog changes on a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Quick {
    Qty,
    Price,
    Discount,
}

impl Till {
    /// Opens payment; `print`: the receipt prints after it whatever the
    /// setting.
    pub(super) fn pay(self, st: AppState, print: bool) {
        if self.cart.with_untracked(Vec::is_empty) {
            return;
        }
        self.cart_open.set(false);
        self.print_after
            .set(print || st.biz.auto_print.get_untracked());
        self.paying.set(true);
    }

    /// Full screen on or off (the sidebar goes with it).
    pub(super) fn toggle_full(self) {
        toggle_full_screen();
        self.full.update(|v| *v = !*v);
    }

    /// Pays at once: cash, the exact amount, no payment screen (a sale
    /// that brings goods back or corrects another shows it).
    pub(super) fn pay_now(self, st: AppState, print: bool) {
        if self.cart.with_untracked(Vec::is_empty) {
            return;
        }
        if self.exchange.with_untracked(Option::is_some)
            || self.correcting.with_untracked(Option::is_some)
        {
            self.pay(st, print);
            return;
        }
        self.cart_open.set(false);
        self.print_after
            .set(print || st.biz.auto_print.get_untracked());
        self.direct.update(|n| *n += 1);
    }

    pub(super) fn quick(self, i: usize, q: Quick) {
        self.selected.set(Some(i));
        self.quick.set(Some((i, q)));
        self.quick_open.set(true);
    }
}

/// Payment as a ready-made layout has it: at once, through the payment
/// screen, with the receipt.
pub(super) fn pay_buttons(st: AppState, t: Till, in_sheet: bool) -> View {
    let p = crate::screens::pos_layout::Panel::new(crate::screens::pos_layout::Widget::Pay);
    pay_panel(st, t, &p, in_sheet)
}

/// What may be done to cart line `i` (its right click, a held finger).
pub(super) fn line_acts(st: AppState, t: Till, i: usize) -> Vec<Act> {
    let no_product = t
        .cart
        .with_untracked(|v| v.get(i).is_none_or(|l| l.product.is_none()));
    let mut acts = vec![
        Act::with(Icon::Hash, st.tn("pos.menu.qty"), move || {
            t.quick(i, Quick::Qty)
        }),
        Act::with(Icon::Tag, st.tn("pos.menu.price"), move || {
            t.quick(i, Quick::Price)
        })
        .when(st.can("sales.price_override") || st.can("sales.sell")),
        Act::with(Icon::Percent, st.tn("pos.menu.discount"), move || {
            t.quick(i, Quick::Discount)
        })
        .when(st.can("sales.discount")),
        Act::with(Icon::Pencil, st.tn("pos.edit_line"), move || t.edit(i)).hint("F6"),
        Act::with(Icon::Package, st.tn("pos.menu.edit_product"), move || {
            let id = t.cart.with_untracked(|v| {
                v.get(i)
                    .and_then(|l| l.product.as_ref().map(|p| p.id.clone()))
            });
            if id.is_some() {
                t.product_edit.set(id);
                t.product_edit_open.set(true);
            }
        })
        .when(st.can("products.edit") && !no_product),
        Act::Line,
        Act::with(Icon::Plus, st.tn("pos.menu.more"), move || t.step(i, 1_000)).hint("+"),
        Act::with(Icon::Minus, st.tn("pos.menu.less"), move || {
            t.step(i, -1_000)
        })
        .hint("-"),
        Act::with(Icon::Copy, st.tn("pos.menu.duplicate"), move || {
            t.cart.update(|v| {
                if let Some(l) = v.get(i).cloned() {
                    v.insert(i + 1, l);
                }
            });
            t.selected.set(Some(i + 1));
        }),
    ];
    if let Some(name) = t.cart.with_untracked(|v| v.get(i).map(|l| l.name.clone())) {
        acts.push(Act::copy(st.tn("pos.menu.copy_name"), name));
    }
    acts.push(Act::with(Icon::Trash, st.tn("pos.menu.remove"), move || t.remove(i)).danger());
    acts
}

/// A keypad for touch screens: digits, the decimal point, erase.
fn keypad(value: Signal<String>, fresh: Signal<bool>) -> View {
    let key = move |k: &'static str| {
        view()
            .test_id(&format!("key-{k}"))
            .flex_1()
            .h(px(52.0))
            .items_center()
            .justify_center()
            .rounded(px(radius::md()))
            .border_1()
            .border_color(Tone::Border.color())
            .bg(Tone::Surface.color())
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Subtle.color()))
            .on_press(move || {
                // The first key replaces what was there (it shows selected).
                let replace = fresh.get_untracked();
                fresh.set(false);
                value.update(|v| match k {
                    "⌫" => {
                        if replace {
                            v.clear();
                        } else {
                            v.pop();
                        }
                    }
                    _ => {
                        if replace {
                            v.clear();
                        }
                        if k != "." || !v.contains('.') {
                            v.push_str(k);
                        }
                    }
                });
            })
            .child(figure(
                move || k.to_owned(),
                ty::XL,
                FontWeight::SEMIBOLD,
                || Tone::Ink,
            ))
    };
    let mut pad = view().gap(px(6.0)).direction(LayoutDirection::Ltr);
    for row in [
        ["7", "8", "9"],
        ["4", "5", "6"],
        ["1", "2", "3"],
        [".", "0", "⌫"],
    ] {
        let mut r = view().flex_row().gap(px(6.0));
        for k in row {
            r = r.child(key(k));
        }
        pad = pad.child(r);
    }
    pad
}

/// One number for line `i`: its quantity, price or discount, shown
/// selected so typing replaces it, with a keypad.
pub(super) fn quick_dialog(
    st: AppState,
    t: Till,
    i: usize,
    q: Quick,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(it) = t.cart.with_untracked(|v| v.get(i).cloned()) else {
        return view();
    };
    let c = currency(st);
    let first = match q {
        Quick::Qty => Quantity::from_milli(it.qty_milli).to_string(),
        Quick::Price => Money::from_minor(it.price_minor, c).to_plain_string(),
        Quick::Discount => Quantity::from_milli(it.discount_bps * 10).to_string(),
    };
    let n = first.encode_utf16().count() as u32;
    let value = signal(first.clone());
    let fresh = signal(true);
    let error = signal(String::new());
    let r = element_ref();
    let apply = {
        let close = close.clone();
        move || {
            let v = value.get_untracked();
            let ok = match q {
                Quick::Qty => parse_qty(&v).filter(|q| *q > 0).map(|q| {
                    t.cart.update(|x| {
                        if let Some(l) = x.get_mut(i) {
                            l.qty_milli = q;
                        }
                    })
                }),
                Quick::Price => parse_price(&v, c).ok().filter(|p| p.minor() >= 0).map(|p| {
                    t.cart.update(|x| {
                        if let Some(l) = x.get_mut(i) {
                            l.price_minor = p.minor();
                        }
                    })
                }),
                Quick::Discount => {
                    let d = if v.trim().is_empty() {
                        Some(0)
                    } else {
                        v.trim()
                            .replace(',', ".")
                            .parse::<Quantity>()
                            .ok()
                            .and_then(|q| nb_store::repo::to_milli(q).ok())
                            .map(|m| m / 10)
                            .filter(|b| (0..=10_000).contains(b))
                    };
                    d.map(|d| {
                        t.cart.update(|x| {
                            if let Some(l) = x.get_mut(i) {
                                l.discount_bps = d;
                            }
                        })
                    })
                }
            };
            if ok.is_some() {
                close();
                t.search.focus();
            } else {
                error.set(error_text(st, "invalid_value"));
            }
        }
    };
    on_key("enter", {
        let apply = apply.clone();
        move |_| apply()
    });
    let caption = match q {
        Quick::Qty => "pos.qty",
        Quick::Price => "pos.unit_price",
        Quick::Discount => "pos.discount_pct",
    };
    let suffix = match q {
        Quick::Qty => crate::screens::common::unit_short(st, &it.unit),
        Quick::Price => String::new(),
        Quick::Discount => "%".into(),
    };
    let field_box = view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(64.0))
        .px(px(16.0))
        .rounded(px(radius::md()))
        .border_2()
        .border_color(Tone::Primary.color())
        .bg(Tone::Surface.color())
        .child(
            input()
                .test_id("quick-value")
                .element_ref(&r)
                .auto_focus(true)
                .flex_1()
                .keyboard_type(KeyboardType::Decimal)
                .value(move || value.get())
                .on_change(move |s: &str| {
                    // The first key typed replaces the number shown (where
                    // the selection did not, it was typed after it).
                    if fresh.get_untracked() {
                        fresh.set(false);
                        if let Some(rest) = s.strip_prefix(first.as_str())
                            && !rest.is_empty()
                        {
                            value.set(rest.to_owned());
                            return;
                        }
                    }
                    value.set(s.to_owned());
                })
                .on_submit({
                    let apply = apply.clone();
                    move |_: &str| apply()
                })
                .text_size(px(28.0))
                .font_family(crate::ui::tokens::FIGURE_FONT)
                .text_color(Tone::Ink.color()),
        )
        .child(label_line(
            move || suffix.clone(),
            ty::LG,
            FontWeight::MEDIUM,
            Tone::Muted,
        ));
    // Shown selected once on screen: the cashier types over it.
    drop(set_timeout(
        std::time::Duration::from_millis(40),
        move || {
            r.focus();
            r.set_selection(0, n);
        },
    ));
    let mut steps = view().flex_row().flex_wrap().gap(px(6.0));
    if q == Quick::Qty {
        for k in [1, 2, 3, 5, 6, 10, 12, 24] {
            steps = steps.child(
                button(
                    Variant::Secondary,
                    Size::Sm,
                    None,
                    move || k.to_string(),
                    move || {
                        fresh.set(false);
                        value.set(k.to_string());
                    },
                )
                .test_id(&format!("quick-{k}")),
            );
        }
    }
    let name = it.name.clone();
    let body = view()
        .gap(px(14.0))
        .child(label(
            move || name.clone(),
            ty::MD,
            FontWeight::SEMIBOLD,
            Tone::Ink,
        ))
        .child(field_box)
        .child(steps)
        .child(keypad(value, fresh))
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
                st.t("pos.apply"),
                apply,
            )
            .test_id("quick-apply"),
        );
    dialog_frame(
        st,
        st.t(caption),
        st.t("pos.quick.help"),
        body,
        footer,
        close,
    )
}

/// Every key of the till, behind the help icon.
pub(super) fn keys_dialog(st: AppState, close: impl Fn() + Clone + 'static) -> View {
    let mut list = view().gap(px(8.0));
    for (key, what) in [
        ("F2", "pos.key.search"),
        ("F4", "pos.key.free"),
        ("F6", "pos.key.edit"),
        ("Alt ↑↓", "pos.key.select"),
        ("Alt +/−", "pos.key.qty"),
        ("Alt Del", "pos.key.remove"),
        ("F10", "pos.key.customer"),
        ("F5", "pos.key.discount"),
        ("F7", "pos.key.hold"),
        ("F3", "pos.key.held"),
        ("F12", "pos.key.return"),
        ("F1", "pos.key.sales"),
        ("F8", "pos.key.clear"),
        ("F9", "pos.key.pay"),
    ] {
        list = list.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(view().w(px(80.0)).child(kbd(move || key.to_owned())))
                .child(label_line(
                    st.t(what),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Body,
                )),
        );
    }
    list = list.child(label(
        st.t("pos.keys.mouse"),
        ty::XS,
        FontWeight::NORMAL,
        Tone::Muted,
    ));
    let footer = view().child(button(
        Variant::Primary,
        Size::Md,
        None,
        st.t("action.close"),
        close.clone(),
    ));
    dialog_frame(
        st,
        st.t("pos.keys"),
        st.t("pos.keys.help"),
        list,
        footer,
        close,
    )
}

/// The categories as chips above the products: one press filters them.
pub(super) fn category_chips(st: AppState, t: Till) -> View {
    let cats = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| nb_store::usecases::catalog::list_categories(s))
                    .await
            }
        },
    );
    let chip = move |id: Option<String>, name: String, test: String| {
        let on = {
            let id = id.clone();
            computed(move || t.category.get() == id)
        };
        view()
            .test_id(&test)
            .h(px(34.0))
            .px(px(14.0))
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(move || {
                if on.get() {
                    Tone::Primary.color()
                } else {
                    Tone::Border.color()
                }
            })
            .bg(move || {
                if on.get() {
                    Tone::PrimarySoft.color()
                } else {
                    Tone::Surface.color()
                }
            })
            .cursor_pointer()
            .on_press(move || t.category.set(id.clone()))
            .child(label_line(
                move || name.clone(),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            ))
    };
    view().child(dynamic(move || {
        let list = cats.get().unwrap_or_default();
        // Top-level ones: a path of one name.
        let top: Vec<_> = list.into_iter().filter(|c| c.parent_id.is_none()).collect();
        if top.is_empty() {
            return view().into_element();
        }
        let mut row = view().flex_row().gap(px(8.0)).pb(px(2.0));
        row = row.child(chip(
            None,
            st.tn("pos.categories.all"),
            "pos-cat-all".into(),
        ));
        for (i, c) in top.into_iter().enumerate() {
            row = row.child(chip(Some(c.id), c.name, format!("pos-cat-{i}")));
        }
        scroll().horizontal().child(row).into_element()
    }))
}

/// A product as a line of the list layout: name, reference, stock, price.
pub(super) fn product_row(
    st: AppState,
    t: Till,
    i: usize,
    p: ProductRow,
    clear: Option<Signal<String>>,
) -> View {
    let (name, sku, price, stock) = (
        p.name.clone(),
        p.sku.clone(),
        p.price_minor,
        p.on_hand_milli,
    );
    let service = p.kind == "service";
    let pr = st.biz.pos;
    view()
        .test_id(&format!("pos-item-{i}"))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(12.0))
        .h(px(52.0))
        .border_b_1()
        .border_color(Tone::Border.color())
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Subtle.color()))
        .on_press(move || {
            t.add(p.clone(), 1_000);
            // From the search: it is done, the next scan goes in.
            if let Some(text) = clear {
                text.set(String::new());
                t.search.focus();
            }
        })
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
                .child(when(
                    move || pr.sku.get(),
                    move || {
                        let sku = sku.clone();
                        label_line(move || sku.clone(), ty::XS, FontWeight::NORMAL, Tone::Muted)
                            .into_element()
                    },
                )),
        )
        .child(when(
            move || pr.stock.get() && !service,
            move || {
                label_line(
                    move || qty_text(st, stock),
                    ty::XS,
                    FontWeight::MEDIUM,
                    if stock <= 0 {
                        Tone::Danger
                    } else {
                        Tone::Muted
                    },
                )
                .into_element()
            },
        ))
        .child(view().min_w(px(110.0)).items_end().child(figure(
            move || money(st, price),
            ty::MD,
            FontWeight::BOLD,
            || Tone::Primary,
        )))
}

/// A card's narrowest width and its height (all cards alike: a grid
/// with no gaps, and a card that never changes size).
pub(super) fn card_size(tile: usize, tiles: bool) -> (f32, f32) {
    let (w, h) = match tile {
        4 => (112.0, 92.0),
        0 => (132.0, 96.0),
        2 => (210.0, 132.0),
        3 => (260.0, 164.0),
        _ => (168.0, 112.0),
    };
    // A tile adds its coloured block above the name.
    if tiles {
        (w, w * 0.42 + h * 0.62 + 6.0)
    } else {
        (w, h)
    }
}

/// The products' grid: as many equal columns as fit (or as the layout
/// says), every card the same size.
pub(super) fn card_grid(st: AppState, tiles: bool, compact: bool) -> View {
    let pr = st.biz.pos;
    let (w, _) = card_size(if compact { 0 } else { pr.tile.get_untracked() }, tiles);
    let cols = pr.cols.get_untracked();
    let tracks = if cols == 0 || compact {
        format!("repeat(auto-fill, minmax({w}px, 1fr))")
    } else {
        format!("repeat({cols}, minmax(0px, 1fr))")
    };
    view()
        .grid()
        .grid_template_columns(SharedString::from(tracks))
        .gap(px(if tiles { 12.0 } else { 10.0 }))
}

/// How many of a product are in the cart.
pub(super) fn in_cart(t: Till, id: String) -> Memo<i64> {
    computed(move || {
        t.cart.with(|v| {
            v.iter()
                .filter(|l| l.product.as_ref().is_some_and(|x| x.id == id))
                .map(|l| l.qty_milli)
                .sum::<i64>()
        })
    })
}

/// The quantity in the cart over a card's corner (it takes no room: the
/// card keeps its size).
pub(super) fn cart_badge(st: AppState, n: Memo<i64>) -> View {
    let v = view().absolute().top(px(6.0));
    // At the card's end corner, whichever way the text reads.
    let v = if st.rtl() {
        v.left(px(6.0))
    } else {
        v.right(px(6.0))
    };
    v.child(when(
        move || n.get() > 0,
        move || {
            view()
                .test_id("pos-in-cart")
                .min_w(px(24.0))
                .h(px(24.0))
                .px(px(7.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(Tone::Primary.color())
                .shadow_sm()
                .child(label_line(
                    move || qty_text(st, n.get()),
                    ty::XS,
                    FontWeight::BOLD,
                    Tone::OnPrimary,
                ))
                .into_element()
        },
    ))
}

/// A product as a large tile (restaurants, cafés): its initial on a
/// coloured block, its name and price.
pub(super) fn product_tile(st: AppState, t: Till, i: usize, p: ProductRow) -> View {
    let (name, price) = (p.name.clone(), p.price_minor);
    let initial: String = p
        .name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    // A colour per product, steady from its name.
    let hues = [
        0x4338CA, 0x0F766E, 0xB45309, 0xBE123C, 0x7C3AED, 0x047857, 0x1D4ED8, 0x9D174D,
    ];
    let h = p
        .name
        .bytes()
        .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
    let color = hues[h % hues.len()];
    let (size, height) = card_size(st.biz.pos.tile.get_untracked(), true);
    let n = in_cart(t, p.id.clone());
    view()
        .test_id(&format!("pos-item-{i}"))
        .relative()
        .min_w(px(0.0))
        .h(px(height))
        .rounded(px(radius::lg()))
        .overflow_hidden()
        .border_1()
        .border_color(move || {
            if n.get() > 0 {
                Tone::Primary.color()
            } else {
                Tone::Border.color()
            }
        })
        .bg(Tone::Surface.color())
        .cursor_pointer()
        .hover(|s| s.border_color(Tone::Primary.color()))
        .on_press(move || t.add(p.clone(), 1_000))
        .child(
            view()
                .h(px(size * 0.42))
                .flex_shrink(0.0)
                .items_center()
                .justify_center()
                .bg(rgb(color))
                .child(
                    text(move || initial.clone())
                        .text_size(px(size * 0.2))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xFFFFFF)),
                ),
        )
        .child(
            view()
                .p(px(10.0))
                .gap(px(4.0))
                .child(label(
                    move || name.clone(),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(figure(
                    move || money(st, price),
                    ty::MD,
                    FontWeight::BOLD,
                    || Tone::Primary,
                )),
        )
        .child(cart_badge(st, n))
}

/// The total in a large band.
pub(super) fn total_band(st: AppState, t: Till) -> View {
    let total = computed(move || t.totals(currency(st)).map_or(0, |x| x.total.minor()));
    view()
        .test_id("pos-total")
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(16.0))
        .py(px(14.0))
        .rounded(px(radius::lg()))
        .bg(Tone::Ink.color())
        .child(view().flex_1().child(label_line(
            st.t("pos.total"),
            ty::LG,
            FontWeight::BOLD,
            Tone::Surface,
        )))
        .child(figure(
            move || money(st, total.get()),
            38.0,
            FontWeight::BOLD,
            || Tone::Surface,
        ))
}

/// Which columns the cart's table shows (`pos_layout` items of the cart:
/// `num`, `sku`, `qty`, `price`, `discount`, `total`; none chosen: all).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cols {
    pub num: bool,
    pub sku: bool,
    pub qty: bool,
    pub price: bool,
    pub discount: bool,
    pub total: bool,
}

impl Cols {
    pub fn all() -> Self {
        Cols {
            num: true,
            sku: true,
            qty: true,
            price: true,
            discount: true,
            total: true,
        }
    }

    pub fn of(items: &[String]) -> Self {
        let has = |c: &str| items.iter().any(|i| i == c);
        let c = Cols {
            num: has("num"),
            sku: has("sku"),
            qty: has("qty"),
            price: has("price"),
            discount: has("discount"),
            total: has("total"),
        };
        // A table needs a figure: none chosen is all of them.
        if !(c.qty || c.price || c.total) {
            Cols::all()
        } else {
            c
        }
    }
}

/// The header of the cart's table (above its scroll when it stays put).
pub(super) fn cart_table_head(st: AppState, cols: Cols) -> View {
    let head = |key: &'static str, w: Option<f32>, end: bool| {
        let v = match w {
            Some(w) => view().w(px(w)).flex_shrink_0(),
            None => view().flex_1().min_w(px(0.0)),
        };
        let v = if end { v.items_end() } else { v.items_start() };
        v.child(label_line(
            st.t(key),
            ty::XS,
            FontWeight::SEMIBOLD,
            Tone::Muted,
        ))
    };
    let mut header = view()
        .test_id("cart-table-head")
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(40.0))
        .px(px(14.0))
        .bg(Tone::Subtle.color())
        .border_b_1()
        .border_color(Tone::Border.color());
    if cols.num {
        header = header.child(view().w(px(28.0)).child(label_line(
            || "#".to_owned(),
            ty::XS,
            FontWeight::SEMIBOLD,
            Tone::Muted,
        )));
    }
    header = header.child(head("print.col.item", None, false));
    if cols.qty {
        header = header.child(head("pos.qty", Some(124.0), false));
    }
    if cols.price {
        header = header.child(head("pos.unit_price", Some(96.0), true));
    }
    if cols.discount {
        header = header.child(head("pos.discount_pct", Some(56.0), true));
    }
    if cols.total {
        header = header.child(head("pos.line_total", Some(104.0), true));
    }
    header.child(view().w(px(40.0)))
}

/// The cart as a table: every line on one row, the columns chosen; its
/// header above (`with_head`) or kept apart by the caller.
pub(super) fn cart_table(st: AppState, t: Till, cols: Cols, with_head: bool) -> View {
    let totals = computed(move || t.totals(currency(st)));
    let rows = dynamic(move || {
        let items = t.cart.get();
        if items.is_empty() {
            return view()
                .p(px(48.0))
                .items_center()
                .gap(px(10.0))
                .child(icon(Icon::Barcode, 40.0, || Tone::Faint))
                .child(label(
                    st.t("pos.cart.empty"),
                    ty::MD,
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
            let selected = computed(move || t.selected.get() == Some(i));
            let (name, unit, sku) = (
                it.name.clone(),
                crate::screens::common::unit_short(st, &it.unit),
                it.product
                    .as_ref()
                    .map(|p| p.sku.clone())
                    .unwrap_or_default(),
            );
            let (qty, price, disc) = (it.qty_milli, it.price_minor, it.discount_bps);
            let cell = |w: f32, end: bool| {
                let v = view().w(px(w)).flex_shrink_0();
                if end { v.items_end() } else { v.items_start() }
            };
            let pressable = |id: String, q: Quick, content: View| {
                content
                    .test_id(&id)
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(radius::sm()))
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Subtle.color()))
                    .on_press(move || t.quick(i, q))
            };
            v = v.child(with_menu(
                view()
                    .test_id(&format!("cart-line-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .min_h(px(54.0))
                    .px(px(14.0))
                    .border_b_1()
                    .border_color(Tone::Border.color())
                    .bg(move || {
                        if selected.get() {
                            Tone::PrimarySoft.color()
                        } else {
                            Tone::Surface.color()
                        }
                    })
                    .cursor_pointer()
                    .on_press(move || t.selected.set(Some(i)))
                    .child(if cols.num {
                        view().w(px(28.0)).child(figure(
                            move || (i + 1).to_string(),
                            ty::SM,
                            FontWeight::MEDIUM,
                            || Tone::Muted,
                        ))
                    } else {
                        view()
                    })
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(label_line(
                                move || name.clone(),
                                ty::MD,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            ))
                            .child(if cols.sku {
                                label_line(
                                    move || sku.clone(),
                                    ty::XS,
                                    FontWeight::NORMAL,
                                    Tone::Muted,
                                )
                                .into_element()
                            } else {
                                view().into_element()
                            }),
                    )
                    .child(
                        cell(124.0, false).hidden_if(!cols.qty).child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap(px(4.0))
                                .child(stepper(
                                    st,
                                    Icon::Minus,
                                    "pos.less",
                                    move || t.step(i, -1_000),
                                    format!("cart-less-{i}"),
                                ))
                                .child(pressable(
                                    format!("cart-qty-{i}"),
                                    Quick::Qty,
                                    view().min_w(px(44.0)).items_center().child(label_line(
                                        move || format!("{} {unit}", qty_text(st, qty)),
                                        ty::SM,
                                        FontWeight::SEMIBOLD,
                                        Tone::Ink,
                                    )),
                                ))
                                .child(stepper(
                                    st,
                                    Icon::Plus,
                                    "pos.more",
                                    move || t.step(i, 1_000),
                                    format!("cart-more-{i}"),
                                )),
                        ),
                    )
                    .child(cell(96.0, true).hidden_if(!cols.price).child(pressable(
                        format!("cart-price-{i}"),
                        Quick::Price,
                        view().child(figure(
                            move || money(st, price),
                            ty::SM,
                            FontWeight::MEDIUM,
                            || Tone::Body,
                        )),
                    )))
                    .child(cell(56.0, true).hidden_if(!cols.discount).child(pressable(
                        format!("cart-disc-{i}"),
                        Quick::Discount,
                        view().child(figure(
                            move || {
                                if disc > 0 {
                                    format!(
                                        "{}%",
                                        st.fmt().quantity(Quantity::from_milli(disc * 10), 2)
                                    )
                                } else {
                                    "—".into()
                                }
                            },
                            ty::SM,
                            FontWeight::MEDIUM,
                            || Tone::Muted,
                        )),
                    )))
                    .child(cell(104.0, true).hidden_if(!cols.total).child(figure(
                        move || money(st, line_total),
                        ty::MD,
                        FontWeight::BOLD,
                        || Tone::Ink,
                    )))
                    .child(
                        view().w(px(40.0)).child(
                            icon_button(
                                || Icon::Trash,
                                st.t("action.delete"),
                                Variant::Ghost,
                                move || t.remove(i),
                            )
                            .test_id(&format!("cart-remove-{i}")),
                        ),
                    ),
                move || line_acts(st, t, i),
            ));
        }
        v.into_element()
    });
    if with_head {
        view().child(cart_table_head(st, cols)).child(rows)
    } else {
        view().child(rows)
    }
}

/// The cart's figures before paying, as a receipt marked as such.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn prebill_sale(st: AppState, t: Till) -> Option<nb_docs::templates::Sale> {
    use nb_docs::templates::{Line, Sale};
    let c = currency(st);
    let items = t.cart.get_untracked();
    let totals = cart_of(&items, t.discount.get_untracked(), c)
        .totals()
        .ok()?;
    let lines = items
        .iter()
        .zip(&totals.lines)
        .map(|(it, l)| Line {
            name: it.name.clone(),
            qty_milli: it.qty_milli,
            unit: it.unit.clone(),
            unit_price_minor: it.price_minor,
            amount_minor: l.taxed.gross.minor(),
            discount_minor: l.line_discount.minor(),
            sku: it
                .product
                .as_ref()
                .map(|p| p.sku.clone())
                .unwrap_or_default(),
            tax_bps: it.tax_bps,
            net_minor: l.taxed.net.minor(),
        })
        .collect();
    Some(Sale {
        number: st.tn("pos.prebill.number"),
        when: crate::screens::printing::now_local(),
        cashier: st.user.get_untracked().map(|u| u.name).unwrap_or_default(),
        register: String::new(),
        customer: t
            .customer
            .get_untracked()
            .map(|c| (c.name.clone(), String::new())),
        currency: c,
        lines,
        discount_minor: totals.discounts.minor(),
        net_minor: totals.net.minor(),
        taxes: totals
            .by_rate
            .iter()
            .map(|r| (r.rate.bps(), r.net.minor(), r.tax.minor()))
            .collect(),
        stamp_minor: 0,
        total_minor: totals.total.minor(),
        payments: Vec::new(),
        change_minor: 0,
        owed_minor: 0,
        balance: None,
        settled_minor: 0,
    })
}

/// Hides a view when `off` (a column the table does not show).
trait HideIf {
    fn hidden_if(self, off: bool) -> Self;
}

impl HideIf for View {
    fn hidden_if(self, off: bool) -> Self {
        if off { self.hidden() } else { self }
    }
}

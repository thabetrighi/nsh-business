//! "Add a product": name, code, category, unit, price, opening stock —
//! saved by the `create_product` use case in one operation.
//!
//! Typed text is checked here for what only the form knows (a price typed
//! with a comma, an empty field); the rules themselves (unique code,
//! barcode, precision…) come back from the use case as codes shown under
//! their fields. The operation id is fixed when the form opens, so a double
//! tap on Save creates one product.

use std::rc::Rc;

use nb_core::{CoreError, Currency, Money, Quantity};
use nb_i18n::{tr, tr_args};
use nb_store::repo::catalog::{Category, ProductRow};
use nb_store::usecases::products::{Opening, ProductInput, create_product};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::db::Session;
use crate::state::AppState;
use crate::ui::form::{chips, field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, button, label};
use crate::ui::overlay::dialog_frame;
use crate::ui::tokens::{Tone, radius, ty};

/// Units a product can be counted in: (code, message key, decimals).
pub const BASE_UNITS: [(&str, &str, u32); 4] = [
    ("pc", "unit.pc", 0),
    ("box", "unit.box", 0),
    ("kg", "unit.kg", 3),
    ("l", "unit.l", 3),
];

/// Form state; errors are already translated.
#[derive(Clone, Copy)]
struct Draft {
    name: Signal<String>,
    sku: Signal<String>,
    price: Signal<String>,
    stock: Signal<String>,
    category: Signal<usize>,
    unit: Signal<usize>,
    name_err: Signal<String>,
    sku_err: Signal<String>,
    price_err: Signal<String>,
    stock_err: Signal<String>,
    form_err: Signal<String>,
    saving: Signal<bool>,
}

impl Draft {
    fn new() -> Self {
        Self {
            name: signal(String::new()),
            sku: signal(String::new()),
            price: signal(String::new()),
            stock: signal("0".into()),
            category: signal(0),
            unit: signal(0),
            name_err: signal(String::new()),
            sku_err: signal(String::new()),
            price_err: signal(String::new()),
            stock_err: signal(String::new()),
            form_err: signal(String::new()),
            saving: signal(false),
        }
    }

    fn clear_errors(&self) {
        for s in [
            self.name_err,
            self.sku_err,
            self.price_err,
            self.stock_err,
            self.form_err,
        ] {
            s.set(String::new());
        }
    }
}

/// Parses a price typed by a person: `1250`, `1 250,50`, `1250.5`.
pub fn parse_price(input: &str, currency: Currency) -> Result<Money, CoreError> {
    let normalized: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{00A0}' && *c != '\u{202F}')
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    Money::parse(&normalized, currency)
}

/// The message for a rule code (`error.<code>`), or a generic one.
pub fn error_text(st: AppState, code: &str) -> String {
    let key = format!("error.{code}");
    let t = tr(st.lang(), &key);
    if t == key {
        tr(st.lang(), "error.internal").into_owned()
    } else {
        t.into_owned()
    }
}

/// Checks what the form can check; returns the input for the use case.
fn read(
    st: AppState,
    d: Draft,
    session: &Session,
    categories: &[Category],
) -> Option<ProductInput> {
    let lang = st.lang();
    d.clear_errors();
    let name = d.name.get_untracked();
    if name.trim().is_empty() {
        d.name_err.set(tr(lang, "form.required").into_owned());
    }
    let price = match parse_price(&d.price.get_untracked(), session.currency) {
        Ok(m) if m.is_negative() => Err("products.form.price_negative"),
        Ok(m) if m.is_zero() => Err("products.form.price_zero"),
        Ok(m) => Ok(m),
        Err(CoreError::TooManyDecimals { .. }) => Err("products.form.price_decimals"),
        Err(_) => Err("products.form.price_invalid"),
    };
    if let Err(key) = price {
        d.price_err.set(tr(lang, key).into_owned());
    }
    let (unit, _, precision) = BASE_UNITS[d.unit.get_untracked().min(BASE_UNITS.len() - 1)];
    let stock_text = d.stock.get_untracked().trim().replace(',', ".");
    let stock = match stock_text.parse::<Quantity>() {
        Ok(q) if !q.is_negative() && q.fits_precision(precision) => Some(q),
        _ => {
            d.stock_err
                .set(tr(lang, "products.form.stock_invalid").into_owned());
            None
        }
    };
    let price = price.ok()?;
    let stock = stock?;
    if name.trim().is_empty() {
        return None;
    }
    let category_id = categories
        .get(d.category.get_untracked())
        .map(|c| c.id.clone());
    Some(ProductInput {
        name,
        sku: d.sku.get_untracked(),
        kind: "goods".into(),
        category_id,
        base_unit: unit.into(),
        base_precision: precision,
        extra_units: vec![],
        barcodes: vec![],
        tax_bps: 1_900,
        price_minor: price.minor(),
        min_price_minor: None,
        min_stock: None,
        opening: (!stock.is_zero()).then(|| Opening {
            warehouse_id: session.warehouse().to_string(),
            qty: stock.to_string(),
            unit_cost_minor: 0,
        }),
        ..Default::default()
    })
}

/// The form body. `on_saved` receives the new product; the caller closes the modal.
pub fn product_form(
    st: AppState,
    session: Session,
    categories: Vec<Category>,
    next_sku: String,
    on_cancel: impl Fn() + Clone + 'static,
    on_saved: impl Fn(ProductRow) + 'static,
) -> View {
    let d = Draft::new();
    // One operation per opening of the form: a second tap returns the first result.
    let meta = session.meta("product.create");
    let on_saved = Rc::new(on_saved);
    let cats = Rc::new(categories);
    let options = cats
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let name = c.name.clone();
            (i, Box::new(move || name.clone()) as Box<dyn Fn() -> String>)
        })
        .collect();
    let units = BASE_UNITS
        .iter()
        .enumerate()
        .map(|(i, (_, key, _))| (i, Box::new(st.t(key)) as Box<dyn Fn() -> String>))
        .collect();
    let save = {
        let cats = cats.clone();
        move || {
            if d.saving.get_untracked() {
                return;
            }
            let Some(input) = read(st, d, &session, &cats) else {
                return;
            };
            d.saving.set(true);
            let on_saved = on_saved.clone();
            let db = st.db();
            let task = spawn(async move {
                let result = db.call(move |s| create_product(s, meta, input)).await;
                d.saving.set(false);
                match result {
                    Ok(row) => on_saved(row),
                    Err(e) => {
                        for (field, code) in &e.codes {
                            let text = error_text(st, code);
                            let target = match field.unwrap_or("") {
                                "name" => d.name_err,
                                "sku" => d.sku_err,
                                "price" => d.price_err,
                                "opening_qty" | "quantity" => d.stock_err,
                                _ => d.form_err,
                            };
                            if target.get_untracked().is_empty() {
                                target.set(text);
                            }
                        }
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let err = |s: Signal<String>| move || s.get();

    let body = view()
        .gap(px(16.0))
        .child(when(
            move || !d.form_err.get().is_empty(),
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::DangerSoft.color())
                    .child(icon(Icon::Alert, 16.0, || Tone::Danger))
                    .child(label(
                        move || d.form_err.get(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Danger,
                    ))
                    .into_element()
            },
        ))
        .child(field(
            st.t("products.col.name"),
            d.name,
            st.t("products.form.name_hint"),
            err(d.name_err),
            KeyboardType::Default,
            "product-name",
        ))
        .child(
            view()
                .flex_row()
                .gap(px(12.0))
                .child(view().flex_1().child(field(
                    st.t("products.col.price"),
                    d.price,
                    move || {
                        tr_args(
                            st.lang(),
                            "products.form.price_hint",
                            &[("currency", &st.fmt().currency_symbol(session.currency))],
                        )
                    },
                    err(d.price_err),
                    KeyboardType::Decimal,
                    "product-price",
                )))
                .child(view().w(px(140.0)).child(field(
                    st.t("products.form.opening_stock"),
                    d.stock,
                    || "0".to_owned(),
                    err(d.stock_err),
                    KeyboardType::Decimal,
                    "product-stock",
                ))),
        )
        .child(field(
            st.t("products.col.sku"),
            d.sku,
            move || next_sku.clone(),
            err(d.sku_err),
            KeyboardType::Default,
            "product-sku",
        ))
        .child(chips(st.t("products.col.category"), options, d.category))
        .child(chips(st.t("products.col.unit"), units, d.unit));
    let cancel = on_cancel.clone();
    let footer = view()
        .child(button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("form.cancel"),
            cancel,
        ))
        .child(button(
            Variant::Primary,
            Size::Md,
            Some(Icon::Check),
            st.t("form.save"),
            save,
        ));
    dialog_frame(
        st,
        st.t("products.add"),
        st.t("products.form.description"),
        body,
        footer,
        on_cancel,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_typed_by_people() -> Result<(), CoreError> {
        let da = Currency::DZD;
        assert_eq!(parse_price("1250", da)?.minor(), 125_000);
        assert_eq!(parse_price("1 250,50", da)?.minor(), 125_050);
        assert_eq!(parse_price("1250.5", da)?.minor(), 125_050);
        assert!(matches!(
            parse_price("12,345", da),
            Err(CoreError::TooManyDecimals { .. })
        ));
        assert!(parse_price("abc", da).is_err());
        assert!(parse_price("", da).is_err());
        Ok(())
    }
}

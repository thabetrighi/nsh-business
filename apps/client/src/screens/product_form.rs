//! "Add a product": name, code, category, unit, price, opening stock.
//!
//! Validation uses nb-core: the price is parsed exactly into centimes
//! (`Money::parse`), never through a float; a French decimal comma is accepted.

use nb_core::{CoreError, Currency, Money};
use nb_i18n::{Lang, tr, tr_args};
use nsh_core::element::KeyboardType;
use nsh_core::prelude::*;

use super::products::{CATEGORIES, SampleProduct, UNITS};
use crate::state::AppState;
use crate::ui::form::{chips, field};
use crate::ui::icon::Icon;
use crate::ui::kit::{label, primary_button, quiet_button, rule, size};
use crate::ui::palette::Tone;

/// Form state; errors are message keys resolved in the current language.
#[derive(Clone, Copy)]
struct Draft {
    name: Signal<String>,
    sku: Signal<String>,
    price: Signal<String>,
    stock: Signal<String>,
    category: Signal<usize>,
    unit: Signal<usize>,
    name_err: Signal<String>,
    price_err: Signal<String>,
    stock_err: Signal<String>,
}

impl Draft {
    fn new(next_sku: String) -> Self {
        Self {
            name: signal(String::new()),
            sku: signal(next_sku),
            price: signal(String::new()),
            stock: signal("0".into()),
            category: signal(0),
            unit: signal(0),
            name_err: signal(String::new()),
            price_err: signal(String::new()),
            stock_err: signal(String::new()),
        }
    }
}

/// Parses a price typed by a person: `1250`, `1 250,50`, `1250.5`.
pub fn parse_price(input: &str) -> Result<Money, CoreError> {
    let normalized: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{00A0}' && *c != '\u{202F}')
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    Money::parse(&normalized, Currency::DZD)
}

fn validate(st: AppState, d: Draft) -> Option<SampleProduct> {
    let lang = st.lang();
    let name = d.name.get_untracked().trim().to_owned();
    d.name_err.set(if name.is_empty() {
        tr(lang, "form.required").into_owned()
    } else {
        String::new()
    });

    let price = match parse_price(&d.price.get_untracked()) {
        Ok(m) if m.is_negative() => {
            d.price_err
                .set(tr(lang, "products.form.price_negative").into_owned());
            None
        }
        Ok(m) if m.is_zero() => {
            d.price_err
                .set(tr(lang, "products.form.price_zero").into_owned());
            None
        }
        Ok(m) => {
            d.price_err.set(String::new());
            Some(m)
        }
        Err(CoreError::TooManyDecimals { .. }) => {
            d.price_err
                .set(tr(lang, "products.form.price_decimals").into_owned());
            None
        }
        Err(_) => {
            d.price_err
                .set(tr(lang, "products.form.price_invalid").into_owned());
            None
        }
    };

    let stock = match d.stock.get_untracked().trim().parse::<i64>() {
        Ok(n) if n >= 0 => {
            d.stock_err.set(String::new());
            Some(n)
        }
        _ => {
            d.stock_err
                .set(tr(lang, "products.form.stock_invalid").into_owned());
            None
        }
    };

    if name.is_empty() {
        return None;
    }
    let (price, stock) = (price?, stock?);
    Some(SampleProduct {
        sku: d.sku.get_untracked(),
        name_ar: name.clone(),
        name_fr: name,
        category: d.category.get_untracked(),
        unit: d.unit.get_untracked(),
        price_minor: price.minor(),
        stock,
        min_stock: 10,
    })
}

/// The form body. `on_saved` receives the new product; the caller closes the modal.
pub fn product_form(
    st: AppState,
    next_sku: String,
    on_cancel: impl Fn() + 'static,
    on_saved: impl Fn(SampleProduct) + 'static,
) -> impl IntoElement {
    let d = Draft::new(next_sku);
    let pick = move |pair: (&'static str, &'static str)| -> Box<dyn Fn() -> String> {
        Box::new(move || {
            if st.lang() == Lang::Ar {
                pair.0.to_owned()
            } else {
                pair.1.to_owned()
            }
        })
    };
    let categories = CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, &p)| (i, pick(p)))
        .collect();
    let units = UNITS
        .iter()
        .enumerate()
        .map(|(i, &p)| (i, pick(p)))
        .collect();
    let save = move || {
        if let Some(p) = validate(st, d) {
            on_saved(p);
        }
    };
    let err = |s: Signal<String>| move || s.get();

    scroll().max_h(px(720.0)).child(
        view()
            .p_6()
            .gap(px(10.0))
            .child(label(
                st.t("products.add"),
                size::TITLE,
                FontWeight::BOLD,
                Tone::Ink,
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
                                &[("currency", &st.fmt().currency_symbol(Currency::DZD))],
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
                        KeyboardType::Number,
                        "product-stock",
                    ))),
            )
            .child(field(
                st.t("products.col.sku"),
                d.sku,
                || String::new(),
                || String::new(),
                KeyboardType::Default,
                "product-sku",
            ))
            .child(chips(st.t("products.col.category"), categories, d.category))
            .child(chips(st.t("products.col.unit"), units, d.unit))
            .child(rule())
            .child(
                view()
                    .flex_row()
                    .gap(px(12.0))
                    .justify_end()
                    .child(quiet_button(st.t("form.cancel"), on_cancel))
                    .child(primary_button(st.t("form.save"), Some(Icon::Synced), save)),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_typed_by_people() -> Result<(), CoreError> {
        assert_eq!(parse_price("1250")?.minor(), 125_000);
        assert_eq!(parse_price("1 250,50")?.minor(), 125_050);
        assert_eq!(parse_price("1250.5")?.minor(), 125_050);
        assert!(matches!(
            parse_price("12,345"),
            Err(CoreError::TooManyDecimals { .. })
        ));
        assert!(parse_price("abc").is_err());
        assert!(parse_price("").is_err());
        Ok(())
    }
}

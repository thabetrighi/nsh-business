//! The product editor (M5.10): one page with tabs — general, prices, units
//! and barcodes, stock, variants, photos — and a summary beside them on
//! wide screens. Everything is saved at once by `save_product` (one
//! operation: all or nothing, a double tap saves once); errors come back
//! under their fields and the tab holding the first one opens.

use nb_core::{Currency, Decimal, Money, Quantity};
use nb_domain::catalog::{Attribute, combinations, variant_name};
use nb_domain::features::Feature;
use nb_domain::units::STANDARD_UNITS;
use nb_i18n::tr_args;
use nb_store::usecases::catalog::{
    AttributeInput, CategoryRow, ImageRow, Named, PriceInput, PriceListRow, ProductDetail,
    ProductSave, TaxClass, image_bytes, list_brands, list_categories, list_price_lists,
    list_tax_classes, next_internal_barcodes, product_detail, save_product,
};
use nb_store::usecases::products::{Opening, ProductInput};
use nb_store::usecases::tracked::Tracked;
use nsh_core::element::Owner;
use nsh_core::element::{KeyboardType, View, image};
use nsh_core::prelude::*;

use crate::screens::forms::{error_text, parse_price};
use crate::state::AppState;
use crate::ui::form::{chips, field, labelled, row_input, text_area};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, divider, figure, icon_button, label, label_line, segmented,
};
use crate::ui::media::{barcode, data_url};
use crate::ui::overlay::show_toast;
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon, page_pad};
use crate::ui::tokens::{Status, Tone, radius, sp, ty};

/// What the editor opens on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    New,
    Edit(String),
    /// A new product starting from another one's data.
    Copy(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Prices,
    Units,
    Stock,
    Variants,
    Photos,
}

const TABS: [(Tab, &str, Icon); 6] = [
    (Tab::General, "editor.tab.general", Icon::FileText),
    (Tab::Prices, "editor.tab.prices", Icon::Dollar),
    (Tab::Units, "editor.tab.units", Icon::Barcode),
    (Tab::Stock, "editor.tab.stock", Icon::Boxes),
    (Tab::Variants, "editor.tab.variants", Icon::Layers),
    (Tab::Photos, "editor.tab.photos", Icon::ImagePlus),
];

/// The tab a field's error belongs to.
fn tab_of(field: &str) -> Tab {
    match field {
        "price" | "min_price" | "cost" | "tax_rate" | "tax_class" | "prices" | "list"
        | "min_qty" => Tab::Prices,
        "units" | "unit" | "barcode" | "barcodes" | "plu" => Tab::Units,
        "opening_qty" | "unit_cost" | "min_stock" | "quantity" | "lot" | "expiry" | "serials"
        | "tracking" => Tab::Stock,
        "attributes" => Tab::Variants,
        "image" => Tab::Photos,
        _ => Tab::General,
    }
}

/// Count units a product may be packed in.
const PACKS: [&str; 3] = ["box", "pack", "ctn"];

/// The lists the editor chooses from.
#[derive(Debug, Clone, Default, PartialEq)]
struct Lists {
    categories: Vec<CategoryRow>,
    taxes: Vec<TaxClass>,
    brands: Vec<Named>,
    price_lists: Vec<PriceListRow>,
}

// ------------------------------------------------------------------ state

#[derive(Clone, Copy, PartialEq)]
struct PackRow {
    unit: Signal<usize>,
    factor: Signal<String>,
}

#[derive(Clone, Copy, PartialEq)]
struct CodeRow {
    code: Signal<String>,
    /// 0 = base unit, then the packaging rows in order.
    unit: Signal<usize>,
}

#[derive(Clone, Copy, PartialEq)]
struct PriceRow {
    list: Signal<usize>,
    min_qty: Signal<String>,
    price: Signal<String>,
}

#[derive(Clone, Copy, PartialEq)]
struct AttrRow {
    name: Signal<String>,
    values: Signal<String>,
}

/// A picture shown in the editor: stored (with its id) or added now.
#[derive(Clone, PartialEq)]
struct Pic {
    id: Option<String>,
    mime: String,
    bytes: Vec<u8>,
}

/// Every field of the editor, and its errors by field.
#[derive(Clone, Copy)]
struct Form {
    name: Signal<String>,
    alt_name: Signal<String>,
    sku: Signal<String>,
    service: Signal<bool>,
    category: Signal<usize>,
    brand: Signal<usize>,
    tags: Signal<String>,
    description: Signal<String>,
    price: Signal<String>,
    min_price: Signal<String>,
    cost: Signal<String>,
    tax: Signal<usize>,
    prices: Signal<Vec<PriceRow>>,
    base_unit: Signal<usize>,
    packs: Signal<Vec<PackRow>>,
    codes: Signal<Vec<CodeRow>>,
    plu: Signal<String>,
    opening: Signal<String>,
    opening_cost: Signal<String>,
    min_stock: Signal<String>,
    /// 0 none, 1 lots with expiry, 2 serial numbers.
    tracking: Signal<u8>,
    opening_lot: Signal<String>,
    opening_expiry: Signal<String>,
    opening_serials: Signal<String>,
    attrs: Signal<Vec<AttrRow>>,
    pics: Signal<Vec<Pic>>,
    removed: Signal<Vec<String>>,
    errors: Signal<Vec<(String, String)>>,
    saving: Signal<bool>,
    /// The editor's owner: rows added later (from a button's handler) are
    /// made under it, so their signals live as long as the editor.
    owner: Owner,
}

fn s(v: &str) -> Signal<String> {
    signal(v.to_owned())
}

impl Form {
    fn empty() -> Self {
        Self {
            name: s(""),
            alt_name: s(""),
            sku: s(""),
            service: signal(false),
            category: signal(0),
            brand: signal(0),
            tags: s(""),
            description: s(""),
            price: s(""),
            min_price: s(""),
            cost: s(""),
            tax: signal(0),
            prices: signal(vec![]),
            base_unit: signal(0),
            packs: signal(vec![]),
            codes: signal(vec![CodeRow {
                code: s(""),
                unit: signal(0),
            }]),
            plu: s(""),
            opening: s(""),
            opening_cost: s(""),
            min_stock: s(""),
            tracking: signal(0),
            opening_lot: s(""),
            opening_expiry: s(""),
            opening_serials: s(""),
            attrs: signal(vec![]),
            pics: signal(vec![]),
            removed: signal(vec![]),
            errors: signal(vec![]),
            saving: signal(false),
            owner: Owner::current(),
        }
    }

    fn s(self, v: &str) -> Signal<String> {
        let v = v.to_owned();
        self.owner.run(|| signal(v))
    }

    fn n(self, v: usize) -> Signal<usize> {
        self.owner.run(|| signal(v))
    }

    /// The error text of `field`, or empty.
    fn err(self, field: &'static str) -> impl Fn() -> String + Clone + 'static {
        move || {
            self.errors
                .get()
                .iter()
                .find(|(f, _)| f == field)
                .map(|(_, t)| t.clone())
                .unwrap_or_default()
        }
    }

    fn base_code(self) -> &'static str {
        STANDARD_UNITS
            .get(self.base_unit.get_untracked())
            .map_or("pc", |u| u.code)
    }

    /// Unit codes in order: the base, then each packaging row's.
    fn unit_codes(self) -> Vec<String> {
        let mut v = vec![self.base_code().to_owned()];
        for p in self.packs.get_untracked() {
            v.push(PACKS[p.unit.get_untracked().min(PACKS.len() - 1)].to_owned());
        }
        v
    }

    /// Fills the fields from a stored product (edit, or copy without codes).
    fn load(self, st: AppState, d: &ProductDetail, lists: &Lists, copy: bool, currency: Currency) {
        let p = &d.product;
        let money = |m: i64| {
            st.fmt()
                .amount(Money::from_minor(m, currency))
                .replace(['\u{2066}', '\u{2069}', '\u{a0}'], "")
        };
        self.name.set(if copy {
            format!("{} (2)", p.name)
        } else {
            p.name.clone()
        });
        self.alt_name.set(p.alt_name.clone().unwrap_or_default());
        self.sku
            .set(if copy { String::new() } else { p.sku.clone() });
        self.service.set(p.kind == "service");
        self.tracking.set(match p.tracking.as_str() {
            "lot" => 1,
            "serial" => 2,
            _ => 0,
        });
        self.category.set(
            p.category_id
                .as_ref()
                .and_then(|c| lists.categories.iter().position(|x| &x.id == c))
                .map_or(0, |i| i + 1),
        );
        self.brand.set(
            p.brand_id
                .as_ref()
                .and_then(|c| lists.brands.iter().position(|x| &x.id == c))
                .map_or(0, |i| i + 1),
        );
        self.tags.set(p.tags.join(", "));
        self.description
            .set(p.description.clone().unwrap_or_default());
        self.price.set(money(p.price_minor));
        self.min_price
            .set(p.min_price_minor.map(money).unwrap_or_default());
        self.cost.set(p.cost_minor.map(money).unwrap_or_default());
        if let Some(i) = p
            .tax_class_id
            .as_ref()
            .and_then(|t| lists.taxes.iter().position(|x| &x.id == t))
        {
            self.tax.set(i);
        }
        let base = p.units.first().map(|u| u.code.as_str()).unwrap_or("pc");
        self.base_unit.set(
            STANDARD_UNITS
                .iter()
                .position(|u| u.code == base)
                .unwrap_or(0),
        );
        self.packs.set(
            p.units
                .iter()
                .skip(1)
                .map(|u| PackRow {
                    unit: self.n(PACKS.iter().position(|c| *c == u.code).unwrap_or(0)),
                    factor: self.s(&u.factor),
                })
                .collect(),
        );
        let codes = self.unit_codes();
        let rows: Vec<CodeRow> = if copy {
            vec![CodeRow {
                code: self.s(""),
                unit: self.n(0),
            }]
        } else {
            p.barcodes
                .iter()
                .map(|c| CodeRow {
                    code: self.s(c),
                    unit: self.n(0),
                })
                .chain(p.pack_barcodes.iter().map(|(c, u)| CodeRow {
                    code: self.s(c),
                    unit: self.n(codes.iter().position(|x| x == u).unwrap_or(0)),
                }))
                .collect()
        };
        self.codes.set(if rows.is_empty() {
            vec![CodeRow {
                code: self.s(""),
                unit: self.n(0),
            }]
        } else {
            rows
        });
        self.plu.set(if copy {
            String::new()
        } else {
            p.plu.clone().unwrap_or_default()
        });
        self.min_stock.set(
            p.min_stock_milli
                .map(|m| Quantity::from_milli(m).to_string())
                .unwrap_or_default(),
        );
        self.prices.set(
            d.prices
                .iter()
                .map(|x| PriceRow {
                    list: self.n(lists
                        .price_lists
                        .iter()
                        .position(|l| l.code == x.list)
                        .unwrap_or(0)),
                    min_qty: self.s(if x.min_qty == "0" { "" } else { &x.min_qty }),
                    price: self.s(&money(x.price_minor)),
                })
                .collect(),
        );
        if !copy {
            self.pics.set(Vec::new());
        }
    }
}

// ------------------------------------------------------------------ reading the form

/// Builds what `save_product` takes; errors only the form can see.
fn read(
    st: AppState,
    f: Form,
    lists: &Lists,
    existing: Option<(String, i64)>,
    currency: Currency,
    warehouse: String,
) -> Result<ProductSave, Vec<(String, String)>> {
    let mut errors: Vec<(String, String)> = Vec::new();
    let mut e = |field: &str, key: &str| errors.push((field.to_owned(), st.tn(key)));
    let price = |text: &str| -> Option<Option<i64>> {
        match text.trim() {
            "" => Some(None),
            t => parse_price(t, currency)
                .ok()
                .filter(|m| !m.is_negative())
                .map(|m| Some(m.minor())),
        }
    };
    if f.name.get_untracked().trim().is_empty() {
        e("name", "form.required");
    }
    let retail = price(&f.price.get_untracked()).flatten();
    if retail.is_none() {
        e("price", "products.form.price_invalid");
    }
    let min_price = price(&f.min_price.get_untracked()).unwrap_or_else(|| {
        e("min_price", "products.form.price_invalid");
        None
    });
    let cost = price(&f.cost.get_untracked()).unwrap_or_else(|| {
        e("cost", "products.form.price_invalid");
        None
    });
    let base = STANDARD_UNITS
        .get(f.base_unit.get_untracked())
        .copied()
        .unwrap_or(STANDARD_UNITS[0]);
    let mut extra_units = Vec::new();
    for p in f.packs.get_untracked() {
        let code = PACKS[p.unit.get_untracked().min(PACKS.len() - 1)];
        let factor = p.factor.get_untracked().trim().replace(',', ".");
        if factor
            .parse::<Decimal>()
            .map_or(true, |d| d <= Decimal::ONE)
        {
            e("units", "editor.pack_factor");
        }
        extra_units.push((code.to_owned(), factor));
    }
    let units = f.unit_codes();
    let mut barcodes = Vec::new();
    let mut pack_barcodes = Vec::new();
    for c in f.codes.get_untracked() {
        let code = c.code.get_untracked().trim().to_owned();
        if code.is_empty() {
            continue;
        }
        match c.unit.get_untracked() {
            0 => barcodes.push(code),
            i => pack_barcodes.push((code, units.get(i).cloned().unwrap_or_default())),
        }
    }
    let mut prices = Vec::new();
    for r in f.prices.get_untracked() {
        match price(&r.price.get_untracked()).flatten() {
            Some(m) => prices.push(PriceInput {
                list: lists
                    .price_lists
                    .get(r.list.get_untracked())
                    .map(|l| l.code.clone())
                    .unwrap_or_default(),
                price_minor: m,
                min_qty: r.min_qty.get_untracked(),
                branch_id: None,
                customer_id: None,
            }),
            None => e("prices", "products.form.price_invalid"),
        }
    }
    let opening = f.opening.get_untracked().trim().replace(',', ".");
    let opening_cost = price(&f.opening_cost.get_untracked()).unwrap_or_else(|| {
        e("unit_cost", "products.form.price_invalid");
        None
    });
    let attrs: Vec<AttributeInput> = f
        .attrs
        .get_untracked()
        .iter()
        .map(|a| AttributeInput {
            name: a.name.get_untracked(),
            values: a
                .values
                .get_untracked()
                .split([',', '،', ';'])
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .collect(),
        })
        .filter(|a| !a.name.trim().is_empty() || !a.values.is_empty())
        .collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    let text =
        |sig: Signal<String>| Some(sig.get_untracked().trim().to_owned()).filter(|v| !v.is_empty());
    let tax = lists.taxes.get(f.tax.get_untracked());
    let input = ProductInput {
        name: f.name.get_untracked(),
        sku: f.sku.get_untracked(),
        kind: if f.service.get_untracked() {
            "service"
        } else {
            "goods"
        }
        .into(),
        category_id: f
            .category
            .get_untracked()
            .checked_sub(1)
            .and_then(|i| lists.categories.get(i))
            .map(|c| c.id.clone()),
        base_unit: base.code.into(),
        base_precision: base.precision,
        extra_units,
        barcodes,
        tax_bps: tax.map_or(1_900, |t| t.rate_bps),
        price_minor: retail.unwrap_or(0),
        min_price_minor: min_price,
        min_stock: text(f.min_stock),
        opening: (existing.is_none() && !opening.is_empty() && opening != "0").then(|| Opening {
            warehouse_id: warehouse,
            qty: opening,
            unit_cost_minor: opening_cost.or(cost).unwrap_or(0),
            tracked: Tracked {
                lot: text(f.opening_lot),
                expiry: text(f.opening_expiry),
                serials: f
                    .opening_serials
                    .get_untracked()
                    .split(['\n', ',', '،', ';'])
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
                    .collect(),
            },
        }),
        tracking: Some(
            match f.tracking.get_untracked() {
                1 => "lot",
                2 => "serial",
                _ => "none",
            }
            .into(),
        ),
        pack_barcodes,
        tax_class_id: tax.map(|t| t.id.clone()),
        brand_id: f
            .brand
            .get_untracked()
            .checked_sub(1)
            .and_then(|i| lists.brands.get(i))
            .map(|b| b.id.clone()),
        tags: f
            .tags
            .get_untracked()
            .split([',', '،'])
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect(),
        alt_name: text(f.alt_name),
        description: text(f.description),
        cost_minor: cost,
        plu: text(f.plu),
    };
    Ok(ProductSave {
        existing,
        input,
        prices,
        variants: attrs,
        add_images: f
            .pics
            .get_untracked()
            .into_iter()
            .filter(|p| p.id.is_none())
            .map(|p| p.bytes)
            .collect(),
        remove_images: f.removed.get_untracked(),
    })
}

// ------------------------------------------------------------------ the page

pub fn product_editor(
    st: AppState,
    target: Target,
    compact: bool,
    close: impl Fn() + Clone + 'static,
) -> View {
    let f = Form::empty();
    // NB_EDITOR_TAB (development, screenshots): open on a given tab.
    let first = match std::env::var("NB_EDITOR_TAB").as_deref() {
        Ok("prices") => Tab::Prices,
        Ok("units") => Tab::Units,
        Ok("stock") => Tab::Stock,
        Ok("variants") => Tab::Variants,
        Ok("photos") => Tab::Photos,
        _ => Tab::General,
    };
    let tab = signal(first);
    let currency = move || {
        st.session
            .get_untracked()
            .map_or(Currency::DZD, |s| s.currency)
    };
    let id = match &target {
        Target::Edit(id) | Target::Copy(id) => Some(id.clone()),
        Target::New => None,
    };
    let editing = matches!(target, Target::Edit(_));
    // The lists and, for an edit or a copy, the product with its prices,
    // variants and pictures.
    let loaded = resource(move || st.session.get().map(|_| ()), {
        let id = id.clone();
        move |_, _| {
            let db = st.db();
            let id = id.clone();
            async move {
                db.call(move |s| {
                    let lists = Lists {
                        categories: list_categories(s)?,
                        taxes: list_tax_classes(s)?,
                        brands: list_brands(s)?,
                        price_lists: list_price_lists(s)?,
                    };
                    let detail = match id {
                        Some(i) => Some(product_detail(s, nb_store::repo::parse_id(&i)?)?),
                        None => None,
                    };
                    let pics = match &detail {
                        Some(d) => d
                            .images
                            .iter()
                            .map(|img: &ImageRow| {
                                Ok(Pic {
                                    id: Some(img.id.clone()),
                                    mime: img.mime.clone(),
                                    bytes: image_bytes(s, &img.id)?,
                                })
                            })
                            .collect::<nb_store::Result<Vec<_>>>()?,
                        None => vec![],
                    };
                    Ok((lists, detail, pics))
                })
                .await
            }
        }
    });
    let lists = computed(move || {
        loaded
            .with(|l: Option<&(Lists, Option<ProductDetail>, Vec<Pic>)>| l.map(|l| l.0.clone()))
            .unwrap_or_default()
    });
    let detail = computed(move || {
        loaded.with(|l: Option<&(Lists, Option<ProductDetail>, Vec<Pic>)>| {
            l.and_then(|l| l.1.clone())
        })
    });
    let version = computed(move || detail.get().map_or(0, |d| d.product.version));
    // Fill the fields once loaded.
    let filled = signal(false);
    effect(move || {
        let Some((l, d, pics)) = loaded.get() else {
            return;
        };
        if filled.get_untracked() {
            return;
        }
        filled.set(true);
        f.tax
            .set(l.taxes.iter().position(|t| t.is_default).unwrap_or(0));
        if let Some(d) = &d {
            f.load(st, d, &l, !editing, currency());
            if editing {
                f.pics.set(pics);
            }
        }
    });

    let save = {
        let close = close.clone();
        let target = target.clone();
        let meta = st.session.get_untracked().map(|s| s.meta("product.save"));
        move || {
            let (Some(session), Some(meta)) = (st.session.get_untracked(), meta) else {
                return;
            };
            if f.saving.get_untracked() {
                return;
            }
            let existing = match &target {
                Target::Edit(id) => Some((id.clone(), version.get_untracked())),
                _ => None,
            };
            let input = match read(
                st,
                f,
                &lists.get_untracked(),
                existing,
                session.currency,
                session.warehouse().to_string(),
            ) {
                Ok(i) => i,
                Err(errs) => {
                    if let Some((field, _)) = errs.first() {
                        tab.set(tab_of(field));
                    }
                    f.errors.set(errs);
                    return;
                }
            };
            f.errors.set(Vec::new());
            f.saving.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                let r = db.call(move |s| save_product(s, meta, input)).await;
                f.saving.set(false);
                match r {
                    Ok(row) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "products.saved", &[("name", &row.name)]),
                        );
                    }
                    Err(e) => {
                        let errs: Vec<(String, String)> = e
                            .codes
                            .iter()
                            .map(|(field, code)| {
                                (field.unwrap_or("form").to_owned(), error_text(st, code))
                            })
                            .collect();
                        if let Some((field, _)) = errs.first() {
                            tab.set(tab_of(field));
                        }
                        f.errors.set(errs);
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };

    // ---- header
    let title = {
        let target = target.clone();
        move || match &target {
            Target::Edit(_) => {
                let n = f.name.get();
                if n.trim().is_empty() {
                    st.tn("editor.edit")
                } else {
                    n
                }
            }
            Target::Copy(_) => st.tn("editor.copy"),
            Target::New => st.tn("products.add"),
        }
    };
    let back = close.clone();
    let mut actions = view().flex_row().items_center().gap(px(8.0));
    if !compact {
        actions = actions.child(button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("form.cancel"),
            close.clone(),
        ));
    }
    actions = actions.child(
        button(
            Variant::Primary,
            Size::Md,
            Some(Icon::Check),
            st.t("form.save"),
            save.clone(),
        )
        .test_id("product-save"),
    );
    let header = view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .child(
            icon_button(
                move || {
                    if st.rtl() {
                        Icon::ArrowForward
                    } else {
                        Icon::ArrowBack
                    }
                },
                st.t("lock.back"),
                Variant::Secondary,
                back,
            )
            .test_id("editor-back"),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(label_line(
                    st.t("nav.products"),
                    ty::XS,
                    FontWeight::SEMIBOLD,
                    Tone::Primary,
                ))
                .child(label_line(
                    title,
                    if compact { ty::LG } else { ty::H2 },
                    FontWeight::BOLD,
                    Tone::Ink,
                )),
        )
        .child(actions);

    // ---- tabs with a dot where an error is
    let mut tabs = view().flex_row().items_end().gap(px(4.0));
    for (t, key, which) in TABS {
        let active = computed(move || tab.get() == t);
        let has_error =
            computed(move || f.errors.get().iter().any(|(field, _)| tab_of(field) == t));
        tabs = tabs.child(
            view()
                .test_id(&format!(
                    "editor-tab-{}",
                    key.trim_start_matches("editor.tab.")
                ))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .h(px(sp(40.0)))
                .px(px(10.0))
                .cursor_pointer()
                .border_b(px(2.0))
                .border_color(move || {
                    if active.get() {
                        Tone::Primary.color()
                    } else {
                        rgb(0x000000).opacity(0.0).into()
                    }
                })
                .hover(|s| s.bg(Tone::Subtle.color()))
                .on_press(move || tab.set(t))
                .child(dynamic(move || {
                    icon(which, 16.0, move || {
                        if active.get() {
                            Tone::Primary
                        } else {
                            Tone::Faint
                        }
                    })
                    .into_element()
                }))
                .child(
                    label_line(st.t(key), ty::SM, FontWeight::MEDIUM, Tone::Muted)
                        .style_when(active, |s| s.font_weight(FontWeight::SEMIBOLD))
                        .text_color(move || {
                            if active.get() {
                                Tone::Ink.color()
                            } else {
                                Tone::Muted.color()
                            }
                        }),
                )
                .child(when(has_error, || {
                    view()
                        .w(px(7.0))
                        .h(px(7.0))
                        .rounded_full()
                        .bg(Tone::Danger.color())
                        .into_element()
                })),
        );
    }
    let tabs = view()
        .border_b_1()
        .border_color(Tone::Border.color())
        .child(scroll().horizontal().child(tabs));

    // ---- the open tab
    let panel = dynamic(move || {
        let l = lists.get();
        match tab.get() {
            Tab::General => general(st, f, &l, compact),
            Tab::Prices => prices(st, f, &l, compact, currency()),
            Tab::Units => units(st, f, compact),
            Tab::Stock => stock(st, f, detail.get(), editing, compact),
            Tab::Variants => variants(st, f, detail.get(), compact),
            Tab::Photos => photos(st, f),
        }
        .into_element()
    });
    let form_error = when(
        move || !f.err("form")().is_empty(),
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
                    f.err("form"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                ))
                .into_element()
        },
    );
    let main = view()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(20.0))
        .child(form_error)
        .child(panel);
    let body = if compact {
        view().child(main)
    } else {
        view()
            .flex_row()
            .items_start()
            .gap(px(20.0))
            .child(main)
            .child(summary(st, f, lists, currency))
    };
    view()
        .w_full()
        .p(px(page_pad(compact)))
        .gap(px(if compact { 14.0 } else { 18.0 }))
        .child(header)
        .child(tabs)
        .child(body)
}

// ------------------------------------------------------------------ tabs

fn grid2(compact: bool, a: View, b: View) -> View {
    if compact {
        view().gap(px(16.0)).child(a).child(b)
    } else {
        view()
            .flex_row()
            .gap(px(16.0))
            .child(a.flex_1().min_w(px(0.0)))
            .child(b.flex_1().min_w(px(0.0)))
    }
}

fn section(
    st: AppState,
    which: Icon,
    title: &'static str,
    hint: Option<&'static str>,
    actions: View,
    body: View,
) -> View {
    card()
        .child(card_header_icon(
            Some(which),
            st.t(title),
            hint.map(|h| Box::new(st.t(h)) as Box<dyn Fn() -> String>),
            actions,
        ))
        .child(divider())
        .child(body.p(px(20.0)).gap(px(16.0)))
}

fn general(st: AppState, f: Form, l: &Lists, compact: bool) -> View {
    let cats: Vec<String> = l.categories.iter().map(|c| c.path.join(" › ")).collect();
    let brands: Vec<String> = l.brands.iter().map(|b| b.name.clone()).collect();
    let identity = view()
        .child(field(
            st.t("products.col.name"),
            f.name,
            st.t("products.form.name_hint"),
            f.err("name"),
            KeyboardType::Default,
            "product-name",
        ))
        .child(field(
            st.t("editor.alt_name"),
            f.alt_name,
            st.t("editor.alt_name_hint"),
            String::new,
            KeyboardType::Default,
            "product-alt-name",
        ))
        .child(grid2(
            compact,
            field(
                st.t("products.col.sku"),
                f.sku,
                st.t("products.form.sku_auto"),
                f.err("sku"),
                KeyboardType::Default,
                "product-sku",
            ),
            labelled(
                st.t("editor.kind"),
                segmented(
                    vec![
                        (
                            false,
                            Box::new(st.t("editor.kind.goods")) as Box<dyn Fn() -> String>,
                        ),
                        (true, Box::new(st.t("editor.kind.service"))),
                    ],
                    f.service,
                ),
                Some(Box::new(st.t("editor.kind.help"))),
            ),
        ));
    let classing = view()
        .child(grid2(
            compact,
            labelled(
                st.t("products.col.category"),
                dropdown(
                    move || {
                        std::iter::once((0, st.tn("products.form.no_category")))
                            .chain(cats.iter().enumerate().map(|(i, n)| (i + 1, n.clone())))
                            .collect()
                    },
                    f.category,
                    Some(Icon::FolderTree),
                    st.t("products.col.category"),
                    "product-category",
                ),
                None,
            ),
            labelled(
                st.t("editor.brand"),
                dropdown(
                    move || {
                        std::iter::once((0, st.tn("editor.no_brand")))
                            .chain(brands.iter().enumerate().map(|(i, n)| (i + 1, n.clone())))
                            .collect()
                    },
                    f.brand,
                    Some(Icon::Tag),
                    st.t("editor.brand"),
                    "product-brand",
                ),
                None,
            ),
        ))
        .child(field(
            st.t("editor.tags"),
            f.tags,
            st.t("editor.tags_hint"),
            String::new,
            KeyboardType::Default,
            "product-tags",
        ))
        .child(text_area(
            st.t("editor.description"),
            f.description,
            st.t("editor.description_hint"),
            96.0,
            "product-description",
        ));
    view()
        .gap(px(20.0))
        .child(section(
            st,
            Icon::FileText,
            "editor.card.identity",
            None,
            view(),
            identity,
        ))
        .child(section(
            st,
            Icon::FolderTree,
            "editor.card.classing",
            None,
            view(),
            classing,
        ))
}

fn prices(st: AppState, f: Form, l: &Lists, compact: bool, currency: Currency) -> View {
    let symbol = move || st.fmt().currency_symbol(currency);
    let margin = move || {
        let p = parse_price(&f.price.get(), currency).ok();
        let c = parse_price(&f.cost.get(), currency).ok();
        match (p, c) {
            (Some(p), Some(c)) if p.minor() > 0 && !f.cost.get().trim().is_empty() => {
                let gain = p.minor() - c.minor();
                let bps = gain * 10_000 / p.minor();
                tr_args(
                    st.lang(),
                    "editor.margin_value",
                    &[
                        ("amount", &st.fmt().money(Money::from_minor(gain, currency))),
                        ("percent", &st.fmt().percent_signed(bps)),
                    ],
                )
            }
            _ => st.tn("editor.margin_none"),
        }
    };
    let taxes = l
        .taxes
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let n = t.name.clone();
            (i, Box::new(move || n.clone()) as Box<dyn Fn() -> String>)
        })
        .collect();
    let selling = view()
        .child(grid2(
            compact,
            field(
                st.t("products.col.price"),
                f.price,
                move || {
                    tr_args(
                        st.lang(),
                        "products.form.price_hint",
                        &[("currency", &symbol())],
                    )
                },
                f.err("price"),
                KeyboardType::Decimal,
                "product-price",
            ),
            field(
                st.t("editor.min_price"),
                f.min_price,
                st.t("editor.min_price_hint"),
                f.err("min_price"),
                KeyboardType::Decimal,
                "product-min-price",
            ),
        ))
        .child(grid2(
            compact,
            field(
                st.t("products.form.cost"),
                f.cost,
                st.t("editor.cost_hint"),
                f.err("cost"),
                KeyboardType::Decimal,
                "product-cost",
            ),
            labelled(
                st.t("editor.margin"),
                view()
                    .h(px(40.0))
                    .px(px(12.0))
                    .justify_center()
                    .rounded(px(radius::md()))
                    .bg(Tone::Subtle.color())
                    .child(label_line(margin, ty::SM, FontWeight::SEMIBOLD, Tone::Ink)),
                None,
            ),
        ))
        .child(
            view()
                .gap(px(6.0))
                .child(chips(st.t("editor.tax_class"), taxes, f.tax))
                .child(label(
                    move || {
                        st.tn(if st.biz.prices_incl_tax.get() {
                            "editor.tax_included"
                        } else {
                            "editor.tax_excluded"
                        })
                    },
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        );
    let list_names: Vec<String> = l
        .price_lists
        .iter()
        .map(|p| {
            if p.name.is_empty() {
                st.tn(&format!("price_list.{}", p.code))
            } else {
                p.name.clone()
            }
        })
        .collect();
    let rows = dynamic(move || {
        let rows = f.prices.get();
        let mut v = view().gap(px(10.0));
        if rows.is_empty() {
            v = v.child(label(
                st.t("editor.prices_empty"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        } else if !compact {
            v = v.child(
                view()
                    .flex_row()
                    .gap(px(10.0))
                    .child(view().flex_1().child(label(
                        st.t("editor.price_list"),
                        ty::XS,
                        FontWeight::SEMIBOLD,
                        Tone::Muted,
                    )))
                    .child(view().w(px(140.0)).child(label(
                        st.t("editor.from_qty"),
                        ty::XS,
                        FontWeight::SEMIBOLD,
                        Tone::Muted,
                    )))
                    .child(view().w(px(160.0)).child(label(
                        st.t("products.col.price"),
                        ty::XS,
                        FontWeight::SEMIBOLD,
                        Tone::Muted,
                    )))
                    .child(view().w(px(36.0))),
            );
        }
        for (i, r) in rows.iter().copied().enumerate() {
            let names = list_names.clone();
            let list = dropdown(
                move || {
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (i, n.clone()))
                        .collect()
                },
                r.list,
                Some(Icon::Tags),
                st.t("editor.price_list"),
                "price-row-list",
            );
            let del = icon_button(
                || Icon::Trash,
                st.t("action.delete"),
                Variant::Ghost,
                move || {
                    f.prices.update(|v| {
                        if i < v.len() {
                            v.remove(i);
                        }
                    })
                },
            );
            let qty = row_input(
                r.min_qty,
                st.t("editor.any_qty"),
                KeyboardType::Decimal,
                format!("price-row-qty-{i}"),
                || false,
            );
            let price = row_input(
                r.price,
                String::new,
                KeyboardType::Decimal,
                format!("price-row-price-{i}"),
                || false,
            );
            v = v.child(if compact {
                view()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::Subtle.color())
                    .child(list)
                    .child(
                        view()
                            .flex_row()
                            .gap(px(8.0))
                            .child(qty.flex_1())
                            .child(price.flex_1())
                            .child(del),
                    )
            } else {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(view().flex_1().child(list))
                    .child(view().w(px(140.0)).child(qty))
                    .child(view().w(px(160.0)).child(price))
                    .child(del)
            });
        }
        v.child(when(
            move || {
                !f.err("prices")().is_empty()
                    || !f.err("price")().is_empty() && !f.prices.get().is_empty()
            },
            move || {
                label(
                    move || {
                        let a = f.err("prices")();
                        if a.is_empty() { f.err("price")() } else { a }
                    },
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                )
                .into_element()
            },
        ))
        .into_element()
    });
    let add = button(
        Variant::Secondary,
        Size::Sm,
        Some(Icon::Plus),
        st.t("editor.add_price"),
        move || {
            f.prices.update(|v| {
                v.push(PriceRow {
                    list: f.n(0),
                    min_qty: f.s(""),
                    price: f.s(""),
                })
            })
        },
    )
    .test_id("price-row-add");
    view()
        .gap(px(20.0))
        .child(section(
            st,
            Icon::Dollar,
            "editor.card.selling",
            None,
            view(),
            selling,
        ))
        .child(section(
            st,
            Icon::Tags,
            "editor.card.lists",
            Some("editor.card.lists.help"),
            add,
            view().child(rows),
        ))
}

fn units(st: AppState, f: Form, compact: bool) -> View {
    let base = labelled(
        st.t("editor.base_unit"),
        dropdown(
            move || {
                STANDARD_UNITS
                    .iter()
                    .enumerate()
                    .map(|(i, u)| {
                        (
                            i,
                            format!("{} ({})", st.tn(&format!("unit.{}", u.code)), u.code),
                        )
                    })
                    .collect()
            },
            f.base_unit,
            Some(Icon::Ruler),
            st.t("editor.base_unit"),
            "product-unit",
        ),
        Some(Box::new(move || {
            let u = STANDARD_UNITS
                .get(f.base_unit.get())
                .copied()
                .unwrap_or(STANDARD_UNITS[0]);
            tr_args(
                st.lang(),
                "editor.base_unit_help",
                &[("count", &st.fmt().integer(i64::from(u.precision)))],
            )
        })),
    );
    let packs = dynamic(move || {
        let rows = f.packs.get();
        let base_code = STANDARD_UNITS
            .get(f.base_unit.get())
            .map_or("pc", |u| u.code);
        let mut v = view().gap(px(10.0));
        if rows.is_empty() {
            v = v.child(label(
                st.t("editor.packs_empty"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        for (i, r) in rows.iter().copied().enumerate() {
            let del = icon_button(
                || Icon::Trash,
                st.t("action.delete"),
                Variant::Ghost,
                move || {
                    f.packs.update(|v| {
                        if i < v.len() {
                            v.remove(i);
                        }
                    })
                },
            );
            v = v.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(label(
                        || "1".to_owned(),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    ))
                    .child(
                        view()
                            .w(px(if compact { 120.0 } else { 180.0 }))
                            .child(dropdown(
                                move || {
                                    PACKS
                                        .iter()
                                        .enumerate()
                                        .map(|(i, c)| (i, st.tn(&format!("unit.{c}"))))
                                        .collect()
                                },
                                r.unit,
                                Some(Icon::Boxes),
                                st.t("editor.pack_unit"),
                                "pack-unit",
                            )),
                    )
                    .child(label(
                        st.t("editor.contains"),
                        ty::SM,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    ))
                    .child(view().w(px(100.0)).child(row_input(
                        r.factor,
                        || "12".into(),
                        KeyboardType::Decimal,
                        format!("pack-factor-{i}"),
                        || false,
                    )))
                    .child(label(
                        move || base_code.to_owned(),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    ))
                    .child(view().flex_1())
                    .child(del),
            );
        }
        v.child(when(
            move || !f.err("units")().is_empty(),
            move || label(f.err("units"), ty::XS, FontWeight::MEDIUM, Tone::Danger).into_element(),
        ))
        .into_element()
    });
    let add_pack = button(
        Variant::Secondary,
        Size::Sm,
        Some(Icon::Plus),
        st.t("editor.add_pack"),
        move || {
            f.packs.update(|v| {
                v.push(PackRow {
                    unit: f.n(2),
                    factor: f.s(""),
                })
            })
        },
    )
    .test_id("pack-add");
    let codes = dynamic(move || {
        let rows = f.codes.get();
        // Choices: the base unit, then each packaging row.
        let base_code = STANDARD_UNITS
            .get(f.base_unit.get())
            .map_or("pc", |u| u.code);
        let mut unit_names = vec![st.tn(&format!("unit.{base_code}"))];
        for p in f.packs.get() {
            unit_names.push(st.tn(&format!("unit.{}", PACKS[p.unit.get().min(2)])));
        }
        let mut v = view().gap(px(10.0));
        for (i, r) in rows.iter().copied().enumerate() {
            let names = unit_names.clone();
            let generate = button(
                Variant::Ghost,
                Size::Sm,
                Some(Icon::Wand),
                st.t("editor.generate"),
                move || generate_into(st, f, r.code),
            )
            .test_id(&format!("barcode-generate-{i}"));
            let del = icon_button(
                || Icon::Trash,
                st.t("action.delete"),
                Variant::Ghost,
                move || {
                    f.codes.update(|v| {
                        if i < v.len() {
                            v.remove(i);
                        }
                    })
                },
            );
            let unit = dropdown(
                move || {
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (i, n.clone()))
                        .collect()
                },
                r.unit,
                None,
                st.t("products.col.unit"),
                "barcode-unit",
            );
            let code = row_input(
                r.code,
                st.t("products.form.barcode_hint"),
                KeyboardType::Number,
                format!("barcode-{i}"),
                move || !f.err("barcodes")().is_empty() || !f.err("barcode")().is_empty(),
            );
            v = v.child(if compact {
                view()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::Subtle.color())
                    .child(code)
                    .child(
                        view()
                            .flex_row()
                            .gap(px(8.0))
                            .child(view().flex_1().child(unit))
                            .child(generate)
                            .child(del),
                    )
            } else {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(view().flex_1().child(code))
                    .child(view().w(px(150.0)).child(unit))
                    .child(generate)
                    .child(del)
            });
        }
        v.child(when(
            move || !f.err("barcodes")().is_empty() || !f.err("barcode")().is_empty(),
            move || {
                label(
                    move || {
                        let a = f.err("barcodes")();
                        if a.is_empty() { f.err("barcode")() } else { a }
                    },
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Danger,
                )
                .into_element()
            },
        ))
        .into_element()
    });
    let add_code = button(
        Variant::Secondary,
        Size::Sm,
        Some(Icon::Plus),
        st.t("editor.add_barcode"),
        move || {
            f.codes.update(|v| {
                v.push(CodeRow {
                    code: f.s(""),
                    unit: f.n(0),
                })
            })
        },
    )
    .test_id("barcode-add");
    let mut out = view()
        .gap(px(20.0))
        .child(section(
            st,
            Icon::Ruler,
            "editor.card.unit",
            None,
            view(),
            view().child(base),
        ))
        .child(section(
            st,
            Icon::Boxes,
            "editor.card.packs",
            Some("editor.card.packs.help"),
            add_pack,
            view().child(packs),
        ))
        .child(section(
            st,
            Icon::Barcode,
            "editor.card.barcodes",
            Some("editor.card.barcodes.help"),
            add_code,
            view().child(codes),
        ));
    if st.has(Feature::Scale)
        || st.has(Feature::WeighedBarcodes)
        || !f.plu.get_untracked().is_empty()
    {
        out = out.child(section(
            st,
            Icon::Scale,
            "editor.card.scale",
            Some("editor.card.scale.help"),
            view(),
            view().child(field(
                st.t("editor.plu"),
                f.plu,
                || "42".to_owned(),
                f.err("plu"),
                KeyboardType::Number,
                "product-plu",
            )),
        ));
    }
    out
}

/// Fills `target` with the next free in-store barcode, skipping those
/// already typed in the form.
fn generate_into(st: AppState, f: Form, target: Signal<String>) {
    let typed: Vec<String> = f
        .codes
        .get_untracked()
        .iter()
        .map(|c| c.code.get_untracked())
        .collect();
    let db = st.db();
    let task = spawn(async move {
        let n = typed.len() + 1;
        if let Ok(codes) = db.call(move |s| next_internal_barcodes(s, n)).await
            && let Some(c) = codes.into_iter().find(|c| !typed.contains(c))
        {
            target.set(c);
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

fn stock(
    st: AppState,
    f: Form,
    detail: Option<ProductDetail>,
    editing: bool,
    compact: bool,
) -> View {
    let mut out = view().gap(px(20.0));
    if editing {
        let on_hand = detail.map(|d| d.product.on_hand_milli).unwrap_or(0);
        let unit = f.base_code();
        out = out.child(section(
            st,
            Icon::Boxes,
            "editor.card.on_hand",
            Some("editor.card.on_hand.help"),
            view(),
            view().child(figure(
                move || {
                    format!(
                        "{} {}",
                        st.fmt().quantity(Quantity::from_milli(on_hand), 3),
                        unit
                    )
                },
                ty::H2,
                FontWeight::BOLD,
                || Tone::Ink,
            )),
        ));
    } else {
        out = out.child(section(
            st,
            Icon::Boxes,
            "editor.card.opening",
            Some("editor.card.opening.help"),
            view(),
            view()
                .child(grid2(
                    compact,
                    field(
                        st.t("products.form.opening_stock"),
                        f.opening,
                        || "0".to_owned(),
                        f.err("opening_qty"),
                        KeyboardType::Decimal,
                        "product-stock",
                    ),
                    field(
                        st.t("editor.opening_cost"),
                        f.opening_cost,
                        st.t("editor.opening_cost_hint"),
                        f.err("unit_cost"),
                        KeyboardType::Decimal,
                        "product-opening-cost",
                    ),
                ))
                .child(
                    grid2(
                        compact,
                        field(
                            st.t("editor.opening_lot"),
                            f.opening_lot,
                            st.t("editor.opening_lot_hint"),
                            f.err("lot"),
                            KeyboardType::Default,
                            "product-opening-lot",
                        ),
                        field(
                            st.t("editor.opening_expiry"),
                            f.opening_expiry,
                            || "2027-06-30".to_owned(),
                            f.err("expiry"),
                            KeyboardType::Default,
                            "product-opening-expiry",
                        ),
                    )
                    .style_when(move || f.tracking.get() != 1, |s| s.hidden()),
                )
                .child(
                    text_area(
                        st.t("editor.opening_serials"),
                        f.opening_serials,
                        st.t("editor.opening_serials_hint"),
                        90.0,
                        "product-opening-serials",
                    )
                    .style_when(move || f.tracking.get() != 2, |s| s.hidden()),
                ),
        ));
    }
    out = out.child(section(
        st,
        Icon::Layers,
        "editor.card.tracking",
        Some("editor.card.tracking.help"),
        view(),
        view().child(segmented(
            vec![
                (
                    0u8,
                    Box::new(st.t("editor.tracking.none")) as Box<dyn Fn() -> String>,
                ),
                (1, Box::new(st.t("editor.tracking.lot"))),
                (2, Box::new(st.t("editor.tracking.serial"))),
            ],
            f.tracking,
        )),
    ));
    out.child(section(
        st,
        Icon::Bell,
        "editor.card.alert",
        None,
        view(),
        view().child(field(
            st.t("editor.min_stock"),
            f.min_stock,
            st.t("editor.min_stock_hint"),
            f.err("min_stock"),
            KeyboardType::Decimal,
            "product-min-stock",
        )),
    ))
}

fn variants(st: AppState, f: Form, detail: Option<ProductDetail>, compact: bool) -> View {
    let mut out = view().gap(px(20.0));
    let existing = detail
        .as_ref()
        .map(|d| d.variants.clone())
        .unwrap_or_default();
    if detail
        .as_ref()
        .is_some_and(|d| d.product.parent_id.is_some())
    {
        return out.child(section(
            st,
            Icon::Layers,
            "editor.tab.variants",
            None,
            view(),
            view().child(label(
                st.t("editor.is_variant"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )),
        ));
    }
    if !existing.is_empty() {
        let mut list = view().gap(px(8.0));
        for v in existing {
            let (name, sku) = (v.name.clone(), v.sku.clone());
            list = list.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(icon(Icon::SubItem, 14.0, || Tone::Faint))
                    .child(view().flex_1().min_w(px(0.0)).child(label_line(
                        move || name.clone(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Ink,
                    )))
                    .child(badge(Status::Neutral, None, move || sku.clone())),
            );
        }
        out = out.child(section(
            st,
            Icon::Layers,
            "editor.card.existing_variants",
            None,
            view(),
            list,
        ));
    }
    let rows = dynamic(move || {
        let mut v = view().gap(px(10.0));
        let rows = f.attrs.get();
        if rows.is_empty() {
            v = v.child(label(
                st.t("editor.attrs_empty"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        for (i, r) in rows.iter().copied().enumerate() {
            let del = icon_button(
                || Icon::Trash,
                st.t("action.delete"),
                Variant::Ghost,
                move || {
                    f.attrs.update(|v| {
                        if i < v.len() {
                            v.remove(i);
                        }
                    })
                },
            );
            let name = row_input(
                r.name,
                st.t("editor.attr_name_hint"),
                KeyboardType::Default,
                format!("attr-name-{i}"),
                || false,
            );
            let values = row_input(
                r.values,
                st.t("editor.attr_values_hint"),
                KeyboardType::Default,
                format!("attr-values-{i}"),
                || false,
            );
            v = v.child(if compact {
                view()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::Subtle.color())
                    .child(name)
                    .child(
                        view()
                            .flex_row()
                            .gap(px(8.0))
                            .child(values.flex_1())
                            .child(del),
                    )
            } else {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(view().w(px(180.0)).child(name))
                    .child(view().flex_1().child(values))
                    .child(del)
            });
        }
        v.into_element()
    });
    let add = button(
        Variant::Secondary,
        Size::Sm,
        Some(Icon::Plus),
        st.t("editor.add_attr"),
        move || {
            f.attrs.update(|v| {
                v.push(AttrRow {
                    name: f.s(""),
                    values: f.s(""),
                })
            })
        },
    )
    .test_id("attr-add");
    // What will be made, as the rules see it.
    let preview = dynamic(move || {
        let attrs: Vec<Attribute> = f
            .attrs
            .get()
            .iter()
            .map(|a| Attribute {
                name: a.name.get(),
                values: a
                    .values
                    .get()
                    .split([',', '،', ';'])
                    .map(|v| v.trim().to_owned())
                    .collect(),
            })
            .filter(|a| !a.name.trim().is_empty())
            .collect();
        if attrs.is_empty() {
            return view().into_element();
        }
        let base = f.name.get();
        match combinations(&attrs) {
            Ok(c) => {
                let n = c.len() as i64;
                let mut v = view().gap(px(6.0)).child(label(
                    move || {
                        tr_args(
                            st.lang(),
                            "editor.variants_count",
                            &[("count", &st.fmt().integer(n))],
                        )
                    },
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ));
                let shown: Vec<String> =
                    c.iter().take(12).map(|x| variant_name(&base, x)).collect();
                let mut wrap = view().flex_row().flex_wrap().gap(px(6.0));
                for n in shown {
                    wrap = wrap.child(badge(Status::Neutral, None, move || n.clone()));
                }
                v = v.child(wrap);
                v.test_id("variants-preview").into_element()
            }
            Err(e) => label(
                move || error_text(st, e.code),
                ty::SM,
                FontWeight::MEDIUM,
                Tone::Danger,
            )
            .into_element(),
        }
    });
    out.child(section(
        st,
        Icon::Layers,
        "editor.card.attributes",
        Some("editor.card.attributes.help"),
        add,
        view()
            .child(rows)
            .child(divider())
            .child(preview)
            .child(when(
                move || !f.err("attributes")().is_empty(),
                move || {
                    label(
                        f.err("attributes"),
                        ty::XS,
                        FontWeight::MEDIUM,
                        Tone::Danger,
                    )
                    .into_element()
                },
            )),
    ))
}

fn photos(st: AppState, f: Form) -> View {
    let pick = move || {
        let task = spawn(async move {
            let dialog = nsh_core::file_dialog::OpenDialog::files()
                .multiple(true)
                .title(st.tn("editor.add_photos"));
            let Ok(Some(paths)) = pick_paths(dialog).await else {
                return;
            };
            for p in paths {
                let Ok(bytes) = std::fs::read(&p) else {
                    continue;
                };
                match nb_store::usecases::catalog::sniff_image(&bytes) {
                    Some(mime) => f.pics.update(|v| {
                        v.push(Pic {
                            id: None,
                            mime: mime.into(),
                            bytes,
                        })
                    }),
                    None => f
                        .errors
                        .set(vec![("image".into(), error_text(st, "image.format"))]),
                }
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let grid = dynamic(move || {
        let pics = f.pics.get();
        if pics.is_empty() {
            return label(
                st.t("editor.photos_empty"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        }
        let mut g = view().flex_row().flex_wrap().gap(px(12.0));
        for (i, p) in pics.into_iter().enumerate() {
            let src = data_url(&p.mime, &p.bytes);
            let id = p.id.clone();
            let remove = move || {
                if let Some(id) = id.clone() {
                    f.removed.update(|r| r.push(id));
                }
                f.pics.update(|v| {
                    if i < v.len() {
                        v.remove(i);
                    }
                });
            };
            g = g.child(
                view()
                    .gap(px(6.0))
                    .items_center()
                    .child(
                        view()
                            .w(px(120.0))
                            .h(px(120.0))
                            .rounded(px(radius::md()))
                            .overflow_hidden()
                            .border_1()
                            .border_color(Tone::Border.color())
                            .bg(Tone::Subtle.color())
                            .child(image(src).w(px(120.0)).h(px(120.0))),
                    )
                    .child(button(
                        Variant::Ghost,
                        Size::Sm,
                        Some(Icon::Trash),
                        st.t("action.delete"),
                        remove,
                    )),
            );
        }
        g.into_element()
    });
    section(
        st,
        Icon::ImagePlus,
        "editor.tab.photos",
        Some("editor.card.photos.help"),
        button(
            Variant::Secondary,
            Size::Sm,
            Some(Icon::ImagePlus),
            st.t("editor.add_photos"),
            pick,
        ),
        view().child(grid).child(when(
            move || !f.err("image")().is_empty(),
            move || label(f.err("image"), ty::XS, FontWeight::MEDIUM, Tone::Danger).into_element(),
        )),
    )
}

// ------------------------------------------------------------------ summary

fn summary(
    st: AppState,
    f: Form,
    lists: Memo<Lists>,
    currency: impl Fn() -> Currency + Copy + 'static,
) -> View {
    let line = move |key: &'static str, value: Box<dyn Fn() -> String>| {
        view()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(view().flex_1().child(label_line(
                st.t(key),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )))
            .child(label_line(value, ty::SM, FontWeight::SEMIBOLD, Tone::Ink))
    };
    let price = move || {
        parse_price(&f.price.get(), currency())
            .map(|m| st.fmt().money(m))
            .unwrap_or_else(|_| "—".into())
    };
    let first_code = computed(move || {
        f.codes
            .get()
            .first()
            .map(|c| c.code.get().trim().to_owned())
            .unwrap_or_default()
    });
    let code_view = dynamic(move || {
        let c = first_code.get();
        match barcode(&c, 220.0, 64.0) {
            Some(bars) => view()
                .items_center()
                .gap(px(4.0))
                .p(px(12.0))
                .rounded(px(radius::md()))
                .bg(rgb(0xFFFFFF))
                .border_1()
                .border_color(Tone::Border.color())
                .child(bars)
                .child(
                    label(move || c.clone(), ty::XS, FontWeight::MEDIUM, Tone::Ink)
                        .text_color(rgb(0x0A0D12)),
                )
                .into_element(),
            None => view()
                .h(px(64.0))
                .items_center()
                .justify_center()
                .rounded(px(radius::md()))
                .bg(Tone::Subtle.color())
                .child(label(
                    st.t("editor.no_barcode"),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .into_element(),
        }
    });
    let unit = move || {
        let base = STANDARD_UNITS
            .get(f.base_unit.get())
            .copied()
            .unwrap_or(STANDARD_UNITS[0]);
        let mut parts = vec![base.code.to_owned()];
        for p in f.packs.get() {
            parts.push(format!(
                "{} = {} {}",
                PACKS[p.unit.get().min(2)],
                p.factor.get(),
                base.code
            ));
        }
        parts.join(" · ")
    };
    let tax = move || {
        lists
            .get()
            .taxes
            .get(f.tax.get())
            .map(|t| t.name.clone())
            .unwrap_or_default()
    };
    card()
        .w(px(300.0))
        .p(px(20.0))
        .gap(px(14.0))
        .child(label_line(
            st.t("editor.summary"),
            ty::XS,
            FontWeight::SEMIBOLD,
            Tone::Primary,
        ))
        .child(label(
            move || {
                let n = f.name.get();
                if n.trim().is_empty() {
                    st.tn("editor.unnamed")
                } else {
                    n
                }
            },
            ty::LG,
            FontWeight::BOLD,
            Tone::Ink,
        ))
        .child(code_view)
        .child(divider())
        .child(line("products.col.price", Box::new(price)))
        .child(line("products.form.tax", Box::new(tax)))
        .child(line("products.col.unit", Box::new(unit)))
        .child(line(
            "editor.kind",
            Box::new(move || {
                st.tn(if f.service.get() {
                    "editor.kind.service"
                } else {
                    "editor.kind.goods"
                })
            }),
        ))
}

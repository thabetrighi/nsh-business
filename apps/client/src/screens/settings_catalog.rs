//! Settings for the catalog (M5.6, M5.8): the tax classes products refer to,
//! and the layouts of scale labels the scanner reads. Both are the
//! organization's data, saved through the catalog use cases.

use nb_core::Decimal;
use nb_i18n::tr_args;
use nb_store::usecases::catalog::{
    ScaleFormatRow, TaxClass, list_scale_formats, list_tax_classes, save_tax_class,
    set_scale_formats,
};
use nsh_core::element::{KeyboardType, Owner, View};
use nsh_core::prelude::*;

use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::form::row_input;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, button, checkbox, divider, icon_button, label, label_line};
use crate::ui::overlay::show_toast;
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon};
use crate::ui::tokens::{Tone, radius, ty};

fn header_cell(st: AppState, key: &'static str, w: Option<f32>) -> View {
    let v = view().child(label_line(
        st.t(key),
        ty::XS,
        FontWeight::SEMIBOLD,
        Tone::Muted,
    ));
    match w {
        Some(w) => v.w(px(w)),
        None => v.flex_1().min_w(px(0.0)),
    }
}

// ------------------------------------------------------------------ tax classes

#[derive(Clone, Copy, PartialEq)]
struct TaxRow {
    id: Signal<String>,
    name: Signal<String>,
    rate: Signal<String>,
    default: Signal<bool>,
}

/// Rate typed as a percentage (`19`, `9,5`) to basis points.
fn bps_of(text: &str) -> Option<i64> {
    let d: Decimal = text
        .trim()
        .trim_end_matches('%')
        .trim()
        .replace(',', ".")
        .parse()
        .ok()?;
    let bps = d.checked_mul(Decimal::from(100))?;
    if bps.fract().is_zero() {
        i64::try_from(bps).ok()
    } else {
        None
    }
}

pub fn tax_classes_card(st: AppState, compact: bool) -> View {
    let owner = Owner::current();
    let rows: Signal<Vec<TaxRow>> = signal(Vec::new());
    let error = signal(String::new());
    let make = move |c: &TaxClass| {
        let (id, name, rate, def) = (c.id.clone(), c.name.clone(), c.rate_bps, c.is_default);
        owner.run(|| TaxRow {
            id: signal(id),
            name: signal(name),
            rate: signal(
                Decimal::new(rate, 2)
                    .normalize()
                    .to_string()
                    .replace('.', ","),
            ),
            default: signal(def),
        })
    };
    let loaded = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_tax_classes(s)).await }
        },
    );
    effect(move || {
        if let Some(list) = loaded.get() {
            rows.set(list.iter().map(make).collect());
        }
    });
    let save = move || {
        let Some(session) = st.session.get_untracked() else {
            return;
        };
        error.set(String::new());
        let mut classes = Vec::new();
        for r in rows.get_untracked() {
            let Some(bps) = bps_of(&r.rate.get_untracked()) else {
                error.set(error_text(st, "tax.rate_range"));
                return;
            };
            classes.push(TaxClass {
                id: r.id.get_untracked(),
                name: r.name.get_untracked(),
                rate_bps: bps,
                is_default: r.default.get_untracked(),
            });
        }
        let db = st.db();
        let task = spawn(async move {
            let r = db
                .call(move |s| {
                    for c in classes {
                        save_tax_class(s, session.meta("tax.save"), c)?;
                    }
                    Ok(())
                })
                .await;
            match r {
                Ok(()) => {
                    st.changed();
                    show_toast(st, st.tn("settings.saved"));
                }
                Err(e) => error.set(error_text(st, e.codes.first().map_or("internal", |c| c.1))),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let list = dynamic(move || {
        let mut v = view().gap(px(10.0));
        if !compact {
            v = v.child(
                view()
                    .flex_row()
                    .gap(px(10.0))
                    .child(header_cell(st, "tax.class_name", None))
                    .child(header_cell(st, "tax.rate", Some(110.0)))
                    .child(header_cell(st, "tax.default", Some(80.0))),
            );
        }
        for (i, r) in rows.get().into_iter().enumerate() {
            v = v.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(view().flex_1().min_w(px(0.0)).child(row_input(
                        r.name,
                        String::new,
                        KeyboardType::Default,
                        format!("tax-name-{i}"),
                        || false,
                    )))
                    .child(view().w(px(110.0)).child(row_input(
                        r.rate,
                        || "19".into(),
                        KeyboardType::Decimal,
                        format!("tax-rate-{i}"),
                        || false,
                    )))
                    .child(view().w(px(80.0)).items_center().child(checkbox(
                        move || Some(r.default.get()),
                        move || {
                            // One default: choosing a class unchecks the others.
                            for o in rows.get_untracked() {
                                o.default.set(o == r);
                            }
                        },
                    ))),
            );
        }
        v.into_element()
    });
    let add = button(
        Variant::Secondary,
        Size::Sm,
        Some(Icon::Plus),
        st.t("tax.add_class"),
        move || {
            let r = owner.run(|| TaxRow {
                id: signal(String::new()),
                name: signal(String::new()),
                rate: signal(String::new()),
                default: signal(false),
            });
            rows.update(|v| v.push(r));
        },
    )
    .test_id("tax-add");
    card()
        .child(card_header_icon(
            Some(Icon::PercentPlain),
            st.t("tax.classes"),
            Some(Box::new(st.t("tax.classes.help"))),
            add,
        ))
        .child(divider())
        .child(
            view()
                .p(px(20.0))
                .gap(px(14.0))
                .child(list)
                .child(when(
                    move || !error.get().is_empty(),
                    move || {
                        label(
                            move || error.get(),
                            ty::XS,
                            FontWeight::MEDIUM,
                            Tone::Danger,
                        )
                        .into_element()
                    },
                ))
                .child(
                    view().flex_row().justify_end().child(
                        button(
                            Variant::Primary,
                            Size::Md,
                            Some(Icon::Check),
                            st.t("action.save_changes"),
                            save,
                        )
                        .test_id("tax-save"),
                    ),
                ),
        )
}

// ------------------------------------------------------------------ scale labels

#[derive(Clone, Copy, PartialEq)]
struct ScaleRow {
    prefix: Signal<String>,
    item: Signal<String>,
    value: Signal<String>,
    /// 0 = weight, 1 = price.
    kind: Signal<usize>,
    decimals: Signal<String>,
}

/// An example label in a layout, and what it reads as.
fn example(st: AppState, r: ScaleRow) -> String {
    let (Ok(item), Ok(value), Ok(dec)) = (
        r.item.get().trim().parse::<usize>(),
        r.value.get().trim().parse::<usize>(),
        r.decimals.get().trim().parse::<u32>(),
    ) else {
        return String::new();
    };
    let prefix = r.prefix.get().trim().to_owned();
    if prefix.len() + item + value > 12 || item == 0 || value == 0 || dec > 3 {
        return st.tn("error.scale.layout");
    }
    let art = format!("{:0>item$}", "42");
    let raw = format!(
        "{:0>value$}",
        if r.kind.get() == 0 { "1235" } else { "18000" }
    );
    let filler = "0".repeat(12 - prefix.len() - item - value);
    let shown_value = Decimal::new(raw.parse().unwrap_or(0), dec)
        .normalize()
        .to_string();
    tr_args(
        st.lang(),
        if r.kind.get() == 0 {
            "scale.example_weight"
        } else {
            "scale.example_price"
        },
        &[
            (
                "code",
                &nb_i18n::bidi::isolate_ltr(&format!("{prefix} {art} {raw}{filler} C")),
            ),
            ("article", art.trim_start_matches('0')),
            ("value", &nb_i18n::bidi::isolate_ltr(&shown_value)),
        ],
    )
}

pub fn scale_formats_card(st: AppState, compact: bool) -> View {
    let owner = Owner::current();
    let rows: Signal<Vec<ScaleRow>> = signal(Vec::new());
    let error = signal(String::new());
    let make = move |f: &ScaleFormatRow| {
        let f = f.clone();
        owner.run(|| ScaleRow {
            prefix: signal(f.prefix),
            item: signal(f.item_digits.to_string()),
            value: signal(f.value_digits.to_string()),
            kind: signal(usize::from(f.value_kind == "price")),
            decimals: signal(f.decimals.to_string()),
        })
    };
    let loaded = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| list_scale_formats(s)).await }
        },
    );
    effect(move || {
        if let Some(list) = loaded.get() {
            rows.set(list.iter().map(make).collect());
        }
    });
    let save = move || {
        let Some(session) = st.session.get_untracked() else {
            return;
        };
        error.set(String::new());
        let mut formats = Vec::new();
        for r in rows.get_untracked() {
            let n = |s: Signal<String>| s.get_untracked().trim().parse::<u32>().ok();
            let (Some(item), Some(value), Some(decimals)) = (n(r.item), n(r.value), n(r.decimals))
            else {
                error.set(error_text(st, "scale.layout"));
                return;
            };
            formats.push(ScaleFormatRow {
                prefix: r.prefix.get_untracked(),
                item_digits: item,
                value_digits: value,
                value_kind: if r.kind.get_untracked() == 0 {
                    "weight"
                } else {
                    "price"
                }
                .into(),
                decimals,
            });
        }
        let db = st.db();
        let task = spawn(async move {
            match db
                .call(move |s| set_scale_formats(s, session.meta("scale.formats"), formats))
                .await
            {
                Ok(_) => {
                    st.changed();
                    show_toast(st, st.tn("settings.saved"));
                }
                Err(e) => error.set(error_text(st, e.codes.first().map_or("internal", |c| c.1))),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let list = dynamic(move || {
        let mut v = view().gap(px(12.0));
        for (i, r) in rows.get().into_iter().enumerate() {
            let small = |sig: Signal<String>, id: String| {
                view()
                    .w(px(if compact { 64.0 } else { 84.0 }))
                    .child(row_input(
                        sig,
                        String::new,
                        KeyboardType::Number,
                        id,
                        || false,
                    ))
            };
            let del = icon_button(
                || Icon::Trash,
                st.t("action.delete"),
                Variant::Ghost,
                move || {
                    rows.update(|v| {
                        if i < v.len() {
                            v.remove(i);
                        }
                    })
                },
            );
            v = v.child(
                view()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .rounded(px(radius::md()))
                    .bg(Tone::Subtle.color())
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap(px(8.0))
                            .child(label(
                                st.t("scale.prefix"),
                                ty::XS,
                                FontWeight::SEMIBOLD,
                                Tone::Muted,
                            ))
                            .child(small(r.prefix, format!("scale-prefix-{i}")))
                            .child(label(
                                st.t("scale.item_digits"),
                                ty::XS,
                                FontWeight::SEMIBOLD,
                                Tone::Muted,
                            ))
                            .child(small(r.item, format!("scale-item-{i}")))
                            .child(label(
                                st.t("scale.value_digits"),
                                ty::XS,
                                FontWeight::SEMIBOLD,
                                Tone::Muted,
                            ))
                            .child(small(r.value, format!("scale-value-{i}")))
                            .child(view().w(px(130.0)).child(dropdown(
                                move || {
                                    vec![(0usize, st.tn("scale.weight")), (1, st.tn("scale.price"))]
                                },
                                r.kind,
                                None,
                                st.t("scale.kind"),
                                "scale-kind",
                            )))
                            .child(label(
                                st.t("scale.decimals"),
                                ty::XS,
                                FontWeight::SEMIBOLD,
                                Tone::Muted,
                            ))
                            .child(small(r.decimals, format!("scale-dec-{i}")))
                            .child(view().flex_1())
                            .child(del),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(icon(Icon::Barcode, 14.0, || Tone::Faint))
                            .child(label(
                                move || example(st, r),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    ),
            );
        }
        v.into_element()
    });
    let add = button(
        Variant::Secondary,
        Size::Sm,
        Some(Icon::Plus),
        st.t("scale.add"),
        move || {
            let r = owner.run(|| ScaleRow {
                prefix: signal("22".into()),
                item: signal("5".into()),
                value: signal("5".into()),
                kind: signal(1),
                decimals: signal("2".into()),
            });
            rows.update(|v| v.push(r));
        },
    )
    .test_id("scale-add");
    card()
        .child(card_header_icon(
            Some(Icon::Barcode),
            st.t("scale.formats"),
            Some(Box::new(st.t("scale.formats.help"))),
            add,
        ))
        .child(divider())
        .child(
            view()
                .p(px(20.0))
                .gap(px(14.0))
                .child(list)
                .child(when(
                    move || !error.get().is_empty(),
                    move || {
                        label(
                            move || error.get(),
                            ty::XS,
                            FontWeight::MEDIUM,
                            Tone::Danger,
                        )
                        .into_element()
                    },
                ))
                .child(
                    view().flex_row().justify_end().child(
                        button(
                            Variant::Primary,
                            Size::Md,
                            Some(Icon::Check),
                            st.t("action.save_changes"),
                            save,
                        )
                        .test_id("scale-save"),
                    ),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::bps_of;

    #[test]
    fn rates_typed_as_percent() {
        assert_eq!(bps_of("19"), Some(1_900));
        assert_eq!(bps_of("9,5 %"), Some(950));
        assert_eq!(bps_of("0"), Some(0));
        assert_eq!(bps_of("19,005"), None);
        assert_eq!(bps_of("abc"), None);
    }
}

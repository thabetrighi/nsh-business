//! Choosing a product by typing part of its name, code or barcode (or
//! scanning it): a search field and the first matches.

use nb_store::repo::catalog::{ProductQuery, ProductRow};
use nb_store::usecases::products::list_products;
use nsh_core::element::View;
use nsh_core::prelude::*;

use crate::state::AppState;
use crate::ui::form::search_field;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::label_line;
use crate::ui::tokens::{Tone, radius, ty};

/// A product search; `on_pick` gets the product chosen, and the field clears.
pub fn product_picker(
    st: AppState,
    test_id: &'static str,
    on_pick: impl Fn(ProductRow) + Clone + 'static,
) -> View {
    let text = signal(String::new());
    let found = resource(
        move || {
            let t = text.get();
            (t.trim().len() >= 2).then_some(t)
        },
        move |t, _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    Ok(list_products(
                        s,
                        &ProductQuery {
                            text: t,
                            limit: 8,
                            with_counts: false,
                            ..Default::default()
                        },
                    )?
                    .rows
                    .into_iter()
                    .filter(|p| p.kind == "goods")
                    .collect::<Vec<_>>())
                })
                .await
            }
        },
    );
    let results = dynamic(move || {
        let rows = if text.get().trim().len() >= 2 {
            found
                .with(|r: Option<&Vec<ProductRow>>| r.cloned())
                .unwrap_or_default()
        } else {
            vec![]
        };
        if rows.is_empty() {
            return view().into_element();
        }
        let mut list = view()
            .rounded(px(radius::md()))
            .border_1()
            .border_color(Tone::Border.color())
            .bg(Tone::Surface.color())
            .overflow_hidden();
        for (i, p) in rows.into_iter().enumerate() {
            let pick = on_pick.clone();
            let (name, sku) = (p.name.clone(), p.sku.clone());
            list = list.child(
                view()
                    .test_id(&format!("{test_id}-result-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(12.0))
                    .py(px(9.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .on_press(move || {
                        pick(p.clone());
                        text.set(String::new());
                    })
                    .child(icon(Icon::Package, 16.0, || Tone::Faint))
                    .child(view().flex_1().min_w(px(0.0)).child(label_line(
                        move || name.clone(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Ink,
                    )))
                    .child(label_line(
                        move || sku.clone(),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )),
            );
        }
        list.into_element()
    });
    view()
        .gap(px(6.0))
        .child(search_field(text, st.t("picker.search"), test_id))
        .child(results)
}

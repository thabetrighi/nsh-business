//! M0.5 spike: Arabic text, RTL layout direction, mixed bidi text.

use nsh_base::theme;
use nsh_core::prelude::*;

#[native_module("hello/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

fn row_item(label: &'static str, value: &'static str) -> impl IntoElement {
    view()
        .flex_row()
        .justify_between()
        .px_4()
        .py_3()
        .gap_2()
        .bg(move || theme().muted)
        .rounded(px(8.0))
        .child(text(label).text_size(px(17.0)))
        .child(text(value).text_size(px(17.0)))
}

fn panel(title: &'static str, dir: LayoutDirection) -> impl IntoElement {
    view()
        .direction(dir)
        .flex_1()
        .p_4()
        .gap_2()
        .child(text(title).text_size(px(20.0)))
        .child(row_item("المنتج: حليب كامل الدسم 1L", "120.00 دج"))
        .child(row_item("الكمية", "3 × كرتون (12 علبة)"))
        .child(row_item("SKU: MLK-001 / باركود 6130000000001", "✓"))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    view()
                        .px_4()
                        .py_2()
                        .rounded(px(8.0))
                        .bg(move || theme().primary)
                        .child(text("دفع").text_color(move || theme().primary_foreground)),
                )
                .child(
                    view()
                        .px_4()
                        .py_2()
                        .rounded(px(8.0))
                        .bg(move || theme().secondary)
                        .child(text("تعليق الفاتورة")),
                ),
        )
}

#[component]
pub fn App() -> impl IntoElement {
    view()
        .flex_1()
        .flex_row()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(panel("اتجاه RTL", LayoutDirection::Rtl))
        .child(panel("LTR direction", LayoutDirection::Ltr))
}

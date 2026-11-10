//! M0.5 spike: Arabic text, RTL layout direction, mixed bidi text.

use nsh_base::theme;
use nsh_core::prelude::*;

#[native_module("hello/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// How the panel prepares mixed-direction text (web bidi spike).
#[derive(Clone, Copy)]
enum Fix {
    None,
    /// Explicit paragraph direction on each text.
    WritingRtl,
    /// Wrap Latin/number runs in Unicode isolates (FSI ... PDI).
    Isolates,
}

fn label(s: &'static str, fix: Fix) -> impl IntoElement {
    let content = match fix {
        Fix::Isolates => isolate_ltr_runs(s),
        _ => s.to_owned(),
    };
    let t = text(content).text_size(px(17.0));
    match fix {
        Fix::WritingRtl => t.writing_direction(WritingDirection::Rtl),
        _ => t,
    }
}

/// Wraps every run of non-Arabic, non-space characters in FSI/PDI.
fn isolate_ltr_runs(s: &str) -> String {
    let is_ar = |c: char| ('\u{0600}'..='\u{06FF}').contains(&c);
    let mut out = String::new();
    let mut run = String::new();
    for c in s.chars() {
        if is_ar(c) || c == ' ' || c == ':' {
            if !run.is_empty() {
                out.push('\u{2068}');
                out.push_str(&run);
                out.push('\u{2069}');
                run.clear();
            }
            out.push(c);
        } else {
            run.push(c);
        }
    }
    if !run.is_empty() {
        out.push('\u{2068}');
        out.push_str(&run);
        out.push('\u{2069}');
    }
    out
}

fn row_item(label_s: &'static str, value: &'static str, fix: Fix) -> impl IntoElement {
    view()
        .flex_row()
        .justify_between()
        .px_4()
        .py_3()
        .gap_2()
        .bg(move || theme().muted)
        .rounded(px(8.0))
        .child(label(label_s, fix))
        .child(label(value, fix))
}

fn panel(title: &'static str, dir: LayoutDirection, fix: Fix) -> impl IntoElement {
    view()
        .direction(dir)
        .flex_1()
        .p_4()
        .gap_2()
        .child(label(title, fix))
        .child(row_item("المنتج: حليب كامل الدسم 1L", "120.00 دج", fix))
        .child(row_item("الكمية", "3 × كرتون (12 علبة)", fix))
        .child(row_item("SKU: MLK-001 / باركود 6130000000001", "✓", fix))
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
        .child(panel("اتجاه RTL", LayoutDirection::Rtl, Fix::None))
        .child(panel("RTL + writing_direction", LayoutDirection::Rtl, Fix::WritingRtl))
        .child(panel("RTL + عزل FSI/PDI", LayoutDirection::Rtl, Fix::Isolates))
        .child(panel("LTR direction", LayoutDirection::Ltr, Fix::None))
}

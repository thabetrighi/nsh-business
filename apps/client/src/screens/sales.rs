//! Sales pages (M8.13): the sales made, the goods brought back, and the
//! documents before the invoice (quotes and orders) — drafted, approved,
//! turned into an order, invoiced at the till, or cancelled. They show and
//! collect; the use cases decide.

use nb_core::{Money, Quantity, Rate};
use nb_domain::sale::{Cart, CartLine, Discount};
use nb_domain::tax::TaxMode;
use nb_i18n::tr_args;
use nb_store::repo::catalog::ProductRow;
use nb_store::repo::org::Warehouse;
use nb_store::usecases::documents::{
    Document, DocumentInput, DocumentPage, DocumentQuery, DocumentRow, approve_document,
    cancel_document, convert_to_order, get_document, list_documents, save_document,
};
use nb_store::usecases::returns::{ReturnPage, list_returns};
use nb_store::usecases::sales::{SaleLineInput, SalesPage, SalesQuery, list_sales};
use nsh_core::element::{AnyElement, KeyboardType, Owner, View};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::nav::Route;
use crate::screens::common::{date_time, page};
use crate::screens::forms::{error_text, parse_price};
use crate::screens::picker::product_picker;
use crate::state::{AppState, Intent};
use crate::ui::form::{field, labelled, row_input, search_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, figure, icon_button, label, label_line, segmented,
};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, empty_state};
use crate::ui::table::{Paging, pager};
use crate::ui::tokens::{Status, Tone, ty};

// ------------------------------------------------------------------ helpers

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

fn method_text(st: AppState, m: &str) -> String {
    st.tn(&format!("pos.method.{m}"))
}

/// A row of a list: a title line and a detail line at the start, then
/// what follows (badges, the amount).
fn list_row(test_id: String, title: String, detail: String, lead: Icon, tail: View) -> View {
    view()
        .test_id(&test_id)
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(16.0))
        .py(px(11.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(icon(lead, 18.0, || Tone::Faint))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(label_line(
                    move || title.clone(),
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
        .child(tail)
}

/// The figures above a list: how many, and how much.
fn summary_bar(st: AppState, count_key: &'static str, count: Memo<u32>, sum: Memo<i64>) -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(16.0))
        .px(px(16.0))
        .py(px(10.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .bg(Tone::Subtle.color())
        .child(label_line(
            move || {
                tr_args(
                    st.lang(),
                    count_key,
                    &[("count", &st.fmt().integer(i64::from(count.get())))],
                )
            },
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Body,
        ))
        .child(view().flex_1())
        .child(figure(
            move || money(st, sum.get()),
            ty::MD,
            FontWeight::BOLD,
            || Tone::Ink,
        ))
}

// ------------------------------------------------------------------- sales

pub fn sales_page(st: AppState, compact: bool) -> View {
    let text = signal(String::new());
    // The sale whose print dialog is open.
    let printing: Signal<Option<String>> = signal(None);
    let printing_open = signal(false);
    let paging = Paging::new(25);
    effect(move || {
        text.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            Some((
                SalesQuery {
                    search: text.get(),
                    limit: paging.size.get(),
                    offset: paging.offset(),
                },
                st.data_rev.get(),
            ))
        },
        move |(q, _), _| {
            let db = st.db();
            async move { db.call(move |s| list_sales(s, &q)).await }
        },
    );
    let page_data = computed(move || {
        data.with(|p: Option<&SalesPage>| p.cloned())
            .unwrap_or_default()
    });
    let list = dynamic(move || {
        let p = page_data.get();
        if p.rows.is_empty() {
            return empty_state(
                Icon::Receipt,
                st.t("sales.empty"),
                st.t("sales.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, r) in p.rows.into_iter().enumerate() {
            let methods = r
                .methods
                .iter()
                .map(|m| method_text(st, m))
                .collect::<Vec<_>>()
                .join(" · ");
            let who = r
                .customer_name
                .clone()
                .map(|c| format!(" · {c}"))
                .unwrap_or_default();
            let detail = format!(
                "{}{who} · {}",
                date_time(st, r.created_at),
                tr_args(
                    st.lang(),
                    "pos.cart.count",
                    &[("count", &st.fmt().integer(i64::from(r.lines)))]
                )
            );
            let mut tail = view().flex_row().items_center().gap(px(8.0));
            if !compact {
                tail = tail.child(badge(Status::Neutral, None, move || methods.clone()));
            }
            if let Some(d) = r.document.clone() {
                tail = tail.child(badge(Status::Info, Some(Icon::FileText), move || d.clone()));
            }
            if let Some(n) = r.edited_into.clone() {
                tail = tail.child(badge(Status::Warning, Some(Icon::Pencil), move || {
                    tr_args(st.lang(), "pos.sale.corrected", &[("number", &n)])
                }));
            }
            if let Some(n) = r.replaces.clone() {
                tail = tail.child(badge(Status::Info, Some(Icon::Pencil), move || {
                    tr_args(st.lang(), "sales.replaces", &[("number", &n)])
                }));
            }
            if r.credit_minor > 0 {
                let credit = r.credit_minor;
                tail = tail.child(badge(Status::Danger, None, move || {
                    tr_args(
                        st.lang(),
                        "pos.sale.on_account",
                        &[("amount", &money(st, credit))],
                    )
                }));
            }
            if r.returned_minor > 0 && r.edited_into.is_none() {
                let back = r.returned_minor;
                tail = tail.child(badge(Status::Warning, Some(Icon::Undo), move || {
                    tr_args(st.lang(), "sales.returned", &[("amount", &money(st, back))])
                }));
            }
            let total = r.total_minor;
            tail = tail.child(figure(
                move || money(st, total),
                ty::SM,
                FontWeight::BOLD,
                || Tone::Ink,
            ));
            #[cfg(not(target_arch = "wasm32"))]
            {
                let number = r.number.clone();
                tail = tail.child(
                    icon_button(
                        || Icon::Printer,
                        st.t("action.print"),
                        Variant::Ghost,
                        move || {
                            printing.set(Some(number.clone()));
                            printing_open.set(true);
                        },
                    )
                    .test_id(&format!("sale-print-{i}")),
                );
            }
            v = v.child(list_row(
                format!("sale-{i}"),
                r.number,
                detail,
                Icon::Receipt,
                tail,
            ));
        }
        v.into_element()
    });
    let count = computed(move || page_data.get().total);
    let sum = computed(move || page_data.get().total_minor);
    let body = card()
        .child(
            view()
                .p(px(12.0))
                .child(search_field(text, st.t("sales.search"), "sales-search")),
        )
        .child(summary_bar(st, "sales.count", count, sum))
        .child(list)
        .child(pager(st, paging, move || page_data.get().total, compact));
    let actions = view().child(
        button(
            Variant::Primary,
            Size::Md,
            Some(Icon::Cart),
            st.t("sales.to_till"),
            move || st.route.set(Route::Sell),
        )
        .test_id("sales-till"),
    );
    let dialog = modal(printing_open, compact, move || {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(number) = printing.get_untracked() {
            return crate::screens::printing::print_dialog(st, number, move || {
                printing_open.set(false)
            })
            .into_element();
        }
        view().into_element()
    });
    view()
        .child(page(
            st,
            Route::Sales,
            "sales.subtitle",
            actions,
            body,
            compact,
        ))
        .child(dialog)
}

// ----------------------------------------------------------------- returns

pub fn returns_page(st: AppState, compact: bool) -> View {
    let text = signal(String::new());
    let paging = Paging::new(25);
    effect(move || {
        text.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            Some((
                text.get(),
                paging.size.get(),
                paging.offset(),
                st.data_rev.get(),
            ))
        },
        move |(q, limit, offset, _), _| {
            let db = st.db();
            async move { db.call(move |s| list_returns(s, &q, limit, offset)).await }
        },
    );
    let page_data = computed(move || {
        data.with(|p: Option<&ReturnPage>| p.cloned())
            .unwrap_or_default()
    });
    let list = dynamic(move || {
        let p = page_data.get();
        if p.rows.is_empty() {
            return empty_state(
                Icon::Undo,
                st.t("returns.empty"),
                st.t("returns.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, r) in p.rows.into_iter().enumerate() {
            let title = tr_args(
                st.lang(),
                "returns.title_row",
                &[("number", &r.number), ("sale", &r.sale_number)],
            );
            let detail = format!("{} · {}", date_time(st, r.created_at), r.reason);
            let method = method_text(st, &r.refund_method);
            let mut tail = view().flex_row().items_center().gap(px(8.0));
            if !compact {
                tail = tail
                    .child(badge(Status::Neutral, None, move || method.clone()))
                    .child(if r.restocked {
                        badge(
                            Status::Success,
                            Some(Icon::Boxes),
                            st.t("returns.restocked"),
                        )
                    } else {
                        badge(Status::Danger, None, st.t("returns.damaged"))
                    });
            }
            let total = r.total_minor;
            tail = tail.child(figure(
                move || money(st, total),
                ty::SM,
                FontWeight::BOLD,
                || Tone::Ink,
            ));
            v = v.child(list_row(
                format!("return-{i}"),
                title,
                detail,
                Icon::Undo,
                tail,
            ));
        }
        v.into_element()
    });
    let count = computed(move || page_data.get().total);
    let sum = computed(move || page_data.get().total_minor);
    let body = card()
        .child(view().p(px(12.0)).child(search_field(
            text,
            st.t("returns.search"),
            "returns-search",
        )))
        .child(summary_bar(st, "returns.count", count, sum))
        .child(list)
        .child(pager(st, paging, move || page_data.get().total, compact));
    let actions = view().child(
        button(
            Variant::Secondary,
            Size::Md,
            Some(Icon::Cart),
            st.t("returns.at_till"),
            move || st.route.set(Route::Sell),
        )
        .test_id("returns-till"),
    );
    page(
        st,
        Route::Returns,
        "returns.subtitle",
        actions,
        body,
        compact,
    )
}

// --------------------------------------------------------------- documents

fn status_badge(st: AppState, status: &str) -> View {
    let s = match status {
        "draft" => Status::Neutral,
        "approved" => Status::Info,
        "converted" | "invoiced" => Status::Success,
        _ => Status::Danger,
    };
    let key = format!("doc.status.{status}");
    badge(s, None, move || st.tn(&key))
}

/// What the documents page asks for: which kind, which state.
const KINDS: [&str; 3] = ["", "quote", "order"];
const STATES: [&str; 6] = [
    "",
    "draft",
    "approved",
    "converted",
    "invoiced",
    "cancelled",
];

pub fn documents_page(st: AppState, compact: bool) -> View {
    let text = signal(String::new());
    let kind = signal(0usize);
    let status = signal(0usize);
    let paging = Paging::new(25);
    effect(move || {
        text.with(|_| ());
        kind.with(|_| ());
        status.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            Some((
                DocumentQuery {
                    kind: KINDS[kind.get()].into(),
                    status: STATES[status.get()].into(),
                    search: text.get(),
                    limit: paging.size.get(),
                    offset: paging.offset(),
                },
                st.data_rev.get(),
            ))
        },
        move |(q, _), _| {
            let db = st.db();
            async move { db.call(move |s| list_documents(s, &q)).await }
        },
    );
    let page_data = computed(move || {
        data.with(|p: Option<&DocumentPage>| p.cloned())
            .unwrap_or_default()
    });
    // The dialogs: a document's details, or the editor (new or a draft).
    let showing: Signal<Option<String>> = signal(None);
    let showing_open = signal(false);
    effect(move || {
        let open = showing.get().is_some();
        if showing_open.get_untracked() != open {
            showing_open.set(open);
        }
    });
    effect(move || {
        if !showing_open.get() {
            showing.set(None);
        }
    });
    let editing: Signal<Option<Edit>> = signal(None);
    let editing_open = signal(false);
    effect(move || editing_open.set(editing.get().is_some()));
    effect(move || {
        if !editing_open.get() {
            editing.set(None);
        }
    });

    let list = dynamic(move || {
        let p = page_data.get();
        if p.rows.is_empty() {
            return empty_state(
                Icon::FileText,
                st.t("doc.empty"),
                st.t("doc.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, r) in p.rows.into_iter().enumerate() {
            v = v.child(doc_row(st, i, r, compact, showing));
        }
        v.into_element()
    });
    let filters = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
        .p(px(12.0))
        .child(view().flex_1().min_w(px(200.0)).child(search_field(
            text,
            st.t("doc.search"),
            "docs-search",
        )))
        .child(segmented(
            vec![
                (0, Box::new(st.t("doc.kind.all"))),
                (1, Box::new(st.t("doc.kind.quote"))),
                (2, Box::new(st.t("doc.kind.order"))),
            ],
            kind,
        ))
        .child(view().w(px(170.0)).child(dropdown(
            move || {
                STATES
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        (
                            i,
                            if s.is_empty() {
                                st.tn("doc.status.all")
                            } else {
                                st.tn(&format!("doc.status.{s}"))
                            },
                        )
                    })
                    .collect()
            },
            status,
            Some(Icon::Filter),
            st.t("doc.status"),
            "doc-status",
        )));
    let body = card().child(filters).child(list).child(pager(
        st,
        paging,
        move || page_data.get().total,
        compact,
    ));
    let actions = view().flex_row().flex_wrap().gap(px(8.0)).child(when(
        move || st.can("sales.sell"),
        move || {
            view()
                .flex_row()
                .gap(px(8.0))
                .child(
                    button(
                        Variant::Secondary,
                        Size::Md,
                        Some(Icon::ClipboardList),
                        st.t("doc.new_order"),
                        move || editing.set(Some(Edit::New("order"))),
                    )
                    .test_id("doc-new-order"),
                )
                .child(
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Plus),
                        st.t("doc.new_quote"),
                        move || editing.set(Some(Edit::New("quote"))),
                    )
                    .test_id("doc-new-quote"),
                )
                .into_element()
        },
    ));
    view()
        .child(page(
            st,
            Route::Quotes,
            "doc.subtitle",
            actions,
            body,
            compact,
        ))
        .child(modal(showing_open, compact, move || {
            doc_dialog(st, showing, editing, move || showing_open.set(false)).into_element()
        }))
        .child(modal(editing_open, compact, move || {
            match editing.get_untracked() {
                Some(e) => {
                    let after = move |id: String| showing.set(Some(id));
                    editor_dialog(st, e, after, move || editing_open.set(false)).into_element()
                }
                None => view().into_element(),
            }
        }))
}

fn doc_row(
    st: AppState,
    i: usize,
    r: DocumentRow,
    compact: bool,
    showing: Signal<Option<String>>,
) -> View {
    let customer = if r.customer_name.is_empty() {
        st.tn("doc.no_customer")
    } else {
        r.customer_name.clone()
    };
    let title = format!("{} · {}", r.number, customer);
    let mut detail = date_time(st, r.created_at);
    if let Some(d) = r.valid_until.as_deref() {
        detail = format!(
            "{detail} · {}",
            tr_args(st.lang(), "doc.valid_until_short", &[("date", d)])
        );
    }
    if let Some(n) = r.next_number.as_deref() {
        detail = format!("{detail} → {n}");
    }
    let mut tail = view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(status_badge(st, &r.status));
    if !compact {
        let k = format!("doc.kind.{}", r.kind);
        tail = tail.child(label_line(
            move || st.tn(&k),
            ty::XS,
            FontWeight::MEDIUM,
            Tone::Muted,
        ));
    }
    let total = r.total_minor;
    tail = tail.child(figure(
        move || money(st, total),
        ty::SM,
        FontWeight::BOLD,
        || Tone::Ink,
    ));
    let id = r.id.clone();
    list_row(
        format!("doc-{i}"),
        title,
        detail,
        if r.kind == "quote" {
            Icon::FileText
        } else {
            Icon::ClipboardList
        },
        tail,
    )
    .cursor_pointer()
    .hover(|s| s.bg(Tone::Hover.color()))
    .on_press(move || showing.set(Some(id.clone())))
}

/// A document's details and what may be done with it now.
fn doc_dialog(
    st: AppState,
    showing: Signal<Option<String>>,
    editing: Signal<Option<Edit>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    // The document shown (it changes when a quote becomes an order), as
    // the database has it now.
    let loaded = resource(
        move || showing.get().map(|id| (id, st.data_rev.get())),
        move |(id, _), _| {
            let db = st.db();
            async move { db.call(move |s| get_document(s, &id)).await }
        },
    );
    let doc = computed(move || loaded.with(|d: Option<&Document>| d.cloned()));
    let error = signal(String::new());
    let busy = signal(false);
    let cancelling = signal(false);
    let reason = signal(String::new());
    // Each press is one operation (the dialog may act on a quote, then
    // on the order it became); `busy` holds a second press meanwhile.
    let act = move |which: &'static str| {
        move || {
            if busy.get_untracked() {
                return;
            }
            let Some(d) = doc.get_untracked() else {
                return;
            };
            let id = d.row.id.clone();
            let why = reason.get_untracked();
            if which == "cancel" && why.trim().is_empty() {
                error.set(error_text(st, "required"));
                return;
            }
            busy.set(true);
            error.set(String::new());
            let meta = session.meta(match which {
                "approve" => "doc.approve",
                "convert" => "doc.convert",
                _ => "doc.cancel",
            });
            let db = st.db();
            let task = spawn(async move {
                let r = db
                    .call(move |s| match which {
                        "approve" => approve_document(s, meta, &id),
                        "convert" => convert_to_order(s, meta, &id),
                        _ => cancel_document(s, meta, &id, &why),
                    })
                    .await;
                busy.set(false);
                match r {
                    Ok(after) => {
                        st.changed();
                        cancelling.set(false);
                        let key = format!("doc.done.{which}");
                        show_toast(
                            st,
                            tr_args(st.lang(), &key, &[("number", &after.row.number)]),
                        );
                        showing.set(Some(after.row.id));
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };

    let body = dynamic(move || {
        let Some(d) = doc.get() else {
            return view().into_element();
        };
        let customer = if d.row.customer_name.is_empty() {
            st.tn("doc.no_customer")
        } else {
            d.row.customer_name.clone()
        };
        let mut facts = view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(status_badge(st, &d.row.status))
            .child(label_line(
                move || customer.clone(),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            ));
        if let Some(v) = d.row.valid_until.clone() {
            facts = facts.child(badge(Status::Neutral, Some(Icon::Calendar), move || {
                tr_args(st.lang(), "doc.valid_until_short", &[("date", &v)])
            }));
        }
        if let Some(n) = d.source_number.clone() {
            facts = facts.child(badge(Status::Neutral, None, move || {
                tr_args(st.lang(), "doc.from", &[("number", &n)])
            }));
        }
        if let Some(n) = d.row.next_number.clone() {
            facts = facts.child(badge(Status::Success, None, move || {
                tr_args(st.lang(), "doc.became", &[("number", &n)])
            }));
        }
        let mut lines = view();
        for l in d.lines.iter().cloned() {
            let q = st.fmt().quantity(Quantity::from_milli(l.qty_milli), 3);
            let what = format!("{q} {} × {}", l.unit, money(st, l.unit_price_minor));
            lines = lines.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(label_line(
                                move || l.name.clone(),
                                ty::SM,
                                FontWeight::MEDIUM,
                                Tone::Ink,
                            ))
                            .child(label_line(
                                move || what.clone(),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(figure(
                        move || money(st, l.gross_minor),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        || Tone::Ink,
                    )),
            );
        }
        let sum_row = |key: &'static str, v: i64, strong: bool| {
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(label_line(
                    st.t(key),
                    if strong { ty::MD } else { ty::SM },
                    if strong {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    },
                    Tone::Body,
                )))
                .child(figure(
                    move || money(st, v),
                    if strong { ty::LG } else { ty::SM },
                    if strong {
                        FontWeight::BOLD
                    } else {
                        FontWeight::MEDIUM
                    },
                    || Tone::Ink,
                ))
        };
        let mut v = view().gap(px(12.0)).child(facts).child(lines).child(
            view()
                .gap(px(4.0))
                .pt(px(8.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(sum_row("pos.net", d.net_minor, false))
                .child(sum_row("pos.tax", d.tax_minor, false))
                .child(sum_row("pos.total", d.row.total_minor, true)),
        );
        if !d.note.is_empty() {
            let note = d.note.clone();
            v = v.child(label(
                move || note.clone(),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        v.into_element()
    });
    let cancel_box = when(
        move || cancelling.get(),
        move || {
            view()
                .gap(px(8.0))
                .child(field(
                    st.t("doc.cancel_reason"),
                    reason,
                    String::new,
                    String::new,
                    KeyboardType::Default,
                    "doc-cancel-reason",
                ))
                .child(
                    button(
                        Variant::Danger,
                        Size::Md,
                        Some(Icon::XCircle),
                        st.t("doc.cancel_confirm"),
                        act("cancel"),
                    )
                    .test_id("doc-cancel-confirm"),
                )
                .into_element()
        },
    );
    let body = view()
        .gap(px(14.0))
        .child(body)
        .child(cancel_box)
        .child(error_line(error));

    // The actions this state allows.
    let footer = {
        let close = close.clone();
        dynamic(move || {
            let Some(d) = doc.get() else {
                return view().into_element();
            };
            let mut f = view().flex_row().flex_wrap().gap(px(8.0));
            let (status, kind) = (d.row.status.as_str(), d.row.kind.as_str());
            if matches!(status, "draft" | "approved") {
                f = f.child(
                    button(
                        Variant::Ghost,
                        Size::Md,
                        None,
                        st.t("doc.cancel"),
                        move || cancelling.update(|c| *c = !*c),
                    )
                    .test_id("doc-cancel"),
                );
            }
            if status == "draft" {
                let (close, d2) = (close.clone(), d.clone());
                f = f
                    .child(
                        button(
                            Variant::Secondary,
                            Size::Md,
                            Some(Icon::Pencil),
                            st.t("doc.edit"),
                            move || {
                                close();
                                editing.set(Some(Edit::Draft(Box::new(d2.clone()))));
                            },
                        )
                        .test_id("doc-edit"),
                    )
                    .child(
                        button(
                            Variant::Primary,
                            Size::Md,
                            Some(Icon::Check),
                            st.t("doc.approve"),
                            act("approve"),
                        )
                        .test_id("doc-approve"),
                    );
            }
            if status == "approved" && kind == "quote" {
                f = f.child(
                    button(
                        Variant::Secondary,
                        Size::Md,
                        Some(Icon::ClipboardList),
                        st.t("doc.convert"),
                        act("convert"),
                    )
                    .test_id("doc-convert"),
                );
            }
            if status == "approved" {
                let id = d.row.id.clone();
                f = f.child(
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Cart),
                        st.t("doc.invoice"),
                        move || {
                            st.intent.set(Some(Intent::Invoice(id.clone())));
                            st.route.set(Route::Sell);
                        },
                    )
                    .test_id("doc-invoice"),
                );
            }
            f.into_element()
        })
    };
    let title = move || {
        doc.get().map_or_else(String::new, |d| {
            tr_args(
                st.lang(),
                &format!("doc.title.{}", d.row.kind),
                &[("number", &d.row.number)],
            )
        })
    };
    dialog_frame(
        st,
        title,
        st.t("doc.detail.help"),
        body,
        view().child(footer),
        close,
    )
}

// ------------------------------------------------------------------ editor

/// What the editor opens on.
#[derive(Debug, Clone, PartialEq)]
enum Edit {
    New(&'static str),
    Draft(Box<Document>),
}

/// A line being edited.
#[derive(Clone, Copy, PartialEq)]
struct EditLine {
    product_id: Signal<Option<String>>,
    name: Signal<String>,
    unit: Signal<String>,
    tax_bps: Signal<i64>,
    qty: Signal<String>,
    price: Signal<String>,
}

fn parse_qty(s: &str) -> Option<Quantity> {
    let q: Quantity = s.trim().replace(',', ".").parse().ok()?;
    (nb_store::repo::to_milli(q).ok()? > 0).then_some(q)
}

fn parse_percent(s: &str) -> Option<i64> {
    if s.trim().is_empty() {
        return Some(0);
    }
    let q: Quantity = s.trim().replace(',', ".").parse().ok()?;
    let bps = nb_store::repo::to_milli(q).ok()? / 10;
    (0..=10_000).contains(&bps).then_some(bps)
}

fn editor_dialog(
    st: AppState,
    edit: Edit,
    after: impl Fn(String) + Clone + 'static,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let c = session.currency;
    let owner = Owner::current();
    let whs = warehouses(st);
    let (kind, draft) = match &edit {
        Edit::New(k) => (*k, None),
        Edit::Draft(d) => (
            if d.row.kind == "quote" {
                "quote"
            } else {
                "order"
            },
            Some(d.clone()),
        ),
    };
    let customer = signal(
        draft
            .as_ref()
            .map(|d| d.row.customer_name.clone())
            .unwrap_or_default(),
    );
    let valid_until = signal(
        draft
            .as_ref()
            .and_then(|d| d.row.valid_until.clone())
            .unwrap_or_default(),
    );
    let discount = signal(
        draft
            .as_ref()
            .filter(|d| d.discount_bps > 0)
            .map(|d| Quantity::from_milli(d.discount_bps * 10).to_string())
            .unwrap_or_default(),
    );
    let note = signal(draft.as_ref().map(|d| d.note.clone()).unwrap_or_default());
    let warehouse_id: Option<String> = draft.as_ref().map(|d| d.warehouse_id.clone());
    let wh = signal(0usize);
    {
        let wanted = warehouse_id.clone();
        effect(move || {
            let list = whs.get();
            if let Some(w) = wanted.as_deref()
                && let Some(i) = list.iter().position(|x| x.id == w)
            {
                wh.set(i);
            }
        });
    }
    let lines: Signal<Vec<EditLine>> = signal(
        draft
            .as_ref()
            .map(|d| {
                d.lines
                    .iter()
                    .map(|l| EditLine {
                        product_id: signal(l.product_id.clone()),
                        name: signal(l.name.clone()),
                        unit: signal(l.unit.clone()),
                        tax_bps: signal(l.tax_bps),
                        qty: signal(Quantity::from_milli(l.qty_milli).to_string()),
                        price: signal(Money::from_minor(l.unit_price_minor, c).to_plain_string()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    );
    let error = signal(String::new());
    let busy = signal(false);
    let meta = session.meta("doc.save");

    let pick = move |p: ProductRow| {
        let l = owner.run(|| EditLine {
            product_id: signal(Some(p.id.clone())),
            name: signal(p.name.clone()),
            unit: signal(p.units.first().map(|u| u.code.clone()).unwrap_or_default()),
            tax_bps: signal(p.tax_bps),
            qty: signal("1".into()),
            price: signal(Money::from_minor(p.price_minor, c).to_plain_string()),
        });
        lines.update(|v| v.push(l));
    };

    // The figures as the sale would price them.
    let totals = computed(move || {
        let ls = lines.get();
        if ls.is_empty() {
            return None;
        }
        let mut cart_lines = Vec::with_capacity(ls.len());
        for l in &ls {
            cart_lines.push(CartLine {
                qty: parse_qty(&l.qty.get())?,
                unit_price: parse_price(&l.price.get(), c).ok()?,
                min_price: None,
                tax_rate: Rate::from_bps(l.tax_bps.get()),
                discount: Discount::None,
            });
        }
        let d = parse_percent(&discount.get())?;
        Cart {
            mode: TaxMode::Inclusive,
            lines: cart_lines,
            discount: if d > 0 {
                Discount::Percent(d)
            } else {
                Discount::None
            },
        }
        .totals()
        .ok()
    });

    let save = {
        let (close, after) = (close.clone(), after.clone());
        move || {
            if busy.get_untracked() {
                return;
            }
            let ls = lines.get_untracked();
            let mut input_lines = Vec::with_capacity(ls.len());
            for l in &ls {
                let Ok(price) = parse_price(&l.price.get_untracked(), c) else {
                    error.set(error_text(st, "invalid_amount"));
                    return;
                };
                let product = l.product_id.get_untracked();
                input_lines.push(SaleLineInput {
                    name: product.is_none().then(|| l.name.get_untracked()),
                    tax_bps: product.is_none().then(|| l.tax_bps.get_untracked()),
                    product_id: product.unwrap_or_default(),
                    qty: l.qty.get_untracked().trim().replace(',', "."),
                    unit: l.unit.get_untracked(),
                    unit_price_minor: Some(price.minor()),
                    ..Default::default()
                });
            }
            let Some(d) = parse_percent(&discount.get_untracked()) else {
                error.set(error_text(st, "pos.discount_invalid"));
                return;
            };
            let Some(warehouse) =
                whs.with_untracked(|w| w.get(wh.get_untracked()).map(|w| w.id.clone()))
            else {
                return;
            };
            let input = DocumentInput {
                id: draft.as_ref().map(|d| d.row.id.clone()),
                kind: kind.into(),
                warehouse_id: warehouse,
                customer_id: None,
                customer_name: customer.get_untracked(),
                valid_until: Some(valid_until.get_untracked().trim().to_owned())
                    .filter(|v| !v.is_empty()),
                lines: input_lines,
                discount_bps: (d > 0).then_some(d),
                discount_minor: None,
                note: note.get_untracked(),
            };
            busy.set(true);
            error.set(String::new());
            let (close, after) = (close.clone(), after.clone());
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| save_document(s, meta, input)).await {
                    Ok(doc) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "doc.saved", &[("number", &doc.row.number)]),
                        );
                        after(doc.row.id);
                    }
                    Err(e) => {
                        busy.set(false);
                        error.set(first_error(st, &e));
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };

    let rows = dynamic(move || {
        let ls = lines.get();
        if ls.is_empty() {
            return label(
                st.t("doc.no_lines"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        }
        let mut v = view().gap(px(8.0));
        for (i, l) in ls.into_iter().enumerate() {
            let gross = move || {
                totals
                    .get()
                    .and_then(|t| t.lines.get(i).map(|x| x.taxed.gross.minor()))
                    .map_or_else(|| "—".to_owned(), |g| money(st, g))
            };
            v = v.child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .py(px(6.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(140.0))
                            .child(label_line(
                                move || l.name.get(),
                                ty::SM,
                                FontWeight::MEDIUM,
                                Tone::Ink,
                            ))
                            .child(label_line(
                                move || l.unit.get(),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(view().w(px(80.0)).child(row_input(
                        l.qty,
                        String::new,
                        KeyboardType::Decimal,
                        format!("doc-qty-{i}"),
                        move || parse_qty(&l.qty.get()).is_none(),
                    )))
                    .child(view().w(px(110.0)).child(row_input(
                        l.price,
                        String::new,
                        KeyboardType::Decimal,
                        format!("doc-price-{i}"),
                        move || parse_price(&l.price.get(), c).is_err(),
                    )))
                    .child(view().w(px(110.0)).items_end().child(figure(
                        gross,
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        || Tone::Ink,
                    )))
                    .child(
                        icon_button(
                            || Icon::Trash,
                            st.t("action.delete"),
                            Variant::Ghost,
                            move || {
                                lines.update(|v| {
                                    if i < v.len() {
                                        v.remove(i);
                                    }
                                })
                            },
                        )
                        .test_id(&format!("doc-remove-{i}")),
                    ),
            );
        }
        v.into_element()
    });

    let mut top =
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(12.0))
            .child(view().flex_1().min_w(px(200.0)).child(field(
                st.t("doc.customer"),
                customer,
                st.t("doc.customer.placeholder"),
                String::new,
                KeyboardType::Default,
                "doc-customer",
            )));
    if kind == "quote" {
        top = top.child(view().w(px(160.0)).child(field(
            st.t("doc.valid_until"),
            valid_until,
            st.t("doc.date_hint"),
            String::new,
            KeyboardType::Default,
            "doc-valid",
        )));
    }
    top = top.child(view().w(px(180.0)).child(labelled(
        st.t("doc.warehouse"),
        dropdown(
            move || {
                whs.get()
                    .into_iter()
                    .enumerate()
                    .map(|(i, w)| (i, w.name))
                    .collect()
            },
            wh,
            Some(Icon::Warehouse),
            st.t("doc.warehouse"),
            "doc-warehouse",
        ),
        None,
    )));
    let figures = view()
        .gap(px(4.0))
        .pt(px(8.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(
            view()
                .flex_row()
                .items_end()
                .gap(px(12.0))
                .child(view().w(px(140.0)).child(field(
                    st.t("doc.discount"),
                    discount,
                    || "0".to_owned(),
                    String::new,
                    KeyboardType::Decimal,
                    "doc-discount",
                )))
                .child(view().flex_1())
                .child(
                    view()
                        .items_end()
                        .child(label_line(
                            st.t("pos.total"),
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::Muted,
                        ))
                        .child(figure(
                            move || {
                                totals
                                    .get()
                                    .map_or_else(|| money(st, 0), |t| money(st, t.total.minor()))
                            },
                            ty::H2,
                            FontWeight::BOLD,
                            || Tone::Ink,
                        )),
                ),
        );
    let body = view()
        .gap(px(14.0))
        .child(top)
        .child(labelled(
            st.t("doc.add_item"),
            product_picker(st, "doc-search", pick),
            None,
        ))
        .child(rows)
        .child(figures)
        .child(field(
            st.t("doc.note"),
            note,
            String::new,
            String::new,
            KeyboardType::Default,
            "doc-note",
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
                Some(Icon::Check),
                st.t("doc.save"),
                save,
            )
            .test_id("doc-save"),
        );
    let title_key = match (&edit, kind) {
        (Edit::New(_), "quote") => "doc.new_quote",
        (Edit::New(_), _) => "doc.new_order",
        _ => "doc.edit_draft",
    };
    dialog_frame(
        st,
        st.t(title_key),
        st.t("doc.editor.help"),
        body,
        footer,
        close,
    )
}

fn warehouses(st: AppState) -> Memo<Vec<Warehouse>> {
    let r = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| {
                    s.read(|r| {
                        let o = nb_store::repo::org::the_org(r.conn())?
                            .ok_or(nb_store::Error::NotFound("organization"))?;
                        nb_store::repo::org::warehouses(r.conn(), o.org_id()?)
                    })
                })
                .await
            }
        },
    );
    computed(move || {
        r.with(|w: Option<&Vec<Warehouse>>| w.cloned())
            .unwrap_or_default()
    })
}

//! Buying (M9.10, M10): suppliers and what is owed to them, purchase
//! invoices that bring goods in at their landed cost, returns to the
//! supplier and payments. The pages show and collect; the use cases decide.

use nb_core::{Money, Quantity, Rate};
use nb_domain::sale::{Cart, CartLine, Discount};
use nb_domain::tax::TaxMode;
use nb_i18n::tr_args;
use nb_store::repo::catalog::ProductRow;
use nb_store::repo::org::Warehouse;
use nb_store::usecases::purchase_orders::{
    Order, OrderInput, OrderLine, OrderPage, OrderQuery, approve_order, cancel_order, close_order,
    get_order, list_orders, save_order,
};
use nb_store::usecases::purchasing::{
    BoughtLine, PurchaseInput, PurchaseLineInput, PurchasePage, PurchaseReturnInput, PurchaseRow,
    SupplierInput, SupplierPage, SupplierPaymentInput, SupplierRow, SupplierStatementLine,
    get_supplier, list_purchases, list_suppliers, open_purchases, pay_supplier, purchase_lines,
    record_purchase, return_purchase, save_supplier, supplier_statement,
};
use nsh_core::element::{AnyElement, KeyboardType, Owner, View};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::nav::Route;
use crate::screens::approval::{Ask, approval_dialog, needs_approval};
use crate::screens::common::{date_time, page};
use crate::screens::customers::{branch_till, opener};
use crate::screens::forms::{error_text, parse_price};
use crate::screens::picker::product_picker;
use crate::state::AppState;
use crate::ui::form::{field, labelled, row_input, search_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, figure, icon_button, label, label_line, segmented,
};
use crate::ui::overlay::{dialog_frame, modal, show_toast, wide_modal};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, empty_state};
use crate::ui::table::{Paging, pager};
use crate::ui::tokens::{Status, Tone, ty};

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

fn row_view(test_id: String, lead: Icon, title: String, detail: String, tail: View) -> View {
    view()
        .test_id(&test_id)
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(16.0))
        .py(px(11.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
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

fn amount_tile(st: AppState, key: &'static str, v: i64, tone: Tone) -> View {
    view()
        .flex_1()
        .min_w(px(120.0))
        .gap(px(2.0))
        .p(px(10.0))
        .rounded(px(8.0))
        .bg(Tone::Subtle.color())
        .child(label_line(
            st.t(key),
            ty::XS,
            FontWeight::MEDIUM,
            Tone::Muted,
        ))
        .child(figure(
            move || money(st, v),
            ty::LG,
            FontWeight::BOLD,
            move || tone,
        ))
}

fn suppliers_memo(st: AppState) -> Memo<Vec<SupplierRow>> {
    let r = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| Ok(list_suppliers(s, "", false, 500, 0)?.rows))
                    .await
            }
        },
    );
    computed(move || {
        r.with(|v: Option<&Vec<SupplierRow>>| v.cloned())
            .unwrap_or_default()
    })
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

// --------------------------------------------------------------- suppliers

pub fn suppliers_page(st: AppState, compact: bool) -> View {
    let text = signal(String::new());
    let owed = signal(0usize);
    let paging = Paging::new(25);
    effect(move || {
        text.with(|_| ());
        owed.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            Some((
                text.get(),
                owed.get() == 1,
                paging.size.get(),
                paging.offset(),
                st.data_rev.get(),
            ))
        },
        move |(q, o, limit, offset, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| list_suppliers(s, &q, o, limit, offset))
                    .await
            }
        },
    );
    let page_data = computed(move || {
        data.with(|p: Option<&SupplierPage>| p.cloned())
            .unwrap_or_default()
    });
    let (showing, showing_open) = opener::<String>();
    let (editing, editing_open) = opener::<Option<SupplierRow>>();
    let list = dynamic(move || {
        let p = page_data.get();
        if p.rows.is_empty() {
            return empty_state(
                Icon::Building,
                st.t("suppliers.empty"),
                st.t("suppliers.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, sp) in p.rows.into_iter().enumerate() {
            let id = sp.id.clone();
            let detail = [sp.code.clone(), sp.phone.clone(), sp.city.clone()]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let balance = sp.balance_minor;
            let mut tail = view().flex_row().items_center().gap(px(8.0));
            if sp.status == "blocked" {
                tail = tail.child(badge(Status::Danger, None, st.t("customers.blocked")));
            }
            tail = tail.child(
                view()
                    .w(px(130.0))
                    .items_end()
                    .child(label_line(
                        st.t("suppliers.owed"),
                        ty::XS,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    ))
                    .child(figure(
                        move || money(st, balance),
                        ty::SM,
                        FontWeight::BOLD,
                        move || if balance > 0 { Tone::Ink } else { Tone::Muted },
                    )),
            );
            v = v.child(
                row_view(
                    format!("supplier-{i}"),
                    Icon::Building,
                    sp.name.clone(),
                    detail,
                    tail,
                )
                .on_press(move || showing.set(Some(id.clone()))),
            );
        }
        v.into_element()
    });
    let summary = view()
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
                    "suppliers.count",
                    &[("count", &st.fmt().integer(i64::from(page_data.get().total)))],
                )
            },
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Body,
        ))
        .child(view().flex_1())
        .child(label_line(
            st.t("suppliers.owed_total"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(figure(
            move || money(st, page_data.get().owed_minor),
            ty::MD,
            FontWeight::BOLD,
            || Tone::Ink,
        ));
    let body = card()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(10.0))
                .p(px(12.0))
                .child(view().flex_1().min_w(px(200.0)).child(search_field(
                    text,
                    st.t("suppliers.search"),
                    "suppliers-search",
                )))
                .child(segmented(
                    vec![
                        (0, Box::new(st.t("customers.filter.all"))),
                        (1, Box::new(st.t("suppliers.filter.owed"))),
                    ],
                    owed,
                )),
        )
        .child(summary)
        .child(list)
        .child(pager(st, paging, move || page_data.get().total, compact));
    let importing = signal(false);
    let actions = view().child(when(
        move || st.can("suppliers.edit"),
        move || {
            view()
                .flex_row()
                .gap(px(8.0))
                .child(
                    button(
                        Variant::Secondary,
                        Size::Md,
                        Some(Icon::FileUp),
                        st.t("import.button"),
                        move || importing.set(true),
                    )
                    .test_id("suppliers-import"),
                )
                .child(
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Plus),
                        st.t("suppliers.new"),
                        move || editing.set(Some(None)),
                    )
                    .test_id("supplier-new"),
                )
                .into_element()
        },
    ));
    view()
        .child(page(
            st,
            Route::Suppliers,
            "suppliers.subtitle",
            actions,
            body,
            compact,
        ))
        .child(modal(showing_open, compact, move || {
            supplier_card(st, showing, editing, move || showing_open.set(false)).into_element()
        }))
        .child(wide_modal(importing, compact, move || {
            crate::screens::import::import_dialog(
                st,
                nb_domain::import::Kind::Suppliers,
                move || importing.set(false),
            )
            .into_element()
        }))
        .child(modal(editing_open, compact, move || {
            match editing.get_untracked() {
                Some(sp) => supplier_dialog(st, sp, move || editing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
}

fn supplier_dialog(
    st: AppState,
    sp: Option<SupplierRow>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let meta = session.meta("supplier.save");
    let get = |f: fn(&SupplierRow) -> String| signal(sp.as_ref().map(f).unwrap_or_default());
    let name = get(|c| c.name.clone());
    let phone = get(|c| c.phone.clone());
    let email = get(|c| c.email.clone());
    let city = get(|c| c.city.clone());
    let address = get(|c| c.address.clone());
    let nif = get(|c| c.nif.clone());
    let rc = get(|c| c.rc.clone());
    let days = signal(
        sp.as_ref()
            .filter(|c| c.payment_days > 0)
            .map(|c| c.payment_days.to_string())
            .unwrap_or_default(),
    );
    let company = signal(sp.as_ref().is_none_or(|c| c.kind == "company"));
    let error = signal(String::new());
    let base = sp.clone();
    let save = {
        let close = close.clone();
        move || {
            let payment_days = days.get_untracked().trim().parse::<i64>().unwrap_or(0);
            let input = SupplierInput {
                id: base.as_ref().map(|b| b.id.clone()),
                kind: if company.get_untracked() {
                    "company"
                } else {
                    "person"
                }
                .into(),
                name: name.get_untracked(),
                phone: phone.get_untracked(),
                email: email.get_untracked(),
                address: address.get_untracked(),
                city: city.get_untracked(),
                nif: nif.get_untracked(),
                rc: rc.get_untracked(),
                payment_days,
                status: base
                    .as_ref()
                    .map_or_else(|| "active".to_owned(), |b| b.status.clone()),
                note: base.as_ref().map(|b| b.note.clone()).unwrap_or_default(),
                ..Default::default()
            };
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| save_supplier(s, meta, input)).await {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "customers.saved", &[("name", &r.name)]),
                        );
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let two = |a: View, b: View| {
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(12.0))
            .child(view().flex_1().min_w(px(180.0)).child(a))
            .child(view().flex_1().min_w(px(180.0)).child(b))
    };
    let f = |caption: &'static str, sig: Signal<String>, kb: KeyboardType, id: &'static str| {
        field(st.t(caption), sig, String::new, String::new, kb, id)
    };
    use crate::ui::form::section;
    let body = view()
        .gap(px(14.0))
        .child(section(
            Icon::Truck,
            st.t("form.section.identity"),
            view()
                .child(segmented(
                    vec![
                        (false, Box::new(st.t("customers.kind.person"))),
                        (true, Box::new(st.t("customers.kind.company"))),
                    ],
                    company,
                ))
                .child(f(
                    "customers.name",
                    name,
                    KeyboardType::Default,
                    "supplier-name",
                )),
        ))
        .child(section(
            Icon::MapPin,
            st.t("form.section.contact"),
            view()
                .child(two(
                    f(
                        "customers.phone",
                        phone,
                        KeyboardType::Phone,
                        "supplier-phone",
                    ),
                    f(
                        "customers.email",
                        email,
                        KeyboardType::Email,
                        "supplier-email",
                    ),
                ))
                .child(f(
                    "customers.city",
                    city,
                    KeyboardType::Default,
                    "supplier-city",
                ))
                .child(f(
                    "customers.address",
                    address,
                    KeyboardType::Default,
                    "supplier-address",
                )),
        ))
        .child(section(
            Icon::Landmark,
            st.t("form.section.terms"),
            view()
                .child(two(
                    f("customers.nif", nif, KeyboardType::Number, "supplier-nif"),
                    f("customers.rc", rc, KeyboardType::Default, "supplier-rc"),
                ))
                .child(f(
                    "customers.days",
                    days,
                    KeyboardType::Number,
                    "supplier-days",
                )),
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
                st.t("form.save"),
                save,
            )
            .test_id("supplier-save"),
        );
    let title = sp.as_ref().map(|c| c.name.clone());
    dialog_frame(
        st,
        move || title.clone().unwrap_or_else(|| st.tn("suppliers.new")),
        st.t("suppliers.edit.help"),
        body,
        footer,
        close,
    )
}

/// A supplier at a glance: what is owed to it, its statement and its open
/// invoices; pay it, edit it.
fn supplier_card(
    st: AppState,
    showing: Signal<Option<String>>,
    editing: Signal<Option<Option<SupplierRow>>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let loaded = resource(
        move || showing.get().map(|id| (id, st.data_rev.get())),
        move |(id, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    Ok((
                        get_supplier(s, &id)?,
                        supplier_statement(s, &id)?,
                        open_purchases(s, &id)?,
                    ))
                })
                .await
            }
        },
    );
    type Card = (SupplierRow, Vec<SupplierStatementLine>, Vec<PurchaseRow>);
    let data = computed(move || loaded.with(|d: Option<&Card>| d.cloned()));
    let tab = signal(0usize);
    let paying = signal(false);
    let head = dynamic(move || {
        let Some((sp, _, open)) = data.get() else {
            return view().into_element();
        };
        let open_sum: i64 = open.iter().map(|p| p.open_minor).sum();
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(8.0))
            .child(amount_tile(
                st,
                "suppliers.owed",
                sp.balance_minor,
                Tone::Ink,
            ))
            .child(amount_tile(
                st,
                "suppliers.open_invoices",
                open_sum,
                Tone::Ink,
            ))
            .into_element()
    });
    let content = dynamic(move || {
        let Some((_, stmt, open)) = data.get() else {
            return view().into_element();
        };
        let mut v = view();
        if tab.get() == 0 {
            if stmt.is_empty() {
                return label(
                    st.t("customers.statement.empty"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )
                .into_element();
            }
            for l in stmt.into_iter().rev() {
                let what = {
                    let key = format!("acc.source.{}", l.source);
                    let t = st.tn(&key);
                    if t == key { l.source.clone() } else { t }
                };
                let detail = format!("{} · {}", l.date, l.memo);
                let (o, p, b) = (l.owed_minor, l.paid_minor, l.balance_minor);
                v = v.child(
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
                                    move || what.clone(),
                                    ty::SM,
                                    FontWeight::MEDIUM,
                                    Tone::Ink,
                                ))
                                .child(label_line(
                                    move || detail.clone(),
                                    ty::XS,
                                    FontWeight::NORMAL,
                                    Tone::Muted,
                                )),
                        )
                        .child(view().w(px(110.0)).items_end().child(figure(
                            move || {
                                if o > 0 {
                                    format!("+{}", money(st, o))
                                } else {
                                    format!("−{}", money(st, p))
                                }
                            },
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            move || if o > 0 { Tone::Ink } else { Tone::Success },
                        )))
                        .child(view().w(px(110.0)).items_end().child(figure(
                            move || money(st, b),
                            ty::SM,
                            FontWeight::NORMAL,
                            || Tone::Muted,
                        ))),
                );
            }
        } else {
            if open.is_empty() {
                return label(
                    st.t("customers.no_open"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )
                .into_element();
            }
            for p in open {
                let title = if p.supplier_ref.is_empty() {
                    p.number.clone()
                } else {
                    format!("{} · {}", p.number, p.supplier_ref)
                };
                let detail = date_time(st, p.created_at);
                let o = p.open_minor;
                v = v.child(
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
                                    move || title.clone(),
                                    ty::SM,
                                    FontWeight::MEDIUM,
                                    Tone::Ink,
                                ))
                                .child(label_line(
                                    move || detail.clone(),
                                    ty::XS,
                                    FontWeight::NORMAL,
                                    Tone::Muted,
                                )),
                        )
                        .child(figure(
                            move || money(st, o),
                            ty::SM,
                            FontWeight::BOLD,
                            || Tone::Ink,
                        )),
                );
            }
        }
        v.into_element()
    });
    let body = view()
        .gap(px(14.0))
        .child(head)
        .child(segmented(
            vec![
                (0, Box::new(st.t("customers.tab.statement"))),
                (1, Box::new(st.t("customers.tab.open"))),
            ],
            tab,
        ))
        .child(content);
    let footer = {
        let close = close.clone();
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(8.0))
            .child(when(
                move || st.can("suppliers.edit"),
                move || {
                    let close = close.clone();
                    button(
                        Variant::Secondary,
                        Size::Md,
                        Some(Icon::Pencil),
                        st.t("customers.edit"),
                        move || {
                            if let Some((sp, ..)) = data.get_untracked() {
                                close();
                                editing.set(Some(Some(sp)));
                            }
                        },
                    )
                    .into_element()
                },
            ))
            .child(when(
                move || st.can("suppliers.pay"),
                move || {
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Coins),
                        st.t("suppliers.pay"),
                        move || paying.set(true),
                    )
                    .test_id("supplier-pay")
                    .into_element()
                },
            ))
    };
    let title = move || data.get().map_or_else(String::new, |(sp, ..)| sp.name);
    view()
        .flex_shrink(1.0)
        .min_h(px(0.0))
        .child(dialog_frame(
            st,
            title,
            st.t("suppliers.card.help"),
            body,
            footer,
            close,
        ))
        .child(modal(paying, false, move || match data.get_untracked() {
            Some((sp, ..)) => pay_dialog(st, sp, move || paying.set(false)).into_element(),
            None => view().into_element(),
        }))
}

/// Pays a supplier, the oldest invoices first.
fn pay_dialog(st: AppState, sp: SupplierRow, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    let meta = session.meta("supplier.pay");
    let till = branch_till(st);
    let amount = signal(if sp.balance_minor > 0 {
        Money::from_minor(sp.balance_minor, cur).to_plain_string()
    } else {
        String::new()
    });
    // 0: the till, 1: the bank.
    let method = signal(1usize);
    let reference = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let id = sp.id.clone();
    let save = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Ok(a) = parse_price(&amount.get_untracked(), cur) else {
                error.set(error_text(st, "invalid_amount"));
                return;
            };
            let input = SupplierPaymentInput {
                supplier_id: id.clone(),
                amount_minor: a.minor(),
                method: if method.get_untracked() == 0 {
                    "cash"
                } else {
                    "bank"
                }
                .into(),
                register_id: till.get_untracked(),
                allocations: vec![],
                reference: reference.get_untracked(),
            };
            busy.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| pay_supplier(s, meta, input)).await {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "suppliers.paid",
                                &[("number", &r.number), ("amount", &money(st, a.minor()))],
                            ),
                        );
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
    let body = view()
        .gap(px(14.0))
        .child(field(
            st.t("suppliers.pay.amount"),
            amount,
            String::new,
            String::new,
            KeyboardType::Decimal,
            "sp-amount",
        ))
        .child(labelled(
            st.t("expenses.method"),
            segmented(
                vec![
                    (0, Box::new(st.t("expenses.method.cash"))),
                    (1, Box::new(st.t("expenses.method.bank"))),
                ],
                method,
            ),
            None,
        ))
        .child(field(
            st.t("expenses.reference"),
            reference,
            String::new,
            String::new,
            KeyboardType::Default,
            "sp-reference",
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
                st.t("suppliers.pay"),
                save,
            )
            .test_id("sp-confirm"),
        );
    let name = sp.name.clone();
    dialog_frame(
        st,
        move || tr_args(st.lang(), "suppliers.pay.title", &[("name", &name)]),
        st.t("suppliers.pay.help"),
        body,
        footer,
        close,
    )
}

// --------------------------------------------------------------- purchases

/// A purchase invoice as a row of text (to copy or export).
fn purchase_cells(st: AppState, r: &PurchaseRow) -> Vec<String> {
    let c = currency(st);
    vec![
        r.number.clone(),
        r.supplier.clone(),
        r.supplier_ref.clone(),
        date_time(st, r.created_at),
        nb_core::Money::from_minor(r.total_minor, c).to_plain_string(),
        nb_core::Money::from_minor(r.open_minor, c).to_plain_string(),
    ]
}

pub fn purchases_page(st: AppState, compact: bool) -> View {
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
            async move {
                db.call(move |s| list_purchases(s, &q, None, limit, offset))
                    .await
            }
        },
    );
    let page_data = computed(move || {
        data.with(|p: Option<&PurchasePage>| p.cloned())
            .unwrap_or_default()
    });
    let creating = signal(false);
    let (showing, showing_open) = opener::<PurchaseRow>();
    let list = dynamic(move || {
        let p = page_data.get();
        if p.rows.is_empty() {
            return empty_state(
                Icon::Truck,
                st.t("purchases.empty"),
                st.t("purchases.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, r) in p.rows.into_iter().enumerate() {
            let title = format!("{} · {}", r.number, r.supplier);
            let mut detail = date_time(st, r.created_at);
            if !r.supplier_ref.is_empty() {
                detail = format!("{detail} · {}", r.supplier_ref);
            }
            let (total, open) = (r.total_minor, r.open_minor);
            let mut tail = view().flex_row().items_center().gap(px(8.0));
            if open > 0 {
                tail = tail.child(badge(Status::Warning, None, move || {
                    tr_args(st.lang(), "purchases.open", &[("amount", &money(st, open))])
                }));
            }
            tail = tail.child(figure(
                move || money(st, total),
                ty::SM,
                FontWeight::BOLD,
                || Tone::Ink,
            ));
            let acts = {
                let r = r.clone();
                move || {
                    use crate::ui::menu::{Act, tsv};
                    let c = currency(st);
                    let open_it = r.clone();
                    vec![
                        Act::new(st.tn("sales.act.details"), move || {
                            showing.set(Some(open_it.clone()))
                        }),
                        Act::Line,
                        Act::copy(st.tn("sales.act.copy_number"), r.number.clone()),
                        Act::copy(st.tn("purchases.act.copy_ref"), r.supplier_ref.clone()),
                        Act::copy(
                            st.tn("sales.act.copy_total"),
                            nb_core::Money::from_minor(r.total_minor, c).to_plain_string(),
                        ),
                        Act::copy(st.tn("action.copy_row"), tsv(&[purchase_cells(st, &r)])),
                    ]
                }
            };
            tail = tail.child(crate::ui::menu::more_button(
                st,
                acts.clone(),
                &format!("purchase-more-{i}"),
            ));
            let row = r.clone();
            v = v.child(crate::ui::menu::with_menu(
                row_view(format!("purchase-{i}"), Icon::Truck, title, detail, tail)
                    .on_press(move || showing.set(Some(row.clone()))),
                acts,
            ));
        }
        v.into_element()
    });
    let body = card()
        .child(view().p(px(12.0)).child(search_field(
            text,
            st.t("purchases.search"),
            "purchases-search",
        )))
        .child(list)
        .child(pager(st, paging, move || page_data.get().total, compact));
    let export_all = move || {
        let q = text.get_untracked();
        let task = spawn(async move {
            let Ok(p) = st
                .db()
                .call(move |s| list_purchases(s, &q, None, 100_000, 0))
                .await
            else {
                return;
            };
            let mut rows = vec![vec![
                st.tn("sales.col.number"),
                st.tn("purchases.col.supplier"),
                st.tn("purchases.col.ref"),
                st.tn("sales.col.date"),
                st.tn("sales.col.total"),
                st.tn("purchases.col.open"),
            ]];
            rows.extend(p.rows.iter().map(|r| purchase_cells(st, r)));
            crate::screens::reports::export(st, format!("{}.csv", st.tn("purchases.file")), rows);
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let actions = view()
        .flex_row()
        .gap(px(8.0))
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Download),
                st.t("reports.export"),
                export_all,
            )
            .test_id("purchases-export"),
        )
        .child(when(
            move || st.can("purchases.receive"),
            move || {
                button(
                    Variant::Primary,
                    Size::Md,
                    Some(Icon::Plus),
                    st.t("purchases.new"),
                    move || creating.set(true),
                )
                .test_id("purchase-new")
                .into_element()
            },
        ));
    view()
        .child(page(
            st,
            Route::Purchases,
            "purchases.subtitle",
            actions,
            body,
            compact,
        ))
        .child(modal(creating, compact, move || {
            purchase_dialog(st, None, move || creating.set(false)).into_element()
        }))
        .child(modal(showing_open, compact, move || {
            match showing.get_untracked() {
                Some(p) => purchase_detail(st, p, move || showing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
}

/// What a line buys: a product, or an order line to receive.
#[derive(Clone, PartialEq)]
struct Item {
    product_id: String,
    name: String,
    /// Empty: the product's base unit.
    unit: String,
    tax_bps: i64,
    /// The order line it receives, and what is left of it.
    position: Option<u32>,
    left_milli: Option<i64>,
}

impl Item {
    fn of(p: ProductRow) -> Self {
        Item {
            product_id: p.id,
            name: p.name,
            unit: String::new(),
            tax_bps: p.tax_bps,
            position: None,
            left_milli: None,
        }
    }

    fn ordered(l: &OrderLine, receiving: bool) -> Self {
        Item {
            product_id: l.product_id.clone(),
            name: l.name.clone(),
            unit: l.unit.clone(),
            tax_bps: l.tax_bps,
            position: receiving.then_some(l.position),
            left_milli: receiving.then_some(l.remaining_milli()),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct BuyLine {
    item: Signal<Item>,
    qty: Signal<String>,
    cost: Signal<String>,
}

fn parse_qty(s: &str) -> Option<Quantity> {
    let q: Quantity = s.trim().replace(',', ".").parse().ok()?;
    (nb_store::repo::to_milli(q).ok()? > 0).then_some(q)
}

fn plain_qty(milli: i64) -> String {
    Quantity::from_milli(milli).to_string()
}

fn new_line(owner: Owner, item: Item, qty: String, cost: String) -> BuyLine {
    owner.run(|| BuyLine {
        item: signal(item),
        qty: signal(qty),
        cost: signal(cost),
    })
}

/// The lines before and after tax (tracked), when every line reads.
fn lines_totals(ls: &[BuyLine], cur: nb_core::Currency) -> Option<(i64, i64)> {
    if ls.is_empty() {
        return None;
    }
    let mut cl = Vec::new();
    for l in ls {
        cl.push(CartLine {
            qty: parse_qty(&l.qty.get())?,
            unit_price: parse_price(&l.cost.get(), cur).ok()?,
            min_price: None,
            tax_rate: Rate::from_bps(l.item.with(|i| i.tax_bps)),
            discount: Discount::None,
        });
    }
    let t = Cart {
        mode: TaxMode::Exclusive,
        lines: cl,
        discount: Discount::None,
    }
    .totals()
    .ok()?;
    Some((t.net.minor(), t.tax.minor()))
}

/// The lines as the store takes them, or the first that does not read.
fn lines_input(
    ls: &[BuyLine],
    cur: nb_core::Currency,
) -> Result<Vec<PurchaseLineInput>, &'static str> {
    let mut out = Vec::with_capacity(ls.len());
    for l in ls {
        let item = l.item.get_untracked();
        let qty = l.qty.get_untracked();
        if parse_qty(&qty).is_none() {
            return Err("invalid_quantity");
        }
        let Ok(c) = parse_price(&l.cost.get_untracked(), cur) else {
            return Err("invalid_amount");
        };
        out.push(PurchaseLineInput {
            product_id: item.product_id,
            qty: qty.trim().replace(',', "."),
            unit: item.unit,
            unit_cost_minor: c.minor(),
            tax_bps: None,
            order_position: item.position,
        });
    }
    Ok(out)
}

/// The lines being bought: quantity and cost before tax; removable.
fn lines_view(
    st: AppState,
    lines: Signal<Vec<BuyLine>>,
    cur: nb_core::Currency,
    prefix: &'static str,
) -> AnyElement {
    dynamic(move || {
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
        let mut v = view().gap(px(6.0));
        for (i, l) in ls.into_iter().enumerate() {
            let item = l.item.get_untracked();
            let name = item.name.clone();
            let mut info = view().flex_1().min_w(px(140.0)).child(label_line(
                move || name.clone(),
                ty::SM,
                FontWeight::MEDIUM,
                Tone::Ink,
            ));
            if let Some(left) = item.left_milli {
                info = info.child(label_line(
                    move || {
                        tr_args(
                            st.lang(),
                            "orders.left",
                            &[("qty", &st.fmt().quantity(Quantity::from_milli(left), 3))],
                        )
                    },
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ));
            }
            v = v.child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .py(px(4.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(info)
                    .child(view().w(px(80.0)).child(row_input(
                        l.qty,
                        String::new,
                        KeyboardType::Decimal,
                        format!("{prefix}-qty-{i}"),
                        move || parse_qty(&l.qty.get()).is_none(),
                    )))
                    .child(view().w(px(120.0)).child(row_input(
                        l.cost,
                        st.t("purchases.unit_cost"),
                        KeyboardType::Decimal,
                        format!("{prefix}-cost-{i}"),
                        move || parse_price(&l.cost.get(), cur).is_err(),
                    )))
                    .child(icon_button(
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
                    )),
            );
        }
        v.into_element()
    })
    .into_element()
}

fn totals_box(st: AppState, totals: Memo<Option<(i64, i64)>>) -> AnyElement {
    dynamic(move || {
        let Some((net, vat)) = totals.get() else {
            return view().into_element();
        };
        let row = |key: &'static str, v: i64, strong: bool| {
            view()
                .flex_row()
                .child(view().flex_1().child(label_line(
                    st.t(key),
                    ty::SM,
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
                    FontWeight::BOLD,
                    || Tone::Ink,
                ))
        };
        view()
            .gap(px(4.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .bg(Tone::Subtle.color())
            .child(row("pos.net", net, false))
            .child(row("pos.tax", vat, false))
            .child(row("pos.total", net + vat, true))
            .into_element()
    })
    .into_element()
}

fn two(a: View, b: View) -> View {
    view()
        .flex_row()
        .flex_wrap()
        .gap(px(12.0))
        .child(view().flex_1().min_w(px(170.0)).child(a))
        .child(view().flex_1().min_w(px(170.0)).child(b))
}

fn supplier_picker(
    st: AppState,
    sups: Memo<Vec<SupplierRow>>,
    at: Signal<usize>,
    id: &'static str,
) -> View {
    labelled(
        st.t("purchases.supplier"),
        dropdown(
            move || {
                sups.get()
                    .into_iter()
                    .enumerate()
                    .map(|(i, s)| (i, s.name))
                    .collect()
            },
            at,
            Some(Icon::Building),
            st.t("purchases.supplier"),
            id,
        ),
        None,
    )
}

fn warehouse_picker(
    st: AppState,
    whs: Memo<Vec<Warehouse>>,
    at: Signal<usize>,
    id: &'static str,
) -> View {
    labelled(
        st.t("doc.warehouse"),
        dropdown(
            move || {
                whs.get()
                    .into_iter()
                    .enumerate()
                    .map(|(i, w)| (i, w.name))
                    .collect()
            },
            at,
            Some(Icon::Warehouse),
            st.t("doc.warehouse"),
            id,
        ),
        None,
    )
}

/// Asks a supervisor for `ask`, then runs `retry` (under the dialog's
/// owner: the approval dialog closes first).
fn approval_modal(
    st: AppState,
    asking: Signal<Option<Ask>>,
    asking_open: Signal<bool>,
    approval: Signal<Option<String>>,
    retry: impl Fn() + Clone + 'static,
) -> impl IntoElement {
    let owner = Owner::current();
    modal(asking_open, false, move || match asking.get_untracked() {
        Some(ask) => {
            let retry = retry.clone();
            approval_dialog(
                st,
                ask,
                move |id| {
                    approval.set(Some(id));
                    owner.run(&retry);
                },
                move || asking_open.set(false),
            )
            .into_element()
        }
        None => view().into_element(),
    })
}

/// A supplier's invoice: the goods and their prices before tax, the costs
/// of bringing them in, what is paid now. Against an order, its lines are
/// what is left to receive, at the cost ordered.
fn purchase_dialog(st: AppState, from: Option<Order>, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    let owner = Owner::current();
    let sups = suppliers_memo(st);
    let whs = warehouses(st);
    let till = branch_till(st);
    let supplier = signal(0usize);
    let wh = signal(0usize);
    let reference = signal(String::new());
    let lines: Signal<Vec<BuyLine>> = signal(
        from.as_ref()
            .map(|o| {
                o.lines
                    .iter()
                    .filter(|l| l.remaining_milli() > 0)
                    .map(|l| {
                        new_line(
                            owner,
                            Item::ordered(l, true),
                            plain_qty(l.remaining_milli()),
                            Money::from_minor(l.unit_cost_minor, cur).to_plain_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    );
    let charges = signal(String::new());
    let charges_vat = signal(String::new());
    let by_qty = signal(0usize);
    let paid = signal(String::new());
    // 0: the till, 1: the bank.
    let method = signal(1usize);
    let error = signal(String::new());
    let busy = signal(false);
    let approval: Signal<Option<String>> = signal(None);
    let (asking, asking_open) = opener::<Ask>();
    let pick = move |p: ProductRow| {
        let l = new_line(owner, Item::of(p), "1".into(), String::new());
        lines.update(|v| v.push(l));
    };
    let parse_opt = move |s: &str| -> Option<i64> {
        if s.trim().is_empty() {
            Some(0)
        } else {
            parse_price(s, cur).ok().map(|m| m.minor())
        }
    };
    // What the invoice comes to, as the store will compute it.
    let totals = computed(move || {
        let (net, vat) = lines_totals(&lines.get(), cur)?;
        let c = parse_opt(&charges.get())?;
        let cv = parse_opt(&charges_vat.get())?;
        Some((net + c, vat + cv))
    });
    let order = from.clone();
    let save = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let (sup, warehouse, branch) = match &order {
                Some(o) => (
                    o.row.supplier_id.clone(),
                    o.warehouse_id.clone(),
                    Some(o.branch_id.clone()),
                ),
                None => {
                    let Some(sup) = sups
                        .with_untracked(|v| v.get(supplier.get_untracked()).map(|s| s.id.clone()))
                    else {
                        error.set(error_text(st, "supplier.unknown"));
                        return;
                    };
                    let Some(w) = whs.with_untracked(|v| v.get(wh.get_untracked()).cloned()) else {
                        return;
                    };
                    (sup, w.id, Some(w.branch_id))
                }
            };
            let input_lines = match lines_input(&lines.get_untracked(), cur) {
                Ok(l) => l,
                Err(code) => {
                    error.set(error_text(st, code));
                    return;
                }
            };
            let (Some(c), Some(cv), Some(now)) = (
                parse_opt(&charges.get_untracked()),
                parse_opt(&charges_vat.get_untracked()),
                parse_opt(&paid.get_untracked()),
            ) else {
                error.set(error_text(st, "invalid_amount"));
                return;
            };
            let cash = method.get_untracked() == 0;
            let input = PurchaseInput {
                supplier_id: sup,
                warehouse_id: warehouse,
                supplier_ref: reference.get_untracked(),
                lines: input_lines,
                charges_minor: c,
                charges_vat_minor: cv,
                charges_by: if by_qty.get_untracked() == 1 {
                    "qty"
                } else {
                    "value"
                }
                .into(),
                cash_minor: if cash { now } else { 0 },
                bank_minor: if cash { 0 } else { now },
                register_id: till.get_untracked(),
                note: String::new(),
                order_id: order.as_ref().map(|o| o.row.id.clone()),
                approval_id: approval.get_untracked(),
            };
            // A new operation per attempt (one may need a supervisor first).
            let meta = session.meta("purchase.record");
            busy.set(true);
            error.set(String::new());
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| record_purchase(s, meta, input)).await {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "purchases.recorded",
                                &[("number", &r.number), ("amount", &money(st, r.total_minor))],
                            ),
                        );
                    }
                    Err(e) => {
                        busy.set(false);
                        if let Some(p) = needs_approval(&e) {
                            asking.set(Some(Ask {
                                permission: p,
                                branch_id: branch.clone(),
                                register_id: None,
                            }));
                        }
                        error.set(first_error(st, &e));
                    }
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let retry = save.clone();
    let mut body = view().gap(px(14.0));
    body = match &from {
        Some(o) => {
            let text = format!("{} · {}", o.row.number, o.row.supplier);
            body.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .p(px(10.0))
                    .rounded(px(8.0))
                    .bg(Tone::Subtle.color())
                    .child(icon(Icon::ClipboardList, 18.0, || Tone::Faint))
                    .child(label_line(
                        move || text.clone(),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        Tone::Ink,
                    )),
            )
        }
        None => body.child(two(
            supplier_picker(st, sups, supplier, "buy-supplier"),
            warehouse_picker(st, whs, wh, "buy-warehouse"),
        )),
    };
    body = body.child(field(
        st.t("purchases.supplier_ref"),
        reference,
        String::new,
        String::new,
        KeyboardType::Default,
        "buy-ref",
    ));
    if from.is_none() {
        body = body.child(labelled(
            st.t("doc.add_item"),
            product_picker(st, "buy-search", pick),
            Some(Box::new(st.t("purchases.prices_help"))),
        ));
    }
    let body = body
        .child(lines_view(st, lines, cur, "buy"))
        .child(two(
            field(
                st.t("purchases.charges"),
                charges,
                || "0".to_owned(),
                String::new,
                KeyboardType::Decimal,
                "buy-charges",
            ),
            field(
                st.t("purchases.charges_vat"),
                charges_vat,
                || "0".to_owned(),
                String::new,
                KeyboardType::Decimal,
                "buy-charges-vat",
            ),
        ))
        .child(labelled(
            st.t("purchases.charges_by"),
            segmented(
                vec![
                    (0, Box::new(st.t("purchases.charges_by.value"))),
                    (1, Box::new(st.t("purchases.charges_by.qty"))),
                ],
                by_qty,
            ),
            None,
        ))
        .child(totals_box(st, totals))
        .child(two(
            field(
                st.t("purchases.paid_now"),
                paid,
                || "0".to_owned(),
                String::new,
                KeyboardType::Decimal,
                "buy-paid",
            ),
            labelled(
                st.t("expenses.method"),
                segmented(
                    vec![
                        (0, Box::new(st.t("expenses.method.cash"))),
                        (1, Box::new(st.t("expenses.method.bank"))),
                    ],
                    method,
                ),
                None,
            ),
        ))
        .child(label(
            st.t("purchases.rest_owed"),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
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
                st.t("purchases.save"),
                save,
            )
            .test_id("buy-save"),
        );
    let receiving = from.is_some();
    view()
        .flex_shrink(1.0)
        .min_h(px(0.0))
        .child(dialog_frame(
            st,
            move || {
                st.tn(if receiving {
                    "orders.receive"
                } else {
                    "purchases.new"
                })
            },
            st.t(if receiving {
                "orders.receive.help"
            } else {
                "purchases.new.help"
            }),
            body,
            footer,
            close,
        ))
        .child(approval_modal(st, asking, asking_open, approval, retry))
}

// ------------------------------------------------------------ purchase orders

fn status_badge(st: AppState, status: &str) -> View {
    let tone = match status {
        "draft" => Status::Neutral,
        "approved" => Status::Info,
        "partial" => Status::Warning,
        "received" => Status::Success,
        _ => Status::Neutral,
    };
    let key = format!("orders.status.{status}");
    badge(tone, None, move || st.tn(&key))
}

pub fn orders_page(st: AppState, compact: bool) -> View {
    let text = signal(String::new());
    // 0: open, 1: drafts, 2: everything.
    let which = signal(0usize);
    let paging = Paging::new(25);
    effect(move || {
        text.with(|_| ());
        which.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            Some((
                text.get(),
                which.get(),
                paging.size.get(),
                paging.offset(),
                st.data_rev.get(),
            ))
        },
        move |(q, w, limit, offset, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    list_orders(
                        s,
                        &OrderQuery {
                            status: ["open", "draft", ""][w.min(2)].into(),
                            search: q,
                            limit,
                            offset,
                            ..Default::default()
                        },
                    )
                })
                .await
            }
        },
    );
    let page_data = computed(move || {
        data.with(|p: Option<&OrderPage>| p.cloned())
            .unwrap_or_default()
    });
    let (showing, showing_open) = opener::<String>();
    let (editing, editing_open) = opener::<Option<Order>>();
    let (receiving, receiving_open) = opener::<Order>();
    let list = dynamic(move || {
        let p = page_data.get();
        if p.rows.is_empty() {
            return empty_state(
                Icon::ClipboardList,
                st.t("orders.empty"),
                st.t("orders.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, r) in p.rows.into_iter().enumerate() {
            let title = format!("{} · {}", r.number, r.supplier);
            let mut detail = date_time(st, r.created_at);
            if let Some(d) = &r.expected_date {
                detail = format!(
                    "{detail} · {}",
                    tr_args(st.lang(), "orders.expected_on", &[("date", d)])
                );
            }
            let total = r.total_minor;
            let tail = view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(status_badge(st, &r.status))
                .child(figure(
                    move || money(st, total),
                    ty::SM,
                    FontWeight::BOLD,
                    || Tone::Ink,
                ));
            let id = r.id.clone();
            v = v.child(
                row_view(
                    format!("order-{i}"),
                    Icon::ClipboardList,
                    title,
                    detail,
                    tail,
                )
                .on_press(move || showing.set(Some(id.clone()))),
            );
        }
        v.into_element()
    });
    let body = card()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(10.0))
                .p(px(12.0))
                .child(view().flex_1().min_w(px(200.0)).child(search_field(
                    text,
                    st.t("orders.search"),
                    "orders-search",
                )))
                .child(segmented(
                    vec![
                        (0, Box::new(st.t("orders.filter.open"))),
                        (1, Box::new(st.t("orders.filter.drafts"))),
                        (2, Box::new(st.t("customers.filter.all"))),
                    ],
                    which,
                )),
        )
        .child(list)
        .child(pager(st, paging, move || page_data.get().total, compact));
    let actions = view().child(when(
        move || st.can("purchases.order"),
        move || {
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("orders.new"),
                move || editing.set(Some(None)),
            )
            .test_id("order-new")
            .into_element()
        },
    ));
    view()
        .child(page(
            st,
            Route::PurchaseOrders,
            "orders.subtitle",
            actions,
            body,
            compact,
        ))
        .child(modal(showing_open, compact, move || {
            order_detail(st, showing, editing, receiving, move || {
                showing_open.set(false)
            })
            .into_element()
        }))
        .child(modal(editing_open, compact, move || {
            match editing.get_untracked() {
                Some(o) => order_dialog(st, o, move || editing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
        .child(modal(receiving_open, compact, move || {
            match receiving.get_untracked() {
                Some(o) => {
                    purchase_dialog(st, Some(o), move || receiving_open.set(false)).into_element()
                }
                None => view().into_element(),
            }
        }))
}

/// A new order, or a draft changed: supplier, warehouse, date expected,
/// lines at their cost before tax.
fn order_dialog(st: AppState, base: Option<Order>, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    let meta = session.meta("purchase_order.save");
    let owner = Owner::current();
    let sups = suppliers_memo(st);
    let whs = warehouses(st);
    let supplier = signal(0usize);
    let wh = signal(0usize);
    // A draft's supplier and warehouse, once the lists are there.
    if let Some(o) = &base {
        let (sid, wid) = (o.row.supplier_id.clone(), o.warehouse_id.clone());
        effect(move || {
            if let Some(i) = sups.with(|v| v.iter().position(|s| s.id == sid)) {
                supplier.set(i);
            }
            if let Some(i) = whs.with(|v| v.iter().position(|w| w.id == wid)) {
                wh.set(i);
            }
        });
    }
    let expected = signal(
        base.as_ref()
            .and_then(|o| o.row.expected_date.clone())
            .unwrap_or_default(),
    );
    let note = signal(base.as_ref().map(|o| o.note.clone()).unwrap_or_default());
    let lines: Signal<Vec<BuyLine>> = signal(
        base.as_ref()
            .map(|o| {
                o.lines
                    .iter()
                    .map(|l| {
                        new_line(
                            owner,
                            Item::ordered(l, false),
                            plain_qty(l.qty_milli),
                            Money::from_minor(l.unit_cost_minor, cur).to_plain_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    );
    let error = signal(String::new());
    let busy = signal(false);
    let pick = move |p: ProductRow| {
        let l = new_line(owner, Item::of(p), "1".into(), String::new());
        lines.update(|v| v.push(l));
    };
    let totals = computed(move || lines_totals(&lines.get(), cur));
    let id = base.as_ref().map(|o| o.row.id.clone());
    let save = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Some(sup) =
                sups.with_untracked(|v| v.get(supplier.get_untracked()).map(|s| s.id.clone()))
            else {
                error.set(error_text(st, "supplier.unknown"));
                return;
            };
            let Some(warehouse) =
                whs.with_untracked(|v| v.get(wh.get_untracked()).map(|w| w.id.clone()))
            else {
                return;
            };
            let input_lines = match lines_input(&lines.get_untracked(), cur) {
                Ok(l) => l,
                Err(code) => {
                    error.set(error_text(st, code));
                    return;
                }
            };
            let date = expected.get_untracked().trim().to_owned();
            let input = OrderInput {
                id: id.clone(),
                supplier_id: sup,
                warehouse_id: warehouse,
                expected_date: (!date.is_empty()).then_some(date),
                lines: input_lines,
                note: note.get_untracked(),
            };
            busy.set(true);
            error.set(String::new());
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| save_order(s, meta, input)).await {
                    Ok(o) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "orders.saved", &[("number", &o.row.number)]),
                        );
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
    let body = view()
        .gap(px(14.0))
        .child(two(
            supplier_picker(st, sups, supplier, "order-supplier"),
            warehouse_picker(st, whs, wh, "order-warehouse"),
        ))
        .child(two(
            field(
                st.t("orders.expected"),
                expected,
                || "2026-12-31".to_owned(),
                String::new,
                KeyboardType::Default,
                "order-expected",
            ),
            field(
                st.t("orders.note"),
                note,
                String::new,
                String::new,
                KeyboardType::Default,
                "order-note",
            ),
        ))
        .child(labelled(
            st.t("doc.add_item"),
            product_picker(st, "order-search", pick),
            Some(Box::new(st.t("purchases.prices_help"))),
        ))
        .child(lines_view(st, lines, cur, "order"))
        .child(totals_box(st, totals))
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
                st.t("orders.save"),
                save,
            )
            .test_id("order-save"),
        );
    let title = base.as_ref().map(|o| o.row.number.clone());
    dialog_frame(
        st,
        move || title.clone().unwrap_or_else(|| st.tn("orders.new")),
        st.t("orders.edit.help"),
        body,
        footer,
        close,
    )
}

/// An order: its lines, what came, what is left; approve it, receive
/// against it, cancel or close it.
fn order_detail(
    st: AppState,
    showing: Signal<Option<String>>,
    editing: Signal<Option<Option<Order>>>,
    receiving: Signal<Option<Order>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let loaded = resource(
        move || showing.get().map(|id| (id, st.data_rev.get())),
        move |(id, _), _| {
            let db = st.db();
            async move { db.call(move |s| get_order(s, &id)).await }
        },
    );
    let data = computed(move || loaded.with(|d: Option<&Order>| d.cloned()));
    let reason = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let approval: Signal<Option<String>> = signal(None);
    let (asking, asking_open) = opener::<Ask>();
    // What the reason field is for: cancelling or closing.
    let ending: Signal<Option<&'static str>> = signal(None);
    let act = move |what: &'static str| {
        if busy.get_untracked() {
            return;
        }
        let Some(o) = data.get_untracked() else {
            return;
        };
        let id = o.row.id.clone();
        let why = reason.get_untracked();
        let ok = approval.get_untracked();
        // Each press is its own operation: approve, then cancel, differ.
        let meta = session.meta(match what {
            "approve" => "purchase_order.approve",
            "cancel" => "purchase_order.cancel",
            _ => "purchase_order.close",
        });
        busy.set(true);
        error.set(String::new());
        let branch = o.branch_id.clone();
        let db = st.db();
        let task = spawn(async move {
            let r = db
                .call(move |s| match what {
                    "approve" => approve_order(s, meta, &id, ok.as_deref()),
                    "cancel" => cancel_order(s, meta, &id, &why),
                    _ => close_order(s, meta, &id, &why),
                })
                .await;
            busy.set(false);
            match r {
                Ok(o) => {
                    ending.set(None);
                    reason.set(String::new());
                    st.changed();
                    let key = format!("orders.done.{}", o.row.status);
                    show_toast(st, tr_args(st.lang(), &key, &[("number", &o.row.number)]));
                }
                Err(e) => {
                    if let Some(p) = needs_approval(&e) {
                        asking.set(Some(Ask {
                            permission: p,
                            branch_id: Some(branch),
                            register_id: None,
                        }));
                    }
                    error.set(first_error(st, &e));
                }
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let head = dynamic(move || {
        let Some(o) = data.get() else {
            return view().into_element();
        };
        let mut chips = view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(status_badge(st, &o.row.status));
        if let Some(d) = o.row.expected_date.clone() {
            chips = chips.child(label_line(
                move || tr_args(st.lang(), "orders.expected_on", &[("date", &d)]),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        if !o.receipts.is_empty() {
            let r = o.receipts.join(", ");
            chips = chips.child(label_line(
                move || tr_args(st.lang(), "orders.receipts", &[("numbers", &r)]),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        if !o.reason.is_empty() {
            let r = o.reason.clone();
            chips = chips.child(label_line(
                move || r.clone(),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
        }
        view()
            .gap(px(8.0))
            .child(chips)
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(amount_tile(st, "pos.net", o.net_minor, Tone::Ink))
                    .child(amount_tile(st, "pos.total", o.row.total_minor, Tone::Ink)),
            )
            .into_element()
    });
    let rows = dynamic(move || {
        let Some(o) = data.get() else {
            return view().into_element();
        };
        let mut v = view();
        for l in o.lines {
            let name = l.name.clone();
            let detail = tr_args(
                st.lang(),
                "orders.line",
                &[
                    (
                        "ordered",
                        &st.fmt().quantity(Quantity::from_milli(l.qty_milli), 3),
                    ),
                    (
                        "received",
                        &st.fmt().quantity(Quantity::from_milli(l.received_milli), 3),
                    ),
                    ("cost", &money(st, l.unit_cost_minor)),
                ],
            );
            let done = l.remaining_milli() == 0;
            let net = l.net_minor;
            v = v.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(icon(
                        if done { Icon::Check } else { Icon::Package },
                        16.0,
                        move || if done { Tone::Success } else { Tone::Faint },
                    ))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(label_line(
                                move || name.clone(),
                                ty::SM,
                                FontWeight::MEDIUM,
                                Tone::Ink,
                            ))
                            .child(label_line(
                                move || detail.clone(),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(figure(
                        move || money(st, net),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        || Tone::Ink,
                    )),
            );
        }
        v.into_element()
    });
    let reason_box = when(
        move || ending.get().is_some(),
        move || {
            field(
                st.t("orders.reason"),
                reason,
                String::new,
                String::new,
                KeyboardType::Default,
                "order-reason",
            )
            .into_element()
        },
    );
    let body = view()
        .gap(px(14.0))
        .child(head)
        .child(rows)
        .child(reason_box)
        .child(error_line(error));
    let status = move || data.get().map(|o| o.row.status).unwrap_or_default();
    let footer = {
        let close = close.clone();
        let close2 = close.clone();
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(8.0))
            .child(when(
                move || status() == "draft" && st.can("purchases.order"),
                move || {
                    let close = close.clone();
                    button(
                        Variant::Secondary,
                        Size::Md,
                        Some(Icon::Pencil),
                        st.t("orders.edit"),
                        move || {
                            if let Some(o) = data.get_untracked() {
                                close();
                                editing.set(Some(Some(o)));
                            }
                        },
                    )
                    .test_id("order-edit")
                    .into_element()
                },
            ))
            .child(when(
                move || {
                    matches!(status().as_str(), "draft" | "approved" | "partial")
                        && st.can("purchases.order")
                },
                move || {
                    button(
                        Variant::Ghost,
                        Size::Md,
                        Some(Icon::XCircle),
                        move || {
                            st.tn(match (status().as_str(), ending.get()) {
                                ("partial", None) => "orders.close",
                                (_, None) => "orders.cancel",
                                _ => "orders.confirm_end",
                            })
                        },
                        move || match ending.get_untracked() {
                            None => ending.set(Some(if status() == "partial" {
                                "close"
                            } else {
                                "cancel"
                            })),
                            Some(what) => act(what),
                        },
                    )
                    .test_id("order-end")
                    .into_element()
                },
            ))
            .child(when(
                move || status() == "draft" && st.can("purchases.order"),
                move || {
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Check),
                        st.t("orders.approve"),
                        move || act("approve"),
                    )
                    .test_id("order-approve")
                    .into_element()
                },
            ))
            .child(when(
                move || {
                    matches!(status().as_str(), "approved" | "partial")
                        && st.can("purchases.receive")
                },
                move || {
                    let close = close2.clone();
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Truck),
                        st.t("orders.receive"),
                        move || {
                            if let Some(o) = data.get_untracked() {
                                close();
                                receiving.set(Some(o));
                            }
                        },
                    )
                    .test_id("order-receive")
                    .into_element()
                },
            ))
    };
    let retry = move || act("approve");
    let title = move || {
        data.get()
            .map(|o| format!("{} · {}", o.row.number, o.row.supplier))
            .unwrap_or_default()
    };
    view()
        .flex_shrink(1.0)
        .min_h(px(0.0))
        .child(dialog_frame(
            st,
            title,
            st.t("orders.detail.help"),
            body,
            footer,
            close,
        ))
        .child(approval_modal(st, asking, asking_open, approval, retry))
}

/// A purchase's lines, and goods sent back to the supplier.
fn purchase_detail(st: AppState, p: PurchaseRow, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    let meta = session.meta("purchase.return");
    let owner = Owner::current();
    let till = branch_till(st);
    let id = p.id.clone();
    let loaded = resource(move || Some(st.data_rev.get()), {
        let id = id.clone();
        move |_, _| {
            let id = id.clone();
            let db = st.db();
            async move { db.call(move |s| purchase_lines(s, &id)).await }
        }
    });
    let bought = computed(move || {
        loaded
            .with(|v: Option<&Vec<BoughtLine>>| v.cloned())
            .unwrap_or_default()
    });
    let qtys: Signal<Vec<Signal<String>>> = signal(Vec::new());
    effect(move || {
        let n = bought.with(Vec::len);
        if qtys.with_untracked(Vec::len) != n {
            let v = owner.run(|| (0..n).map(|_| signal(String::new())).collect::<Vec<_>>());
            qtys.set(v);
        }
    });
    // 0: off what is owed, 1: cash into the till, 2: bank.
    let refund = signal(0usize);
    let reason = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let save = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let lines: Vec<(u32, String)> = bought
                .get_untracked()
                .iter()
                .zip(qtys.get_untracked())
                .filter_map(|(b, q)| {
                    let t = q.get_untracked().trim().replace(',', ".");
                    (!t.is_empty()).then_some((b.position, t))
                })
                .collect();
            let input = PurchaseReturnInput {
                purchase_id: id.clone(),
                lines,
                refund: ["credit", "cash", "bank"][refund.get_untracked()].into(),
                register_id: till.get_untracked(),
                reason: reason.get_untracked(),
            };
            busy.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| return_purchase(s, meta, input)).await {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "purchases.returned",
                                &[("number", &r.number), ("amount", &money(st, r.total_minor))],
                            ),
                        );
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
        let bs = bought.get();
        let qs = qtys.get();
        let mut v = view();
        for (i, (b, q)) in bs.into_iter().zip(qs).enumerate() {
            let left = b.qty_milli - b.returned_milli;
            let detail = format!(
                "{} {} × {}",
                st.fmt().quantity(Quantity::from_milli(b.qty_milli), 3),
                b.unit,
                money(st, b.unit_cost_minor)
            );
            let name = b.name.clone();
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
                            move || name.clone(),
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::Ink,
                        ))
                        .child(label_line(
                            move || detail.clone(),
                            ty::XS,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                );
            if left > 0 {
                row = row.child(view().w(px(90.0)).child(row_input(
                    q,
                    || "0".to_owned(),
                    KeyboardType::Decimal,
                    format!("pr-qty-{i}"),
                    || false,
                )));
            } else {
                row = row.child(badge(Status::Neutral, None, st.t("pos.return.done")));
            }
            v = v.child(row);
        }
        v.into_element()
    });
    let body = view()
        .gap(px(14.0))
        .child(rows)
        .child(labelled(
            st.t("purchases.refund"),
            segmented(
                vec![
                    (0, Box::new(st.t("purchases.refund.credit"))),
                    (1, Box::new(st.t("expenses.method.cash"))),
                    (2, Box::new(st.t("expenses.method.bank"))),
                ],
                refund,
            ),
            None,
        ))
        .child(field(
            st.t("pos.return.reason"),
            reason,
            String::new,
            String::new,
            KeyboardType::Default,
            "pr-reason",
        ))
        .child(error_line(error));
    let footer = view()
        .child(button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("action.close"),
            close.clone(),
        ))
        .child(when(
            move || st.can("purchases.receive"),
            move || {
                button(
                    Variant::Primary,
                    Size::Md,
                    Some(Icon::Undo),
                    st.t("purchases.return"),
                    save.clone(),
                )
                .test_id("pr-confirm")
                .into_element()
            },
        ));
    let title = format!("{} · {}", p.number, p.supplier);
    let _ = cur;
    dialog_frame(
        st,
        move || title.clone(),
        st.t("purchases.detail.help"),
        body,
        footer,
        close,
    )
}

//! Buying (M9.10, M10): suppliers and what is owed to them, purchase
//! invoices that bring goods in at their landed cost, returns to the
//! supplier and payments. The pages show and collect; the use cases decide.

use nb_core::{Money, Quantity, Rate};
use nb_domain::sale::{Cart, CartLine, Discount};
use nb_domain::tax::TaxMode;
use nb_i18n::tr_args;
use nb_store::repo::catalog::ProductRow;
use nb_store::repo::org::Warehouse;
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
use crate::ui::overlay::{dialog_frame, modal, show_toast};
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
    let actions = view().child(when(
        move || st.can("suppliers.edit"),
        move || {
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("suppliers.new"),
                move || editing.set(Some(None)),
            )
            .test_id("supplier-new")
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
    let body = view()
        .gap(px(14.0))
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
        ))
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
        .child(two(
            f(
                "customers.city",
                city,
                KeyboardType::Default,
                "supplier-city",
            ),
            f(
                "customers.days",
                days,
                KeyboardType::Number,
                "supplier-days",
            ),
        ))
        .child(f(
            "customers.address",
            address,
            KeyboardType::Default,
            "supplier-address",
        ))
        .child(two(
            f("customers.nif", nif, KeyboardType::Number, "supplier-nif"),
            f("customers.rc", rc, KeyboardType::Default, "supplier-rc"),
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
            let row = r.clone();
            v = v.child(
                row_view(format!("purchase-{i}"), Icon::Truck, title, detail, tail)
                    .on_press(move || showing.set(Some(row.clone()))),
            );
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
    let actions = view().child(when(
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
            purchase_dialog(st, move || creating.set(false)).into_element()
        }))
        .child(modal(showing_open, compact, move || {
            match showing.get_untracked() {
                Some(p) => purchase_detail(st, p, move || showing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
}

#[derive(Clone, Copy, PartialEq)]
struct BuyLine {
    product: Signal<Option<ProductRow>>,
    qty: Signal<String>,
    cost: Signal<String>,
}

fn parse_qty(s: &str) -> Option<Quantity> {
    let q: Quantity = s.trim().replace(',', ".").parse().ok()?;
    (nb_store::repo::to_milli(q).ok()? > 0).then_some(q)
}

/// A supplier's invoice: the goods and their prices before tax, the costs
/// of bringing them in, what is paid now.
fn purchase_dialog(st: AppState, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    let meta = session.meta("purchase.record");
    let owner = Owner::current();
    let sups = suppliers_memo(st);
    let whs = warehouses(st);
    let till = branch_till(st);
    let supplier = signal(0usize);
    let wh = signal(0usize);
    let reference = signal(String::new());
    let lines: Signal<Vec<BuyLine>> = signal(Vec::new());
    let charges = signal(String::new());
    let charges_vat = signal(String::new());
    let by_qty = signal(0usize);
    let paid = signal(String::new());
    // 0: the till, 1: the bank.
    let method = signal(1usize);
    let error = signal(String::new());
    let busy = signal(false);
    let pick = move |p: ProductRow| {
        let l = owner.run(|| BuyLine {
            product: signal(Some(p)),
            qty: signal("1".into()),
            cost: signal(String::new()),
        });
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
        let ls = lines.get();
        if ls.is_empty() {
            return None;
        }
        let mut cl = Vec::new();
        for l in &ls {
            let p = l.product.get()?;
            cl.push(CartLine {
                qty: parse_qty(&l.qty.get())?,
                unit_price: parse_price(&l.cost.get(), cur).ok()?,
                min_price: None,
                tax_rate: Rate::from_bps(p.tax_bps),
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
        let c = parse_opt(&charges.get())?;
        let cv = parse_opt(&charges_vat.get())?;
        Some((t.net.minor() + c, t.tax.minor() + cv))
    });
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
            let mut input_lines = Vec::new();
            for l in lines.get_untracked() {
                let Some(p) = l.product.get_untracked() else {
                    continue;
                };
                let Ok(c) = parse_price(&l.cost.get_untracked(), cur) else {
                    error.set(error_text(st, "invalid_amount"));
                    return;
                };
                input_lines.push(PurchaseLineInput {
                    product_id: p.id,
                    qty: l.qty.get_untracked().trim().replace(',', "."),
                    unit: String::new(),
                    unit_cost_minor: c.minor(),
                    tax_bps: None,
                });
            }
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
            };
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
        let mut v = view().gap(px(6.0));
        for (i, l) in ls.into_iter().enumerate() {
            let name = l
                .product
                .get_untracked()
                .map(|p| p.name)
                .unwrap_or_default();
            v = v.child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .py(px(4.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(view().flex_1().min_w(px(140.0)).child(label_line(
                        move || name.clone(),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Ink,
                    )))
                    .child(view().w(px(80.0)).child(row_input(
                        l.qty,
                        String::new,
                        KeyboardType::Decimal,
                        format!("buy-qty-{i}"),
                        move || parse_qty(&l.qty.get()).is_none(),
                    )))
                    .child(view().w(px(120.0)).child(row_input(
                        l.cost,
                        st.t("purchases.unit_cost"),
                        KeyboardType::Decimal,
                        format!("buy-cost-{i}"),
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
    });
    let two = |a: View, b: View| {
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(12.0))
            .child(view().flex_1().min_w(px(170.0)).child(a))
            .child(view().flex_1().min_w(px(170.0)).child(b))
    };
    let body = view()
        .gap(px(14.0))
        .child(two(
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
                    supplier,
                    Some(Icon::Building),
                    st.t("purchases.supplier"),
                    "buy-supplier",
                ),
                None,
            ),
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
                    wh,
                    Some(Icon::Warehouse),
                    st.t("doc.warehouse"),
                    "buy-warehouse",
                ),
                None,
            ),
        ))
        .child(field(
            st.t("purchases.supplier_ref"),
            reference,
            String::new,
            String::new,
            KeyboardType::Default,
            "buy-ref",
        ))
        .child(labelled(
            st.t("doc.add_item"),
            product_picker(st, "buy-search", pick),
            Some(Box::new(st.t("purchases.prices_help"))),
        ))
        .child(rows)
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
        .child(dynamic(move || {
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
        }))
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
    dialog_frame(
        st,
        st.t("purchases.new"),
        st.t("purchases.new.help"),
        body,
        footer,
        close,
    )
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

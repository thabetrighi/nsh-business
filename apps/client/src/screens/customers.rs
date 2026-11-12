//! Customers (M9.1): who they are, what they owe (their receivable lines,
//! M9.2) and how much credit they get (M9.3). The page shows and collects;
//! the use cases decide (a credit limit needs its own right).

use nb_core::Money;
use nb_i18n::tr_args;
use nb_store::usecases::customers::{
    CustomerGroup, CustomerInput, CustomerPage, CustomerQuery, CustomerRow, customer_groups,
    get_customer, list_customers, save_customer,
};
use nb_store::usecases::debts::{
    AdjustInput, Ageing, AllocationInput, CustomerStatement, OpenInvoice, ReceiptInput, ReceiptRow,
    adjust_debt, ageing, collect, customer_statement, open_invoices, receipts_of,
};
use nb_store::usecases::installments::{
    DueRow, Plan, PlanInput, RescheduleInput, create_plan, due_installments, pay_installments,
    plans_of, reschedule,
};
use nsh_core::element::{AnyElement, KeyboardType, View};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::nav::Route;
use crate::screens::common::{date_time, page};
use crate::screens::forms::{error_text, parse_price};
use crate::state::AppState;
use crate::ui::form::{field, labelled, search_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, checkbox, figure, icon_button, label, label_line, segmented,
    switch,
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

const FILTERS: [&str; 4] = ["", "debtors", "over_limit", "blocked"];

pub fn customers_page(st: AppState, compact: bool) -> View {
    let text = signal(String::new());
    let filter = signal(0usize);
    let paging = Paging::new(25);
    effect(move || {
        text.with(|_| ());
        filter.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            Some((
                CustomerQuery {
                    search: text.get(),
                    filter: FILTERS[filter.get()].into(),
                    limit: paging.size.get(),
                    offset: paging.offset(),
                },
                st.data_rev.get(),
            ))
        },
        move |(q, _), _| {
            let db = st.db();
            async move { db.call(move |s| list_customers(s, &q)).await }
        },
    );
    let page_data = computed(move || {
        data.with(|p: Option<&CustomerPage>| p.cloned())
            .unwrap_or_default()
    });
    // A customer's card (tapped in the list), and the editor: a new
    // customer (None inside) or the one of the card.
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
    let editing: Signal<Option<Option<CustomerRow>>> = signal(None);
    let editing_open = signal(false);
    effect(move || {
        let open = editing.get().is_some();
        if editing_open.get_untracked() != open {
            editing_open.set(open);
        }
    });
    effect(move || {
        if !editing_open.get() {
            editing.set(None);
        }
    });
    let may_edit = computed(move || st.can("customers.edit"));

    let list = dynamic(move || {
        let p = page_data.get();
        if p.rows.is_empty() {
            return empty_state(
                Icon::Users,
                st.t("customers.empty"),
                st.t("customers.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, c) in p.rows.into_iter().enumerate() {
            v = v.child(customer_row(st, i, c, compact, showing));
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
            st.t("customers.search"),
            "customers-search",
        )))
        .child(segmented(
            vec![
                (0, Box::new(st.t("customers.filter.all"))),
                (1, Box::new(st.t("customers.filter.debtors"))),
                (2, Box::new(st.t("customers.filter.over_limit"))),
                (3, Box::new(st.t("customers.filter.blocked"))),
            ],
            filter,
        ));
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
                    "customers.count",
                    &[("count", &st.fmt().integer(i64::from(page_data.get().total)))],
                )
            },
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Body,
        ))
        .child(view().flex_1())
        .child(label_line(
            st.t("customers.owed"),
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
        .child(filters)
        .child(summary)
        .child(list)
        .child(pager(st, paging, move || page_data.get().total, compact));
    let importing = signal(false);
    let actions = view().child(when(
        move || may_edit.get(),
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
                    .test_id("customers-import"),
                )
                .child(
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Plus),
                        st.t("customers.new"),
                        move || editing.set(Some(None)),
                    )
                    .test_id("customer-new"),
                )
                .into_element()
        },
    ));
    view()
        .child(page(
            st,
            Route::Customers,
            "customers.subtitle",
            actions,
            body,
            compact,
        ))
        .child(modal(showing_open, compact, move || {
            customer_card(st, showing, editing, move || showing_open.set(false)).into_element()
        }))
        .child(modal(editing_open, compact, move || {
            match editing.get_untracked() {
                Some(c) => customer_dialog(st, c, move || editing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
        .child(modal(importing, compact, move || {
            crate::screens::import::import_dialog(
                st,
                nb_domain::import::Kind::Customers,
                move || importing.set(false),
            )
            .into_element()
        }))
}

fn customer_row(
    st: AppState,
    i: usize,
    c: CustomerRow,
    compact: bool,
    showing: Signal<Option<String>>,
) -> View {
    let detail = [
        c.code.clone(),
        c.phone.clone(),
        c.city.clone(),
        c.group_name.clone().unwrap_or_default(),
    ]
    .into_iter()
    .filter(|x| !x.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let name = c.name.clone();
    let mut tail = view().flex_row().items_center().gap(px(8.0));
    if c.status == "blocked" {
        tail = tail.child(badge(Status::Danger, None, st.t("customers.blocked")));
    } else if c.over_limit() && c.balance_minor > 0 {
        tail = tail.child(badge(
            Status::Warning,
            Some(Icon::Alert),
            st.t("customers.over_limit"),
        ));
    }
    if !compact && c.credit_limit_minor > 0 {
        let limit = c.credit_limit_minor;
        tail = tail.child(
            view()
                .items_end()
                .child(label_line(
                    st.t("customers.limit"),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .child(figure(
                    move || money(st, limit),
                    ty::XS,
                    FontWeight::MEDIUM,
                    || Tone::Body,
                )),
        );
    }
    let balance = c.balance_minor;
    tail = tail.child(
        view()
            .w(px(if compact { 96.0 } else { 130.0 }))
            .items_end()
            .child(label_line(
                st.t(if balance >= 0 {
                    "customers.owes"
                } else {
                    "customers.credit"
                }),
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            ))
            .child(figure(
                move || money(st, balance.abs()),
                ty::SM,
                FontWeight::BOLD,
                move || {
                    if balance > 0 { Tone::Ink } else { Tone::Muted }
                },
            )),
    );
    let id = c.id.clone();
    view()
        .test_id(&format!("customer-{i}"))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(16.0))
        .py(px(11.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(icon(
            if c.kind == "company" {
                Icon::Store
            } else {
                Icon::User
            },
            18.0,
            || Tone::Faint,
        ))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(label_line(
                    move || name.clone(),
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
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
        .on_press(move || showing.set(Some(id.clone())))
}

fn groups(st: AppState) -> Memo<Vec<CustomerGroup>> {
    let r = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| customer_groups(s)).await }
        },
    );
    computed(move || {
        r.with(|g: Option<&Vec<CustomerGroup>>| g.cloned())
            .unwrap_or_default()
    })
}

fn customer_dialog(
    st: AppState,
    c: Option<CustomerRow>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    let meta = session.meta("customer.save");
    let is_new = c.is_none();
    let base = c.clone();
    let get = |f: fn(&CustomerRow) -> String| signal(c.as_ref().map(f).unwrap_or_default());
    let name = get(|c| c.name.clone());
    let phone = get(|c| c.phone.clone());
    let email = get(|c| c.email.clone());
    let city = get(|c| c.city.clone());
    let address = get(|c| c.address.clone());
    let nif = get(|c| c.nif.clone());
    let nis = get(|c| c.nis.clone());
    let rc = get(|c| c.rc.clone());
    let ai = get(|c| c.ai.clone());
    let note = get(|c| c.note.clone());
    let limit = signal(
        c.as_ref()
            .filter(|c| c.credit_limit_minor > 0)
            .map(|c| Money::from_minor(c.credit_limit_minor, cur).to_plain_string())
            .unwrap_or_default(),
    );
    let days = signal(
        c.as_ref()
            .filter(|c| c.payment_days > 0)
            .map(|c| c.payment_days.to_string())
            .unwrap_or_default(),
    );
    let company = signal(c.as_ref().is_some_and(|c| c.kind == "company"));
    let blocked = signal(c.as_ref().is_some_and(|c| c.status == "blocked"));
    let all_groups = groups(st);
    // usize::MAX: no group.
    let group = signal(usize::MAX);
    {
        let wanted = c.as_ref().and_then(|c| c.group_id.clone());
        effect(move || {
            let list = all_groups.get();
            if let Some(w) = wanted.as_deref()
                && let Some(i) = list.iter().position(|g| g.id == w)
            {
                group.set(i);
            }
        });
    }
    let error = signal(String::new());
    let busy = signal(false);
    let save = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let credit = if limit.get_untracked().trim().is_empty() {
                0
            } else {
                match parse_price(&limit.get_untracked(), cur) {
                    Ok(m) => m.minor(),
                    Err(_) => {
                        error.set(error_text(st, "invalid_amount"));
                        return;
                    }
                }
            };
            let payment_days = if days.get_untracked().trim().is_empty() {
                0
            } else {
                match days.get_untracked().trim().parse::<i64>() {
                    Ok(d) => d,
                    Err(_) => {
                        error.set(error_text(st, "invalid_value"));
                        return;
                    }
                }
            };
            let input = CustomerInput {
                id: base.as_ref().map(|b| b.id.clone()),
                kind: if company.get_untracked() {
                    "company"
                } else {
                    "person"
                }
                .into(),
                name: name.get_untracked(),
                group_id: all_groups
                    .with_untracked(|g| g.get(group.get_untracked()).map(|g| g.id.clone())),
                phone: phone.get_untracked(),
                email: email.get_untracked(),
                address: address.get_untracked(),
                city: city.get_untracked(),
                nif: nif.get_untracked(),
                nis: nis.get_untracked(),
                rc: rc.get_untracked(),
                ai: ai.get_untracked(),
                credit_limit_minor: credit,
                payment_days,
                status: if blocked.get_untracked() {
                    "blocked"
                } else {
                    "active"
                }
                .into(),
                note: note.get_untracked(),
            };
            busy.set(true);
            error.set(String::new());
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| save_customer(s, meta, input)).await {
                    Ok(c) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "customers.saved", &[("name", &c.name)]),
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
    let two = |a: View, b: View| {
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(12.0))
            .child(view().flex_1().min_w(px(180.0)).child(a))
            .child(view().flex_1().min_w(px(180.0)).child(b))
    };
    let kind_choice = segmented(
        vec![
            (false, Box::new(st.t("customers.kind.person"))),
            (true, Box::new(st.t("customers.kind.company"))),
        ],
        company,
    );
    let legal = when(
        move || company.get(),
        move || {
            view()
                .gap(px(12.0))
                .child(two(
                    field(
                        st.t("customers.nif"),
                        nif,
                        String::new,
                        String::new,
                        KeyboardType::Number,
                        "customer-nif",
                    ),
                    field(
                        st.t("customers.nis"),
                        nis,
                        String::new,
                        String::new,
                        KeyboardType::Number,
                        "customer-nis",
                    ),
                ))
                .child(two(
                    field(
                        st.t("customers.rc"),
                        rc,
                        String::new,
                        String::new,
                        KeyboardType::Default,
                        "customer-rc",
                    ),
                    field(
                        st.t("customers.ai"),
                        ai,
                        String::new,
                        String::new,
                        KeyboardType::Number,
                        "customer-ai",
                    ),
                ))
                .into_element()
        },
    );
    let group_choice = labelled(
        st.t("customers.group"),
        dropdown(
            move || {
                let mut v = vec![(usize::MAX, st.tn("customers.group.none"))];
                v.extend(
                    all_groups
                        .get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, g)| (i, g.name)),
                );
                v
            },
            group,
            Some(Icon::Layers),
            st.t("customers.group"),
            "customer-group",
        ),
        None,
    );
    let body = view()
        .gap(px(14.0))
        .child(kind_choice)
        .child(field(
            st.t("customers.name"),
            name,
            String::new,
            String::new,
            KeyboardType::Default,
            "customer-name",
        ))
        .child(two(
            field(
                st.t("customers.phone"),
                phone,
                String::new,
                String::new,
                KeyboardType::Phone,
                "customer-phone",
            ),
            field(
                st.t("customers.email"),
                email,
                String::new,
                String::new,
                KeyboardType::Email,
                "customer-email",
            ),
        ))
        .child(two(
            field(
                st.t("customers.city"),
                city,
                String::new,
                String::new,
                KeyboardType::Default,
                "customer-city",
            ),
            group_choice,
        ))
        .child(field(
            st.t("customers.address"),
            address,
            String::new,
            String::new,
            KeyboardType::Default,
            "customer-address",
        ))
        .child(legal)
        .child(
            view()
                .gap(px(12.0))
                .pt(px(12.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(label_line(
                    st.t("customers.credit_section"),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(two(
                    field(
                        st.t("customers.limit"),
                        limit,
                        || "0,00".to_owned(),
                        String::new,
                        KeyboardType::Decimal,
                        "customer-limit",
                    ),
                    field(
                        st.t("customers.days"),
                        days,
                        || "0".to_owned(),
                        String::new,
                        KeyboardType::Number,
                        "customer-days",
                    ),
                ))
                .child(label(
                    st.t("customers.limit.help"),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(switch(blocked, st.t("customers.block")).test_id("customer-block"))
                        .child(label_line(
                            st.t("customers.block"),
                            ty::SM,
                            FontWeight::MEDIUM,
                            Tone::Body,
                        )),
                ),
        )
        .child(field(
            st.t("customers.note"),
            note,
            String::new,
            String::new,
            KeyboardType::Default,
            "customer-note",
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
            .test_id("customer-save"),
        );
    let title = c.as_ref().map(|c| c.name.clone());
    dialog_frame(
        st,
        move || title.clone().unwrap_or_else(|| st.tn("customers.new")),
        st.t(if is_new {
            "customers.new.help"
        } else {
            "customers.edit.help"
        }),
        body,
        footer,
        close,
    )
}

// ------------------------------------------------------------------ the card

/// A customer at a glance (M9.12): what it owes and may still owe, its
/// statement, its open invoices and its receipts; collect or edit.
pub fn customer_card(
    st: AppState,
    showing: Signal<Option<String>>,
    editing: Signal<Option<Option<CustomerRow>>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let loaded = resource(
        move || showing.get().map(|id| (id, st.data_rev.get())),
        move |(id, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    Ok((
                        get_customer(s, &id)?,
                        customer_statement(s, &id, None, None)?,
                        open_invoices(s, &id)?,
                        receipts_of(s, &id)?,
                    ))
                })
                .await
            }
        },
    );
    type Card = (
        CustomerRow,
        CustomerStatement,
        Vec<OpenInvoice>,
        Vec<ReceiptRow>,
    );
    let data = computed(move || loaded.with(|d: Option<&Card>| d.cloned()));
    let plans_r = resource(
        move || showing.get().map(|id| (id, st.data_rev.get())),
        move |(id, _), _| {
            let db = st.db();
            async move { db.call(move |s| plans_of(s, &id)).await }
        },
    );
    let plans = computed(move || {
        plans_r
            .with(|p: Option<&Vec<Plan>>| p.cloned())
            .unwrap_or_default()
    });
    let tab = signal(0usize);
    let collecting = signal(false);
    // Instalments: a new plan for an invoice, a payment, a new schedule.
    let (planning, planning_open) = opener::<OpenInvoice>();
    let (paying, paying_open) = opener::<Plan>();
    let (moving, moving_open) = opener::<Plan>();
    // A receipt's voucher or a plan's schedule, to print.
    let (printout, printout_open) = opener::<Printout>();
    let adjusting = signal(false);

    let head = dynamic(move || {
        let Some((c, _, open, _)) = data.get() else {
            return view().into_element();
        };
        let overdue: i64 = open
            .iter()
            .filter(|o| o.overdue)
            .map(|o| o.open_minor)
            .sum();
        let tile = |key: &'static str, v: i64, tone: Tone, id: &'static str| {
            view()
                .test_id(id)
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
        };
        let mut facts = view().flex_row().flex_wrap().items_center().gap(px(8.0));
        for x in [c.code.clone(), c.phone.clone(), c.city.clone()] {
            if !x.is_empty() {
                facts = facts.child(badge(Status::Neutral, None, move || x.clone()));
            }
        }
        if c.status == "blocked" {
            facts = facts.child(badge(Status::Danger, None, st.t("customers.blocked")));
        }
        if c.payment_days > 0 {
            let d = c.payment_days;
            facts = facts.child(badge(Status::Info, Some(Icon::Calendar), move || {
                tr_args(
                    st.lang(),
                    "customers.days_short",
                    &[("days", &st.fmt().integer(d))],
                )
            }));
        }
        view()
            .gap(px(10.0))
            .child(facts)
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(tile(
                        "customers.owes",
                        c.balance_minor,
                        if c.over_limit() && c.balance_minor > 0 {
                            Tone::Danger
                        } else {
                            Tone::Ink
                        },
                        "card-balance",
                    ))
                    .child(tile(
                        "customers.overdue",
                        overdue,
                        if overdue > 0 { Tone::Danger } else { Tone::Ink },
                        "card-overdue",
                    ))
                    .child(tile(
                        "customers.credit_left",
                        c.credit_left_minor(),
                        Tone::Ink,
                        "card-left",
                    )),
            )
            .into_element()
    });

    let statement = move || {
        let Some((_, stt, _, _)) = data.get() else {
            return view().into_element();
        };
        if stt.lines.is_empty() {
            return label(
                st.t("customers.statement.empty"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        }
        let mut v = view();
        // The latest first, as people read a statement on screen.
        for (i, l) in stt.lines.into_iter().rev().enumerate() {
            let what = {
                let key = format!("acc.source.{}", l.source);
                let t = st.tn(&key);
                if t == key { l.source.clone() } else { t }
            };
            let (d, c, b) = (l.debit_minor, l.credit_minor, l.balance_minor);
            let detail = format!("{} · {}", l.date, l.memo);
            v = v.child(
                view()
                    .test_id(&format!("stmt-{i}"))
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
                            if d > 0 {
                                format!("+{}", money(st, d))
                            } else {
                                format!("−{}", money(st, c))
                            }
                        },
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        move || if d > 0 { Tone::Ink } else { Tone::Success },
                    )))
                    .child(view().w(px(110.0)).items_end().child(figure(
                        move || money(st, b),
                        ty::SM,
                        FontWeight::NORMAL,
                        || Tone::Muted,
                    ))),
            );
        }
        v.into_element()
    };
    let invoices = move || {
        let Some((_, _, open, _)) = data.get() else {
            return view().into_element();
        };
        if open.is_empty() {
            return label(
                st.t("customers.no_open"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        }
        let mut v = view();
        let planned: Vec<String> = plans
            .get()
            .into_iter()
            .filter(|p| p.status == "active")
            .map(|p| p.sale_id)
            .collect();
        for (i, o) in open.into_iter().enumerate() {
            let has_plan = planned.contains(&o.sale_id);
            let inv = o.clone();
            let title = o.number.clone();
            let detail = format!(
                "{} · {}",
                date_time(st, o.created_at),
                tr_args(
                    st.lang(),
                    "customers.age",
                    &[("days", &st.fmt().integer(o.age_days))]
                )
            );
            let (open_minor, overdue) = (o.open_minor, o.overdue);
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
                    .child(if overdue {
                        badge(Status::Danger, None, st.t("customers.overdue"))
                    } else {
                        view()
                    })
                    .child(if has_plan {
                        badge(
                            Status::Info,
                            Some(Icon::CalendarClock),
                            st.t("plans.in_plan"),
                        )
                    } else {
                        button(
                            Variant::Ghost,
                            Size::Sm,
                            Some(Icon::CalendarClock),
                            st.t("plans.split"),
                            move || planning.set(Some(inv.clone())),
                        )
                        .test_id(&format!("inv-plan-{i}"))
                    })
                    .child(figure(
                        move || money(st, open_minor),
                        ty::SM,
                        FontWeight::BOLD,
                        || Tone::Ink,
                    )),
            );
        }
        v.into_element()
    };
    let receipts = move || {
        let Some((_, _, _, rs)) = data.get() else {
            return view().into_element();
        };
        if rs.is_empty() {
            return label(
                st.t("customers.no_receipts"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        }
        let mut v = view();
        for r in rs {
            let title = format!(
                "{} · {}",
                r.number,
                st.tn(&format!("pos.method.{}", r.method))
            );
            let detail = if r.reference.is_empty() {
                date_time(st, r.created_at)
            } else {
                format!("{} · {}", date_time(st, r.created_at), r.reference)
            };
            let (amount, advance) = (r.amount_minor, r.unallocated_minor);
            let mut row = view()
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
                );
            if advance > 0 {
                row = row.child(badge(Status::Info, None, move || {
                    tr_args(
                        st.lang(),
                        "customers.advance",
                        &[("amount", &money(st, advance))],
                    )
                }));
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let number = r.number.clone();
                row = row.child(
                    icon_button(
                        || Icon::Printer,
                        st.t("action.print"),
                        Variant::Ghost,
                        move || printout.set(Some(Printout::Voucher(number.clone()))),
                    )
                    .test_id(&format!("receipt-print-{}", r.number)),
                );
            }
            v = v.child(row.child(figure(
                move || money(st, amount),
                ty::SM,
                FontWeight::BOLD,
                || Tone::Success,
            )));
        }
        v.into_element()
    };
    let body = view()
        .gap(px(14.0))
        .child(head)
        .child(segmented(
            vec![
                (0, Box::new(st.t("customers.tab.statement"))),
                (1, Box::new(st.t("customers.tab.open"))),
                (2, Box::new(st.t("customers.tab.receipts"))),
                (3, Box::new(st.t("customers.tab.plans"))),
            ],
            tab,
        ))
        .child(dynamic(move || match tab.get() {
            0 => statement(),
            1 => invoices(),
            2 => receipts(),
            _ => plans_view(st, plans.get(), paying, moving, printout).into_element(),
        }));
    let footer = {
        let close = close.clone();
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(8.0))
            .child(when(
                move || st.can("customers.edit"),
                move || {
                    let close = close.clone();
                    button(
                        Variant::Secondary,
                        Size::Md,
                        Some(Icon::Pencil),
                        st.t("customers.edit"),
                        move || {
                            if let Some((c, ..)) = data.get_untracked() {
                                close();
                                editing.set(Some(Some(c)));
                            }
                        },
                    )
                    .test_id("card-edit")
                    .into_element()
                },
            ))
            .child(when(
                move || st.can("debts.adjust") || st.can("debts.collect"),
                move || {
                    button(
                        Variant::Ghost,
                        Size::Md,
                        Some(Icon::Sliders),
                        st.t("adjust.open"),
                        move || adjusting.set(true),
                    )
                    .test_id("card-adjust")
                    .into_element()
                },
            ))
            .child(when(
                move || st.can("debts.collect"),
                move || {
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Coins),
                        st.t("customers.collect"),
                        move || collecting.set(true),
                    )
                    .test_id("card-collect")
                    .into_element()
                },
            ))
    };
    let title = move || data.get().map_or_else(String::new, |(c, ..)| c.name);
    view()
        .flex_shrink(1.0)
        .min_h(px(0.0))
        .child(dialog_frame(
            st,
            title,
            st.t("customers.card.help"),
            body,
            footer,
            close,
        ))
        .child(modal(adjusting, false, move || {
            match data.get_untracked() {
                Some((c, ..)) => adjust_dialog(st, c, move || adjusting.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
        .child(modal(planning_open, false, move || {
            match planning.get_untracked() {
                Some(inv) => plan_dialog(st, inv, move || {
                    planning_open.set(false);
                    tab.set(3);
                })
                .into_element(),
                None => view().into_element(),
            }
        }))
        .child(modal(paying_open, false, move || {
            match paying.get_untracked() {
                Some(p) => plan_pay_dialog(st, p, move || paying_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
        .child(modal(moving_open, false, move || {
            match moving.get_untracked() {
                Some(p) => reschedule_dialog(st, p, move || moving_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
        .child(modal(collecting, false, move || {
            match data.get_untracked() {
                Some((c, _, open, _)) => {
                    receipt_dialog(st, c, open, move || collecting.set(false)).into_element()
                }
                None => view().into_element(),
            }
        }))
        .child(modal(printout_open, false, move || {
            printout_dialog(st, printout.get_untracked(), move || {
                printout_open.set(false)
            })
        }))
}

/// What the customer card prints.
#[derive(Debug, Clone, PartialEq)]
enum Printout {
    Voucher(String),
    Schedule(Box<Plan>),
}

fn printout_dialog(
    st: AppState,
    what: Option<Printout>,
    close: impl Fn() + Clone + 'static,
) -> AnyElement {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::printers::Role;
        use crate::screens::printing::{document_dialog, schedule_maker, voucher_maker};
        match what {
            Some(Printout::Voucher(n)) => {
                let title = tr_args(st.lang(), "print.voucher_no", &[("number", &n)]);
                let file = format!("{n}.pdf");
                return document_dialog(
                    st,
                    title,
                    file,
                    Role::Receipt,
                    move || voucher_maker(st, n.clone()),
                    close,
                )
                .into_element();
            }
            Some(Printout::Schedule(p)) => {
                let title = tr_args(
                    st.lang(),
                    "print.schedule_no",
                    &[("number", &p.number), ("version", &p.version.to_string())],
                );
                let file = format!("{}.pdf", p.number);
                return document_dialog(
                    st,
                    title,
                    file,
                    Role::Invoice,
                    move || schedule_maker(st, (*p).clone()),
                    close,
                )
                .into_element();
            }
            None => {}
        }
    }
    let _ = (st, what, close);
    view().into_element()
}

/// The till money is collected into: this branch's first till with an
/// open session (or its first till: card and transfer need none).
pub(crate) fn branch_till(st: AppState) -> Memo<Option<String>> {
    let r = resource(
        move || {
            st.session
                .get()
                .map(|s| (s.branch().to_string(), st.data_rev.get()))
        },
        move |(branch, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    let all = s.read(|r| {
                        let o = nb_store::repo::org::the_org(r.conn())?
                            .ok_or(nb_store::Error::NotFound("organization"))?;
                        nb_store::usecases::org::registers(r.conn(), o.org_id()?)
                    })?;
                    let mine: Vec<_> = all.into_iter().filter(|r| r.branch_id == branch).collect();
                    for r in &mine {
                        if nb_store::usecases::sales::current_shift(s, &r.id)?.is_some() {
                            return Ok(Some(r.id.clone()));
                        }
                    }
                    Ok(mine.first().map(|r| r.id.clone()))
                })
                .await
            }
        },
    );
    computed(move || r.with(|v: Option<&Option<String>>| v.cloned().flatten()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Cash,
    Card,
    Transfer,
}

/// Money received from a customer: how much, how, and which invoices it
/// settles (none ticked: the oldest first).
fn receipt_dialog(
    st: AppState,
    c: CustomerRow,
    open: Vec<OpenInvoice>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    // One operation per receipt screen: a second tap records it once.
    let meta = session.meta("receipt.collect");
    let till = branch_till(st);
    let amount = signal(if c.balance_minor > 0 {
        Money::from_minor(c.balance_minor, cur).to_plain_string()
    } else {
        String::new()
    });
    let method = signal(Method::Cash);
    let reference = signal(String::new());
    let picked: Signal<Vec<bool>> = signal(vec![false; open.len()]);
    let error = signal(String::new());
    let busy = signal(false);
    let open_for_save = open.clone();
    let save = {
        let close = close.clone();
        let customer = c.id.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Ok(total) = parse_price(&amount.get_untracked(), cur) else {
                error.set(error_text(st, "invalid_amount"));
                return;
            };
            // Ticked invoices take what they owe, in order, while it lasts.
            let mut left = total.minor();
            let mut allocations = Vec::new();
            for (inv, on) in open_for_save.iter().zip(picked.get_untracked()) {
                if on && left > 0 {
                    let take = inv.open_minor.min(left);
                    left -= take;
                    allocations.push(AllocationInput {
                        sale_id: inv.sale_id.clone(),
                        amount_minor: take,
                    });
                }
            }
            let input = ReceiptInput {
                customer_id: customer.clone(),
                amount_minor: total.minor(),
                method: match method.get_untracked() {
                    Method::Cash => "cash",
                    Method::Card => "card",
                    Method::Transfer => "transfer",
                }
                .into(),
                register_id: till.get_untracked(),
                allocations,
                unallocated: false,
                reference: reference.get_untracked(),
                note: String::new(),
            };
            busy.set(true);
            error.set(String::new());
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| collect(s, meta, input)).await {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "customers.collected",
                                &[
                                    ("number", &r.number),
                                    ("amount", &money(st, r.amount_minor)),
                                ],
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
    let mut list = view();
    for (i, inv) in open.into_iter().enumerate() {
        let title = inv.number.clone();
        let o = inv.open_minor;
        list = list.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .py(px(6.0))
                .child(
                    checkbox(
                        move || Some(picked.with(|p| p.get(i).copied().unwrap_or(false))),
                        move || {
                            picked.update(|p| {
                                if let Some(x) = p.get_mut(i) {
                                    *x = !*x;
                                }
                            })
                        },
                    )
                    .test_id(&format!("rc-pick-{i}")),
                )
                .child(view().flex_1().min_w(px(0.0)).child(label_line(
                    move || title.clone(),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Ink,
                )))
                .child(figure(
                    move || money(st, o),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    || Tone::Ink,
                )),
        );
    }
    let owes = c.balance_minor;
    let body = view()
        .gap(px(14.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(label_line(
                    st.t("customers.owes"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )))
                .child(figure(
                    move || money(st, owes),
                    ty::LG,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        )
        .child(field(
            st.t("customers.receipt.amount"),
            amount,
            String::new,
            String::new,
            KeyboardType::Decimal,
            "rc-amount",
        ))
        .child(labelled(
            st.t("pos.method"),
            segmented(
                vec![
                    (Method::Cash, Box::new(st.t("pos.method.cash"))),
                    (Method::Card, Box::new(st.t("pos.method.card"))),
                    (Method::Transfer, Box::new(st.t("pos.method.transfer"))),
                ],
                method,
            ),
            None,
        ))
        .child(when(
            move || method.get() != Method::Cash,
            move || {
                field(
                    st.t("customers.receipt.reference"),
                    reference,
                    String::new,
                    String::new,
                    KeyboardType::Default,
                    "rc-reference",
                )
                .into_element()
            },
        ))
        .child(labelled(
            st.t("customers.receipt.invoices"),
            list,
            Some(Box::new(st.t("customers.receipt.invoices.help"))),
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
                st.t("customers.receipt.confirm"),
                save,
            )
            .test_id("rc-confirm"),
        );
    let name = c.name.clone();
    dialog_frame(
        st,
        move || tr_args(st.lang(), "customers.receipt.title", &[("name", &name)]),
        st.t("customers.receipt.help"),
        body,
        footer,
        close,
    )
}

// ------------------------------------------------------------------ debts

/// What customers owe by age (M9.5), the most overdue first; tapped, a
/// customer's card to collect.
pub fn debts_page(st: AppState, compact: bool) -> View {
    let data = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| ageing(s)).await }
        },
    );
    let ag = computed(move || {
        data.with(|a: Option<&Ageing>| a.cloned())
            .unwrap_or_default()
    });
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
    let editing: Signal<Option<Option<CustomerRow>>> = signal(None);
    let editing_open = signal(false);
    effect(move || {
        let open = editing.get().is_some();
        if editing_open.get_untracked() != open {
            editing_open.set(open);
        }
    });
    effect(move || {
        if !editing_open.get() {
            editing.set(None);
        }
    });

    const BUCKETS: [&str; 4] = [
        "debts.bucket.0",
        "debts.bucket.1",
        "debts.bucket.2",
        "debts.bucket.3",
    ];
    let tiles = dynamic(move || {
        let a = ag.get();
        let tile = |key: &'static str, v: i64, tone: Tone| {
            view()
                .flex_1()
                .min_w(px(140.0))
                .gap(px(4.0))
                .p(px(14.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(Tone::Border.color())
                .bg(Tone::Surface.color())
                .child(label_line(
                    st.t(key),
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                ))
                .child(figure(
                    move || money(st, v),
                    ty::XL,
                    FontWeight::BOLD,
                    move || tone,
                ))
        };
        let mut row = view()
            .flex_row()
            .flex_wrap()
            .gap(px(10.0))
            .child(tile("debts.total", a.balance_minor, Tone::Ink))
            .child(tile(
                "debts.overdue",
                a.overdue_minor,
                if a.overdue_minor > 0 {
                    Tone::Danger
                } else {
                    Tone::Ink
                },
            ));
        for (i, key) in BUCKETS.into_iter().enumerate() {
            row = row.child(tile(key, a.totals[i], Tone::Body));
        }
        row.into_element()
    });
    let list = dynamic(move || {
        let a = ag.get();
        if a.rows.is_empty() {
            return empty_state(
                Icon::Coins,
                st.t("debts.empty"),
                st.t("debts.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, r) in a.rows.into_iter().enumerate() {
            let id = r.customer_id.clone();
            let detail = [r.code.clone(), r.phone.clone()]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let name = r.name.clone();
            let mut tail = view().flex_row().items_center().gap(px(12.0));
            if !compact {
                for b in r.buckets {
                    tail = tail.child(view().w(px(96.0)).items_end().child(figure(
                        move || {
                            if b == 0 {
                                "—".to_owned()
                            } else {
                                money(st, b)
                            }
                        },
                        ty::SM,
                        FontWeight::NORMAL,
                        || Tone::Body,
                    )));
                }
            }
            if r.overdue_minor > 0 {
                tail = tail.child(badge(Status::Danger, None, st.t("customers.overdue")));
            }
            let balance = r.balance_minor;
            tail = tail.child(view().w(px(120.0)).items_end().child(figure(
                move || money(st, balance),
                ty::SM,
                FontWeight::BOLD,
                || Tone::Ink,
            )));
            v = v.child(
                view()
                    .test_id(&format!("debt-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(16.0))
                    .py(px(11.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .on_press(move || showing.set(Some(id.clone())))
                    .child(icon(Icon::User, 18.0, || Tone::Faint))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(label_line(
                                move || name.clone(),
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
                    .child(tail),
            );
        }
        v.into_element()
    });
    let mut header = view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(16.0))
        .py(px(8.0))
        .bg(Tone::Subtle.color())
        .child(view().flex_1().child(label_line(
            st.t("debts.customer"),
            ty::XS,
            FontWeight::SEMIBOLD,
            Tone::Muted,
        )));
    if !compact {
        for key in BUCKETS {
            header = header.child(view().w(px(96.0)).items_end().child(label_line(
                st.t(key),
                ty::XS,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            )));
        }
    }
    header = header.child(view().w(px(120.0)).items_end().child(label_line(
        st.t("debts.balance"),
        ty::XS,
        FontWeight::SEMIBOLD,
        Tone::Muted,
    )));
    let body = view()
        .gap(px(16.0))
        .child(tiles)
        .child(card().child(header).child(list));
    view()
        .child(page(
            st,
            Route::Debts,
            "debts.subtitle",
            view(),
            body,
            compact,
        ))
        .child(modal(showing_open, compact, move || {
            customer_card(st, showing, editing, move || showing_open.set(false)).into_element()
        }))
        .child(modal(editing_open, compact, move || {
            match editing.get_untracked() {
                Some(c) => customer_dialog(st, c, move || editing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
}

// ------------------------------------------------------------ instalments

/// A dialog's subject and its open flag, kept in step.
pub(crate) fn opener<T: Clone + PartialEq + 'static>() -> (Signal<Option<T>>, Signal<bool>) {
    let subject: Signal<Option<T>> = signal(None);
    let open = signal(false);
    effect(move || {
        let o = subject.with(Option::is_some);
        if open.get_untracked() != o {
            open.set(o);
        }
    });
    effect(move || {
        if !open.get() && subject.with_untracked(Option::is_some) {
            subject.set(None);
        }
    });
    (subject, open)
}

fn state_badge(st: AppState, state: &str) -> View {
    let s = match state {
        "paid" => Status::Success,
        "late" => Status::Danger,
        "due" | "partial" => Status::Warning,
        "rescheduled" => Status::Neutral,
        _ => Status::Info,
    };
    let key = format!("plans.state.{state}");
    badge(s, None, move || st.tn(&key))
}

/// A customer's plans with their instalments.
fn plans_view(
    st: AppState,
    plans: Vec<Plan>,
    paying: Signal<Option<Plan>>,
    moving: Signal<Option<Plan>>,
    printout: Signal<Option<Printout>>,
) -> View {
    if plans.is_empty() {
        return view().child(label(
            st.t("plans.none"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ));
    }
    let mut v = view().gap(px(14.0));
    for (pi, p) in plans.into_iter().enumerate() {
        let active = p.status == "active";
        let title = tr_args(
            st.lang(),
            "plans.title_row",
            &[
                ("number", &p.number),
                ("version", &st.fmt().integer(i64::from(p.version))),
                ("sale", &p.sale_number),
            ],
        );
        let (open, late) = (p.open_minor(), p.late_minor());
        let mut head = view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(view().flex_1().min_w(px(160.0)).child(label_line(
                move || title.clone(),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            )))
            .child(badge(
                if active {
                    Status::Info
                } else {
                    Status::Neutral
                },
                None,
                {
                    let key = format!("plans.status.{}", p.status);
                    move || st.tn(&key)
                },
            ));
        if late > 0 {
            head = head.child(badge(Status::Danger, None, move || {
                tr_args(st.lang(), "plans.late", &[("amount", &money(st, late))])
            }));
        }
        head = head.child(figure(
            move || money(st, open),
            ty::SM,
            FontWeight::BOLD,
            || Tone::Ink,
        ));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let plan = p.clone();
            head = head.child(
                icon_button(
                    || Icon::Printer,
                    st.t("action.print"),
                    Variant::Ghost,
                    move || printout.set(Some(Printout::Schedule(Box::new(plan.clone())))),
                )
                .test_id(&format!("plan-print-{pi}")),
            );
        }
        #[cfg(target_arch = "wasm32")]
        let _ = printout;
        let mut rows = view();
        for i in &p.installments {
            let due = i.due_date.clone();
            let seq = format!("{}/{}", i.seq, p.count);
            let (amount, paid) = (i.amount_minor, i.paid_minor);
            rows = rows.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(6.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(view().w(px(48.0)).child(figure(
                        move || seq.clone(),
                        ty::XS,
                        FontWeight::MEDIUM,
                        || Tone::Muted,
                    )))
                    .child(view().flex_1().child(figure(
                        move || due.clone(),
                        ty::SM,
                        FontWeight::NORMAL,
                        || Tone::Body,
                    )))
                    .child(state_badge(st, &i.state))
                    .child(view().w(px(110.0)).items_end().child(figure(
                        move || {
                            if paid > 0 && paid < amount {
                                format!("{} / {}", money(st, paid), money(st, amount))
                            } else {
                                money(st, amount)
                            }
                        },
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        || Tone::Ink,
                    ))),
            );
        }
        let mut card_v = view()
            .test_id(&format!("plan-{pi}"))
            .gap(px(8.0))
            .p(px(12.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(Tone::Border.color())
            .child(head)
            .child(rows);
        if active {
            let (p1, p2) = (p.clone(), p.clone());
            card_v = card_v.child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(when(
                        move || st.can("installments.reschedule") || st.can("debts.collect"),
                        move || {
                            let p2 = p2.clone();
                            button(
                                Variant::Ghost,
                                Size::Sm,
                                Some(Icon::CalendarClock),
                                st.t("plans.reschedule"),
                                move || moving.set(Some(p2.clone())),
                            )
                            .test_id("plan-reschedule")
                            .into_element()
                        },
                    ))
                    .child(when(
                        move || st.can("debts.collect"),
                        move || {
                            let p1 = p1.clone();
                            button(
                                Variant::Primary,
                                Size::Sm,
                                Some(Icon::Coins),
                                st.t("plans.pay"),
                                move || paying.set(Some(p1.clone())),
                            )
                            .test_id("plan-pay")
                            .into_element()
                        },
                    )),
            );
        }
        v = v.child(card_v);
    }
    v
}

fn in_days(n: i64) -> String {
    let ms = crate::state::now_ms() + n * 86_400_000;
    nb_store::time::iso_date(ms, 60)
}

const FREQUENCIES: [&str; 3] = ["weekly", "monthly", "quarterly"];

/// The schedule's fields, with its instalments shown as they will be.
fn schedule_form(
    st: AppState,
    financed: i64,
    count: Signal<String>,
    frequency: Signal<usize>,
    first: Signal<String>,
) -> View {
    let cur = currency(st);
    let preview = computed(move || {
        let n: u32 = count.get().trim().parse().ok()?;
        let f = nb_domain::installment::Frequency::from_code(FREQUENCIES[frequency.get()])?;
        let d = nb_domain::fiscal::Date::parse(&first.get()).ok()?;
        nb_domain::installment::schedule(Money::from_minor(financed, cur), n, f, d).ok()
    });
    view()
        .gap(px(12.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(12.0))
                .child(view().w(px(110.0)).child(field(
                    st.t("plans.count"),
                    count,
                    String::new,
                    String::new,
                    KeyboardType::Number,
                    "plan-count",
                )))
                .child(view().flex_1().min_w(px(200.0)).child(labelled(
                    st.t("plans.frequency"),
                    segmented(
                        vec![
                            (0, Box::new(st.t("plans.frequency.weekly"))),
                            (1, Box::new(st.t("plans.frequency.monthly"))),
                            (2, Box::new(st.t("plans.frequency.quarterly"))),
                        ],
                        frequency,
                    ),
                    None,
                )))
                .child(view().w(px(150.0)).child(field(
                    st.t("plans.first_due"),
                    first,
                    st.t("doc.date_hint"),
                    String::new,
                    KeyboardType::Default,
                    "plan-first",
                ))),
        )
        .child(dynamic(move || {
            let Some(dues) = preview.get() else {
                return label(
                    st.t("plans.preview.invalid"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )
                .into_element();
            };
            let mut v = view()
                .gap(px(4.0))
                .p(px(10.0))
                .rounded(px(8.0))
                .bg(Tone::Subtle.color());
            let n = dues.len();
            for (i, d) in dues.into_iter().enumerate() {
                if n > 8 && i == 4 {
                    v = v.child(label_line(
                        || "…".to_owned(),
                        ty::SM,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    ));
                }
                if n > 8 && (4..n - 3).contains(&i) {
                    continue;
                }
                let (date, amount) = (d.date.iso(), d.amount.minor());
                let seq = format!("{}/{n}", i + 1);
                v = v.child(
                    view()
                        .flex_row()
                        .gap(px(10.0))
                        .child(view().w(px(48.0)).child(figure(
                            move || seq.clone(),
                            ty::XS,
                            FontWeight::MEDIUM,
                            || Tone::Muted,
                        )))
                        .child(view().flex_1().child(figure(
                            move || date.clone(),
                            ty::SM,
                            FontWeight::NORMAL,
                            || Tone::Body,
                        )))
                        .child(figure(
                            move || money(st, amount),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            || Tone::Ink,
                        )),
                );
            }
            v.into_element()
        }))
        .child(label(
            st.t("plans.rounding"),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
}

/// Splits what an invoice still owes into instalments.
fn plan_dialog(st: AppState, inv: OpenInvoice, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let meta = session.meta("plan.create");
    let count = signal("3".to_owned());
    let frequency = signal(1usize);
    let first = signal(in_days(30));
    let error = signal(String::new());
    let busy = signal(false);
    let sale = inv.sale_id.clone();
    let save = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Ok(n) = count.get_untracked().trim().parse::<u32>() else {
                error.set(error_text(st, "plan.count"));
                return;
            };
            let input = PlanInput {
                sale_id: sale.clone(),
                count: n,
                frequency: FREQUENCIES[frequency.get_untracked()].into(),
                first_due: first.get_untracked().trim().to_owned(),
            };
            busy.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| create_plan(s, meta, input)).await {
                    Ok(p) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "plans.created", &[("number", &p.number)]),
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
    let financed = inv.open_minor;
    let body = view()
        .gap(px(14.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(label_line(
                    st.t("plans.financed"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )))
                .child(figure(
                    move || money(st, financed),
                    ty::LG,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        )
        .child(schedule_form(st, financed, count, frequency, first))
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
                st.t("plans.create"),
                save,
            )
            .test_id("plan-save"),
        );
    let number = inv.number.clone();
    dialog_frame(
        st,
        move || tr_args(st.lang(), "plans.new", &[("number", &number)]),
        st.t("plans.new.help"),
        body,
        footer,
        close,
    )
}

/// Pays instalments of a plan (the next one by default).
fn plan_pay_dialog(st: AppState, plan: Plan, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    let meta = session.meta("plan.pay");
    let till = branch_till(st);
    let next = plan.next().map_or(0, |i| i.open_minor());
    let amount = signal(Money::from_minor(next, cur).to_plain_string());
    let method = signal(Method::Cash);
    let error = signal(String::new());
    let busy = signal(false);
    let id = plan.id.clone();
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
            let m = match method.get_untracked() {
                Method::Cash => "cash",
                Method::Card => "card",
                Method::Transfer => "transfer",
            };
            let (id, reg) = (id.clone(), till.get_untracked());
            busy.set(true);
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db
                    .call(move |s| pay_installments(s, meta, &id, a.minor(), m, reg))
                    .await
                {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "customers.collected",
                                &[
                                    ("number", &r.number),
                                    ("amount", &money(st, r.amount_minor)),
                                ],
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
    let (left, late) = (plan.open_minor(), plan.late_minor());
    let mut quick = view().flex_row().flex_wrap().gap(px(8.0));
    for (key, v, id) in [
        ("plans.pay.next", next, "pay-next"),
        ("plans.pay.late", late, "pay-late"),
        ("plans.pay.all", left, "pay-all"),
    ] {
        if v > 0 {
            quick = quick.child(
                button(
                    Variant::Secondary,
                    Size::Sm,
                    None,
                    move || format!("{} · {}", st.tn(key), money(st, v)),
                    move || amount.set(Money::from_minor(v, cur).to_plain_string()),
                )
                .test_id(id),
            );
        }
    }
    let body = view()
        .gap(px(14.0))
        .child(field(
            st.t("customers.receipt.amount"),
            amount,
            String::new,
            String::new,
            KeyboardType::Decimal,
            "plan-amount",
        ))
        .child(quick)
        .child(labelled(
            st.t("pos.method"),
            segmented(
                vec![
                    (Method::Cash, Box::new(st.t("pos.method.cash"))),
                    (Method::Card, Box::new(st.t("pos.method.card"))),
                    (Method::Transfer, Box::new(st.t("pos.method.transfer"))),
                ],
                method,
            ),
            None,
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
                st.t("customers.receipt.confirm"),
                save,
            )
            .test_id("plan-pay-confirm"),
        );
    let number = plan.number.clone();
    dialog_frame(
        st,
        move || tr_args(st.lang(), "plans.pay.title", &[("number", &number)]),
        st.t("plans.pay.help"),
        body,
        footer,
        close,
    )
}

/// A new schedule for what is left; the old one is kept. Needs a
/// supervisor when the user may not reschedule.
fn reschedule_dialog(st: AppState, plan: Plan, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let meta = session.meta("plan.reschedule");
    let owner = nsh_core::element::Owner::current();
    let count = signal(
        plan.installments
            .iter()
            .filter(|i| i.open_minor() > 0)
            .count()
            .max(1)
            .to_string(),
    );
    let frequency = signal(
        FREQUENCIES
            .iter()
            .position(|f| *f == plan.frequency)
            .unwrap_or(1),
    );
    let first = signal(in_days(30));
    let reason = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let approval: Signal<Option<String>> = signal(None);
    let (asking, asking_open) = opener::<crate::screens::approval::Ask>();
    let id = plan.id.clone();
    let save = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Ok(n) = count.get_untracked().trim().parse::<u32>() else {
                error.set(error_text(st, "plan.count"));
                return;
            };
            let input = RescheduleInput {
                count: n,
                frequency: FREQUENCIES[frequency.get_untracked()].into(),
                first_due: first.get_untracked().trim().to_owned(),
                reason: reason.get_untracked(),
                approval: approval.get_untracked(),
            };
            busy.set(true);
            error.set(String::new());
            let (id, close) = (id.clone(), close.clone());
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| reschedule(s, meta, &id, input)).await {
                    Ok(p) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "plans.rescheduled", &[("number", &p.number)]),
                        );
                    }
                    Err(e) => {
                        busy.set(false);
                        if let Some(p) = crate::screens::approval::needs_approval(&e) {
                            asking.set(Some(crate::screens::approval::Ask {
                                permission: p,
                                branch_id: None,
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
    let left = plan.open_minor();
    let body = view()
        .gap(px(14.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(label_line(
                    st.t("plans.left"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )))
                .child(figure(
                    move || money(st, left),
                    ty::LG,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        )
        .child(schedule_form(st, left, count, frequency, first))
        .child(field(
            st.t("plans.reason"),
            reason,
            String::new,
            String::new,
            KeyboardType::Default,
            "plan-reason",
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
                st.t("plans.reschedule"),
                save,
            )
            .test_id("plan-reschedule-confirm"),
        );
    let number = plan.number.clone();
    view()
        .flex_shrink(1.0)
        .min_h(px(0.0))
        .child(dialog_frame(
            st,
            move || tr_args(st.lang(), "plans.reschedule.title", &[("number", &number)]),
            st.t("plans.reschedule.help"),
            body,
            footer,
            close,
        ))
        .child(modal(asking_open, false, move || {
            match asking.get_untracked() {
                Some(ask) => {
                    let retry = retry.clone();
                    crate::screens::approval::approval_dialog(
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
            }
        }))
}

/// Instalments due within a month, late ones first (their date is past).
pub fn installments_page(st: AppState, compact: bool) -> View {
    let data = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| due_installments(s, 30)).await }
        },
    );
    let rows = computed(move || {
        data.with(|d: Option<&Vec<DueRow>>| d.cloned())
            .unwrap_or_default()
    });
    let (showing, showing_open) = opener::<String>();
    let (editing, editing_open) = opener::<Option<CustomerRow>>();
    let tiles = dynamic(move || {
        let r = rows.get();
        let late: i64 = r
            .iter()
            .filter(|d| d.state == "late")
            .map(|d| d.open_minor)
            .sum();
        let soon: i64 = r
            .iter()
            .filter(|d| d.state != "late")
            .map(|d| d.open_minor)
            .sum();
        let tile = |key: &'static str, v: i64, tone: Tone| {
            view()
                .flex_1()
                .min_w(px(160.0))
                .gap(px(4.0))
                .p(px(14.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(Tone::Border.color())
                .bg(Tone::Surface.color())
                .child(label_line(
                    st.t(key),
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                ))
                .child(figure(
                    move || money(st, v),
                    ty::XL,
                    FontWeight::BOLD,
                    move || tone,
                ))
        };
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(10.0))
            .child(tile(
                "plans.page.late",
                late,
                if late > 0 { Tone::Danger } else { Tone::Ink },
            ))
            .child(tile("plans.page.soon", soon, Tone::Ink))
            .into_element()
    });
    let list = dynamic(move || {
        let r = rows.get();
        if r.is_empty() {
            return empty_state(
                Icon::CalendarClock,
                st.t("plans.page.empty"),
                st.t("plans.page.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, d) in r.into_iter().enumerate() {
            let id = d.customer_id.clone();
            let name = d.customer_name.clone();
            let detail = [
                format!("{} · {}/{}", d.plan_number, d.seq, d.count),
                d.phone.clone(),
            ]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            let when_text = if d.days_late > 0 {
                tr_args(
                    st.lang(),
                    "plans.days_late",
                    &[("days", &st.fmt().integer(d.days_late))],
                )
            } else {
                d.due_date.clone()
            };
            let open = d.open_minor;
            let mut tail = view().flex_row().items_center().gap(px(10.0));
            if !compact {
                tail = tail.child(label_line(
                    move || when_text.clone(),
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                ));
            }
            tail =
                tail.child(state_badge(st, &d.state))
                    .child(view().w(px(110.0)).items_end().child(figure(
                        move || money(st, open),
                        ty::SM,
                        FontWeight::BOLD,
                        || Tone::Ink,
                    )));
            v = v.child(
                view()
                    .test_id(&format!("due-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(16.0))
                    .py(px(11.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .on_press(move || showing.set(Some(id.clone())))
                    .child(icon(Icon::CalendarClock, 18.0, || Tone::Faint))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(label_line(
                                move || name.clone(),
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
                    .child(tail),
            );
        }
        v.into_element()
    });
    let body = view().gap(px(16.0)).child(tiles).child(card().child(list));
    view()
        .child(page(
            st,
            Route::Installments,
            "plans.page.subtitle",
            view(),
            body,
            compact,
        ))
        .child(modal(showing_open, compact, move || {
            customer_card(st, showing, editing, move || showing_open.set(false)).into_element()
        }))
        .child(modal(editing_open, compact, move || {
            match editing.get_untracked() {
                Some(c) => customer_dialog(st, c, move || editing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
}

// ------------------------------------------------------------ adjustments

const ADJUST_KINDS: [&str; 3] = ["allowance", "write_off", "opening"];

/// An allowance, a debt given up, or what the customer owed before the
/// books started here (M9.9); a supervisor approves when needed.
fn adjust_dialog(st: AppState, c: CustomerRow, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    let meta = session.meta("debt.adjust");
    let owner = nsh_core::element::Owner::current();
    let kind = signal(if c.balance_minor > 0 { 0usize } else { 2 });
    let amount = signal(String::new());
    // Opening: the shop owed the customer (an advance it had paid).
    let we_owe = signal(false);
    let reason = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let approval: Signal<Option<String>> = signal(None);
    let (asking, asking_open) = opener::<crate::screens::approval::Ask>();
    let id = c.id.clone();
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
            let k = ADJUST_KINDS[kind.get_untracked()];
            let signed = if k == "opening" && we_owe.get_untracked() {
                -a.minor()
            } else {
                a.minor()
            };
            let input = AdjustInput {
                customer_id: id.clone(),
                kind: k.into(),
                amount_minor: signed,
                reason: reason.get_untracked(),
                approval: approval.get_untracked(),
            };
            busy.set(true);
            error.set(String::new());
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| adjust_debt(s, meta, input)).await {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "adjust.done",
                                &[("balance", &money(st, r.balance_minor))],
                            ),
                        );
                    }
                    Err(e) => {
                        busy.set(false);
                        if let Some(p) = crate::screens::approval::needs_approval(&e) {
                            asking.set(Some(crate::screens::approval::Ask {
                                permission: p,
                                branch_id: None,
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
    let owes = c.balance_minor;
    let body = view()
        .gap(px(14.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(label_line(
                    st.t("customers.owes"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )))
                .child(figure(
                    move || money(st, owes),
                    ty::LG,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        )
        .child(segmented(
            vec![
                (0, Box::new(st.t("adjust.kind.allowance"))),
                (1, Box::new(st.t("adjust.kind.write_off"))),
                (2, Box::new(st.t("adjust.kind.opening"))),
            ],
            kind,
        ))
        .child(label(
            move || st.tn(&format!("adjust.help.{}", ADJUST_KINDS[kind.get()])),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(field(
            st.t("adjust.amount"),
            amount,
            String::new,
            String::new,
            KeyboardType::Decimal,
            "adj-amount",
        ))
        .child(when(
            move || kind.get() == 2,
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(switch(we_owe, st.t("adjust.we_owe")).test_id("adj-we-owe"))
                    .child(label_line(
                        st.t("adjust.we_owe"),
                        ty::SM,
                        FontWeight::MEDIUM,
                        Tone::Body,
                    ))
                    .into_element()
            },
        ))
        .child(field(
            st.t("plans.reason"),
            reason,
            String::new,
            String::new,
            KeyboardType::Default,
            "adj-reason",
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
                st.t("adjust.confirm"),
                save,
            )
            .test_id("adj-confirm"),
        );
    let name = c.name.clone();
    view()
        .flex_shrink(1.0)
        .min_h(px(0.0))
        .child(dialog_frame(
            st,
            move || tr_args(st.lang(), "adjust.title", &[("name", &name)]),
            st.t("adjust.dialog.help"),
            body,
            footer,
            close,
        ))
        .child(modal(asking_open, false, move || {
            match asking.get_untracked() {
                Some(ask) => {
                    let retry = retry.clone();
                    crate::screens::approval::approval_dialog(
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
            }
        }))
}

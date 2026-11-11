//! Customers (M9.1): who they are, what they owe (their receivable lines,
//! M9.2) and how much credit they get (M9.3). The page shows and collects;
//! the use cases decide (a credit limit needs its own right).

use nb_core::Money;
use nb_i18n::tr_args;
use nb_store::usecases::customers::{
    CustomerGroup, CustomerInput, CustomerPage, CustomerQuery, CustomerRow, customer_groups,
    list_customers, save_customer,
};
use nsh_core::element::{AnyElement, KeyboardType, View};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::nav::Route;
use crate::screens::common::page;
use crate::screens::forms::{error_text, parse_price};
use crate::state::AppState;
use crate::ui::form::{field, labelled, search_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, badge, button, figure, label, label_line, segmented, switch};
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
    // The editor: a new customer (None inside) or one of the list.
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
            v = v.child(customer_row(st, i, c, compact, editing, may_edit.get()));
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
    let actions = view().child(when(
        move || may_edit.get(),
        move || {
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("customers.new"),
                move || editing.set(Some(None)),
            )
            .test_id("customer-new")
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
        .child(modal(editing_open, compact, move || {
            match editing.get_untracked() {
                Some(c) => customer_dialog(st, c, move || editing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
}

fn customer_row(
    st: AppState,
    i: usize,
    c: CustomerRow,
    compact: bool,
    editing: Signal<Option<Option<CustomerRow>>>,
    may_edit: bool,
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
    let mut row = view()
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
        .child(tail);
    if may_edit {
        row = row
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Hover.color()))
            .on_press(move || editing.set(Some(Some(c.clone()))));
    }
    row
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

//! Expenses and other income (M9.11): what was spent (rent, power, wages…)
//! or earned besides sales, from the till or the bank, with totals by
//! category. The page shows and collects; the use cases post.

use nb_core::Money;
use nb_i18n::tr_args;
use nb_store::usecases::accounting::{ChartRow, chart};
use nb_store::usecases::expenses::{
    ExpenseCategory, ExpenseInput, ExpensePage, ExpenseQuery, create_category, expense_categories,
    list_expenses, record_expense,
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
use crate::ui::kit::{Size, Variant, badge, button, figure, label, label_line, segmented};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon, empty_state};
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

/// The start of this month, in ms (the store's time zone).
fn month_start() -> i64 {
    let now = crate::state::now_ms();
    let (y, m, _) = nb_store::time::civil_date(now, 60);
    let epoch = nb_domain::fiscal::Date {
        y: 1970,
        m: 1,
        d: 1,
    };
    let first = nb_domain::fiscal::Date { y, m, d: 1 };
    // Midnight of the first, local time, back to UTC.
    nb_domain::installment::days_between(epoch, first) * 86_400_000 - 60 * 60_000
}

const KINDS: [&str; 3] = ["", "expense", "income"];

fn categories(st: AppState) -> Memo<Vec<ExpenseCategory>> {
    let r = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| expense_categories(s)).await }
        },
    );
    computed(move || {
        r.with(|v: Option<&Vec<ExpenseCategory>>| v.cloned())
            .unwrap_or_default()
    })
}

pub fn expenses_page(st: AppState, compact: bool) -> View {
    let text = signal(String::new());
    let kind = signal(0usize);
    // 0: this month, 1: everything.
    let span = signal(0usize);
    let paging = Paging::new(25);
    effect(move || {
        text.with(|_| ());
        kind.with(|_| ());
        span.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            Some((
                ExpenseQuery {
                    kind: KINDS[kind.get()].into(),
                    from_ms: if span.get() == 0 { month_start() } else { 0 },
                    to_ms: 0,
                    search: text.get(),
                    limit: paging.size.get(),
                    offset: paging.offset(),
                },
                st.data_rev.get(),
            ))
        },
        move |(q, _), _| {
            let db = st.db();
            async move { db.call(move |s| list_expenses(s, &q)).await }
        },
    );
    let page_data = computed(move || {
        data.with(|p: Option<&ExpensePage>| p.cloned())
            .unwrap_or_default()
    });
    let recording: Signal<Option<&'static str>> = signal(None);
    let recording_open = signal(false);
    effect(move || {
        let o = recording.get().is_some();
        if recording_open.get_untracked() != o {
            recording_open.set(o);
        }
    });
    effect(move || {
        if !recording_open.get() && recording.get_untracked().is_some() {
            recording.set(None);
        }
    });
    let adding = signal(false);

    let tiles = dynamic(move || {
        let p = page_data.get();
        let tile = |key: &'static str, v: i64, tone: Tone, id: &'static str| {
            view()
                .test_id(id)
                .flex_1()
                .min_w(px(150.0))
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
                "expenses.total",
                p.expenses_minor,
                Tone::Danger,
                "exp-total",
            ))
            .child(tile(
                "expenses.income",
                p.income_minor,
                Tone::Success,
                "exp-income",
            ))
            .child(tile(
                "expenses.net",
                p.income_minor - p.expenses_minor,
                Tone::Ink,
                "exp-net",
            ))
            .into_element()
    });
    let by_category = dynamic(move || {
        let p = page_data.get();
        if p.by_category.is_empty() {
            return view().into_element();
        }
        let top = p.by_category.iter().map(|c| c.2).max().unwrap_or(1).max(1);
        let mut v = card().child(card_header_icon(
            Some(Icon::Chart),
            st.t("expenses.by_category"),
            None,
            view(),
        ));
        let mut rows = view().gap(px(8.0)).p(px(16.0));
        for (name, kind, total) in p.by_category {
            // The bar's length in thousandths (amounts stay integers).
            let permille = (total.saturating_mul(1_000) / top).clamp(20, 1_000);
            let share = u16::try_from(permille).map_or(1.0, |p| f32::from(p) / 1_000.0);
            let tone = if kind == "income" {
                Tone::Success
            } else {
                Tone::Primary
            };
            rows = rows.child(
                view()
                    .gap(px(4.0))
                    .child(
                        view()
                            .flex_row()
                            .child(view().flex_1().child(label_line(
                                move || name.clone(),
                                ty::SM,
                                FontWeight::MEDIUM,
                                Tone::Ink,
                            )))
                            .child(figure(
                                move || money(st, total),
                                ty::SM,
                                FontWeight::SEMIBOLD,
                                || Tone::Ink,
                            )),
                    )
                    .child(
                        view()
                            .h(px(6.0))
                            .rounded_full()
                            .bg(Tone::Subtle.color())
                            .child(
                                view()
                                    .h_full()
                                    .w(relative(share))
                                    .rounded_full()
                                    .bg(tone.color()),
                            ),
                    ),
            );
        }
        v = v.child(rows);
        v.into_element()
    });
    let list = dynamic(move || {
        let p = page_data.get();
        if p.rows.is_empty() {
            return empty_state(
                Icon::Banknote,
                st.t("expenses.empty"),
                st.t("expenses.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, e) in p.rows.into_iter().enumerate() {
            let income = e.kind == "income";
            let title = if e.payee.is_empty() {
                e.category.clone()
            } else {
                format!("{} · {}", e.category, e.payee)
            };
            let detail = [
                e.number.clone(),
                date_time(st, e.created_at),
                st.tn(&format!("expenses.method.{}", e.method)),
                e.reference.clone(),
            ]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            let total = e.total_minor;
            let mut tail = view().flex_row().items_center().gap(px(8.0));
            if e.vat_minor > 0 && !compact {
                let vat = e.vat_minor;
                tail = tail.child(badge(Status::Neutral, None, move || {
                    tr_args(st.lang(), "expenses.vat_of", &[("amount", &money(st, vat))])
                }));
            }
            tail = tail.child(figure(
                move || {
                    if income {
                        format!("+{}", money(st, total))
                    } else {
                        format!("−{}", money(st, total))
                    }
                },
                ty::SM,
                FontWeight::BOLD,
                move || if income { Tone::Success } else { Tone::Ink },
            ));
            v = v.child(
                view()
                    .test_id(&format!("expense-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(16.0))
                    .py(px(11.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(icon(
                        if income { Icon::In } else { Icon::Out },
                        18.0,
                        move || if income { Tone::Success } else { Tone::Faint },
                    ))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
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
                    .child(tail),
            );
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
            st.t("expenses.search"),
            "expenses-search",
        )))
        .child(segmented(
            vec![
                (0, Box::new(st.t("expenses.kind.all"))),
                (1, Box::new(st.t("expenses.kind.expense"))),
                (2, Box::new(st.t("expenses.kind.income"))),
            ],
            kind,
        ))
        .child(segmented(
            vec![
                (0, Box::new(st.t("expenses.span.month"))),
                (1, Box::new(st.t("expenses.span.all"))),
            ],
            span,
        ));
    let body = view().gap(px(16.0)).child(tiles).child(by_category).child(
        card().child(filters).child(list).child(pager(
            st,
            paging,
            move || page_data.get().total,
            compact,
        )),
    );
    let actions = view().child(when(
        move || st.can("cash.expense"),
        move || {
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(8.0))
                .child(
                    button(
                        Variant::Ghost,
                        Size::Md,
                        Some(Icon::Tags),
                        st.t("expenses.category.new"),
                        move || adding.set(true),
                    )
                    .test_id("exp-category-new"),
                )
                .child(
                    button(
                        Variant::Secondary,
                        Size::Md,
                        Some(Icon::In),
                        st.t("expenses.new_income"),
                        move || recording.set(Some("income")),
                    )
                    .test_id("income-new"),
                )
                .child(
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Plus),
                        st.t("expenses.new"),
                        move || recording.set(Some("expense")),
                    )
                    .test_id("expense-new"),
                )
                .into_element()
        },
    ));
    let export = crate::screens::common::export_button(st, "expenses-export", move || {
        let q = ExpenseQuery {
            kind: KINDS[kind.get_untracked()].into(),
            from_ms: if span.get_untracked() == 0 {
                month_start()
            } else {
                0
            },
            to_ms: 0,
            search: text.get_untracked(),
            limit: 100_000,
            offset: 0,
        };
        crate::screens::common::export_from_db(
            st,
            "expenses.file",
            move |s| list_expenses(s, &q),
            move |p: ExpensePage| {
                let c = currency(st);
                let m = |v: i64| Money::from_minor(v, c).to_plain_string();
                let mut out = vec![vec![
                    st.tn("col.number"),
                    st.tn("col.date"),
                    st.tn("expenses.kind.all"),
                    st.tn("expenses.category"),
                    st.tn("expenses.method"),
                    st.tn("expenses.payee"),
                    st.tn("expenses.net"),
                    st.tn("expenses.vat"),
                    st.tn("col.amount"),
                    st.tn("expenses.reference"),
                ]];
                for e in p.rows {
                    out.push(vec![
                        e.number,
                        date_time(st, e.created_at),
                        st.tn(if e.kind == "income" {
                            "expenses.kind.income"
                        } else {
                            "expenses.kind.expense"
                        }),
                        e.category,
                        st.tn(&format!("expenses.method.{}", e.method)),
                        e.payee,
                        m(e.net_minor),
                        m(e.vat_minor),
                        m(e.total_minor),
                        e.reference,
                    ]);
                }
                out
            },
        );
    });
    view()
        .child(page(
            st,
            Route::Expenses,
            "expenses.subtitle",
            crate::screens::common::actions_row(vec![export, actions]),
            body,
            compact,
        ))
        .child(modal(recording_open, compact, move || {
            match recording.get_untracked() {
                Some(k) => expense_dialog(st, k, move || recording_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
        .child(modal(adding, compact, move || {
            category_dialog(st, move || adding.set(false)).into_element()
        }))
}

/// The till money moves through: this branch's first till with an open
/// session (else none: the bank only).
fn open_till(st: AppState) -> Memo<Option<String>> {
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
                    for r in all.into_iter().filter(|r| r.branch_id == branch) {
                        if nb_store::usecases::sales::current_shift(s, &r.id)?.is_some() {
                            return Ok(Some(r.id));
                        }
                    }
                    Ok(None)
                })
                .await
            }
        },
    );
    computed(move || r.with(|v: Option<&Option<String>>| v.cloned().flatten()))
}

fn expense_dialog(st: AppState, kind: &'static str, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let cur = session.currency;
    let meta = session.meta("expense.record");
    let cats = categories(st);
    let mine = computed(move || {
        cats.get()
            .into_iter()
            .filter(|c| c.kind == kind)
            .collect::<Vec<_>>()
    });
    let till = open_till(st);
    let chosen = signal(0usize);
    let total = signal(String::new());
    let vat = signal(String::new());
    // 0: cash (the open till), 1: bank.
    let method = signal(0usize);
    effect(move || {
        if till.get().is_none() {
            method.set(1);
        }
    });
    let payee = signal(String::new());
    let reference = signal(String::new());
    let note = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let save = {
        let close = close.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Some(cat) = mine.with_untracked(|v| v.get(chosen.get_untracked()).cloned()) else {
                error.set(error_text(st, "expense.unknown_category"));
                return;
            };
            let Ok(t) = parse_price(&total.get_untracked(), cur) else {
                error.set(error_text(st, "invalid_amount"));
                return;
            };
            let v = if vat.get_untracked().trim().is_empty() {
                0
            } else {
                match parse_price(&vat.get_untracked(), cur) {
                    Ok(m) => m.minor(),
                    Err(_) => {
                        error.set(error_text(st, "invalid_amount"));
                        return;
                    }
                }
            };
            let input = ExpenseInput {
                category_id: cat.id,
                total_minor: t.minor(),
                vat_minor: v,
                method: if method.get_untracked() == 0 {
                    "cash"
                } else {
                    "bank"
                }
                .into(),
                register_id: till.get_untracked(),
                payee: payee.get_untracked(),
                reference: reference.get_untracked(),
                note: note.get_untracked(),
            };
            busy.set(true);
            error.set(String::new());
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| record_expense(s, meta, input)).await {
                    Ok(e) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "expenses.recorded",
                                &[("number", &e.number), ("amount", &money(st, e.total_minor))],
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
    let two = |a: View, b: View| {
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(12.0))
            .child(view().flex_1().min_w(px(170.0)).child(a))
            .child(view().flex_1().min_w(px(170.0)).child(b))
    };
    let mut body = view().gap(px(14.0)).child(labelled(
        st.t("expenses.category"),
        dropdown(
            move || {
                mine.get()
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| (i, format!("{} · {}", c.name, c.account_code)))
                    .collect()
            },
            chosen,
            Some(Icon::Tags),
            st.t("expenses.category"),
            "exp-category",
        ),
        None,
    ));
    body = if kind == "expense" {
        body.child(two(
            field(
                st.t("expenses.amount"),
                total,
                String::new,
                String::new,
                KeyboardType::Decimal,
                "exp-total-input",
            ),
            field(
                st.t("expenses.vat"),
                vat,
                || "0".to_owned(),
                String::new,
                KeyboardType::Decimal,
                "exp-vat",
            ),
        ))
    } else {
        body.child(field(
            st.t("expenses.amount_in"),
            total,
            String::new,
            String::new,
            KeyboardType::Decimal,
            "exp-total-input",
        ))
    };
    body = body
        .child(labelled(
            st.t("expenses.method"),
            segmented(
                vec![
                    (0, Box::new(st.t("expenses.method.cash"))),
                    (1, Box::new(st.t("expenses.method.bank"))),
                ],
                method,
            ),
            Some(Box::new(move || {
                if till.get().is_none() {
                    st.tn("expenses.no_till")
                } else {
                    String::new()
                }
            })),
        ))
        .child(two(
            field(
                st.t(if kind == "expense" {
                    "expenses.payee"
                } else {
                    "expenses.payer"
                }),
                payee,
                String::new,
                String::new,
                KeyboardType::Default,
                "exp-payee",
            ),
            field(
                st.t("expenses.reference"),
                reference,
                String::new,
                String::new,
                KeyboardType::Default,
                "exp-reference",
            ),
        ))
        .child(field(
            st.t("expenses.note"),
            note,
            String::new,
            String::new,
            KeyboardType::Default,
            "exp-note",
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
                st.t("expenses.save"),
                save,
            )
            .test_id("exp-save"),
        );
    dialog_frame(
        st,
        st.t(if kind == "expense" {
            "expenses.new"
        } else {
            "expenses.new_income"
        }),
        st.t("expenses.dialog.help"),
        body,
        footer,
        close,
    )
}

/// A new category on an account of the chart.
fn category_dialog(st: AppState, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let meta = session.meta("expense.category");
    let kind = signal(0usize);
    let name = signal(String::new());
    let accounts_r = resource(
        move || st.session.get().map(|_| ()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| chart(s)).await }
        },
    );
    let accounts = computed(move || {
        let k = if kind.get() == 0 { "expense" } else { "income" };
        accounts_r
            .with(|v: Option<&Vec<ChartRow>>| v.cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.kind == k && !a.is_group && a.active)
            .collect::<Vec<_>>()
    });
    let chosen = signal(0usize);
    effect(move || {
        kind.with(|_| ());
        chosen.set(0);
    });
    let error = signal(String::new());
    let save = {
        let close = close.clone();
        move || {
            let Some(a) = accounts.with_untracked(|v| v.get(chosen.get_untracked()).cloned())
            else {
                return;
            };
            let k = if kind.get_untracked() == 0 {
                "expense"
            } else {
                "income"
            };
            let n = name.get_untracked();
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db
                    .call(move |s| create_category(s, meta, k, &n, &a.code))
                    .await
                {
                    Ok(c) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "expenses.category.created", &[("name", &c.name)]),
                        );
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let body = view()
        .gap(px(14.0))
        .child(segmented(
            vec![
                (0, Box::new(st.t("expenses.kind.expense"))),
                (1, Box::new(st.t("expenses.kind.income"))),
            ],
            kind,
        ))
        .child(field(
            st.t("expenses.category.name"),
            name,
            String::new,
            String::new,
            KeyboardType::Default,
            "cat-name",
        ))
        .child(labelled(
            st.t("expenses.category.account"),
            dropdown(
                move || {
                    accounts
                        .get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, a)| (i, format!("{} — {}", a.code, a.name)))
                        .collect()
                },
                chosen,
                Some(Icon::NotebookPen),
                st.t("expenses.category.account"),
                "cat-account",
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
                st.t("form.save"),
                save,
            )
            .test_id("cat-save"),
        );
    dialog_frame(
        st,
        st.t("expenses.category.new"),
        st.t("expenses.category.help"),
        body,
        footer,
        close,
    )
}

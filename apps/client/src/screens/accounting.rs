//! Accounting pages (M7): the journal (with manual entries and reversals),
//! the chart of accounts (with tills and banks, the accounts of automatic
//! entries, and fiscal periods), an account's ledger, and the trial
//! balance. They show and collect; the accounting use cases decide.

use nb_core::Money;
use nb_i18n::tr_args;
use nb_store::repo::fiscal::YearRow;
use nb_store::usecases::accounting::{
    AccountInput, ChartRow, EntryRow, JournalPage, JournalQuery, MoneyAccountRow, RoleRow,
    Statement, TrialBalance, account_roles, chart, close_period, create_account,
    create_money_account, fiscal_years, journal, money_accounts, reopen_period, set_account_role,
    statement, trial_balance, update_account,
};
use nb_store::usecases::ledger::{EntryLineInput, post_entry, reverse_entry};
use nsh_core::element::{AnyElement, KeyboardType, Owner, View};
use nsh_core::prelude::*;

use crate::db::DbError;
use crate::nav::Route;
use crate::screens::common::page;
use crate::screens::forms::{error_text, parse_price};
use crate::state::AppState;
use crate::ui::form::{field, labelled, row_input, search_field};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, figure, icon_button, label, label_line, segmented, switch,
};
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon, empty_state, kpi};
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

/// An amount without its currency (columns), or a dash for nothing.
fn amount(st: AppState, minor: i64) -> String {
    if minor == 0 {
        return "—".into();
    }
    st.fmt().amount(Money::from_minor(minor, currency(st)))
}

fn money(st: AppState, minor: i64) -> String {
    st.fmt().money(Money::from_minor(minor, currency(st)))
}

/// `YYYY-MM-DD` as the reader writes dates.
fn date(st: AppState, iso: &str) -> String {
    match nb_domain::fiscal::Date::parse(iso) {
        Ok(d) => st.fmt().date(d.y, d.m, d.d),
        Err(_) => iso.to_owned(),
    }
}

fn today() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
    nb_store::time::iso_date(ms, 60)
}

/// The source of an entry, in words.
fn source_text(st: AppState, source: &str) -> String {
    let key = format!("acc.source.{source}");
    let t = st.tn(&key);
    if t == key { source.to_owned() } else { t }
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

/// The chart, reloaded when data changes.
fn chart_memo(st: AppState) -> Memo<Vec<ChartRow>> {
    let r = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| chart(s)).await }
        },
    );
    computed(move || {
        r.with(|v: Option<&Vec<ChartRow>>| v.cloned())
            .unwrap_or_default()
    })
}

fn account_label(a: &ChartRow) -> String {
    format!("{} — {}", a.code, a.name)
}

/// A period: this month, this year, everything, or typed dates.
#[derive(Clone, Copy)]
struct Span {
    from: Signal<String>,
    to: Signal<String>,
}

impl Span {
    fn this_year() -> Self {
        let t = today();
        Self {
            from: signal(format!("{}-01-01", &t[..4])),
            to: signal(String::new()),
        }
    }
    /// The bounds the use cases take (ignored until a date is complete).
    fn bounds(self) -> (Option<String>, Option<String>) {
        let f = |s: Signal<String>| {
            let v = s.get().trim().to_owned();
            (v.len() == 10).then_some(v)
        };
        (f(self.from), f(self.to))
    }
}

fn span_bar(st: AppState, span: Span, compact: bool) -> View {
    let t = today();
    let month = format!("{}-01", &t[..7]);
    let year = format!("{}-01-01", &t[..4]);
    let mut chips = view().flex_row().flex_wrap().gap(px(6.0));
    for (key, from, id) in [
        ("acc.span.month", month, "span-month"),
        ("acc.span.year", year, "span-year"),
        ("acc.span.all", String::new(), "span-all"),
    ] {
        let f2 = from.clone();
        let active = computed(move || span.from.get() == f2 && span.to.get().is_empty());
        chips = chips.child(
            view()
                .test_id(id)
                .h(px(30.0))
                .px(px(12.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .cursor_pointer()
                .border_color(move || {
                    if active.get() {
                        Tone::Primary.color()
                    } else {
                        Tone::Border.color()
                    }
                })
                .bg(move || {
                    if active.get() {
                        Tone::PrimarySoft.color()
                    } else {
                        Tone::Surface.color()
                    }
                })
                .on_press(move || {
                    span.from.set(from.clone());
                    span.to.set(String::new());
                })
                .child(label_line(st.t(key), ty::SM, FontWeight::MEDIUM, Tone::Ink)),
        );
    }
    let dates = view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(
            view()
                .w(px(if compact { 120.0 } else { 130.0 }))
                .child(row_input(
                    span.from,
                    st.t("acc.from"),
                    KeyboardType::Default,
                    "span-from".into(),
                    || false,
                )),
        )
        .child(label_line(
            || "→".to_owned(),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(
            view()
                .w(px(if compact { 120.0 } else { 130.0 }))
                .child(row_input(
                    span.to,
                    st.t("acc.to"),
                    KeyboardType::Default,
                    "span-to".into(),
                    || false,
                )),
        );
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
        .child(chips)
        .child(dates)
}

/// A balance in a column: the amount, then its side (debit or credit) as
/// a word of its own — one text would mix the number with the word and
/// let Arabic reorder the digit groups.
fn balance_cell(st: AppState, minor: i64, w: f32, weight: FontWeight, tone: Tone) -> View {
    let mut row = view()
        .w(px(w))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(px(4.0));
    if minor == 0 {
        return row.child(figure(|| "—".to_owned(), ty::SM, weight, move || tone));
    }
    row = row.child(figure(
        move || {
            st.fmt()
                .amount(Money::from_minor(minor.abs(), currency(st)))
        },
        ty::SM,
        weight,
        move || tone,
    ));
    row.child(label_line(
        st.t(if minor > 0 { "acc.dr" } else { "acc.cr" }),
        ty::XS,
        FontWeight::MEDIUM,
        Tone::Muted,
    ))
}

/// A row of figures: a text cell then fixed amount columns.
fn amount_cell(
    text: impl Fn() -> String + 'static,
    w: f32,
    weight: FontWeight,
    tone: Tone,
) -> View {
    view()
        .w(px(w))
        .flex_shrink_0()
        .items_end()
        .child(figure(text, ty::SM, weight, move || tone))
}

// ------------------------------------------------------------------ journal

pub fn journal_page(st: AppState, compact: bool) -> View {
    let text = signal(String::new());
    let span = Span::this_year();
    let paging = Paging::new(25);
    effect(move || {
        text.with(|_| ());
        span.from.with(|_| ());
        span.to.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            let (from, to) = span.bounds();
            Some((
                JournalQuery {
                    text: text.get(),
                    from,
                    to,
                    limit: paging.size.get(),
                    offset: paging.offset(),
                    ..Default::default()
                },
                st.data_rev.get(),
            ))
        },
        move |(q, _), _| {
            let db = st.db();
            async move { db.call(move |s| journal(s, &q)).await }
        },
    );
    let page_data = computed(move || {
        data.with(|p: Option<&JournalPage>| p.cloned())
            .unwrap_or_default()
    });
    let creating = signal(false);
    let reversing: Signal<Option<EntryRow>> = signal(None);
    let reversing_open = signal(false);
    effect(move || reversing_open.set(reversing.get().is_some()));
    effect(move || {
        if !reversing_open.get() {
            reversing.set(None);
        }
    });
    // Permissions load after the page is built: the actions follow them.
    let actions = view().child(when(
        move || st.can("accounting.post"),
        move || {
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Plus),
                st.t("acc.entry.new"),
                move || creating.set(true),
            )
            .test_id("entry-new")
            .into_element()
        },
    ));
    let may_reverse = computed(move || st.can("accounting.reverse"));
    let list = dynamic(move || {
        let p = page_data.get();
        if p.rows.is_empty() {
            return empty_state(
                Icon::NotebookPen,
                st.t("acc.journal.empty"),
                st.t("acc.journal.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, e) in p.rows.into_iter().enumerate() {
            v = v.child(entry_card(st, i, e, compact, may_reverse.get(), reversing));
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
                    st.t("acc.journal.search"),
                    "journal-search",
                )))
                .child(span_bar(st, span, compact)),
        )
        .child(list)
        .child(pager(st, paging, move || page_data.get().total, compact));
    view()
        .child(page(
            st,
            Route::Journal,
            "acc.journal.subtitle",
            actions,
            body,
            compact,
        ))
        .child(modal(creating, compact, move || {
            entry_dialog(st, move || creating.set(false)).into_element()
        }))
        .child(modal(reversing_open, compact, move || {
            match reversing.get_untracked() {
                Some(e) => reverse_dialog(st, e, move || reversing_open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
}

fn entry_card(
    st: AppState,
    i: usize,
    e: EntryRow,
    compact: bool,
    may_reverse: bool,
    reversing: Signal<Option<EntryRow>>,
) -> View {
    let w = if compact { 92.0 } else { 130.0 };
    let head = {
        let (number, when, memo, source, total) = (
            e.number.clone(),
            date(st, &e.date),
            e.memo.clone(),
            source_text(st, &e.source),
            e.total_minor,
        );
        let mut h = view().flex_row().items_center().gap(px(10.0)).child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(label_line(
                    move || format!("{number} · {when}"),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    move || memo.clone(),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        );
        if !compact {
            h = h.child(badge(Status::Neutral, None, move || source.clone()));
        }
        if let Some(n) = e.reversed_by.clone() {
            h = h.child(badge(Status::Warning, Some(Icon::Undo), move || {
                tr_args(st.lang(), "acc.reversed_by", &[("number", &n)])
            }));
        }
        if let Some(n) = e.reverses.clone() {
            h = h.child(badge(Status::Info, Some(Icon::Undo), move || {
                tr_args(st.lang(), "acc.reverses", &[("number", &n)])
            }));
        }
        h = h.child(figure(
            move || money(st, total),
            ty::SM,
            FontWeight::SEMIBOLD,
            || Tone::Ink,
        ));
        if may_reverse && e.reversed_by.is_none() && e.reverses.is_none() {
            let entry = e.clone();
            h = h.child(
                icon_button(
                    || Icon::Undo,
                    st.t("acc.reverse"),
                    Variant::Ghost,
                    move || reversing.set(Some(entry.clone())),
                )
                .test_id(&format!("entry-reverse-{i}")),
            );
        }
        h
    };
    let mut lines = view().gap(px(4.0)).ps(px(if compact { 0.0 } else { 16.0 }));
    for l in e.lines {
        let name = format!("{} {}", l.account, l.account_name);
        let credit_side = l.credit_minor > 0;
        let (d, c) = (l.debit_minor, l.credit_minor);
        lines = lines.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        // Credits sit a step in, as in a journal.
                        .ps(px(if credit_side { 24.0 } else { 0.0 }))
                        .child(label_line(
                            move || name.clone(),
                            ty::SM,
                            FontWeight::NORMAL,
                            Tone::Body,
                        )),
                )
                .child(amount_cell(
                    move || amount(st, d),
                    w,
                    FontWeight::NORMAL,
                    Tone::Ink,
                ))
                .child(amount_cell(
                    move || amount(st, c),
                    w,
                    FontWeight::NORMAL,
                    Tone::Ink,
                )),
        );
    }
    view()
        .test_id(&format!("entry-{i}"))
        .gap(px(10.0))
        .px(px(16.0))
        .py(px(12.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(head)
        .child(lines)
}

#[derive(Clone, Copy, PartialEq)]
struct LineForm {
    /// Index in the accounts that take entries, plus one (0: none).
    account: Signal<usize>,
    debit: Signal<String>,
    credit: Signal<String>,
}

impl LineForm {
    fn new() -> Self {
        Self {
            account: signal(0),
            debit: signal(String::new()),
            credit: signal(String::new()),
        }
    }
}

/// Test ids of the first lines' account choices.
const ENTRY_ACCOUNT_IDS: [&str; 6] = [
    "entry-account-0",
    "entry-account-1",
    "entry-account-2",
    "entry-account-3",
    "entry-account-4",
    "entry-account-5",
];

fn entry_dialog(st: AppState, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let owner = Owner::current();
    let all = chart_memo(st);
    let leaves = computed(move || {
        all.get()
            .into_iter()
            .filter(|a| !a.is_group && a.active)
            .collect::<Vec<_>>()
    });
    let memo = signal(String::new());
    let lines = signal(vec![LineForm::new(), LineForm::new()]);
    let error = signal(String::new());
    let cur = session.currency;
    let parse = move |s: Signal<String>| -> Option<i64> {
        let v = s.get();
        if v.trim().is_empty() {
            return Some(0);
        }
        parse_price(&v, cur).ok().map(|m| m.minor())
    };
    let totals = computed(move || {
        lines.get().iter().fold((0i64, 0i64), |(d, c), l| {
            (
                d + parse(l.debit).unwrap_or(0),
                c + parse(l.credit).unwrap_or(0),
            )
        })
    });
    let meta = session.meta("journal.post");
    let save = {
        let close = close.clone();
        move || {
            let accounts = leaves.get_untracked();
            let mut input = Vec::new();
            for l in lines.get_untracked() {
                let (Some(d), Some(c)) = (parse(l.debit), parse(l.credit)) else {
                    error.set(error_text(st, "invalid_amount"));
                    return;
                };
                if d == 0 && c == 0 {
                    continue;
                }
                let Some(a) = l
                    .account
                    .get_untracked()
                    .checked_sub(1)
                    .and_then(|i| accounts.get(i))
                else {
                    error.set(error_text(st, "ledger.account_missing"));
                    return;
                };
                input.push(EntryLineInput {
                    account: a.code.clone(),
                    debit_minor: d,
                    credit_minor: c,
                });
            }
            let text = memo.get_untracked();
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| post_entry(s, meta, &text, input)).await {
                    Ok(e) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "acc.entry.posted", &[("number", &e.number)]),
                        );
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let rows = dynamic(move || {
        let mut v = view().gap(px(8.0));
        for (i, l) in lines.get().into_iter().enumerate() {
            v = v.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(view().flex_1().min_w(px(0.0)).child(dropdown(
                        move || {
                            std::iter::once((0, st.tn("acc.entry.pick_account")))
                                .chain(
                                    leaves
                                        .get()
                                        .iter()
                                        .enumerate()
                                        .map(|(i, a)| (i + 1, account_label(a))),
                                )
                                .collect()
                        },
                        l.account,
                        None,
                        st.t("acc.account"),
                        ENTRY_ACCOUNT_IDS.get(i).copied().unwrap_or("entry-account"),
                    )))
                    .child(view().w(px(110.0)).child(row_input(
                        l.debit,
                        st.t("acc.debit"),
                        KeyboardType::Decimal,
                        format!("entry-debit-{i}"),
                        move || parse(l.debit).is_none(),
                    )))
                    .child(view().w(px(110.0)).child(row_input(
                        l.credit,
                        st.t("acc.credit"),
                        KeyboardType::Decimal,
                        format!("entry-credit-{i}"),
                        move || parse(l.credit).is_none(),
                    )))
                    .child(icon_button(
                        || Icon::Trash,
                        st.t("action.delete"),
                        Variant::Ghost,
                        move || {
                            lines.update(|v| {
                                if v.len() > 2 && i < v.len() {
                                    v.remove(i);
                                }
                            })
                        },
                    )),
            );
        }
        v.into_element()
    });
    let summary = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(12.0))
        .child(label_line(
            move || {
                let (d, c) = totals.get();
                format!(
                    "{} {} · {} {}",
                    st.tn("acc.debit"),
                    money(st, d),
                    st.tn("acc.credit"),
                    money(st, c)
                )
            },
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Ink,
        ))
        .child(dynamic(move || {
            let (d, c) = totals.get();
            if d == c && d > 0 {
                badge(
                    Status::Success,
                    Some(Icon::Check),
                    st.t("acc.entry.balanced"),
                )
                .into_element()
            } else {
                badge(Status::Danger, None, move || {
                    tr_args(
                        st.lang(),
                        "acc.entry.gap",
                        &[("amount", &money(st, (d - c).abs()))],
                    )
                })
                .into_element()
            }
        }));
    let body = view()
        .gap(px(14.0))
        .child(field(
            st.t("acc.entry.memo"),
            memo,
            st.t("acc.entry.memo_hint"),
            String::new,
            KeyboardType::Default,
            "entry-memo",
        ))
        .child(rows)
        .child(
            view().flex_row().child(
                button(
                    Variant::Ghost,
                    Size::Sm,
                    Some(Icon::Plus),
                    st.t("acc.entry.add_line"),
                    move || {
                        let l = owner.run(LineForm::new);
                        lines.update(|v| v.push(l));
                    },
                )
                .test_id("entry-add-line"),
            ),
        )
        .child(summary)
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
                st.t("acc.entry.post"),
                save,
            )
            .test_id("entry-post"),
        );
    dialog_frame(
        st,
        st.t("acc.entry.new"),
        st.t("acc.entry.help"),
        body,
        footer,
        close,
    )
}

fn reverse_dialog(st: AppState, e: EntryRow, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let reason = signal(String::new());
    let error = signal(String::new());
    let meta = session.meta("journal.reverse");
    let id = e.id.clone();
    let save = {
        let close = close.clone();
        move || {
            let (id, why, close) = (id.clone(), reason.get_untracked(), close.clone());
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| reverse_entry(s, meta, &id, &why)).await {
                    Ok(r) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "acc.entry.posted", &[("number", &r.number)]),
                        );
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let number = e.number.clone();
    let body = view()
        .gap(px(14.0))
        .child(label(
            move || tr_args(st.lang(), "acc.reverse.body", &[("number", &number)]),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Body,
        ))
        .child(field(
            st.t("acc.reason"),
            reason,
            String::new,
            String::new,
            KeyboardType::Default,
            "reverse-reason",
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
                Variant::Danger,
                Size::Md,
                Some(Icon::Undo),
                st.t("acc.reverse"),
                save,
            )
            .test_id("reverse-confirm"),
        );
    dialog_frame(
        st,
        st.t("acc.reverse"),
        st.t("acc.reverse.help"),
        body,
        footer,
        close,
    )
}

// ------------------------------------------------------------------ chart

pub fn accounts_page(st: AppState, compact: bool) -> View {
    let rows = chart_memo(st);
    let may_edit = computed(move || st.can("accounting.chart"));
    let creating = signal(false);
    let editing: Signal<Option<ChartRow>> = signal(None);
    let editing_open = signal(false);
    effect(move || editing_open.set(editing.get().is_some()));
    effect(move || {
        if !editing_open.get() {
            editing.set(None);
        }
    });
    let actions = view().child(when(may_edit, move || {
        button(
            Variant::Primary,
            Size::Md,
            Some(Icon::Plus),
            st.t("acc.account.new"),
            move || creating.set(true),
        )
        .test_id("account-new")
        .into_element()
    }));
    let tree = dynamic(move || {
        let all = rows.get();
        let mut v = view();
        for (i, a) in all.into_iter().enumerate() {
            v = v.child(chart_row(st, i, a, may_edit.get(), editing));
        }
        v.into_element()
    });
    let chart_card = card()
        .child(card_header_icon(
            Some(Icon::ListTree),
            st.t("nav.accounts"),
            Some(Box::new(st.t("acc.chart.help"))),
            view(),
        ))
        .child(tree);
    let side = view()
        .gap(px(16.0))
        .child(money_card(st, compact, may_edit))
        .child(roles_card(st, compact, may_edit, rows))
        .child(periods_card(st, compact));
    let body = if compact {
        view().gap(px(16.0)).child(chart_card).child(side)
    } else {
        view()
            .flex_row()
            .items_start()
            .gap(px(20.0))
            .child(
                view()
                    .flex_grow(3.0)
                    .basis(px(0.0))
                    .min_w(px(420.0))
                    .child(chart_card),
            )
            .child(
                view()
                    .flex_grow(2.0)
                    .basis(px(0.0))
                    .min_w(px(320.0))
                    .child(side),
            )
    };
    view()
        .child(page(
            st,
            Route::Accounts,
            "acc.chart.subtitle",
            actions,
            body,
            compact,
        ))
        .child(modal(creating, compact, move || {
            account_dialog(st, rows, move || creating.set(false)).into_element()
        }))
        .child(modal(editing_open, compact, move || {
            match editing.get_untracked() {
                Some(a) => {
                    edit_account_dialog(st, a, move || editing_open.set(false)).into_element()
                }
                None => view().into_element(),
            }
        }))
}

fn chart_row(
    st: AppState,
    i: usize,
    a: ChartRow,
    may_edit: bool,
    editing: Signal<Option<ChartRow>>,
) -> View {
    let group = a.is_group;
    let (code, name, bal, depth) = (a.code.clone(), a.name.clone(), a.balance_minor(), a.depth);
    let tone = if a.active { Tone::Ink } else { Tone::Faint };
    let mut row = view()
        .test_id(&format!("account-{}", a.code))
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(if group { 10.0 } else { 8.0 }))
        .border_t_1()
        .border_color(Tone::Border.color())
        .bg(if depth == 0 {
            Tone::Subtle.color()
        } else {
            Tone::Surface.color()
        })
        .child(
            view()
                .w(px(f32::from(u16::try_from(depth).unwrap_or(8)) * 16.0))
                .flex_shrink_0(),
        )
        .child(view().w(px(56.0)).flex_shrink_0().child(label_line(
            move || code.clone(),
            ty::SM,
            if group {
                FontWeight::BOLD
            } else {
                FontWeight::MEDIUM
            },
            tone,
        )))
        .child(view().flex_1().min_w(px(0.0)).child(label_line(
            move || name.clone(),
            ty::SM,
            if group {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            },
            tone,
        )));
    for r in a.roles.iter().take(2) {
        let key = format!("role.{r}");
        row = row.child(badge(Status::Info, None, move || st.tn(&key)));
    }
    if !a.active {
        row = row.child(badge(Status::Neutral, None, st.t("acc.inactive")));
    }
    row = row.child(balance_cell(
        st,
        bal,
        130.0,
        if group {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        },
        tone,
    ));
    if may_edit {
        let a2 = a.clone();
        row = row
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Hover.color()))
            .on_press(move || editing.set(Some(a2.clone())));
    }
    let _ = i;
    row
}

fn account_dialog(
    st: AppState,
    rows: Memo<Vec<ChartRow>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let groups = computed(move || {
        rows.get()
            .into_iter()
            .filter(|a| a.is_group && a.active)
            .collect::<Vec<_>>()
    });
    let code = signal(String::new());
    let name = signal(String::new());
    let parent = signal(0usize);
    let is_group = signal(false);
    let error = signal(String::new());
    let meta = session.meta("account.create");
    let save = {
        let close = close.clone();
        move || {
            let input = AccountInput {
                code: code.get_untracked(),
                name: name.get_untracked(),
                parent_code: parent
                    .get_untracked()
                    .checked_sub(1)
                    .and_then(|i| groups.get_untracked().get(i).map(|g| g.code.clone())),
                kind: None,
                is_group: is_group.get_untracked(),
            };
            let close = close.clone();
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| create_account(s, meta, input)).await {
                    Ok(a) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(st.lang(), "acc.account.created", &[("code", &a.code)]),
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
        .child(labelled(
            st.t("acc.account.parent"),
            dropdown(
                move || {
                    std::iter::once((0, st.tn("acc.account.no_parent")))
                        .chain(
                            groups
                                .get()
                                .iter()
                                .enumerate()
                                .map(|(i, g)| (i + 1, account_label(g))),
                        )
                        .collect()
                },
                parent,
                Some(Icon::FolderTree),
                st.t("acc.account.parent"),
                "account-parent",
            ),
            Some(Box::new(st.t("acc.account.parent_help"))),
        ))
        .child(field(
            st.t("acc.account.code"),
            code,
            || "6581".to_owned(),
            String::new,
            KeyboardType::Number,
            "account-code",
        ))
        .child(field(
            st.t("acc.account.name"),
            name,
            String::new,
            String::new,
            KeyboardType::Default,
            "account-name",
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(switch(is_group, st.t("acc.account.is_group")))
                .child(label_line(
                    st.t("acc.account.is_group"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Body,
                )),
        )
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
            .test_id("account-save"),
        );
    dialog_frame(
        st,
        st.t("acc.account.new"),
        st.t("acc.account.help"),
        body,
        footer,
        close,
    )
}

fn edit_account_dialog(st: AppState, a: ChartRow, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let name = signal(a.name.clone());
    let active = signal(a.active);
    let error = signal(String::new());
    let meta = session.meta("account.update");
    let code = a.code.clone();
    let save = {
        let close = close.clone();
        move || {
            let (code, n, on, close) = (
                code.clone(),
                name.get_untracked(),
                active.get_untracked(),
                close.clone(),
            );
            let db = st.db();
            let task = spawn(async move {
                match db
                    .call(move |s| update_account(s, meta, &code, &n, on))
                    .await
                {
                    Ok(_) => {
                        st.changed();
                        close();
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let body = view()
        .gap(px(14.0))
        .child(field(
            st.t("acc.account.name"),
            name,
            String::new,
            String::new,
            KeyboardType::Default,
            "account-edit-name",
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(switch(active, st.t("acc.active")))
                .child(label_line(
                    st.t("acc.active"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Body,
                )),
        )
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
            .test_id("account-edit-save"),
        );
    let title = format!("{} — {}", a.code, a.name);
    dialog_frame(
        st,
        move || title.clone(),
        st.t("acc.account.edit_help"),
        body,
        footer,
        close,
    )
}

fn money_card(st: AppState, compact: bool, may_edit: Memo<bool>) -> View {
    let data = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| money_accounts(s)).await }
        },
    );
    let adding = signal(false);
    let list = dynamic(move || {
        let all = data
            .with(|v: Option<&Vec<MoneyAccountRow>>| v.cloned())
            .unwrap_or_default();
        let mut v = view();
        for m in all {
            let (name, code, bal) = (m.name.clone(), m.account_code.clone(), m.balance_minor);
            v = v.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(16.0))
                    .py(px(10.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(icon(
                        if m.kind == "cash" {
                            Icon::Wallet
                        } else {
                            Icon::Landmark
                        },
                        18.0,
                        || Tone::Muted,
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
                                move || code.clone(),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(figure(
                        move || money(st, bal),
                        ty::SM,
                        FontWeight::SEMIBOLD,
                        || Tone::Ink,
                    )),
            );
        }
        v.into_element()
    });
    let actions = view().child(when(may_edit, move || {
        button(
            Variant::Secondary,
            Size::Sm,
            Some(Icon::Plus),
            st.t("action.add"),
            move || adding.set(true),
        )
        .test_id("money-new")
        .into_element()
    }));
    card()
        .child(card_header_icon(
            Some(Icon::Wallet),
            st.t("acc.money.title"),
            Some(Box::new(st.t("acc.money.help"))),
            actions,
        ))
        .child(list)
        .child(modal(adding, compact, move || {
            money_dialog(st, move || adding.set(false)).into_element()
        }))
}

fn money_dialog(st: AppState, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let bank = signal(false);
    let name = signal(String::new());
    let error = signal(String::new());
    let meta = session.meta("money_account.create");
    let save = {
        let close = close.clone();
        move || {
            let kind = if bank.get_untracked() { "bank" } else { "cash" };
            let (n, close) = (name.get_untracked(), close.clone());
            let db = st.db();
            let task = spawn(async move {
                match db
                    .call(move |s| create_money_account(s, meta, kind, &n, None))
                    .await
                {
                    Ok(m) => {
                        st.changed();
                        close();
                        show_toast(
                            st,
                            tr_args(
                                st.lang(),
                                "acc.account.created",
                                &[("code", &m.account_code)],
                            ),
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
                (
                    false,
                    Box::new(st.t("acc.money.cash")) as Box<dyn Fn() -> String>,
                ),
                (true, Box::new(st.t("acc.money.bank"))),
            ],
            bank,
        ))
        .child(field(
            st.t("acc.account.name"),
            name,
            st.t("acc.money.name_hint"),
            String::new,
            KeyboardType::Default,
            "money-name",
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
            .test_id("money-save"),
        );
    dialog_frame(
        st,
        st.t("acc.money.new"),
        st.t("acc.money.help"),
        body,
        footer,
        close,
    )
}

fn roles_card(
    st: AppState,
    compact: bool,
    may_edit: Memo<bool>,
    rows: Memo<Vec<ChartRow>>,
) -> View {
    let data = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| account_roles(s)).await }
        },
    );
    let editing: Signal<Option<RoleRow>> = signal(None);
    let open = signal(false);
    effect(move || open.set(editing.get().is_some()));
    effect(move || {
        if !open.get() {
            editing.set(None);
        }
    });
    let list = dynamic(move || {
        let all = data
            .with(|v: Option<&Vec<RoleRow>>| v.cloned())
            .unwrap_or_default();
        let mut v = view();
        for r in all {
            let key = format!("role.{}", r.role);
            let target = match (&r.account_code, &r.account_name) {
                (Some(c), Some(n)) => format!("{c} — {n}"),
                (Some(c), None) => c.clone(),
                _ => "—".into(),
            };
            let mut row = view()
                .test_id(&format!("role-{}", r.role))
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(16.0))
                .py(px(8.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(view().flex_1().min_w(px(0.0)).child(label_line(
                    move || st.tn(&key),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Ink,
                )))
                .child(label_line(
                    move || target.clone(),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ));
            if may_edit.get() {
                row = row
                    .cursor_pointer()
                    .hover(|s| s.bg(Tone::Hover.color()))
                    .on_press(move || editing.set(Some(r.clone())));
            }
            v = v.child(row);
        }
        v.into_element()
    });
    card()
        .child(card_header_icon(
            Some(Icon::Sliders),
            st.t("acc.roles.title"),
            Some(Box::new(st.t("acc.roles.help"))),
            view(),
        ))
        .child(list)
        .child(modal(open, compact, move || {
            match editing.get_untracked() {
                Some(r) => role_dialog(st, r, rows, move || open.set(false)).into_element(),
                None => view().into_element(),
            }
        }))
}

fn role_dialog(
    st: AppState,
    r: RoleRow,
    rows: Memo<Vec<ChartRow>>,
    close: impl Fn() + Clone + 'static,
) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let leaves = computed(move || {
        rows.get()
            .into_iter()
            .filter(|a| !a.is_group && a.active)
            .collect::<Vec<_>>()
    });
    let current = r.account_code.clone().unwrap_or_default();
    let chosen = signal(
        leaves
            .get_untracked()
            .iter()
            .position(|a| a.code == current)
            .unwrap_or(0),
    );
    let error = signal(String::new());
    let meta = session.meta("account.role");
    let role = r.role.clone();
    let save = {
        let close = close.clone();
        move || {
            let Some(a) = leaves.get_untracked().get(chosen.get_untracked()).cloned() else {
                return;
            };
            let (role, close) = (role.clone(), close.clone());
            let db = st.db();
            let task = spawn(async move {
                match db
                    .call(move |s| set_account_role(s, meta, &role, &a.code))
                    .await
                {
                    Ok(()) => {
                        st.changed();
                        close();
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let body = view()
        .gap(px(14.0))
        .child(labelled(
            st.t("acc.account"),
            dropdown(
                move || {
                    leaves
                        .get()
                        .iter()
                        .enumerate()
                        .map(|(i, a)| (i, account_label(a)))
                        .collect()
                },
                chosen,
                None,
                st.t("acc.account"),
                "role-account",
            ),
            Some(Box::new(st.t("acc.roles.change_help"))),
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
            .test_id("role-save"),
        );
    let key = format!("role.{}", r.role);
    dialog_frame(
        st,
        move || st.tn(&key),
        st.t("acc.roles.help"),
        body,
        footer,
        close,
    )
}

fn periods_card(st: AppState, compact: bool) -> View {
    let data = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move { db.call(|s| fiscal_years(s)).await }
        },
    );
    let may_close = computed(move || st.can("accounting.close_period"));
    let reopening: Signal<Option<(String, u32)>> = signal(None);
    let reopen_open = signal(false);
    effect(move || reopen_open.set(reopening.get().is_some()));
    effect(move || {
        if !reopen_open.get() {
            reopening.set(None);
        }
    });
    let body = dynamic(move || {
        let years = data
            .with(|v: Option<&Vec<YearRow>>| v.cloned())
            .unwrap_or_default();
        let Some(year) = years.into_iter().next() else {
            return view()
                .p(px(16.0))
                .child(label(
                    st.t("acc.periods.none"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .into_element();
        };
        let mut grid = view().flex_row().flex_wrap().gap(px(6.0)).p(px(16.0));
        for p in &year.periods {
            let closed = p.closed;
            let m = nb_domain::fiscal::Date::parse(&p.start).map_or(1, |d| d.m);
            grid = grid.child(
                view()
                    .test_id(&format!("period-{}", p.number))
                    .w(px(64.0))
                    .h(px(44.0))
                    .items_center()
                    .justify_center()
                    .gap(px(2.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(Tone::Border.color())
                    .bg(if closed {
                        Tone::Subtle.color()
                    } else {
                        Tone::Surface.color()
                    })
                    .child(label_line(
                        move || st.fmt().month_short(m).to_owned(),
                        ty::XS,
                        FontWeight::SEMIBOLD,
                        if closed { Tone::Muted } else { Tone::Ink },
                    ))
                    .child(icon(
                        if closed { Icon::Lock } else { Icon::Calendar },
                        12.0,
                        move || {
                            if closed { Tone::Muted } else { Tone::Success }
                        },
                    )),
            );
        }
        let mut actions = view()
            .flex_row()
            .flex_wrap()
            .gap(px(8.0))
            .px(px(16.0))
            .pb(px(16.0));
        if may_close.get() {
            if let Some(next) = year.periods.iter().find(|p| !p.closed) {
                let (id, n) = (next.id.clone(), next.number);
                actions = actions.child(
                    button(
                        Variant::Secondary,
                        Size::Sm,
                        Some(Icon::Lock),
                        move || {
                            tr_args(
                                st.lang(),
                                "acc.periods.close",
                                &[("period", &n.to_string())],
                            )
                        },
                        move || {
                            let id = id.clone();
                            let Some(meta) =
                                st.session.get_untracked().map(|s| s.meta("period.close"))
                            else {
                                return;
                            };
                            let db = st.db();
                            let task = spawn(async move {
                                match db.call(move |s| close_period(s, meta, &id)).await {
                                    Ok(()) => st.changed(),
                                    Err(e) => show_toast(st, first_error(st, &e)),
                                }
                            });
                            drop(task); // runs to completion: tasks end with their owner, not their handle
                        },
                    )
                    .test_id("period-close"),
                );
            }
            if let Some(last) = year.periods.iter().rev().find(|p| p.closed) {
                let (id, n) = (last.id.clone(), last.number);
                actions = actions.child(
                    button(
                        Variant::Ghost,
                        Size::Sm,
                        Some(Icon::Undo),
                        move || {
                            tr_args(
                                st.lang(),
                                "acc.periods.reopen",
                                &[("period", &n.to_string())],
                            )
                        },
                        move || reopening.set(Some((id.clone(), n))),
                    )
                    .test_id("period-reopen"),
                );
            }
        }
        let name = year.name.clone();
        view()
            .child(view().px(px(16.0)).pt(px(12.0)).child(label_line(
                move || tr_args(st.lang(), "acc.periods.year", &[("year", &name)]),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            )))
            .child(grid)
            .child(actions)
            .into_element()
    });
    card()
        .child(card_header_icon(
            Some(Icon::CalendarClock),
            st.t("acc.periods.title"),
            Some(Box::new(st.t("acc.periods.help"))),
            view(),
        ))
        .child(body)
        .child(modal(reopen_open, compact, move || {
            match reopening.get_untracked() {
                Some((id, n)) => {
                    reopen_dialog(st, id, n, move || reopen_open.set(false)).into_element()
                }
                None => view().into_element(),
            }
        }))
}

fn reopen_dialog(st: AppState, id: String, n: u32, close: impl Fn() + Clone + 'static) -> View {
    let Some(session) = st.session.get_untracked() else {
        return view();
    };
    let reason = signal(String::new());
    let error = signal(String::new());
    let meta = session.meta("period.reopen");
    let save = {
        let close = close.clone();
        move || {
            let (id, why, close) = (id.clone(), reason.get_untracked(), close.clone());
            let db = st.db();
            let task = spawn(async move {
                match db.call(move |s| reopen_period(s, meta, &id, &why)).await {
                    Ok(()) => {
                        st.changed();
                        close();
                    }
                    Err(e) => error.set(first_error(st, &e)),
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let body = view()
        .gap(px(14.0))
        .child(field(
            st.t("acc.reason"),
            reason,
            String::new,
            String::new,
            KeyboardType::Default,
            "reopen-reason",
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
                Variant::Danger,
                Size::Md,
                Some(Icon::Undo),
                st.t("acc.periods.reopen_confirm"),
                save,
            )
            .test_id("reopen-confirm"),
        );
    dialog_frame(
        st,
        move || {
            tr_args(
                st.lang(),
                "acc.periods.reopen",
                &[("period", &n.to_string())],
            )
        },
        st.t("acc.periods.reopen_help"),
        body,
        footer,
        close,
    )
}

// ------------------------------------------------------------------ ledger

pub fn ledger_page(st: AppState, compact: bool) -> View {
    let rows = chart_memo(st);
    let chosen = signal(0usize);
    // Start on the till of automatic entries once the chart is in.
    effect(move || {
        let all = rows.get();
        if chosen.get_untracked() == 0
            && let Some(i) = all.iter().position(|a| a.roles.iter().any(|r| r == "cash"))
        {
            chosen.set(i + 1);
        }
    });
    let span = Span::this_year();
    let data = resource(
        move || {
            st.session.get()?;
            let code = rows.with(|r| {
                chosen
                    .get()
                    .checked_sub(1)
                    .and_then(|i| r.get(i))
                    .map(|a| a.code.clone())
            })?;
            let (from, to) = span.bounds();
            Some((code, from, to, st.data_rev.get()))
        },
        move |(code, from, to, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| statement(s, &code, from.as_deref(), to.as_deref()))
                    .await
            }
        },
    );
    let st_data = computed(move || data.with(|s: Option<&Statement>| s.cloned()));
    let picker = view()
        .w(px(if compact { 260.0 } else { 360.0 }))
        .child(dropdown(
            move || {
                std::iter::once((0, st.tn("acc.entry.pick_account")))
                    .chain(
                        rows.get()
                            .iter()
                            .enumerate()
                            .map(|(i, a)| (i + 1, account_label(a))),
                    )
                    .collect()
            },
            chosen,
            Some(Icon::Book),
            st.t("acc.account"),
            "ledger-account",
        ));
    let content = dynamic(move || {
        let Some(s) = st_data.get() else {
            return card()
                .child(empty_state(
                    Icon::Book,
                    st.t("acc.ledger.pick"),
                    st.t("acc.ledger.pick.body"),
                    None,
                ))
                .into_element();
        };
        let w = if compact { 92.0 } else { 130.0 };
        let kpis = {
            let k = move |name: &'static str, v: i64, bal: bool| {
                // A balance shows its side under the amount (not in the
                // same text: see `balance_cell`).
                let side = if bal && v != 0 {
                    view().child(label_line(
                        st.t(if v > 0 { "acc.dr" } else { "acc.cr" }),
                        ty::XS,
                        FontWeight::MEDIUM,
                        Tone::Muted,
                    ))
                } else {
                    view()
                };
                let cell = kpi(
                    st.t(name),
                    move || {
                        if bal && v == 0 {
                            "—".to_owned()
                        } else {
                            money(st, v.abs())
                        }
                    },
                    None,
                    side,
                    compact,
                );
                if compact {
                    cell.w(relative(0.47))
                } else {
                    cell.flex_1()
                }
            };
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(16.0))
                .child(k("acc.opening", s.opening_minor, true))
                .child(k("acc.debit", s.debit_minor, false))
                .child(k("acc.credit", s.credit_minor, false))
                .child(k("acc.closing", s.closing_minor, true))
        };
        let head = |a: &'static str, b: &'static str, c: &'static str, d: &'static str| {
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(8.0))
                .bg(Tone::Subtle.color())
                .child(view().flex_1().min_w(px(0.0)).child(label_line(
                    st.t(a),
                    ty::XS,
                    FontWeight::SEMIBOLD,
                    Tone::Muted,
                )))
                .child(amount_cell(st.t(b), w, FontWeight::SEMIBOLD, Tone::Muted))
                .child(amount_cell(st.t(c), w, FontWeight::SEMIBOLD, Tone::Muted))
                .child(amount_cell(st.t(d), w, FontWeight::SEMIBOLD, Tone::Muted))
        };
        let mut list = view().child(head("acc.entry", "acc.debit", "acc.credit", "acc.balance"));
        let opening = s.opening_minor;
        list = list.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(8.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .child(view().flex_1().child(label_line(
                    st.t("acc.brought_forward"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                )))
                .child(amount_cell(String::new, w, FontWeight::NORMAL, Tone::Ink))
                .child(amount_cell(String::new, w, FontWeight::NORMAL, Tone::Ink))
                .child(balance_cell(st, opening, w, FontWeight::MEDIUM, Tone::Ink)),
        );
        for (i, l) in s.lines.into_iter().enumerate() {
            let top = format!("{} · {}", date(st, &l.date), l.number);
            let memo = if l.account == s.code {
                l.memo.clone()
            } else {
                format!("{} · {}", l.account, l.memo)
            };
            let (d, c, b) = (l.debit_minor, l.credit_minor, l.balance_minor);
            list = list.child(
                view()
                    .test_id(&format!("ledger-line-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(16.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(label_line(
                                move || top.clone(),
                                ty::SM,
                                FontWeight::MEDIUM,
                                Tone::Ink,
                            ))
                            .child(label_line(
                                move || memo.clone(),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(amount_cell(
                        move || amount(st, d),
                        w,
                        FontWeight::NORMAL,
                        Tone::Ink,
                    ))
                    .child(amount_cell(
                        move || amount(st, c),
                        w,
                        FontWeight::NORMAL,
                        Tone::Ink,
                    ))
                    .child(balance_cell(st, b, w, FontWeight::MEDIUM, Tone::Ink)),
            );
        }
        let title = format!("{} — {}", s.code, s.name);
        view()
            .gap(px(16.0))
            .child(kpis)
            .child(
                card()
                    .child(card_header_icon(
                        Some(Icon::Book),
                        move || title.clone(),
                        None,
                        view(),
                    ))
                    .child(list),
            )
            .into_element()
    });
    let body = view()
        .gap(px(16.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(12.0))
                .child(picker)
                .child(span_bar(st, span, compact)),
        )
        .child(content);
    page(
        st,
        Route::Ledger,
        "acc.ledger.subtitle",
        view(),
        body,
        compact,
    )
}

// ------------------------------------------------------------------ trial balance

pub fn trial_balance_page(st: AppState, compact: bool) -> View {
    let span = Span::this_year();
    let data = resource(
        move || {
            st.session.get()?;
            let (from, to) = span.bounds();
            Some((from, to, st.data_rev.get()))
        },
        move |(from, to, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| trial_balance(s, from.as_deref(), to.as_deref()))
                    .await
            }
        },
    );
    let tb = computed(move || {
        data.with(|t: Option<&TrialBalance>| t.cloned())
            .unwrap_or_default()
    });
    let w = if compact { 96.0 } else { 140.0 };
    let table = dynamic(move || {
        let t = tb.get();
        if t.rows.is_empty() {
            return empty_state(
                Icon::Scale,
                st.t("acc.trial.empty"),
                st.t("acc.trial.empty.body"),
                None,
            )
            .into_element();
        }
        let mut head = view()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(8.0))
            .bg(Tone::Subtle.color())
            .child(view().flex_1().min_w(px(0.0)).child(label_line(
                st.t("acc.account"),
                ty::XS,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            )));
        if !compact {
            head = head.child(amount_cell(
                st.t("acc.opening"),
                w,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            ));
        }
        head = head
            .child(amount_cell(
                st.t("acc.debit"),
                w,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            ))
            .child(amount_cell(
                st.t("acc.credit"),
                w,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            ))
            .child(amount_cell(
                st.t("acc.closing"),
                w,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            ));
        let mut v = view().child(head);
        for r in t.rows {
            let group = r.is_group;
            let weight = if group {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            };
            let text = format!("{}  {}", r.code, r.name);
            let (o, d, c, b) = (
                r.opening_minor,
                r.debit_minor,
                r.credit_minor,
                r.closing_minor,
            );
            let mut row = view()
                .test_id(&format!("trial-{}", r.code))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(7.0))
                .border_t_1()
                .border_color(Tone::Border.color())
                .bg(if r.depth == 0 {
                    Tone::Subtle.color()
                } else {
                    Tone::Surface.color()
                })
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .ps(px(f32::from(u16::try_from(r.depth).unwrap_or(8)) * 14.0))
                        .child(label_line(move || text.clone(), ty::SM, weight, Tone::Ink)),
                );
            if !compact {
                row = row.child(balance_cell(st, o, w, weight, Tone::Ink));
            }
            row = row
                .child(amount_cell(move || amount(st, d), w, weight, Tone::Ink))
                .child(amount_cell(move || amount(st, c), w, weight, Tone::Ink))
                .child(balance_cell(st, b, w, weight, Tone::Ink));
            v = v.child(row);
        }
        let (td, tc) = (t.debit_minor, t.credit_minor);
        let mut total = view()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(10.0))
            .border_t_1()
            .border_color(Tone::BorderStrong.color())
            .child(view().flex_1().child(label_line(
                st.t("acc.total"),
                ty::SM,
                FontWeight::BOLD,
                Tone::Ink,
            )));
        if !compact {
            total = total.child(amount_cell(String::new, w, FontWeight::BOLD, Tone::Ink));
        }
        total = total
            .child(amount_cell(
                move || money(st, td),
                w,
                FontWeight::BOLD,
                Tone::Ink,
            ))
            .child(amount_cell(
                move || money(st, tc),
                w,
                FontWeight::BOLD,
                Tone::Ink,
            ))
            .child(amount_cell(String::new, w, FontWeight::BOLD, Tone::Ink));
        v.child(total).into_element()
    });
    let state = dynamic(move || {
        let t = tb.get();
        if t.debit_minor == t.credit_minor && t.closing_debit_minor == t.closing_credit_minor {
            badge(
                Status::Success,
                Some(Icon::CheckCircle),
                st.t("acc.trial.balanced"),
            )
            .into_element()
        } else {
            badge(
                Status::Danger,
                Some(Icon::Alert),
                st.t("acc.trial.unbalanced"),
            )
            .into_element()
        }
    });
    let body = view()
        .gap(px(16.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(12.0))
                .child(span_bar(st, span, compact))
                .child(view().flex_1())
                .child(state),
        )
        .child(card().child(table));
    page(
        st,
        Route::TrialBalance,
        "acc.trial.subtitle",
        view(),
        body,
        compact,
    )
}

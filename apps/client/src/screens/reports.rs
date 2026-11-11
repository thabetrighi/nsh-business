//! Reports (M13): sales over a span of days — by day, product, category,
//! cashier and payment method, with returns, and cost and profit for those
//! who may see them (`reports.profit`) — and stock with its value, what
//! runs out and what does not move. Each table exports as CSV.

use nb_core::{Money, Quantity};
use nb_i18n::tr_args;
use nb_store::usecases::reports::{
    GroupRow, SalesReport, Span, StockReport, StockRow, sales_report, stock_report, to_csv,
};
use nsh_core::element::{AnyElement, KeyboardType, View};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::screens::common::{date_time, page};
use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::form::field;
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{Size, Variant, button, figure, label, label_line, segmented};
use crate::ui::overlay::show_toast;
use crate::ui::surface::{card, empty_state};
use crate::ui::table::{Paging, pager};
use crate::ui::tokens::{Tone, ty};

fn currency(st: AppState) -> nb_core::Currency {
    st.session
        .get_untracked()
        .map_or(nb_core::Currency::DZD, |s| s.currency)
}

fn money(st: AppState, minor: i64) -> String {
    st.fmt().money(Money::from_minor(minor, currency(st)))
}

/// An amount as a spreadsheet reads it: digits and a point.
fn plain_money(st: AppState, minor: i64) -> String {
    Money::from_minor(minor, currency(st)).to_plain_string()
}

fn qty(st: AppState, milli: i64) -> String {
    st.fmt().quantity(Quantity::from_milli(milli), 3)
}

fn iso_days_ago(n: i64) -> String {
    nb_store::time::iso_date(crate::state::now_ms() - n * 86_400_000, 60)
}

fn month_start() -> String {
    let today = iso_days_ago(0);
    format!("{}-01", today.get(..7).unwrap_or("2026-01"))
}

/// Saves `rows` as a CSV file the person chooses.
fn export(st: AppState, name: String, rows: Vec<Vec<String>>) {
    let text = to_csv(&rows);
    let task = spawn(async move {
        let dialog = nsh_core::file_dialog::SaveDialog::new()
            .file_name(name)
            .title(st.tn("reports.export"));
        match pick_save_path(dialog).await {
            Ok(Some(path)) => match std::fs::write(&path, text) {
                Ok(()) => show_toast(st, st.tn("reports.exported")),
                Err(_) => show_toast(st, st.tn("reports.export_failed")),
            },
            Ok(None) => {}
            Err(_) => show_toast(st, st.tn("reports.export_failed")),
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

fn tile(
    st: AppState,
    key: &'static str,
    value: impl Fn() -> String + 'static,
    tone: Tone,
    id: &'static str,
) -> View {
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
        .child(figure(value, ty::H2, FontWeight::BOLD, move || tone))
}

/// A table: a header and rows of cells, the first one wide.
fn table(
    heads: Vec<String>,
    rows: Vec<Vec<String>>,
    prefix: &'static str,
    compact: bool,
) -> AnyElement {
    let cell = |text: String, first: bool, strong: bool| {
        let v = if first {
            view().flex_1().min_w(px(120.0))
        } else {
            view().w(px(120.0)).items_end()
        };
        v.px(px(10.0)).child(label_line(
            move || text.clone(),
            if strong { ty::XS } else { ty::SM },
            if strong {
                FontWeight::SEMIBOLD
            } else if first {
                FontWeight::MEDIUM
            } else {
                FontWeight::NORMAL
            },
            if strong { Tone::Muted } else { Tone::Ink },
        ))
    };
    let mut head = view()
        .flex_row()
        .items_center()
        .py(px(8.0))
        .bg(Tone::Subtle.color())
        .border_b_1()
        .border_color(Tone::Border.color());
    for (i, h) in heads.into_iter().enumerate() {
        head = head.child(cell(h, i == 0, true));
    }
    let mut body = view().child(head);
    for (r, row) in rows.into_iter().enumerate() {
        let mut line = view()
            .test_id(&format!("{prefix}-{r}"))
            .flex_row()
            .items_center()
            .py(px(9.0))
            .border_b_1()
            .border_color(Tone::Border.color());
        for (i, c) in row.into_iter().enumerate() {
            line = line.child(cell(c, i == 0, false));
        }
        body = body.child(line);
    }
    // On a phone the columns scroll sideways; on a desktop they fill the card.
    if compact {
        scroll()
            .horizontal()
            .child(body.min_w(px(520.0)))
            .into_element()
    } else {
        body.w_full().into_element()
    }
}

/// A tile's amount: whole units (the tables carry the centimes).
fn whole(st: AppState, minor: i64) -> String {
    st.fmt().money_whole(Money::from_minor(minor, currency(st)))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum By {
    Day,
    Product,
    Category,
    Cashier,
    Method,
}

/// The rows of a breakdown, as shown (`plain`: as exported).
fn breakdown(
    st: AppState,
    r: &SalesReport,
    by: By,
    profit: bool,
    plain: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let m = |v: i64| {
        if plain {
            plain_money(st, v)
        } else {
            money(st, v)
        }
    };
    let q = |v: i64| {
        if plain {
            Quantity::from_milli(v).to_string()
        } else {
            qty(st, v)
        }
    };
    let pct = |net: i64, part: i64| {
        let bps = if net <= 0 {
            0
        } else {
            i64::try_from(i128::from(part) * 10_000 / i128::from(net)).unwrap_or(0)
        };
        if plain {
            format!("{}.{:02}", bps / 100, bps.rem_euclid(100))
        } else {
            st.fmt()
                .percent_signed(bps)
                .trim_start_matches('+')
                .to_owned()
        }
    };
    let mut heads: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    match by {
        By::Day => {
            heads.extend(
                [
                    "reports.col.day",
                    "reports.col.sales",
                    "reports.col.total",
                    "reports.col.returns",
                    "reports.col.net",
                ]
                .map(|k| st.tn(k)),
            );
            if profit {
                heads.extend(["reports.col.cost", "reports.col.profit"].map(|k| st.tn(k)));
            }
            for d in &r.by_day {
                let mut row = vec![
                    d.date.clone(),
                    st.fmt().integer(i64::from(d.sales)),
                    m(d.total_minor),
                    m(d.returns_minor),
                    m(d.net_minor),
                ];
                if profit {
                    row.extend([m(d.cost_minor), m(d.profit_minor())]);
                }
                rows.push(row);
            }
        }
        By::Method => {
            heads.extend(
                [
                    "reports.col.method",
                    "reports.col.sales",
                    "reports.col.amount",
                    "reports.col.share",
                ]
                .map(|k| st.tn(k)),
            );
            let total: i64 = r.by_method.iter().map(|g| g.net_minor).sum();
            for g in &r.by_method {
                rows.push(vec![
                    st.tn(&format!("pay.{}", g.key)),
                    st.fmt().integer(i64::from(g.count)),
                    m(g.net_minor),
                    pct(total, g.net_minor),
                ]);
            }
        }
        _ => {
            let (list, first): (&Vec<GroupRow>, &'static str) = match by {
                By::Product => (&r.by_product, "reports.col.product"),
                By::Category => (&r.by_category, "reports.col.category"),
                _ => (&r.by_cashier, "reports.col.cashier"),
            };
            heads.push(st.tn(first));
            if by == By::Product {
                heads.push(st.tn("reports.col.qty"));
            }
            heads.extend(["reports.col.sales", "reports.col.net"].map(|k| st.tn(k)));
            if profit {
                heads.extend(
                    [
                        "reports.col.cost",
                        "reports.col.profit",
                        "reports.col.margin",
                    ]
                    .map(|k| st.tn(k)),
                );
            }
            for g in list {
                let name = if g.name.is_empty() {
                    st.tn(if by == By::Category {
                        "reports.uncategorized"
                    } else {
                        "reports.unknown"
                    })
                } else {
                    g.name.clone()
                };
                let mut row = vec![name];
                if by == By::Product {
                    row.push(q(g.qty_milli));
                }
                row.extend([st.fmt().integer(i64::from(g.count)), m(g.net_minor)]);
                if profit {
                    row.extend([
                        m(g.cost_minor),
                        m(g.profit_minor()),
                        pct(g.net_minor, g.profit_minor()),
                    ]);
                }
                rows.push(row);
            }
        }
    }
    if plain {
        // Spreadsheets want plain digits for counts too.
        for row in &mut rows {
            for c in row.iter_mut() {
                *c = nb_i18n::bidi::strip_controls(c).replace('\u{a0}', "");
            }
        }
    }
    (heads, rows)
}

pub fn reports_page(st: AppState, compact: bool) -> View {
    let from = signal(month_start());
    let to = signal(iso_days_ago(0));
    // 0: today, 1: 7 days, 2: this month, 3: 30 days.
    let preset = signal(2usize);
    effect(move || {
        let p = preset.get();
        let (a, b) = match p {
            0 => (iso_days_ago(0), iso_days_ago(0)),
            1 => (iso_days_ago(6), iso_days_ago(0)),
            3 => (iso_days_ago(29), iso_days_ago(0)),
            _ => (month_start(), iso_days_ago(0)),
        };
        from.set(a);
        to.set(b);
    });
    let by = signal(By::Day);
    let paging = Paging::new(25);
    effect(move || {
        by.with(|_| ());
        from.with(|_| ());
        to.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            Some((from.get(), to.get(), st.data_rev.get()))
        },
        move |(a, b, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    sales_report(
                        s,
                        &Span {
                            from: a.trim().to_owned(),
                            to: b.trim().to_owned(),
                        },
                    )
                })
                .await
            }
        },
    );
    let report = computed(move || data.with(|r: Option<&SalesReport>| r.cloned()));
    let failed = computed(move || data.error());
    let shown = computed(move || {
        report.with(|r| {
            r.as_ref().map_or(0, |r| {
                let n = match by.get() {
                    By::Day => r.by_day.len(),
                    By::Product => r.by_product.len(),
                    By::Category => r.by_category.len(),
                    By::Cashier => r.by_cashier.len(),
                    By::Method => r.by_method.len(),
                };
                u32::try_from(n).unwrap_or(u32::MAX)
            })
        })
    });
    let profit = move || st.can("reports.profit");
    let tiles = dynamic(move || {
        let Some(r) = report.get().filter(|_| failed.get().is_none()) else {
            return view().into_element();
        };
        let (sales, total, returns, net, basket, cost, gain) = (
            r.sales,
            r.total_minor,
            r.returns_total_minor,
            r.net_after_returns_minor(),
            r.basket_minor(),
            r.cost_minor,
            r.profit_minor(),
        );
        let mut v = view()
            .flex_row()
            .flex_wrap()
            .gap(px(12.0))
            .child(tile(
                st,
                "reports.sales",
                move || st.fmt().integer(i64::from(sales)),
                Tone::Ink,
                "rep-sales",
            ))
            .child(tile(
                st,
                "reports.total",
                move || whole(st, total),
                Tone::Ink,
                "rep-total",
            ))
            .child(tile(
                st,
                "reports.returns",
                move || whole(st, returns),
                Tone::Ink,
                "rep-returns",
            ))
            .child(tile(
                st,
                "reports.net",
                move || whole(st, net),
                Tone::Ink,
                "rep-net",
            ))
            .child(tile(
                st,
                "kpi.basket",
                move || money(st, basket),
                Tone::Ink,
                "rep-basket",
            ));
        if profit() {
            let margin = r.margin_bps();
            v = v
                .child(tile(
                    st,
                    "reports.cost",
                    move || whole(st, cost),
                    Tone::Ink,
                    "rep-cost",
                ))
                .child(tile(
                    st,
                    "reports.profit",
                    move || whole(st, gain),
                    if gain < 0 {
                        Tone::Danger
                    } else {
                        Tone::Success
                    },
                    "rep-profit",
                ))
                .child(tile(
                    st,
                    "reports.margin",
                    move || {
                        st.fmt()
                            .percent_signed(margin)
                            .trim_start_matches('+')
                            .to_owned()
                    },
                    Tone::Ink,
                    "rep-margin",
                ));
        }
        v.into_element()
    });
    let content = dynamic(move || {
        if let Some(e) = failed.get() {
            let text = error_text(st, e.codes.first().map_or("internal", |c| c.1));
            return label(
                move || text.clone(),
                ty::SM,
                FontWeight::MEDIUM,
                Tone::Danger,
            )
            .into_element();
        }
        let Some(r) = report.get() else {
            return view().into_element();
        };
        if r.sales == 0 && r.returns == 0 {
            return empty_state(
                Icon::ChartLine,
                st.t("reports.empty"),
                st.t("reports.empty.body"),
                None,
            )
            .into_element();
        }
        let (heads, rows) = breakdown(st, &r, by.get(), profit(), false);
        let rows: Vec<_> = rows
            .into_iter()
            .skip(paging.offset() as usize)
            .take(paging.size.get() as usize)
            .collect();
        table(heads, rows, "rep-row", compact)
    });
    let filters = view()
        .flex_row()
        .flex_wrap()
        .items_end()
        .gap(px(12.0))
        .p(px(12.0))
        .child(segmented(
            vec![
                (0, Box::new(st.t("period.today"))),
                (1, Box::new(st.t("period.7d"))),
                (2, Box::new(st.t("reports.this_month"))),
                (3, Box::new(st.t("period.30d"))),
            ],
            preset,
        ))
        .child(view().w(px(150.0)).child(field(
            st.t("reports.from"),
            from,
            || "2026-11-01".to_owned(),
            String::new,
            KeyboardType::Default,
            "rep-from",
        )))
        .child(view().w(px(150.0)).child(field(
            st.t("reports.to"),
            to,
            || "2026-11-30".to_owned(),
            String::new,
            KeyboardType::Default,
            "rep-to",
        )));
    let tabs = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
        .p(px(12.0))
        .child(view().flex_1().child(segmented(
            vec![
                (By::Day, Box::new(st.t("reports.by.day"))),
                (By::Product, Box::new(st.t("reports.by.product"))),
                (By::Category, Box::new(st.t("reports.by.category"))),
                (By::Cashier, Box::new(st.t("reports.by.cashier"))),
                (By::Method, Box::new(st.t("reports.by.method"))),
            ],
            by,
        )))
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Download),
                st.t("reports.export"),
                move || {
                    if let Some(r) = report.get_untracked() {
                        let (heads, mut rows) =
                            breakdown(st, &r, by.get_untracked(), profit(), true);
                        rows.insert(0, heads);
                        export(
                            st,
                            format!("ventes-{}-{}.csv", r.span.from, r.span.to),
                            rows,
                        );
                    }
                },
            )
            .test_id("rep-export"),
        );
    let body = view()
        .gap(px(16.0))
        .child(card().child(filters))
        .child(tiles)
        .child(card().child(tabs).child(content).child(pager(
            st,
            paging,
            move || shown.get(),
            compact,
        )));
    page(
        st,
        Route::Reports,
        "reports.subtitle",
        view(),
        body,
        compact,
    )
}

/// Which products the stock report lists.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Show {
    All,
    Short,
    Idle,
}

fn stock_rows(
    st: AppState,
    r: &StockReport,
    show: Show,
    cost: bool,
    plain: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let mut heads: Vec<String> = [
        "reports.col.product",
        "reports.col.sku",
        "reports.col.on_hand",
    ]
    .map(|k| st.tn(k))
    .to_vec();
    if cost {
        heads.push(st.tn("reports.col.value"));
    }
    heads.push(tr_args(
        st.lang(),
        "reports.col.sold_days",
        &[("days", &r.days.to_string())],
    ));
    heads.push(st.tn("reports.col.last_sale"));
    let keep = |row: &StockRow| match show {
        Show::All => true,
        Show::Short => row.out() || row.low(),
        Show::Idle => row.idle(),
    };
    let rows = r
        .rows
        .iter()
        .filter(|row| keep(row))
        .map(|row| {
            let q = |v: i64| {
                if plain {
                    Quantity::from_milli(v).to_string()
                } else {
                    format!("{} {}", qty(st, v), row.unit)
                }
            };
            let mut v = vec![row.name.clone(), row.sku.clone(), q(row.on_hand_milli)];
            if cost {
                v.push(if plain {
                    plain_money(st, row.value_minor)
                } else {
                    money(st, row.value_minor)
                });
            }
            v.push(q(row.sold_milli));
            v.push(row.last_sale_at.map_or_else(
                || "—".to_owned(),
                |t| {
                    if plain {
                        nb_store::time::iso_date(t, 60)
                    } else {
                        date_time(st, t)
                    }
                },
            ));
            if plain {
                for c in v.iter_mut() {
                    *c = nb_i18n::bidi::strip_controls(c).replace('\u{a0}', "");
                }
            }
            v
        })
        .collect();
    (heads, rows)
}

pub fn stock_reports_page(st: AppState, compact: bool) -> View {
    // 0: 30 days, 1: 90 days, 2: 180 days.
    let window = signal(0usize);
    let show = signal(Show::All);
    let paging = Paging::new(25);
    effect(move || {
        window.with(|_| ());
        show.with(|_| ());
        paging.reset();
    });
    let data = resource(
        move || {
            st.session.get()?;
            Some((window.get(), st.data_rev.get()))
        },
        move |(w, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| stock_report(s, [30, 90, 180][w.min(2)]))
                    .await
            }
        },
    );
    let report = computed(move || data.with(|r: Option<&StockReport>| r.cloned()));
    let cost = move || st.can("reports.profit");
    let shown = computed(move || {
        let s = show.get();
        report.with(|r| {
            r.as_ref().map_or(0, |r| {
                let n = r
                    .rows
                    .iter()
                    .filter(|row| match s {
                        Show::All => true,
                        Show::Short => row.out() || row.low(),
                        Show::Idle => row.idle(),
                    })
                    .count();
                u32::try_from(n).unwrap_or(u32::MAX)
            })
        })
    });
    let tiles = dynamic(move || {
        let Some(r) = report.get() else {
            return view().into_element();
        };
        let mut v = view().flex_row().flex_wrap().gap(px(12.0));
        if cost() {
            v = v.child(tile(
                st,
                "reports.stock_value",
                move || whole(st, r.value_minor),
                Tone::Ink,
                "stk-value",
            ));
        }
        v = v
            .child(tile(
                st,
                "stock.out",
                move || st.fmt().integer(i64::from(r.out)),
                if r.out > 0 { Tone::Danger } else { Tone::Ink },
                "stk-out",
            ))
            .child(tile(
                st,
                "stock.low",
                move || st.fmt().integer(i64::from(r.low)),
                if r.low > 0 { Tone::Warning } else { Tone::Ink },
                "stk-low",
            ))
            .child(tile(
                st,
                "reports.idle",
                move || st.fmt().integer(i64::from(r.idle)),
                Tone::Ink,
                "stk-idle",
            ));
        if cost() {
            v = v.child(tile(
                st,
                "reports.idle_value",
                move || whole(st, r.idle_value_minor),
                Tone::Ink,
                "stk-idle-value",
            ));
        }
        v.into_element()
    });
    let content = dynamic(move || {
        let Some(r) = report.get() else {
            return view().into_element();
        };
        let (heads, rows) = stock_rows(st, &r, show.get(), cost(), false);
        let rows: Vec<_> = rows
            .into_iter()
            .skip(paging.offset() as usize)
            .take(paging.size.get() as usize)
            .collect();
        if rows.is_empty() {
            return view()
                .p(px(16.0))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(icon(Icon::Check, 18.0, || Tone::Success))
                .child(label(
                    st.t("reports.stock_none"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .into_element();
        }
        table(heads, rows, "stk-row", compact)
    });
    let bar = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
        .p(px(12.0))
        .child(segmented(
            vec![
                (Show::All, Box::new(st.t("customers.filter.all"))),
                (Show::Short, Box::new(st.t("reports.show.short"))),
                (Show::Idle, Box::new(st.t("reports.show.idle"))),
            ],
            show,
        ))
        .child(segmented(
            vec![
                (0, Box::new(st.t("period.30d"))),
                (1, Box::new(st.t("reports.days_90"))),
                (2, Box::new(st.t("reports.days_180"))),
            ],
            window,
        ))
        .child(view().flex_1())
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Download),
                st.t("reports.export"),
                move || {
                    if let Some(r) = report.get_untracked() {
                        let (heads, mut rows) =
                            stock_rows(st, &r, show.get_untracked(), cost(), true);
                        rows.insert(0, heads);
                        export(st, "stock.csv".to_owned(), rows);
                    }
                },
            )
            .test_id("stk-export"),
        );
    let body =
        view()
            .gap(px(16.0))
            .child(tiles)
            .child(card().child(bar).child(content).child(pager(
                st,
                paging,
                move || shown.get(),
                compact,
            )));
    page(
        st,
        Route::StockReports,
        "reports.stock.subtitle",
        view(),
        body,
        compact,
    )
}

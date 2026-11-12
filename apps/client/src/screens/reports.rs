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

fn year_start() -> String {
    let today = iso_days_ago(0);
    format!("{}-01-01", today.get(..4).unwrap_or("2026"))
}

/// The days a report covers: ready choices, or two dates typed.
fn span_picker(
    st: AppState,
    ids: (&'static str, &'static str),
) -> (Signal<String>, Signal<String>, View) {
    let from = signal(month_start());
    let to = signal(iso_days_ago(0));
    // 0: today, 1: 7 days, 2: this month, 3: 30 days, 4: this year.
    let preset = signal(2usize);
    effect(move || {
        let (a, b) = match preset.get() {
            0 => (iso_days_ago(0), iso_days_ago(0)),
            1 => (iso_days_ago(6), iso_days_ago(0)),
            3 => (iso_days_ago(29), iso_days_ago(0)),
            4 => (year_start(), iso_days_ago(0)),
            _ => (month_start(), iso_days_ago(0)),
        };
        from.set(a);
        to.set(b);
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
                (4, Box::new(st.t("reports.this_year"))),
            ],
            preset,
        ))
        .child(view().w(px(150.0)).child(field(
            st.t("reports.from"),
            from,
            || "2026-11-01".to_owned(),
            String::new,
            KeyboardType::Default,
            ids.0,
        )))
        .child(view().w(px(150.0)).child(field(
            st.t("reports.to"),
            to,
            || "2026-11-30".to_owned(),
            String::new,
            KeyboardType::Default,
            ids.1,
        )));
    (from, to, filters)
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

/// What a report exports: the file's name (no extension), its title and
/// what it covers, and its rows (the header first).
type Export = (String, String, String, Vec<Vec<String>>);

/// Saves the report as a PDF table the person names.
#[cfg(not(target_arch = "wasm32"))]
fn export_pdf(st: AppState, (name, title, subtitle, rows): Export) {
    let lang = st.lang();
    let task = spawn(async move {
        let (shop, docs, t) = st
            .db()
            .call(|s| {
                Ok((
                    nb_store::usecases::printing::print_shop(s)?,
                    nb_store::usecases::printing::document_settings(s)?,
                    crate::screens::printing::layout(s, nb_domain::template::DocKind::Report),
                ))
            })
            .await
            .map_err(|_| ())
            .map_or((None, None, None), |(a, b, c)| (Some(a), Some(b), Some(c)));
        let dialog = nsh_core::file_dialog::SaveDialog::new()
            .file_name(format!("{name}.pdf"))
            .title(st.tn("reports.export_pdf"));
        let Ok(Some(path)) = pick_save_path(dialog).await else {
            return;
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let doc = match (shop, docs, t) {
                (Some(shop), Some(docs), Some(t)) => {
                    use crate::screens::printing::{draw, options, shop_of};
                    let o = options(&docs);
                    let d = nb_docs::templates::report_data(
                        &shop_of(&shop, ""),
                        &title,
                        &subtitle,
                        &rows,
                        &o,
                    );
                    draw(&t, &o, &d, lang, nb_docs::Paper::A4)
                }
                _ => nb_docs::templates::report_table(
                    "",
                    &title,
                    &subtitle,
                    &rows,
                    lang,
                    nb_docs::Paper::A4,
                ),
            };
            let r = nb_docs::pdf(&doc)
                .map_err(|e| e.0)
                .and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()));
            let _ = tx.send(r);
        });
        match rx.await {
            Ok(Ok(())) => show_toast(st, st.tn("reports.exported")),
            _ => show_toast(st, st.tn("reports.export_failed")),
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

/// CSV and PDF buttons for a report.
fn export_buttons(
    st: AppState,
    ids: (&'static str, &'static str),
    make: impl Fn() -> Option<Export> + Clone + 'static,
) -> View {
    let csv = {
        let make = make.clone();
        move || {
            if let Some((name, _, _, rows)) = make() {
                export(st, format!("{name}.csv"), rows);
            }
        }
    };
    let v = view().flex_row().gap(px(8.0)).child(
        button(
            Variant::Secondary,
            Size::Md,
            Some(Icon::Download),
            st.t("reports.export"),
            csv,
        )
        .test_id(ids.0),
    );
    #[cfg(not(target_arch = "wasm32"))]
    let v = v.child(
        button(
            Variant::Secondary,
            Size::Md,
            Some(Icon::FileText),
            st.t("reports.export_pdf"),
            move || {
                if let Some(e) = make() {
                    export_pdf(st, e);
                }
            },
        )
        .test_id(ids.1),
    );
    #[cfg(target_arch = "wasm32")]
    let _ = (ids.1, make);
    v
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
    let (from, to, filters) = span_picker(st, ("rep-from", "rep-to"));
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
        .child(export_buttons(st, ("rep-export", "rep-pdf"), move || {
            let r = report.get_untracked()?;
            let (heads, mut rows) = breakdown(st, &r, by.get_untracked(), profit(), true);
            rows.insert(0, heads);
            Some((
                format!("ventes-{}-{}", r.span.from, r.span.to),
                st.tn("nav.reports"),
                tr_args(
                    st.lang(),
                    "reports.span",
                    &[("from", &r.span.from), ("to", &r.span.to)],
                ),
                rows,
            ))
        }));
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
        .child(export_buttons(st, ("stk-export", "stk-pdf"), move || {
            let r = report.get_untracked()?;
            let (heads, mut rows) = stock_rows(st, &r, show.get_untracked(), cost(), true);
            rows.insert(0, heads);
            Some((
                "stock".to_owned(),
                st.tn("nav.stock_reports"),
                String::new(),
                rows,
            ))
        }));
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Book {
    Result,
    Vat,
    Shifts,
}

fn result_rows(
    st: AppState,
    r: &nb_store::usecases::reports::IncomeStatement,
    plain: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let m = |v: i64| {
        if plain {
            plain_money(st, v)
        } else {
            money(st, v)
        }
    };
    let heads = [
        "reports.col.account",
        "reports.col.name",
        "reports.col.amount",
    ]
    .map(|k| st.tn(k))
    .to_vec();
    let mut rows = Vec::new();
    rows.push(vec![
        String::new(),
        st.tn("reports.revenue"),
        m(r.revenue_minor),
    ]);
    for l in &r.revenue {
        rows.push(vec![l.code.clone(), l.name.clone(), m(l.amount_minor)]);
    }
    rows.push(vec![
        String::new(),
        st.tn("reports.expenses"),
        m(r.expenses_minor),
    ]);
    for l in &r.expenses {
        rows.push(vec![l.code.clone(), l.name.clone(), m(l.amount_minor)]);
    }
    rows.push(vec![
        String::new(),
        st.tn(if r.result_minor() < 0 {
            "reports.loss"
        } else {
            "reports.result"
        }),
        m(r.result_minor()),
    ]);
    (heads, rows)
}

fn vat_rows(
    st: AppState,
    r: &nb_store::usecases::reports::VatReport,
    plain: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let m = |v: i64| {
        if plain {
            plain_money(st, v)
        } else {
            money(st, v)
        }
    };
    let heads = [
        "reports.col.month",
        "reports.vat.collected",
        "reports.vat.deductible",
        "reports.vat.due",
    ]
    .map(|k| st.tn(k))
    .to_vec();
    let rows = r
        .months
        .iter()
        .map(|v| {
            vec![
                v.month.clone(),
                m(v.collected_minor),
                m(v.deductible_minor),
                m(v.due_minor()),
            ]
        })
        .collect();
    (heads, rows)
}

fn shift_rows(
    st: AppState,
    r: &nb_store::usecases::reports::ShiftReport,
    plain: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let m = |v: i64| {
        if plain {
            plain_money(st, v)
        } else {
            money(st, v)
        }
    };
    let opt = |v: Option<i64>| v.map_or_else(|| "—".to_owned(), m);
    let heads = [
        "reports.col.shift",
        "reports.col.opened",
        "reports.col.cashier",
        "reports.col.sales",
        "reports.col.total",
        "reports.col.expected",
        "reports.col.counted",
        "reports.col.difference",
    ]
    .map(|k| st.tn(k))
    .to_vec();
    let rows = r
        .rows
        .iter()
        .map(|h| {
            let mut v = vec![
                format!("{} · {}", h.number, h.register),
                if plain {
                    nb_store::time::iso_date(h.opened_at, 60)
                } else {
                    date_time(st, h.opened_at)
                },
                if h.cashier.is_empty() {
                    "—".to_owned()
                } else {
                    h.cashier.clone()
                },
                st.fmt().integer(i64::from(h.sales)),
                m(h.sales_minor),
                opt(h.expected_minor),
                opt(h.counted_minor),
                if h.status == "open" {
                    st.tn("reports.shift_open")
                } else {
                    opt(h.difference_minor)
                },
            ];
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

/// Finance reports: the result over the days chosen, VAT month by month,
/// till sessions with their differences.
pub fn finance_reports_page(st: AppState, compact: bool) -> View {
    finance(st, compact, Route::FinanceReports, Book::Result)
}

/// Till sessions (the same report, opened on its own page).
pub fn shifts_page(st: AppState, compact: bool) -> View {
    finance(st, compact, Route::Shifts, Book::Shifts)
}

fn finance(st: AppState, compact: bool, route: Route, first: Book) -> View {
    use nb_store::usecases::reports::{income_statement, shift_report, vat_report};
    type Books = (
        nb_store::usecases::reports::IncomeStatement,
        nb_store::usecases::reports::VatReport,
        nb_store::usecases::reports::ShiftReport,
    );
    let (from, to, filters) = span_picker(st, ("fin-from", "fin-to"));
    let book = signal(first);
    let data = resource(
        move || {
            st.session.get()?;
            Some((from.get(), to.get(), st.data_rev.get()))
        },
        move |(a, b, _), _| {
            let db = st.db();
            async move {
                db.call(move |s| {
                    let span = Span {
                        from: a.trim().to_owned(),
                        to: b.trim().to_owned(),
                    };
                    Ok((
                        income_statement(s, &span)?,
                        vat_report(s, &span)?,
                        shift_report(s, &span)?,
                    ))
                })
                .await
            }
        },
    );
    let books = computed(move || data.with(|r: Option<&Books>| r.cloned()));
    let failed = computed(move || data.error());
    let profit = move || st.can("reports.profit");
    let tiles = dynamic(move || {
        let Some((pl, vat, sh)) = books.get().filter(|_| failed.get().is_none()) else {
            return view().into_element();
        };
        let mut v = view().flex_row().flex_wrap().gap(px(12.0));
        match book.get() {
            Book::Result if profit() => {
                let (rev, exp, res) = (pl.revenue_minor, pl.expenses_minor, pl.result_minor());
                v = v
                    .child(tile(
                        st,
                        "reports.revenue",
                        move || whole(st, rev),
                        Tone::Ink,
                        "fin-revenue",
                    ))
                    .child(tile(
                        st,
                        "reports.expenses",
                        move || whole(st, exp),
                        Tone::Ink,
                        "fin-expenses",
                    ))
                    .child(tile(
                        st,
                        if res < 0 {
                            "reports.loss"
                        } else {
                            "reports.result"
                        },
                        move || whole(st, res),
                        if res < 0 { Tone::Danger } else { Tone::Success },
                        "fin-result",
                    ));
            }
            Book::Vat => {
                let (c, d, due) = (vat.collected_minor, vat.deductible_minor, vat.due_minor());
                v = v
                    .child(tile(
                        st,
                        "reports.vat.collected",
                        move || whole(st, c),
                        Tone::Ink,
                        "fin-vat-out",
                    ))
                    .child(tile(
                        st,
                        "reports.vat.deductible",
                        move || whole(st, d),
                        Tone::Ink,
                        "fin-vat-in",
                    ))
                    .child(tile(
                        st,
                        if due < 0 {
                            "reports.vat.credit"
                        } else {
                            "reports.vat.due"
                        },
                        move || whole(st, due.abs()),
                        Tone::Ink,
                        "fin-vat-due",
                    ));
            }
            Book::Shifts => {
                let (n, over, short) = (sh.rows.len(), sh.over_minor, sh.short_minor);
                v = v
                    .child(tile(
                        st,
                        "reports.shifts",
                        move || st.fmt().integer(i64::try_from(n).unwrap_or(0)),
                        Tone::Ink,
                        "fin-shifts",
                    ))
                    .child(tile(
                        st,
                        "reports.over",
                        move || whole(st, over),
                        Tone::Ink,
                        "fin-over",
                    ))
                    .child(tile(
                        st,
                        "reports.short",
                        move || whole(st, short),
                        if short > 0 { Tone::Danger } else { Tone::Ink },
                        "fin-short",
                    ));
            }
            Book::Result => {}
        }
        v.into_element()
    });
    let rows_of = move |b: Book, plain: bool| -> Option<(Vec<String>, Vec<Vec<String>>)> {
        let (pl, vat, sh) = books.get_untracked()?;
        Some(match b {
            Book::Result => result_rows(st, &pl, plain),
            Book::Vat => vat_rows(st, &vat, plain),
            Book::Shifts => shift_rows(st, &sh, plain),
        })
    };
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
        books.with(|_| ());
        let b = book.get();
        if b == Book::Result && !profit() {
            return label(
                st.t("reports.no_right"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        }
        let Some((heads, rows)) = rows_of(b, false) else {
            return view().into_element();
        };
        if rows.is_empty() {
            return empty_state(
                Icon::ChartLine,
                st.t("reports.nothing"),
                st.t("reports.empty.body"),
                None,
            )
            .into_element();
        }
        table(heads, rows, "fin-row", compact)
    });
    let tabs = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
        .p(px(12.0))
        .child(view().flex_1().child(segmented(
            vec![
                (Book::Result, Box::new(st.t("reports.book.result"))),
                (Book::Vat, Box::new(st.t("reports.book.vat"))),
                (Book::Shifts, Box::new(st.t("reports.book.shifts"))),
            ],
            book,
        )))
        .child(export_buttons(st, ("fin-export", "fin-pdf"), move || {
            let b = book.get_untracked();
            if b == Book::Result && !profit() {
                return None;
            }
            let (heads, mut rows) = rows_of(b, true)?;
            rows.insert(0, heads);
            let (name, key) = match b {
                Book::Result => ("resultat", "reports.book.result"),
                Book::Vat => ("tva", "reports.book.vat"),
                Book::Shifts => ("sessions", "reports.book.shifts"),
            };
            let (f, t) = (from.get_untracked(), to.get_untracked());
            Some((
                format!("{name}-{f}-{t}"),
                st.tn(key),
                tr_args(st.lang(), "reports.span", &[("from", &f), ("to", &t)]),
                rows,
            ))
        }));
    let subtitle = if route == Route::Shifts {
        "reports.shifts.subtitle"
    } else {
        "reports.finance.subtitle"
    };
    let body = view()
        .gap(px(16.0))
        .child(card().child(filters))
        .child(tiles)
        .child(card().child(tabs).child(content));
    page(st, route, subtitle, view(), body, compact)
}

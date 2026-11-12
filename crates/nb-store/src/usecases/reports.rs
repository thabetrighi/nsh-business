//! Reports and the dashboard (M13): read-only views computed from the
//! records themselves — sales by day, product, category, cashier and
//! payment method, with returns and profit; stock with its value, what runs
//! out and what does not move; the home dashboard. Days are the store's
//! business days (its UTC offset). Cost and profit are figures the screen
//! shows only with `reports.profit`.

use nb_domain::RuleError;
use nb_domain::fiscal::Date;
use nb_domain::installment::{add_days, days_between};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::time::civil_date;
use crate::usecases::sales::{SaleRow, recent_sales, the_org};
use crate::{Result, Store};

/// Days `from` to `to`, both included (`YYYY-MM-DD`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Span {
    pub from: String,
    pub to: String,
}

impl Span {
    fn dates(&self) -> Result<(Date, Date)> {
        let (a, b) = (Date::parse(&self.from)?, Date::parse(&self.to)?);
        if b < a {
            return Err(RuleError::on("to", "report.span").into());
        }
        if days_between(a, b) > 3_660 {
            return Err(RuleError::on("to", "report.span").into());
        }
        Ok((a, b))
    }
}

/// The store's business day of a timestamp column, as SQL.
fn day(col: &str) -> String {
    format!("date({col} / 1000 + ?3 * 60, 'unixepoch')")
}

/// A total over a day.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DayRow {
    pub date: String,
    pub sales: u32,
    /// With tax, before returns.
    pub total_minor: i64,
    pub returns_minor: i64,
    /// Before tax, less returns.
    pub net_minor: i64,
    pub cost_minor: i64,
}

impl DayRow {
    pub fn profit_minor(&self) -> i64 {
        self.net_minor - self.cost_minor
    }
}

/// A total over a group (a product, a category, a cashier, a method).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GroupRow {
    pub key: String,
    pub name: String,
    /// In base units (products only).
    pub qty_milli: i64,
    pub count: u32,
    /// Before tax (methods: the amount taken).
    pub net_minor: i64,
    pub cost_minor: i64,
}

impl GroupRow {
    pub fn profit_minor(&self) -> i64 {
        self.net_minor - self.cost_minor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SalesReport {
    pub span: Span,
    pub sales: u32,
    /// What customers were charged, with tax.
    pub total_minor: i64,
    pub discounts_minor: i64,
    pub net_minor: i64,
    pub tax_minor: i64,
    pub returns: u32,
    pub returns_total_minor: i64,
    pub returns_net_minor: i64,
    /// Cost of what was sold, less what came back to stock.
    pub cost_minor: i64,
    pub by_day: Vec<DayRow>,
    pub by_product: Vec<GroupRow>,
    pub by_category: Vec<GroupRow>,
    pub by_cashier: Vec<GroupRow>,
    pub by_method: Vec<GroupRow>,
}

impl SalesReport {
    /// Sales before tax, less returns.
    pub fn net_after_returns_minor(&self) -> i64 {
        self.net_minor - self.returns_net_minor
    }
    pub fn profit_minor(&self) -> i64 {
        self.net_after_returns_minor() - self.cost_minor
    }
    /// Profit over sales after returns, in basis points.
    pub fn margin_bps(&self) -> i64 {
        let net = self.net_after_returns_minor();
        if net <= 0 {
            0
        } else {
            i64::try_from(i128::from(self.profit_minor()) * 10_000 / i128::from(net)).unwrap_or(0)
        }
    }
    /// The average sale, with tax.
    pub fn basket_minor(&self) -> i64 {
        if self.sales == 0 {
            0
        } else {
            self.total_minor / i64::from(self.sales)
        }
    }
}

fn groups(conn: &Connection, sql: &str, p: &(i64, i64, i32, String)) -> Result<Vec<GroupRow>> {
    let mut s = conn.prepare(sql)?;
    let rows = s
        .query_map(params![p.0, p.1, p.2, p.3], |r| {
            Ok(GroupRow {
                key: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                qty_milli: r.get(2)?,
                count: r.get(3)?,
                net_minor: r.get(4)?,
                cost_minor: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// Sales over a span of business days.
pub fn sales_report(store: &Store, span: &Span) -> Result<SalesReport> {
    report_of(store, span, Parts::All)
}

/// What a report computes (the dashboard needs less than the reports page).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Parts {
    All,
    /// Totals and days.
    Days,
    /// Totals, payment methods and categories.
    Mix,
}

fn report_of(store: &Store, span: &Span, parts: Parts) -> Result<SalesReport> {
    let (a, b) = span.dates()?;
    // The span as instants, so that the time indexes serve.
    let epoch = Date {
        y: 1970,
        m: 1,
        d: 1,
    };
    let off_ms = i64::from(store.utc_offset_min) * 60_000;
    let from_ms = days_between(epoch, a) * 86_400_000 - off_ms;
    let to_ms = (days_between(epoch, b) + 1) * 86_400_000 - off_ms;
    store.read(|r| {
        let conn = r.conn();
        let (org, _) = the_org(conn)?;
        let p = (from_ms, to_ms, r.utc_offset_min, org.to_string());
        let in_span = |col: &str| format!("{col} >= ?1 AND {col} < ?2");
        let sales_in = format!("s.org_id = ?4 AND {}", in_span("s.created_at"));
        let mut out = SalesReport {
            span: span.clone(),
            ..Default::default()
        };
        (
            out.sales,
            out.total_minor,
            out.discounts_minor,
            out.net_minor,
            out.tax_minor,
            out.cost_minor,
        ) = conn.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(total_minor), 0), COALESCE(SUM(discount_minor), 0),
                        COALESCE(SUM(net_minor), 0), COALESCE(SUM(tax_minor), 0), COALESCE(SUM(cost_minor), 0)
                 FROM sales s WHERE {sales_in}"
            ),
            params![p.0, p.1, p.2, p.3],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
        let returns_in = format!("x.org_id = ?4 AND {}", in_span("x.created_at"));
        let back_cost: i64;
        (out.returns, out.returns_total_minor, out.returns_net_minor, back_cost) = conn.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(total_minor), 0), COALESCE(SUM(net_minor), 0),
                        COALESCE(SUM(cost_minor), 0)
                 FROM sale_returns x WHERE {returns_in}"
            ),
            params![p.0, p.1, p.2, p.3],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        out.cost_minor -= back_cost;

        if parts == Parts::Mix {
            return mix(conn, out, &p, &sales_in);
        }
        // By day: sales and returns side by side.
        let mut days: std::collections::BTreeMap<String, DayRow> = Default::default();
        let mut s = conn.prepare(&format!(
            "SELECT {d}, COUNT(*), SUM(total_minor), SUM(net_minor), SUM(cost_minor)
             FROM sales s WHERE {sales_in} GROUP BY 1",
            d = day("s.created_at")
        ))?;
        for row in s.query_map(params![p.0, p.1, p.2, p.3], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })? {
            let (date, n, total, net, cost) = row?;
            let e = days.entry(date.clone()).or_default();
            e.date = date;
            e.sales = n;
            e.total_minor = total;
            e.net_minor += net;
            e.cost_minor += cost;
        }
        let mut s = conn.prepare(&format!(
            "SELECT {d}, SUM(total_minor), SUM(net_minor), SUM(cost_minor)
             FROM sale_returns x WHERE {returns_in} GROUP BY 1",
            d = day("x.created_at")
        ))?;
        for row in s.query_map(params![p.0, p.1, p.2, p.3], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })? {
            let (date, total, net, cost) = row?;
            let e = days.entry(date.clone()).or_default();
            e.date = date;
            e.returns_minor = total;
            e.net_minor -= net;
            e.cost_minor -= cost;
        }
        out.by_day = days.into_values().collect();

        if parts == Parts::Days {
            return Ok(out);
        }
        // By product, category and cashier, from the lines sold (what came
        // back is in the totals, not here).
        let line_cost = line_cost();
        out.by_product = groups(
            conn,
            &format!(
                "SELECT COALESCE(l.product_id, l.name), MAX(l.name), SUM(l.base_qty_milli), COUNT(DISTINCT s.id),
                        SUM(l.net_minor), SUM({line_cost})
                 FROM sale_lines l JOIN sales s ON s.id = l.sale_id WHERE {sales_in}
                 GROUP BY 1 ORDER BY 5 DESC, 2"
            ),
            &p,
        )?;
        out.by_cashier = groups(
            conn,
            &format!(
                "SELECT s.created_by, u.name, 0, COUNT(*), SUM(s.net_minor), SUM(s.cost_minor)
                 FROM sales s LEFT JOIN users u ON u.id = s.created_by
                 WHERE {sales_in} GROUP BY 1 ORDER BY 5 DESC"
            ),
            &p,
        )?;
        out.by_category = by_category(conn, &p, &sales_in)?;
        out.by_method = by_method(conn, &p, &sales_in)?;
        Ok(out)
    })
}

/// The cost of a sold line.
fn line_cost() -> &'static str {
    "(l.unit_cost_minor * l.base_qty_milli + 500) / 1000"
}

/// The payment methods and categories only (the dashboard's mix).
fn mix(
    conn: &Connection,
    mut out: SalesReport,
    p: &(i64, i64, i32, String),
    sales_in: &str,
) -> Result<SalesReport> {
    out.by_category = by_category(conn, p, sales_in)?;
    out.by_method = by_method(conn, p, sales_in)?;
    Ok(out)
}

fn by_category(
    conn: &Connection,
    p: &(i64, i64, i32, String),
    sales_in: &str,
) -> Result<Vec<GroupRow>> {
    let line_cost = line_cost();
    groups(
        conn,
            &format!(
                "SELECT c.id, c.name, SUM(l.base_qty_milli), COUNT(DISTINCT s.id), SUM(l.net_minor), SUM({line_cost})
                 FROM sale_lines l JOIN sales s ON s.id = l.sale_id
                 LEFT JOIN products pr ON pr.id = l.product_id
                 LEFT JOIN categories c ON c.id = pr.category_id
                 WHERE {sales_in} GROUP BY 1 ORDER BY 5 DESC"
            ),
        p,
    )
}

fn by_method(
    conn: &Connection,
    p: &(i64, i64, i32, String),
    sales_in: &str,
) -> Result<Vec<GroupRow>> {
    // Methods: what was taken (cash after change); goods brought back
    // in an exchange paid part of the cash shown, and are apart.
    let mut methods = groups(
        conn,
        &format!(
            "SELECT y.method, y.method, 0, COUNT(DISTINCT s.id), SUM(y.amount_minor), 0
                 FROM payments y JOIN sales s ON s.id = y.sale_id
                 WHERE {sales_in} GROUP BY 1"
        ),
        p,
    )?;
    let (n, exchanged): (u32, i64) = conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(e.used_minor), 0)
                 FROM sale_exchanges e JOIN sales s ON s.id = e.sale_id WHERE {sales_in}"
        ),
        params![p.0, p.1, p.2, p.3],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if exchanged > 0 {
        if let Some(cash) = methods.iter_mut().find(|m| m.key == "cash") {
            cash.net_minor -= exchanged;
        }
        methods.push(GroupRow {
            key: "exchange".into(),
            name: "exchange".into(),
            count: n,
            net_minor: exchanged,
            ..Default::default()
        });
    }
    methods.retain(|m| m.net_minor != 0);
    methods.sort_by_key(|m| std::cmp::Reverse(m.net_minor));
    Ok(methods)
}

/// A product's stock across warehouses.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StockRow {
    pub product_id: String,
    pub name: String,
    pub sku: String,
    pub category: String,
    pub unit: String,
    pub on_hand_milli: i64,
    pub value_minor: i64,
    pub min_stock_milli: Option<i64>,
    /// Sold in the last `days` days, in base units.
    pub sold_milli: i64,
    pub last_sale_at: Option<i64>,
}

impl StockRow {
    pub fn out(&self) -> bool {
        self.on_hand_milli <= 0
    }
    pub fn low(&self) -> bool {
        !self.out()
            && self
                .min_stock_milli
                .is_some_and(|m| self.on_hand_milli <= m)
    }
    /// In stock, and nothing sold over the window.
    pub fn idle(&self) -> bool {
        self.on_hand_milli > 0 && self.sold_milli == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StockReport {
    pub days: u32,
    pub rows: Vec<StockRow>,
    pub value_minor: i64,
    pub out: u32,
    pub low: u32,
    pub idle: u32,
    pub idle_value_minor: i64,
}

/// Every stocked product: what is there, its value (average or FIFO), what
/// sold over the last `days` days.
pub fn stock_report(store: &Store, days: u32) -> Result<StockReport> {
    let since = store.now_ms() - i64::from(days.clamp(1, 3_650)) * 86_400_000;
    store.read(|r| {
        let conn = r.conn();
        let (org, _) = the_org(conn)?;
        // FIFO: what the layers left are worth; else on hand at the average.
        let fifo = i64::from(
            crate::repo::stock::method(conn)? == nb_domain::valuation::Method::Fifo,
        );
        let mut s = conn.prepare(
            "SELECT p.id, p.name, p.sku, COALESCE(c.name, ''),
                    COALESCE((SELECT u.code FROM product_units u WHERE u.product_id = p.id ORDER BY u.position LIMIT 1), ''),
                    COALESCE(SUM(b.on_hand_milli), 0),
                    CASE WHEN ?3 = 1
                         THEN COALESCE((SELECT SUM((y.qty_milli * y.unit_cost_minor + 500) / 1000)
                                        FROM stock_layers y WHERE y.product_id = p.id), 0)
                         ELSE COALESCE(SUM((b.on_hand_milli * b.avg_cost_minor + 500) / 1000), 0) END,
                    p.min_stock_milli, COALESCE(x.sold, 0), x.last
             FROM products p
             LEFT JOIN categories c ON c.id = p.category_id
             LEFT JOIN stock_balances b ON b.product_id = p.id
             -- What sold, read once for all products.
             LEFT JOIN (SELECT l.product_id,
                               SUM(CASE WHEN s.created_at >= ?2 THEN l.base_qty_milli ELSE 0 END) AS sold,
                               MAX(s.created_at) AS last
                        FROM sale_lines l JOIN sales s ON s.id = l.sale_id
                        WHERE s.org_id = ?1 AND l.product_id IS NOT NULL
                        GROUP BY l.product_id) x ON x.product_id = p.id
             WHERE p.org_id = ?1 AND p.deleted_at IS NULL AND p.kind <> 'service'
             GROUP BY p.id ORDER BY 7 DESC, p.name",
        )?;
        let rows: Vec<StockRow> = s
            .query_map(params![org.to_string(), since, fifo], |r| {
                Ok(StockRow {
                    product_id: r.get(0)?,
                    name: r.get(1)?,
                    sku: r.get(2)?,
                    category: r.get(3)?,
                    unit: r.get(4)?,
                    on_hand_milli: r.get(5)?,
                    value_minor: r.get(6)?,
                    min_stock_milli: r.get(7)?,
                    sold_milli: r.get(8)?,
                    last_sale_at: r.get(9)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        let mut out = StockReport {
            days,
            ..Default::default()
        };
        for row in &rows {
            out.value_minor += row.value_minor.max(0);
            out.out += u32::from(row.out());
            out.low += u32::from(row.low());
            if row.idle() {
                out.idle += 1;
                out.idle_value_minor += row.value_minor;
            }
        }
        out.rows = rows;
        Ok(out)
    })
}

/// The home dashboard's period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Period {
    /// The last 7 days, by day.
    Week,
    /// The last 30 days, by day.
    Month,
    /// The last 12 months, by month.
    Year,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Dashboard {
    /// The first day of each bucket (`YYYY-MM-DD`), oldest first.
    pub labels: Vec<String>,
    /// Sales with tax, less returns, per bucket; and the period before.
    pub current: Vec<i64>,
    pub previous: Vec<i64>,
    pub orders: Vec<i64>,
    pub orders_previous: i64,
    /// What customers owe, and how many owe something.
    pub credit_minor: i64,
    pub customers_owing: u32,
    pub methods: Vec<GroupRow>,
    pub categories: Vec<GroupRow>,
    pub recent: Vec<SaleRow>,
    pub low_stock: Vec<StockRow>,
}

fn first_of_month(d: Date, back: u32) -> Date {
    let months = i64::from(d.y) * 12 + i64::from(d.m) - 1 - i64::from(back);
    Date {
        y: i32::try_from(months.div_euclid(12)).unwrap_or(d.y),
        m: u32::try_from(months.rem_euclid(12) + 1).unwrap_or(1),
        d: 1,
    }
}

/// The home dashboard over a period ending today.
pub fn dashboard(store: &Store, period: Period) -> Result<Dashboard> {
    let offset = store.utc_offset_min;
    let (y, m, d) = civil_date(store.now_ms(), offset);
    let today = Date { y, m, d };
    // Bucket starts, oldest first, and the start of the period before.
    let (starts, before): (Vec<Date>, Date) = match period {
        Period::Week | Period::Month => {
            let n = if period == Period::Week { 7 } else { 30 };
            let starts = (0..n).rev().map(|b| add_days(today, -b)).collect();
            (starts, add_days(today, -(2 * n - 1)))
        }
        Period::Year => (
            (0..12).rev().map(|b| first_of_month(today, b)).collect(),
            first_of_month(today, 23),
        ),
    };
    let first = starts.first().copied().unwrap_or(today);
    let span = Span {
        from: before.iso(),
        to: today.iso(),
    };
    let report = report_of(store, &span, Parts::Days)?;
    let in_period = |date: &str| date >= first.iso().as_str();
    let bucket = |date: &str| -> Option<usize> {
        let dt = Date::parse(date).ok()?;
        starts.iter().rposition(|s| *s <= dt)
    };
    let n = starts.len();
    let mut out = Dashboard {
        labels: starts.iter().map(|s| s.iso()).collect(),
        current: vec![0; n],
        previous: vec![0; n],
        orders: vec![0; n],
        ..Default::default()
    };
    // The period before, bucket for bucket.
    let shift = |date: &str| -> Option<usize> {
        let dt = Date::parse(date).ok()?;
        match period {
            Period::Year => {
                let months = (i64::from(first.y) * 12 + i64::from(first.m))
                    - (i64::from(dt.y) * 12 + i64::from(dt.m));
                usize::try_from(12 - months).ok().filter(|i| *i < n)
            }
            _ => usize::try_from(days_between(before, dt))
                .ok()
                .filter(|i| *i < n),
        }
    };
    for row in &report.by_day {
        let value = row.total_minor - row.returns_minor;
        if in_period(&row.date) {
            if let Some(i) = bucket(&row.date) {
                out.current[i] += value;
                out.orders[i] += i64::from(row.sales);
            }
        } else if let Some(i) = shift(&row.date) {
            out.previous[i] += value;
            out.orders_previous += i64::from(row.sales);
        }
    }
    let current = report_of(
        store,
        &Span {
            from: first.iso(),
            to: today.iso(),
        },
        Parts::Mix,
    )?;
    out.methods = current.by_method;
    out.categories = current.by_category.into_iter().take(5).collect();
    (out.credit_minor, out.customers_owing) = store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        Ok(r.conn().query_row(
            "SELECT COALESCE(SUM(balance_minor), 0), COUNT(*) FROM customers
             WHERE org_id = ?1 AND balance_minor > 0",
            [org.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })?;
    out.recent = recent_sales(store, 6)?;
    out.low_stock = running_out(store, 5)?;
    Ok(out)
}

/// The products that ran out first, then those under their minimum.
fn running_out(store: &Store, limit: u32) -> Result<Vec<StockRow>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT p.id, p.name, p.sku, COALESCE(b.oh, 0), p.min_stock_milli
             FROM products p
             LEFT JOIN (SELECT product_id, SUM(on_hand_milli) AS oh FROM stock_balances GROUP BY product_id) b
                    ON b.product_id = p.id
             WHERE p.org_id = ?1 AND p.deleted_at IS NULL AND p.kind <> 'service'
               AND (COALESCE(b.oh, 0) <= 0 OR (p.min_stock_milli IS NOT NULL AND b.oh <= p.min_stock_milli))
             ORDER BY COALESCE(b.oh, 0) > 0, COALESCE(b.oh, 0), p.name
             LIMIT ?2",
        )?;
        let rows = s
            .query_map(params![org.to_string(), limit], |r| {
                Ok(StockRow {
                    product_id: r.get(0)?,
                    name: r.get(1)?,
                    sku: r.get(2)?,
                    on_hand_milli: r.get(3)?,
                    min_stock_milli: r.get(4)?,
                    ..Default::default()
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    })
}

/// An account's amount over a span: revenue credit less debit, expenses
/// debit less credit.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResultLine {
    pub code: String,
    pub name: String,
    pub amount_minor: i64,
}

/// What the business earned over a span (classes 7 and 6 of the chart).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IncomeStatement {
    pub span: Span,
    pub revenue: Vec<ResultLine>,
    pub expenses: Vec<ResultLine>,
    pub revenue_minor: i64,
    pub expenses_minor: i64,
}

impl IncomeStatement {
    /// Profit when positive, loss when negative.
    pub fn result_minor(&self) -> i64 {
        self.revenue_minor - self.expenses_minor
    }
}

/// Revenue and expenses posted over a span, account by account.
pub fn income_statement(store: &Store, span: &Span) -> Result<IncomeStatement> {
    span.dates()?;
    let tb = crate::usecases::accounting::trial_balance(store, Some(&span.from), Some(&span.to))?;
    let mut out = IncomeStatement {
        span: span.clone(),
        ..Default::default()
    };
    for r in tb.rows.into_iter().filter(|r| !r.is_group) {
        let line = |amount_minor| ResultLine {
            code: r.code.clone(),
            name: r.name.clone(),
            amount_minor,
        };
        if r.code.starts_with('7') {
            let l = line(r.credit_minor - r.debit_minor);
            out.revenue_minor += l.amount_minor;
            if l.amount_minor != 0 {
                out.revenue.push(l);
            }
        } else if r.code.starts_with('6') {
            let l = line(r.debit_minor - r.credit_minor);
            out.expenses_minor += l.amount_minor;
            if l.amount_minor != 0 {
                out.expenses.push(l);
            }
        }
    }
    Ok(out)
}

/// VAT over a month: collected on sales, deductible on purchases.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VatMonth {
    /// `YYYY-MM`.
    pub month: String,
    pub collected_minor: i64,
    pub deductible_minor: i64,
}

impl VatMonth {
    /// To pay when positive, to carry forward when negative.
    pub fn due_minor(&self) -> i64 {
        self.collected_minor - self.deductible_minor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VatReport {
    pub span: Span,
    pub months: Vec<VatMonth>,
    pub collected_minor: i64,
    pub deductible_minor: i64,
}

impl VatReport {
    pub fn due_minor(&self) -> i64 {
        self.collected_minor - self.deductible_minor
    }
}

/// VAT posted over a span, month by month, from the accounts the
/// `vat_out` and `vat_in` roles name.
pub fn vat_report(store: &Store, span: &Span) -> Result<VatReport> {
    span.dates()?;
    store.read(|r| {
        let conn = r.conn();
        let (org, _) = the_org(conn)?;
        let role = |name: &str| -> Result<String> {
            Ok(conn.query_row(
                "SELECT account_code FROM account_roles WHERE org_id = ?1 AND role = ?2",
                params![org.to_string(), name],
                |r| r.get(0),
            )?)
        };
        let (out_code, in_code) = (role("vat_out")?, role("vat_in")?);
        let mut s = conn.prepare(
            "SELECT substr(e.date, 1, 7),
                    COALESCE(SUM(CASE WHEN l.account_code = ?4 THEN l.credit_minor - l.debit_minor END), 0),
                    COALESCE(SUM(CASE WHEN l.account_code = ?5 THEN l.debit_minor - l.credit_minor END), 0)
             FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
             WHERE e.org_id = ?1 AND e.date BETWEEN ?2 AND ?3 AND l.account_code IN (?4, ?5)
             GROUP BY 1 ORDER BY 1",
        )?;
        let months: Vec<VatMonth> = s
            .query_map(
                params![org.to_string(), span.from, span.to, out_code, in_code],
                |r| {
                    Ok(VatMonth {
                        month: r.get(0)?,
                        collected_minor: r.get(1)?,
                        deductible_minor: r.get(2)?,
                    })
                },
            )?
            .collect::<std::result::Result<_, _>>()?;
        Ok(VatReport {
            span: span.clone(),
            collected_minor: months.iter().map(|m| m.collected_minor).sum(),
            deductible_minor: months.iter().map(|m| m.deductible_minor).sum(),
            months,
        })
    })
}

/// A till session in a list.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ShiftLine {
    pub id: String,
    pub number: String,
    pub register: String,
    pub status: String,
    pub opened_at: i64,
    pub closed_at: Option<i64>,
    /// Who opened it.
    pub cashier: String,
    pub sales: u32,
    pub sales_minor: i64,
    pub expected_minor: Option<i64>,
    pub counted_minor: Option<i64>,
    /// Counted less expected: over when positive, short when negative.
    pub difference_minor: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ShiftReport {
    pub span: Span,
    pub rows: Vec<ShiftLine>,
    pub over_minor: i64,
    pub short_minor: i64,
}

/// Till sessions opened over a span, newest first.
pub fn shift_report(store: &Store, span: &Span) -> Result<ShiftReport> {
    span.dates()?;
    store.read(|r| {
        let conn = r.conn();
        let (org, _) = the_org(conn)?;
        let mut s = conn.prepare(&format!(
            "SELECT h.id, h.number, COALESCE(c.name, ''), h.status, h.opened_at, h.closed_at,
                    COALESCE(u.name, ''),
                    (SELECT COUNT(*) FROM sales s WHERE s.shift_id = h.id),
                    (SELECT COALESCE(SUM(total_minor), 0) FROM sales s WHERE s.shift_id = h.id),
                    h.expected_minor, h.counted_minor, h.difference_minor
             FROM shifts h
             LEFT JOIN cash_registers c ON c.id = h.register_id
             LEFT JOIN users u ON u.id = h.opened_by
             WHERE h.org_id = ?4 AND {} BETWEEN ?1 AND ?2
             ORDER BY h.opened_at DESC",
            day("h.opened_at")
        ))?;
        let rows: Vec<ShiftLine> = s
            .query_map(
                params![span.from, span.to, r.utc_offset_min, org.to_string()],
                |r| {
                    Ok(ShiftLine {
                        id: r.get(0)?,
                        number: r.get(1)?,
                        register: r.get(2)?,
                        status: r.get(3)?,
                        opened_at: r.get(4)?,
                        closed_at: r.get(5)?,
                        cashier: r.get(6)?,
                        sales: r.get(7)?,
                        sales_minor: r.get(8)?,
                        expected_minor: r.get(9)?,
                        counted_minor: r.get(10)?,
                        difference_minor: r.get(11)?,
                    })
                },
            )?
            .collect::<std::result::Result<_, _>>()?;
        let mut out = ShiftReport {
            span: span.clone(),
            ..Default::default()
        };
        for d in rows.iter().filter_map(|r| r.difference_minor) {
            if d > 0 {
                out.over_minor += d;
            } else {
                out.short_minor -= d;
            }
        }
        out.rows = rows;
        Ok(out)
    })
}

/// Rows as CSV for a spreadsheet (UTF-8 with its mark, fields quoted when
/// they need it, CRLF lines).
pub fn to_csv(rows: &[Vec<String>]) -> String {
    let mut out = String::from("\u{feff}");
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .map(|f| {
                if f.contains([',', '"', '\n', '\r', ';']) {
                    format!("\"{}\"", f.replace('"', "\"\""))
                } else {
                    f.clone()
                }
            })
            .collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_what_needs_it() {
        let s = to_csv(&[
            vec!["a".into(), "b,c".into()],
            vec!["say \"hi\"".into(), "x".into()],
        ]);
        assert_eq!(s, "\u{feff}a,\"b,c\"\r\n\"say \"\"hi\"\"\",x\r\n");
    }

    #[test]
    fn months_back_cross_the_year() {
        let d = Date {
            y: 2026,
            m: 2,
            d: 14,
        };
        assert_eq!(first_of_month(d, 0).iso(), "2026-02-01");
        assert_eq!(first_of_month(d, 2).iso(), "2025-12-01");
        assert_eq!(first_of_month(d, 13).iso(), "2025-01-01");
    }
}

//! The documents' values (M12.8): a sale (receipt or invoice), a customer
//! receipt, an instalment schedule, a shelf label, a report — formatted in
//! the shop's language as [`DocData`] for a template to show. Amounts
//! arrive in minor units and are formatted here; nothing is computed but
//! what the document already says. The shop's switches ([`Options`]) apply
//! to every template. The functions named after each document print it
//! with the built-in template.

use nb_core::{Currency, Money, Quantity};
use nb_i18n::{Format, Lang, tr, tr_args};

use std::collections::BTreeMap;

use nb_domain::template::{DocKind, ItemField, Section, Template, TotalLine, builtin};

use crate::logo::Gray;
use crate::render::{DocData, render};
use crate::{Doc, Paper};

/// The shop as its documents present it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Shop {
    pub name: String,
    pub address: String,
    pub phone: String,
    pub nif: String,
    pub nis: String,
    pub rc: String,
    pub ai: String,
    /// A line at the bottom of receipts (empty: a thank-you).
    pub footer: String,
}

/// What the shop chose to show on its documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub logo: Option<Gray>,
    /// A line under the shop's name (what it sells, a slogan).
    pub tagline: String,
    /// NIF, NIS, RC, AI in the header.
    pub legal_ids: bool,
    /// The VAT by rate on receipts (invoices always state it).
    pub vat: bool,
    pub cashier: bool,
    /// The lines as a table (item, quantity, price, amount); otherwise
    /// each line on two rows.
    pub table: bool,
    /// The number of items under the lines.
    pub count: bool,
    /// The customer's previous balance, what was paid and the new balance
    /// ("bon par bon").
    pub balance: bool,
    /// The sale's number as a barcode (scanned back at the till).
    pub barcode: bool,
    /// A notice at the very bottom (returns policy…).
    pub notice: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            logo: None,
            tagline: String::new(),
            legal_ids: true,
            vat: true,
            cashier: true,
            table: true,
            count: true,
            balance: true,
            barcode: true,
            notice: String::new(),
        }
    }
}

/// A line of a sale, as sold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub name: String,
    pub qty_milli: i64,
    pub unit: String,
    pub unit_price_minor: i64,
    /// What the line came to (after its discount).
    pub amount_minor: i64,
    pub discount_minor: i64,
    /// The product's reference.
    pub sku: String,
    /// VAT rate in basis points.
    pub tax_bps: i64,
    /// The line before tax.
    pub net_minor: i64,
}

/// A sale as its documents show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sale {
    pub number: String,
    /// Local date and time: year, month, day, hour, minute.
    pub when: (i32, u32, u32, u32, u32),
    pub cashier: String,
    pub register: String,
    /// The customer's name and tax numbers, if any.
    pub customer: Option<(String, String)>,
    pub currency: Currency,
    pub lines: Vec<Line>,
    pub discount_minor: i64,
    pub net_minor: i64,
    /// VAT by rate: (basis points, base before tax, tax).
    pub taxes: Vec<(i64, i64, i64)>,
    /// Stamp duty on a cash payment.
    pub stamp_minor: i64,
    pub total_minor: i64,
    /// (method code, amount handed over).
    pub payments: Vec<(String, i64)>,
    pub change_minor: i64,
    /// Left to pay (on the customer's account).
    pub owed_minor: i64,
    /// The customer's balance before and after this sale (what they owe).
    pub balance: Option<(i64, i64)>,
    /// Of the payments, what paid older debt.
    pub settled_minor: i64,
}

struct Ctx {
    lang: Lang,
    f: Format,
    currency: Currency,
}

impl Ctx {
    fn new(lang: Lang, currency: Currency) -> Self {
        Self {
            lang,
            f: Format::new(lang),
            currency,
        }
    }
    fn t(&self, key: &str) -> String {
        tr(self.lang, key).into_owned()
    }
    fn m(&self, minor: i64) -> String {
        self.f.money(Money::from_minor(minor, self.currency))
    }
    /// An amount without its currency (table cells).
    fn a(&self, minor: i64) -> String {
        self.f.amount(Money::from_minor(minor, self.currency))
    }
    fn q(&self, milli: i64) -> String {
        self.f.quantity(Quantity::from_milli(milli), 3)
    }
    fn n(&self, n: i64) -> String {
        self.f.integer(n)
    }
    fn rate(&self, bps: i64) -> String {
        let whole = bps / 100;
        let frac = bps % 100;
        nb_i18n::bidi::isolate_ltr(&if frac == 0 {
            format!("{whole}%")
        } else {
            format!("{whole}.{frac:02}%")
        })
    }
    fn method(&self, code: &str) -> String {
        let key = format!("pay.{code}");
        let t = tr(self.lang, &key);
        if t == key {
            code.to_owned()
        } else {
            t.into_owned()
        }
    }
}

/// No break inside: spaces become no-break spaces.
fn unbroken(s: &str) -> String {
    s.replace(' ', "\u{a0}")
}

fn when_of(c: &Ctx, (y, m, d, h, mi): (i32, u32, u32, u32, u32)) -> (String, String) {
    let date = nb_i18n::bidi::strip_controls(&c.f.date(y, m, d));
    let time = nb_i18n::bidi::strip_controls(&c.f.time(h, mi));
    (
        nb_i18n::bidi::isolate_ltr(&unbroken(&date)),
        nb_i18n::bidi::isolate_ltr(&unbroken(&time)),
    )
}

fn ltr(s: &str) -> String {
    nb_i18n::bidi::isolate_ltr(&unbroken(s))
}

fn legal_ids(shop: &Shop) -> String {
    [
        ("NIF", &shop.nif),
        ("NIS", &shop.nis),
        ("RC", &shop.rc),
        ("AI", &shop.ai),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(k, v)| ltr(&format!("{k}: {v}")))
    .collect::<Vec<_>>()
    .join(" · ")
}

/// What every document shares: the shop, the switches, the date.
fn common(
    c: &Ctx,
    shop: &Shop,
    o: &Options,
    title: String,
    number: &str,
    when: (i32, u32, u32, u32, u32),
    cashier: &str,
) -> DocData {
    let (date, time) = when_of(c, when);
    let mut d = DocData {
        logo: o.logo.clone(),
        legal_ids: legal_ids(shop),
        ..DocData::default()
    };
    let footer = if shop.footer.is_empty() {
        c.t("print.thanks")
    } else {
        shop.footer.clone()
    };
    for (k, v) in [
        ("shop.name", shop.name.clone()),
        ("shop.tagline", o.tagline.clone()),
        ("shop.address", shop.address.clone()),
        ("shop.phone", shop.phone.clone()),
        ("shop.email", String::new()),
        ("shop.nif", shop.nif.clone()),
        ("shop.nis", shop.nis.clone()),
        ("shop.rc", shop.rc.clone()),
        ("shop.ai", shop.ai.clone()),
        ("doc.title", title),
        ("doc.number", number.to_owned()),
        ("doc.date", format!("{date} {time}")),
        ("doc.time", time),
        ("doc.cashier", cashier.to_owned()),
        ("notice", o.notice.clone()),
        ("footer", footer),
    ] {
        d.vars.insert(k.into(), v);
    }
    d
}

fn details(c: &Ctx, d: &mut DocData, number_key: &str, o: &Options, register: &str) {
    let number = d.var("doc.number").unwrap_or_default();
    d.details.push((c.t(number_key), ltr(&number)));
    d.details.push((
        c.t("print.label.date"),
        d.var("doc.date").unwrap_or_default(),
    ));
    if o.cashier {
        d.details.push((
            c.t("print.label.cashier"),
            d.var("doc.cashier").unwrap_or_default(),
        ));
        if !register.is_empty() {
            d.details
                .push((c.t("print.label.register"), register.to_owned()));
        }
    }
}

fn customer_lines(d: &mut DocData, name: &str, ids: &str) {
    d.customer.push(name.to_owned());
    if !ids.is_empty() {
        d.customer.push(ltr(ids));
    }
    d.vars.insert("customer.name".into(), name.to_owned());
    d.vars.insert("customer.ids".into(), ids.to_owned());
    d.vars.insert("customer.phone".into(), String::new());
}

/// A sale's values (a receipt or an invoice).
pub fn sale_data(shop: &Shop, s: &Sale, lang: Lang, o: &Options, kind: DocKind) -> DocData {
    let c = Ctx::new(lang, s.currency);
    let title = c.t(if kind == DocKind::Invoice {
        "print.title.invoice"
    } else {
        "print.title.receipt"
    });
    let mut d = common(&c, shop, o, title, &s.number, s.when, &s.cashier);
    details(
        &c,
        &mut d,
        if kind == DocKind::Invoice {
            "print.label.invoice_no"
        } else {
            "print.label.receipt_no"
        },
        o,
        &s.register,
    );
    if let Some((name, ids)) = &s.customer {
        customer_lines(&mut d, name, ids);
    }
    d.vars.insert("doc.register".into(), s.register.clone());
    for (i, l) in s.lines.iter().enumerate() {
        let q = ltr(&c.q(l.qty_milli));
        let unit = if l.unit == "pc" || l.unit == "u" {
            String::new()
        } else {
            l.unit.clone()
        };
        let mut r = BTreeMap::new();
        r.insert(ItemField::Index, ltr(&c.n(i as i64 + 1)));
        r.insert(ItemField::Name, l.name.clone());
        r.insert(ItemField::Sku, ltr(&l.sku));
        r.insert(ItemField::Qty, q.clone());
        r.insert(ItemField::Unit, unit.clone());
        r.insert(
            ItemField::QtyUnit,
            if unit.is_empty() {
                q
            } else {
                format!("{q} {unit}")
            },
        );
        r.insert(ItemField::Price, c.a(l.unit_price_minor));
        r.insert(
            ItemField::Discount,
            if l.discount_minor > 0 {
                c.a(-l.discount_minor)
            } else {
                String::new()
            },
        );
        r.insert(
            ItemField::TaxRate,
            if l.tax_bps > 0 {
                c.rate(l.tax_bps)
            } else {
                String::new()
            },
        );
        r.insert(ItemField::Net, c.a(l.net_minor));
        r.insert(ItemField::Amount, c.a(l.amount_minor));
        d.rows.push(r);
    }
    if o.count {
        d.vars.insert(
            "items.count".into(),
            tr_args(
                lang,
                "print.count",
                &[("count", &ltr(&c.n(s.lines.len() as i64)))],
            ),
        );
    }
    let mut t = |k: TotalLine, label: String, value: String, strong: bool| {
        d.totals.push((k, label, value, strong));
    };
    if s.discount_minor > 0 {
        t(
            TotalLine::Discount,
            c.t("print.discount"),
            c.m(-s.discount_minor),
            false,
        );
    }
    // VAT always on invoices; on receipts as the shop chose.
    if kind == DocKind::Invoice || o.vat {
        t(TotalLine::Net, c.t("print.net"), c.m(s.net_minor), false);
        for (bps, _, tax) in &s.taxes {
            if *tax != 0 {
                t(
                    TotalLine::Vat,
                    tr_args(lang, "print.vat", &[("rate", &c.rate(*bps))]),
                    c.m(*tax),
                    false,
                );
            }
        }
    }
    if s.stamp_minor > 0 {
        t(
            TotalLine::Stamp,
            c.t("print.stamp"),
            c.m(s.stamp_minor),
            false,
        );
    }
    t(
        TotalLine::Total,
        c.t("print.total"),
        c.m(s.total_minor),
        true,
    );
    for (m, a) in &s.payments {
        t(TotalLine::Payments, c.method(m), c.m(*a), false);
    }
    if s.change_minor > 0 {
        t(
            TotalLine::Change,
            c.t("print.change"),
            c.m(s.change_minor),
            false,
        );
    }
    let paid = s.payments.iter().map(|p| p.1).sum::<i64>() - s.change_minor;
    let with_balance = o.balance && s.balance.is_some();
    if s.owed_minor > 0 && !with_balance {
        t(TotalLine::Owed, c.t("print.owed"), c.m(s.owed_minor), true);
    }
    if let (true, Some((before, after))) = (o.balance, s.balance) {
        let mut b = vec![
            (c.t("print.balance.before"), c.m(before), false),
            (c.t("print.balance.this"), c.m(s.total_minor), false),
            (c.t("print.balance.sum"), c.m(before + s.total_minor), false),
        ];
        if s.settled_minor > 0 {
            b.push((c.t("print.balance.settled"), c.m(s.settled_minor), false));
        }
        b.push((c.t("print.balance.paid"), c.m(paid), false));
        b.push((c.t("print.balance.after"), c.m(after), true));
        d.balance = b;
        d.vars.insert("balance.before".into(), c.m(before));
        d.vars.insert("balance.after".into(), c.m(after));
    }
    for (k, v) in [
        ("total", c.m(s.total_minor)),
        ("paid", c.m(paid)),
        ("change", c.m(s.change_minor)),
        ("owed", c.m(s.owed_minor)),
    ] {
        d.vars.insert(k.into(), v);
    }
    d
}

/// The shop's switches over a template: what the shop turned off stays
/// off, whatever the template says.
pub fn apply(t: &Template, o: &Options) -> Template {
    let mut t = t.clone();
    t.sections.retain(|s| match s {
        Section::Balance => o.balance,
        Section::Barcode { value } => o.barcode || !value.contains("{doc.number}"),
        _ => true,
    });
    for s in &mut t.sections {
        if let Section::Header { legal_ids } = s
            && !o.legal_ids
            && t.kind == DocKind::Receipt
        {
            *legal_ids = false;
        }
    }
    if !o.table && t.kind == DocKind::Receipt {
        for s in &mut t.sections {
            if let Section::Items { columns } = s {
                columns.retain(|c| {
                    matches!(
                        c.field,
                        ItemField::Name | ItemField::Qty | ItemField::QtyUnit | ItemField::Amount
                    )
                });
            }
        }
    }
    if o.logo.is_none() {
        t.style.logo = false;
    }
    t
}

fn first(kind: DocKind) -> Template {
    builtin(kind).remove(0).1
}

/// A sale's receipt for a thermal roll, with the built-in template.
pub fn receipt(shop: &Shop, sale: &Sale, lang: Lang, paper: Paper, o: &Options) -> Doc {
    let t = apply(&first(DocKind::Receipt), o);
    render(
        &t,
        &sale_data(shop, sale, lang, o, DocKind::Receipt),
        lang,
        paper,
    )
}

/// A sale's invoice on a sheet (A4 or A5), with the built-in template.
pub fn invoice(shop: &Shop, sale: &Sale, lang: Lang, paper: Paper, o: &Options) -> Doc {
    let t = apply(&first(DocKind::Invoice), o);
    render(
        &t,
        &sale_data(shop, sale, lang, o, DocKind::Invoice),
        lang,
        paper,
    )
}

/// A product's shelf label: name, price (and per unit), its barcode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfLabel {
    pub name: String,
    pub price_minor: i64,
    /// The unit the price is for (`kg`, `L`…); empty for a piece.
    pub unit: String,
    pub currency: Currency,
    /// The barcode's digits (EAN-13 or any text for Code 128).
    pub barcode: Option<String>,
    pub sku: String,
}

/// A label's values.
pub fn label_data(l: &ShelfLabel, lang: Lang, shop: Option<&str>) -> DocData {
    let c = Ctx::new(lang, l.currency);
    let price = if l.unit.is_empty() || l.unit == "pc" || l.unit == "u" {
        c.m(l.price_minor)
    } else {
        format!("{} / {}", c.m(l.price_minor), l.unit)
    };
    let mut d = DocData::default();
    for (k, v) in [
        ("shop.name", shop.unwrap_or_default().to_owned()),
        ("item.name", l.name.clone()),
        ("item.price", price),
        ("item.unit", l.unit.clone()),
        ("item.barcode", l.barcode.clone().unwrap_or_default()),
        ("item.sku", l.sku.clone()),
    ] {
        d.vars.insert(k.into(), v);
    }
    d
}

/// A shelf or product label on `paper`, with the built-in template.
pub fn shelf_label(l: &ShelfLabel, lang: Lang, paper: Paper, shop: Option<&str>) -> Doc {
    render(
        &first(DocKind::Label),
        &label_data(l, lang, shop),
        lang,
        paper,
    )
}

/// A customer receipt (سند القبض / bon de versement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voucher {
    pub number: String,
    pub when: (i32, u32, u32, u32, u32),
    pub cashier: String,
    /// Name and tax numbers.
    pub customer: (String, String),
    pub currency: Currency,
    pub method: String,
    pub amount_minor: i64,
    pub reference: String,
    pub note: String,
    /// (sale number, amount).
    pub invoices: Vec<(String, i64)>,
    /// (plan number, instalment, amount).
    pub installments: Vec<(String, u32, i64)>,
    pub unallocated_minor: i64,
}

/// A voucher's values.
pub fn voucher_data(shop: &Shop, v: &Voucher, lang: Lang, o: &Options) -> DocData {
    let c = Ctx::new(lang, v.currency);
    let mut d = common(
        &c,
        shop,
        o,
        c.t("print.voucher.title"),
        &v.number,
        v.when,
        &v.cashier,
    );
    details(&c, &mut d, "print.label.voucher_no", o, "");
    customer_lines(&mut d, &v.customer.0, &v.customer.1);
    for (k, val) in [
        ("amount", c.m(v.amount_minor)),
        ("method", c.method(&v.method)),
        ("reference", v.reference.clone()),
        ("note", v.note.clone()),
    ] {
        d.vars.insert(k.into(), val);
    }
    let mut row = |name: String, amount: i64| {
        let mut r = BTreeMap::new();
        r.insert(ItemField::Index, ltr(&c.n(d.rows.len() as i64 + 1)));
        r.insert(ItemField::Name, name);
        r.insert(ItemField::Amount, c.a(amount));
        d.rows.push(r);
    };
    for (n, a) in &v.invoices {
        row(ltr(n), *a);
    }
    for (plan, seq, a) in &v.installments {
        row(
            tr_args(
                lang,
                "print.voucher.installment",
                &[("plan", &ltr(plan)), ("seq", &ltr(&c.n(i64::from(*seq))))],
            ),
            *a,
        );
    }
    if v.unallocated_minor > 0 {
        row(c.t("print.voucher.advance"), v.unallocated_minor);
    }
    d.totals.push((
        TotalLine::Total,
        c.t("print.voucher.amount"),
        c.m(v.amount_minor),
        true,
    ));
    d
}

/// A receipt voucher, with the built-in template.
pub fn voucher(shop: &Shop, v: &Voucher, lang: Lang, paper: Paper, o: &Options) -> Doc {
    let t = apply(&first(DocKind::Voucher), o);
    render(&t, &voucher_data(shop, v, lang, o), lang, paper)
}

/// An instalment of a schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub seq: u32,
    pub date: (i32, u32, u32),
    pub amount_minor: i64,
    pub paid_minor: i64,
    /// `upcoming`, `due`, `partial`, `paid`, `late`.
    pub state: String,
}

/// An instalment schedule (جدول الأقساط).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub number: String,
    pub version: u32,
    pub when: (i32, u32, u32, u32, u32),
    pub customer: (String, String),
    pub currency: Currency,
    pub sale_number: String,
    pub total_minor: i64,
    pub down_minor: i64,
    pub financed_minor: i64,
    pub dues: Vec<Due>,
}

/// A schedule's values.
pub fn schedule_data(shop: &Shop, s: &Schedule, lang: Lang, o: &Options) -> DocData {
    let c = Ctx::new(lang, s.currency);
    let mut d = common(
        &c,
        shop,
        o,
        c.t("print.schedule.title"),
        &s.number,
        s.when,
        "",
    );
    let paid: i64 = s.dues.iter().map(|x| x.paid_minor).sum();
    d.details
        .push((c.t("print.label.schedule_no"), ltr(&s.number)));
    d.details
        .push((c.t("print.label.version"), ltr(&c.n(i64::from(s.version)))));
    d.details.push((
        c.t("print.label.date"),
        d.var("doc.date").unwrap_or_default(),
    ));
    d.details
        .push((c.t("print.label.sale"), ltr(&s.sale_number)));
    customer_lines(&mut d, &s.customer.0, &s.customer.1);
    for (k, v) in [
        ("doc.version", c.n(i64::from(s.version))),
        ("sale.number", s.sale_number.clone()),
        ("sale.total", c.m(s.total_minor)),
        ("down", c.m(s.down_minor)),
        ("financed", c.m(s.financed_minor)),
        ("paid", c.m(paid)),
        ("left", c.m((s.financed_minor - paid).max(0))),
    ] {
        d.vars.insert(k.into(), v);
    }
    for x in &s.dues {
        let (y, m, dd) = x.date;
        let mut r = BTreeMap::new();
        r.insert(ItemField::Index, ltr(&c.n(i64::from(x.seq))));
        r.insert(ItemField::Due, ltr(&c.f.date(y, m, dd)));
        r.insert(ItemField::Amount, c.a(x.amount_minor));
        r.insert(ItemField::Paid, c.a(x.paid_minor));
        r.insert(ItemField::State, c.t(&format!("plans.state.{}", x.state)));
        d.rows.push(r);
    }
    let mut t = |label: String, value: String, strong: bool| {
        d.totals.push((TotalLine::Total, label, value, strong))
    };
    t(c.t("print.schedule.sale_total"), c.m(s.total_minor), false);
    if s.down_minor > 0 {
        t(c.t("print.schedule.down"), c.m(s.down_minor), false);
    }
    t(c.t("print.schedule.financed"), c.m(s.financed_minor), false);
    t(c.t("print.schedule.paid_total"), c.m(paid), false);
    t(
        c.t("print.schedule.left"),
        c.m((s.financed_minor - paid).max(0)),
        true,
    );
    d
}

/// An instalment schedule, with the built-in template.
pub fn schedule(shop: &Shop, s: &Schedule, lang: Lang, paper: Paper, o: &Options) -> Doc {
    let t = apply(&first(DocKind::Schedule), o);
    render(&t, &schedule_data(shop, s, lang, o), lang, paper)
}

/// A report's values: its title, what it covers, its rows (header first).
pub fn report_data(
    shop: &Shop,
    title: &str,
    period: &str,
    rows: &[Vec<String>],
    o: &Options,
) -> DocData {
    let mut d = DocData {
        logo: o.logo.clone(),
        legal_ids: legal_ids(shop),
        ..DocData::default()
    };
    for (k, v) in [
        ("shop.name", shop.name.clone()),
        ("shop.tagline", o.tagline.clone()),
        ("shop.address", shop.address.clone()),
        ("shop.phone", shop.phone.clone()),
        ("report.title", title.to_owned()),
        ("report.period", period.to_owned()),
        ("report.rows", rows.len().saturating_sub(1).to_string()),
        ("doc.title", title.to_owned()),
    ] {
        d.vars.insert(k.into(), v);
    }
    if let Some((h, r)) = rows.split_first() {
        d.table = Some((h.clone(), r.to_vec()));
    }
    d
}

/// A report's table on sheets (M13.5), with the built-in template.
pub fn report_table(
    shop: &str,
    title: &str,
    subtitle: &str,
    rows: &[Vec<String>],
    lang: Lang,
    paper: Paper,
) -> Doc {
    let s = Shop {
        name: shop.to_owned(),
        ..Shop::default()
    };
    let o = Options::default();
    render(
        &first(DocKind::Report),
        &report_data(&s, title, subtitle, rows, &o),
        lang,
        paper,
    )
}

//! The default documents (M12.8): a sale's receipt for the till (any roll
//! width) and its invoice on a sheet (A4 or A5), and a product's shelf
//! label, in the shop's language. What shows is the shop's choice
//! ([`Options`]). Amounts arrive in minor units and are formatted here;
//! nothing is computed but what the sale already says (the customer's
//! running balance included).

use nb_core::{Currency, Money, Quantity};
use nb_i18n::{Format, Lang, tr, tr_args};

use crate::logo::Gray;
use crate::{Block, Cell, Doc, Paper, Place};

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

fn logo_block(o: &Options, width: f32) -> Option<Block> {
    o.logo.as_ref().map(|g| Block::Image {
        gray: g.px.clone(),
        w: g.w,
        h: g.h,
        // Never blown up past one and a half of its own size.
        width: width.min(g.w as f32 * 72.0 / 203.0 * 1.5),
    })
}

fn header(c: &Ctx, shop: &Shop, o: &Options, big: f32, small: f32, logo_w: f32) -> Vec<Block> {
    let mut b = Vec::new();
    if let Some(l) = logo_block(o, logo_w) {
        b.push(l);
    }
    b.push(Block::text(&shop.name, big, true, Place::Center));
    if !o.tagline.is_empty() {
        b.push(Block::text(&o.tagline, small, false, Place::Center));
    }
    if !shop.address.is_empty() {
        b.push(Block::text(&shop.address, small, false, Place::Center));
    }
    if !shop.phone.is_empty() {
        // One piece: a phone number never breaks across lines.
        b.push(Block::text(
            format!(
                "{} {}",
                c.t("print.phone"),
                nb_i18n::bidi::isolate_ltr(&unbroken(&shop.phone))
            ),
            small,
            false,
            Place::Center,
        ));
    }
    if o.legal_ids {
        let ids: Vec<String> = [
            ("NIF", &shop.nif),
            ("NIS", &shop.nis),
            ("RC", &shop.rc),
            ("AI", &shop.ai),
        ]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| nb_i18n::bidi::isolate_ltr(&unbroken(&format!("{k}: {v}"))))
        .collect();
        if !ids.is_empty() {
            b.push(Block::text(
                ids.join(" · "),
                small - 1.0,
                false,
                Place::Center,
            ));
        }
    }
    b
}

/// No break inside: spaces become no-break spaces.
fn unbroken(s: &str) -> String {
    s.replace(' ', "\u{a0}")
}

/// Date and time as one left-to-right piece.
fn when(c: &Ctx, s: &Sale) -> String {
    let (y, m, d, h, mi) = s.when;
    let plain =
        nb_i18n::bidi::strip_controls(&format!("{} {}", c.f.date(y, m, d), c.f.time(h, mi)));
    nb_i18n::bidi::isolate_ltr(&unbroken(&plain))
}

/// Everything handed over, less the change: what was paid.
fn paid(s: &Sale) -> i64 {
    s.payments.iter().map(|p| p.1).sum::<i64>() - s.change_minor
}

fn totals(c: &Ctx, s: &Sale, o: &Options, size: f32, receipt: bool) -> Vec<Block> {
    let mut b = Vec::new();
    if s.discount_minor > 0 {
        b.push(Block::pair(
            c.t("print.discount"),
            c.m(-s.discount_minor),
            size,
            false,
        ));
    }
    if !receipt || o.vat {
        b.push(Block::pair(c.t("print.net"), c.m(s.net_minor), size, false));
        for (bps, _, tax) in &s.taxes {
            if *tax != 0 {
                b.push(Block::pair(
                    tr_args(c.lang, "print.vat", &[("rate", &c.rate(*bps))]),
                    c.m(*tax),
                    size,
                    false,
                ));
            }
        }
    }
    if s.stamp_minor > 0 {
        b.push(Block::pair(
            c.t("print.stamp"),
            c.m(s.stamp_minor),
            size,
            false,
        ));
    }
    b.push(Block::pair(
        c.t("print.total"),
        c.m(s.total_minor),
        size + 4.0,
        true,
    ));
    for (method, amount) in &s.payments {
        b.push(Block::pair(c.method(method), c.m(*amount), size, false));
    }
    if s.change_minor > 0 {
        b.push(Block::pair(
            c.t("print.change"),
            c.m(s.change_minor),
            size,
            false,
        ));
    }
    match (o.balance, s.balance) {
        (true, Some((before, after))) => {
            b.push(Block::Rule);
            b.push(Block::pair(
                c.t("print.balance.before"),
                c.m(before),
                size,
                false,
            ));
            b.push(Block::pair(
                c.t("print.balance.this"),
                c.m(s.total_minor),
                size,
                false,
            ));
            b.push(Block::pair(
                c.t("print.balance.sum"),
                c.m(before + s.total_minor),
                size,
                false,
            ));
            if s.settled_minor > 0 {
                b.push(Block::pair(
                    c.t("print.balance.settled"),
                    c.m(s.settled_minor),
                    size,
                    false,
                ));
            }
            b.push(Block::pair(
                c.t("print.balance.paid"),
                c.m(paid(s)),
                size,
                false,
            ));
            b.push(Block::pair(
                c.t("print.balance.after"),
                c.m(after),
                size,
                true,
            ));
        }
        _ => {
            if s.owed_minor > 0 {
                b.push(Block::pair(
                    c.t("print.owed"),
                    c.m(s.owed_minor),
                    size,
                    true,
                ));
            }
        }
    }
    b
}

fn count_line(c: &Ctx, s: &Sale, size: f32) -> Block {
    Block::text(
        tr_args(
            c.lang,
            "print.count",
            &[(
                "count",
                &nb_i18n::bidi::isolate_ltr(&c.n(s.lines.len() as i64)),
            )],
        ),
        size,
        false,
        Place::Start,
    )
}

fn barcode(s: &Sale, width: f32) -> Option<Block> {
    crate::barcode::code128(&s.number).map(|modules| Block::Bars {
        modules,
        width,
        height: 28.0,
        caption: Some(s.number.clone()),
    })
}

fn qty_text(c: &Ctx, l: &Line) -> String {
    let q = nb_i18n::bidi::isolate_ltr(&c.q(l.qty_milli));
    if l.unit.is_empty() || l.unit == "pc" || l.unit == "u" {
        q
    } else {
        format!("{q} {}", l.unit)
    }
}

/// A sale's receipt for a thermal roll.
pub fn receipt(shop: &Shop, sale: &Sale, lang: Lang, paper: Paper, o: &Options) -> Doc {
    let c = Ctx::new(lang, sale.currency);
    let narrow = paper.is_narrow();
    let (big, body, small) = if narrow {
        (13.0, 9.0, 8.0)
    } else {
        (15.0, 10.0, 9.0)
    };
    let (pw, _) = paper.sheet();
    let mut b = header(&c, shop, o, big, small, pw * 0.6);
    b.push(Block::Rule);
    b.push(Block::pair(
        tr_args(lang, "print.receipt_no", &[("number", &sale.number)]),
        when(&c, sale),
        small,
        false,
    ));
    if o.cashier {
        b.push(Block::pair(
            tr_args(lang, "print.cashier", &[("name", &sale.cashier)]),
            sale.register.clone(),
            small,
            false,
        ));
    }
    if let Some((name, _)) = &sale.customer {
        b.push(Block::text(
            tr_args(lang, "print.customer", &[("name", name)]),
            small,
            true,
            Place::Start,
        ));
    }
    if o.table {
        // Item | quantity | (price) | amount, as on the tills people know.
        let shares: &[f32] = if narrow {
            &[0.50, 0.18, 0.32]
        } else {
            &[0.40, 0.16, 0.21, 0.23]
        };
        let heads: &[&str] = if narrow {
            &["print.col.item", "print.col.qty", "print.col.amount"]
        } else {
            &[
                "print.col.item",
                "print.col.qty",
                "print.col.unit_price",
                "print.col.amount",
            ]
        };
        b.push(Block::Line);
        b.push(Block::Row {
            cells: heads
                .iter()
                .zip(shares)
                .enumerate()
                .map(|(i, (k, sh))| {
                    Cell::new(c.t(k), *sh, if i == 0 { Place::Start } else { Place::End })
                })
                .collect(),
            size: small,
            bold: true,
        });
        b.push(Block::Line);
        for l in &sale.lines {
            let mut cells = vec![
                Cell::new(&l.name, shares[0], Place::Start),
                // The figure alone: a unit would wrap in so narrow a column.
                Cell::new(
                    nb_i18n::bidi::isolate_ltr(&c.q(l.qty_milli)),
                    shares[1],
                    Place::End,
                ),
            ];
            if !narrow {
                cells.push(Cell::new(c.a(l.unit_price_minor), shares[2], Place::End));
            }
            cells.push(Cell::new(
                c.a(l.amount_minor),
                shares[shares.len() - 1],
                Place::End,
            ));
            b.push(Block::Row {
                cells,
                size: body,
                bold: false,
            });
            if l.discount_minor > 0 {
                b.push(Block::pair(
                    c.t("print.line_discount"),
                    c.m(-l.discount_minor),
                    small,
                    false,
                ));
            }
        }
        b.push(Block::Line);
    } else {
        b.push(Block::Rule);
        for l in &sale.lines {
            b.push(Block::text(&l.name, body, false, Place::Start));
            b.push(Block::pair(
                format!("{} × {}", qty_text(&c, l), c.m(l.unit_price_minor)),
                c.m(l.amount_minor),
                body,
                false,
            ));
            if l.discount_minor > 0 {
                b.push(Block::pair(
                    c.t("print.line_discount"),
                    c.m(-l.discount_minor),
                    small,
                    false,
                ));
            }
        }
        b.push(Block::Rule);
    }
    if o.count {
        b.push(count_line(&c, sale, small));
    }
    b.extend(totals(&c, sale, o, body, true));
    b.push(Block::Rule);
    let footer = if shop.footer.is_empty() {
        c.t("print.thanks")
    } else {
        shop.footer.clone()
    };
    b.push(Block::text(footer, small, true, Place::Center));
    if !o.notice.is_empty() {
        b.push(Block::text(&o.notice, small - 0.5, false, Place::Center));
    }
    if o.barcode
        && let Some(bars) = barcode(sale, pw * 0.8)
    {
        b.push(bars);
    }
    b.push(Block::Gap(6.0));
    Doc {
        paper,
        rtl: lang == Lang::Ar,
        blocks: b,
    }
}

/// A row of the totals on a sheet: the label and the amount in the last
/// columns.
fn total_row(label: String, value: String, size: f32, bold: bool) -> Block {
    Block::Row {
        cells: vec![
            Cell::new("", 0.52, Place::Start),
            Cell::new(label, 0.26, Place::Start),
            Cell::new(value, 0.22, Place::End),
        ],
        size,
        bold,
    }
}

/// A sale's invoice on a sheet (A4 or A5): the shop and its logo, the
/// customer, the lines as a table, the VAT by rate, the totals.
pub fn invoice(shop: &Shop, sale: &Sale, lang: Lang, paper: Paper, o: &Options) -> Doc {
    let c = Ctx::new(lang, sale.currency);
    let a5 = paper == Paper::A5;
    let (big, body, small) = if a5 {
        (14.0, 9.0, 8.0)
    } else {
        (18.0, 10.0, 9.0)
    };
    // Invoices always carry the legal identity.
    let legal = Options {
        legal_ids: true,
        ..o.clone()
    };
    let mut b = header(&c, shop, &legal, big, small, if a5 { 90.0 } else { 120.0 });
    b.push(Block::Line);
    b.push(Block::Gap(6.0));
    b.push(Block::pair(
        tr_args(lang, "print.invoice_no", &[("number", &sale.number)]),
        tr_args(lang, "print.date", &[("date", &when(&c, sale))]),
        big - 4.0,
        true,
    ));
    if o.cashier {
        b.push(Block::text(
            tr_args(lang, "print.cashier", &[("name", &sale.cashier)]),
            small,
            false,
            Place::Start,
        ));
    }
    if let Some((name, ids)) = &sale.customer {
        b.push(Block::Gap(4.0));
        b.push(Block::text(
            tr_args(lang, "print.customer", &[("name", name)]),
            body,
            true,
            Place::Start,
        ));
        if !ids.is_empty() {
            b.push(Block::text(ids.clone(), small, false, Place::Start));
        }
    }
    b.push(Block::Gap(8.0));
    let shares = [0.06, 0.40, 0.14, 0.18, 0.22];
    let head = |key: &str, i: usize, place: Place| Cell::new(c.t(key), shares[i], place);
    b.push(Block::Line);
    b.push(Block::Row {
        cells: vec![
            Cell::new("#", shares[0], Place::Start),
            head("print.col.item", 1, Place::Start),
            head("print.col.qty", 2, Place::End),
            head("print.col.price", 3, Place::End),
            head("print.col.amount", 4, Place::End),
        ],
        size: small,
        bold: true,
    });
    b.push(Block::Line);
    for (i, l) in sale.lines.iter().enumerate() {
        b.push(Block::Row {
            cells: vec![
                Cell::new(
                    nb_i18n::bidi::isolate_ltr(&c.n(i as i64 + 1)),
                    shares[0],
                    Place::Start,
                ),
                Cell::new(&l.name, shares[1], Place::Start),
                Cell::new(qty_text(&c, l), shares[2], Place::End),
                Cell::new(c.a(l.unit_price_minor), shares[3], Place::End),
                Cell::new(c.a(l.amount_minor), shares[4], Place::End),
            ],
            size: body,
            bold: false,
        });
    }
    b.push(Block::Line);
    if o.count {
        b.push(count_line(&c, sale, small));
    }
    // The totals in the last columns, as a column of figures.
    for blk in totals(&c, sale, o, body, false) {
        match blk {
            Block::Pair {
                start,
                end,
                size,
                bold,
            } => b.push(total_row(start, end, size, bold)),
            other => b.push(other),
        }
    }
    b.push(Block::Gap(12.0));
    b.push(Block::text(
        tr_args(
            lang,
            "print.total_words",
            &[("amount", &c.m(sale.total_minor))],
        ),
        small,
        false,
        Place::Start,
    ));
    if !o.notice.is_empty() {
        b.push(Block::Gap(6.0));
        b.push(Block::text(&o.notice, small, false, Place::Start));
    }
    if o.barcode
        && let Some(bars) = barcode(sale, 160.0)
    {
        b.push(Block::Gap(8.0));
        b.push(bars);
    }
    Doc {
        paper,
        rtl: lang == Lang::Ar,
        blocks: b,
    }
}

/// A product's shelf label: name, price (and per unit), its barcode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfLabel {
    pub name: String,
    pub price_minor: i64,
    /// The unit the price is for (`kg`, `L`…); empty for a piece.
    pub unit: String,
    pub currency: Currency,
    /// The barcode's modules (EAN-13 or Code 128) and its digits.
    pub bars: Option<(Vec<bool>, String)>,
}

/// A shelf or product label on `paper` (a label size).
pub fn shelf_label(l: &ShelfLabel, lang: Lang, paper: Paper, shop: Option<&str>) -> Doc {
    let c = Ctx::new(lang, l.currency);
    let (w, h) = paper.sheet();
    // Type follows the label's size: a 40 × 30 mm label takes ~11 pt.
    let base = (h / 8.0).clamp(7.0, 16.0);
    let mut b = Vec::new();
    if let Some(s) = shop {
        b.push(Block::text(s, base * 0.7, false, Place::Center));
    }
    b.push(Block::text(&l.name, base, true, Place::Center));
    let price = if l.unit.is_empty() || l.unit == "pc" || l.unit == "u" {
        c.m(l.price_minor)
    } else {
        format!("{} / {}", c.m(l.price_minor), l.unit)
    };
    b.push(Block::text(price, base * 1.3, true, Place::Center));
    if let Some((modules, digits)) = &l.bars {
        b.push(Block::Bars {
            modules: modules.clone(),
            width: w * 0.85,
            height: (h * 0.28).max(12.0),
            caption: Some(digits.clone()),
        });
    }
    Doc {
        paper,
        rtl: lang == Lang::Ar,
        blocks: b,
    }
}

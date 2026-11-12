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

fn when_of(c: &Ctx, (y, m, d, h, mi): (i32, u32, u32, u32, u32)) -> String {
    let plain =
        nb_i18n::bidi::strip_controls(&format!("{} {}", c.f.date(y, m, d), c.f.time(h, mi)));
    nb_i18n::bidi::isolate_ltr(&unbroken(&plain))
}

/// A receipt voucher, on a roll or a sheet: who paid, how much, how, and
/// what it paid.
pub fn voucher(shop: &Shop, v: &Voucher, lang: Lang, paper: Paper, o: &Options) -> Doc {
    let c = Ctx::new(lang, v.currency);
    let sheet = !paper.is_thermal();
    let (big, body, small) = if sheet {
        (18.0, 11.0, 9.0)
    } else if paper.is_narrow() {
        (13.0, 9.0, 8.0)
    } else {
        (15.0, 10.0, 9.0)
    };
    let (pw, _) = paper.sheet();
    let mut b = header(
        &c,
        shop,
        o,
        big,
        small,
        if sheet { 120.0 } else { pw * 0.6 },
    );
    b.push(Block::Line);
    b.push(Block::text(
        c.t("print.voucher.title"),
        big - 2.0,
        true,
        Place::Center,
    ));
    b.push(Block::pair(
        tr_args(lang, "print.voucher_no", &[("number", &v.number)]),
        when_of(&c, v.when),
        small,
        false,
    ));
    if o.cashier && !v.cashier.is_empty() {
        b.push(Block::text(
            tr_args(lang, "print.cashier", &[("name", &v.cashier)]),
            small,
            false,
            Place::Start,
        ));
    }
    b.push(Block::Rule);
    b.push(Block::text(
        tr_args(lang, "print.voucher.from", &[("name", &v.customer.0)]),
        body,
        true,
        Place::Start,
    ));
    if !v.customer.1.is_empty() {
        b.push(Block::text(
            v.customer.1.clone(),
            small,
            false,
            Place::Start,
        ));
    }
    b.push(Block::Gap(4.0));
    // The amount on its own line, large: it is what the voucher is for.
    b.push(Block::text(
        c.t("print.voucher.amount"),
        small,
        false,
        Place::Center,
    ));
    b.push(Block::text(
        c.m(v.amount_minor),
        body + 6.0,
        true,
        Place::Center,
    ));
    b.push(Block::pair(
        c.t("print.voucher.method"),
        c.method(&v.method),
        body,
        false,
    ));
    if !v.reference.is_empty() {
        b.push(Block::pair(
            c.t("print.voucher.reference"),
            v.reference.clone(),
            small,
            false,
        ));
    }
    if !v.invoices.is_empty() || !v.installments.is_empty() {
        b.push(Block::Rule);
        b.push(Block::text(
            c.t("print.voucher.for"),
            small,
            true,
            Place::Start,
        ));
        for (n, a) in &v.invoices {
            b.push(Block::pair(n.clone(), c.m(*a), small, false));
        }
        for (plan, seq, a) in &v.installments {
            b.push(Block::pair(
                tr_args(
                    lang,
                    "print.voucher.installment",
                    &[
                        ("plan", &nb_i18n::bidi::isolate_ltr(plan)),
                        ("seq", &nb_i18n::bidi::isolate_ltr(&c.n(i64::from(*seq)))),
                    ],
                ),
                c.m(*a),
                small,
                false,
            ));
        }
    }
    if v.unallocated_minor > 0 {
        b.push(Block::pair(
            c.t("print.voucher.advance"),
            c.m(v.unallocated_minor),
            small,
            false,
        ));
    }
    if !v.note.is_empty() {
        b.push(Block::text(v.note.clone(), small, false, Place::Start));
    }
    b.push(Block::Gap(if sheet { 36.0 } else { 18.0 }));
    b.push(Block::pair(
        c.t("print.voucher.sign_customer"),
        c.t("print.voucher.sign_shop"),
        small,
        false,
    ));
    b.push(Block::Gap(if sheet { 48.0 } else { 24.0 }));
    if o.barcode
        && let Some(modules) = crate::barcode::code128(&v.number)
    {
        b.push(Block::Bars {
            modules,
            width: if sheet { 160.0 } else { pw * 0.8 },
            height: 24.0,
            caption: Some(v.number.clone()),
        });
    }
    Doc {
        paper,
        rtl: lang == Lang::Ar,
        blocks: b,
    }
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

/// An instalment schedule on a sheet (or a roll): the plan, every
/// instalment with what is paid, the signatures.
pub fn schedule(shop: &Shop, s: &Schedule, lang: Lang, paper: Paper, o: &Options) -> Doc {
    let c = Ctx::new(lang, s.currency);
    let sheet = !paper.is_thermal();
    let (big, body, small) = if sheet {
        (18.0, 10.0, 9.0)
    } else {
        (14.0, 9.0, 8.0)
    };
    let (pw, _) = paper.sheet();
    let legal = Options {
        legal_ids: true,
        ..o.clone()
    };
    let mut b = header(
        &c,
        shop,
        &legal,
        big,
        small,
        if sheet { 120.0 } else { pw * 0.6 },
    );
    b.push(Block::Line);
    b.push(Block::text(
        c.t("print.schedule.title"),
        big - 2.0,
        true,
        Place::Center,
    ));
    b.push(Block::pair(
        tr_args(
            lang,
            "print.schedule_no",
            &[
                ("number", &nb_i18n::bidi::isolate_ltr(&s.number)),
                (
                    "version",
                    &nb_i18n::bidi::isolate_ltr(&c.n(i64::from(s.version))),
                ),
            ],
        ),
        when_of(&c, s.when),
        small,
        false,
    ));
    b.push(Block::text(
        tr_args(lang, "print.customer", &[("name", &s.customer.0)]),
        body,
        true,
        Place::Start,
    ));
    if !s.customer.1.is_empty() {
        b.push(Block::text(
            s.customer.1.clone(),
            small,
            false,
            Place::Start,
        ));
    }
    b.push(Block::Gap(4.0));
    b.push(Block::pair(
        tr_args(lang, "print.schedule.sale", &[("number", &s.sale_number)]),
        c.m(s.total_minor),
        small,
        false,
    ));
    if s.down_minor > 0 {
        b.push(Block::pair(
            c.t("print.schedule.down"),
            c.m(s.down_minor),
            small,
            false,
        ));
    }
    b.push(Block::pair(
        c.t("print.schedule.financed"),
        c.m(s.financed_minor),
        body,
        true,
    ));
    b.push(Block::Gap(6.0));
    let shares: &[f32] = if sheet {
        &[0.08, 0.27, 0.22, 0.22, 0.21]
    } else {
        &[0.10, 0.30, 0.30, 0.30]
    };
    let mut heads = vec![
        Cell::new("#", shares[0], Place::Start),
        Cell::new(c.t("print.schedule.due"), shares[1], Place::Start),
        Cell::new(c.t("print.col.amount"), shares[2], Place::End),
        Cell::new(c.t("print.schedule.paid"), shares[3], Place::End),
    ];
    if sheet {
        heads.push(Cell::new(
            c.t("print.schedule.state"),
            shares[4],
            Place::End,
        ));
    }
    b.push(Block::Line);
    b.push(Block::Row {
        cells: heads,
        size: small,
        bold: true,
    });
    b.push(Block::Line);
    for d in &s.dues {
        let (y, m, dd) = d.date;
        let mut cells = vec![
            Cell::new(
                nb_i18n::bidi::isolate_ltr(&c.n(i64::from(d.seq))),
                shares[0],
                Place::Start,
            ),
            Cell::new(
                nb_i18n::bidi::isolate_ltr(&c.f.date(y, m, dd)),
                shares[1],
                Place::Start,
            ),
            Cell::new(c.a(d.amount_minor), shares[2], Place::End),
            Cell::new(c.a(d.paid_minor), shares[3], Place::End),
        ];
        if sheet {
            cells.push(Cell::new(
                c.t(&format!("plans.state.{}", d.state)),
                shares[4],
                Place::End,
            ));
        }
        b.push(Block::Row {
            cells,
            size: body,
            bold: false,
        });
    }
    b.push(Block::Line);
    let paid: i64 = s.dues.iter().map(|d| d.paid_minor).sum();
    b.push(Block::pair(
        c.t("print.schedule.paid_total"),
        c.m(paid),
        body,
        false,
    ));
    b.push(Block::pair(
        c.t("print.schedule.left"),
        c.m((s.financed_minor - paid).max(0)),
        body + 2.0,
        true,
    ));
    if !o.notice.is_empty() {
        b.push(Block::Gap(6.0));
        b.push(Block::text(&o.notice, small, false, Place::Start));
    }
    b.push(Block::Gap(if sheet { 36.0 } else { 18.0 }));
    b.push(Block::pair(
        c.t("print.voucher.sign_customer"),
        c.t("print.voucher.sign_shop"),
        small,
        false,
    ));
    b.push(Block::Gap(24.0));
    Doc {
        paper,
        rtl: lang == Lang::Ar,
        blocks: b,
    }
}

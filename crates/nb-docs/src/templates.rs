//! The default documents (M12.8): a sale's receipt for the till (58 or
//! 80 mm) and its invoice on a sheet (A4 or A5), in the shop's language.
//! Amounts arrive in minor units and are formatted here; nothing is
//! computed but what the sale already says.

use nb_core::{Currency, Money, Quantity};
use nb_i18n::{Format, Lang, tr, tr_args};

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
    /// A line at the bottom of receipts.
    pub footer: String,
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
    /// (method code, amount).
    pub payments: Vec<(String, i64)>,
    pub change_minor: i64,
    /// Left to pay (on the customer's account).
    pub owed_minor: i64,
}

struct Ctx {
    lang: Lang,
    f: Format,
    currency: Currency,
}

impl Ctx {
    fn t(&self, key: &str) -> String {
        tr(self.lang, key).into_owned()
    }
    fn m(&self, minor: i64) -> String {
        self.f.money(Money::from_minor(minor, self.currency))
    }
    fn q(&self, milli: i64) -> String {
        self.f.quantity(Quantity::from_milli(milli), 3)
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

fn header(c: &Ctx, shop: &Shop, big: f32, small: f32) -> Vec<Block> {
    let mut b = vec![Block::text(&shop.name, big, true, Place::Center)];
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

fn totals(c: &Ctx, s: &Sale, size: f32) -> Vec<Block> {
    let mut b = Vec::new();
    if s.discount_minor > 0 {
        b.push(Block::pair(
            c.t("print.discount"),
            c.m(-s.discount_minor),
            size,
            false,
        ));
    }
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
    if s.owed_minor > 0 {
        b.push(Block::pair(
            c.t("print.owed"),
            c.m(s.owed_minor),
            size,
            true,
        ));
    }
    b
}

/// A sale's receipt for a thermal roll.
pub fn receipt(shop: &Shop, sale: &Sale, lang: Lang, paper: Paper) -> Doc {
    let c = Ctx {
        lang,
        f: Format::new(lang),
        currency: sale.currency,
    };
    let narrow = paper == Paper::Roll58;
    let (big, body, small) = if narrow {
        (13.0, 9.0, 8.0)
    } else {
        (15.0, 10.0, 9.0)
    };
    let mut b = header(&c, shop, big, small);
    b.push(Block::Rule);
    b.push(Block::pair(
        tr_args(lang, "print.receipt_no", &[("number", &sale.number)]),
        when(&c, sale),
        small,
        false,
    ));
    b.push(Block::pair(
        tr_args(lang, "print.cashier", &[("name", &sale.cashier)]),
        sale.register.clone(),
        small,
        false,
    ));
    if let Some((name, _)) = &sale.customer {
        b.push(Block::text(
            tr_args(lang, "print.customer", &[("name", name)]),
            small,
            false,
            Place::Start,
        ));
    }
    b.push(Block::Rule);
    for l in &sale.lines {
        b.push(Block::text(&l.name, body, false, Place::Start));
        b.push(Block::pair(
            format!(
                "{} {} × {}",
                nb_i18n::bidi::isolate_ltr(&c.q(l.qty_milli)),
                l.unit,
                c.m(l.unit_price_minor)
            ),
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
    b.extend(totals(&c, sale, body));
    b.push(Block::Rule);
    let footer = if shop.footer.is_empty() {
        c.t("print.thanks")
    } else {
        shop.footer.clone()
    };
    b.push(Block::text(footer, small, false, Place::Center));
    b.push(Block::Gap(6.0));
    Doc {
        paper,
        rtl: lang == Lang::Ar,
        blocks: b,
    }
}

/// A sale's invoice on a sheet (A4 or A5): the shop, the customer, the
/// lines as a table, the VAT by rate, the totals.
pub fn invoice(shop: &Shop, sale: &Sale, lang: Lang, paper: Paper) -> Doc {
    let c = Ctx {
        lang,
        f: Format::new(lang),
        currency: sale.currency,
    };
    let a5 = paper == Paper::A5;
    let (big, body, small) = if a5 {
        (14.0, 9.0, 8.0)
    } else {
        (18.0, 10.0, 9.0)
    };
    let mut b = header(&c, shop, big, small);
    b.push(Block::Gap(8.0));
    b.push(Block::text(
        tr_args(lang, "print.invoice_no", &[("number", &sale.number)]),
        big - 2.0,
        true,
        Place::Start,
    ));
    b.push(Block::pair(
        tr_args(lang, "print.date", &[("date", &when(&c, sale))]),
        tr_args(lang, "print.cashier", &[("name", &sale.cashier)]),
        small,
        false,
    ));
    if let Some((name, ids)) = &sale.customer {
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
    let head = |text: String, share: f32, place: Place| Cell::new(text, share, place);
    b.push(Block::Row {
        cells: vec![
            head(c.t("print.col.item"), 0.46, Place::Start),
            head(c.t("print.col.qty"), 0.14, Place::End),
            head(c.t("print.col.price"), 0.20, Place::End),
            head(c.t("print.col.amount"), 0.20, Place::End),
        ],
        size: small,
        bold: true,
    });
    b.push(Block::Rule);
    for l in &sale.lines {
        b.push(Block::Row {
            cells: vec![
                Cell::new(&l.name, 0.46, Place::Start),
                Cell::new(
                    format!(
                        "{} {}",
                        nb_i18n::bidi::isolate_ltr(&c.q(l.qty_milli)),
                        l.unit
                    ),
                    0.14,
                    Place::End,
                ),
                Cell::new(c.m(l.unit_price_minor), 0.20, Place::End),
                Cell::new(c.m(l.amount_minor), 0.20, Place::End),
            ],
            size: body,
            bold: false,
        });
    }
    b.push(Block::Rule);
    b.extend(totals(&c, sale, body));
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
    Doc {
        paper,
        rtl: lang == Lang::Ar,
        blocks: b,
    }
}

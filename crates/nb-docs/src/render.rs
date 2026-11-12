//! Documents from templates (M12.8): a [`Template`] (what shows, in which
//! order and style, as the shop edited it) and a [`DocData`] (the
//! document's values, already formatted in its language) make a [`Doc`].
//! Thermal paper prints black on white whatever the style; sheets get the
//! accent colour, shaded headers and boxes.

use std::collections::BTreeMap;

use nb_domain::template::{
    Align, DocKind, HeaderLayout, ItemColumn, ItemField, Section, TableStyle, Template, TotalLine,
    fill,
};
use nb_i18n::{Lang, tr};

use crate::logo::Gray;
use crate::{Block, Cell, Doc, INK, Paper, Place, Rgb, rgb, tint};

/// A document's values, formatted for print.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DocData {
    /// `shop.name`, `doc.number`, `total`… (see `template::variables`).
    pub vars: BTreeMap<String, String>,
    pub logo: Option<Gray>,
    /// The shop's legal identifiers, one line.
    pub legal_ids: String,
    /// The document's details: (label, value), in order.
    pub details: Vec<(String, String)>,
    /// The customer: name, then other lines.
    pub customer: Vec<String>,
    /// The rows, by field.
    pub rows: Vec<BTreeMap<ItemField, String>>,
    /// A report's own header and rows (its columns are its own).
    pub table: Option<(Vec<String>, Vec<Vec<String>>)>,
    /// Lines of the totals by kind: (label, value, strong).
    pub totals: Vec<(TotalLine, String, String, bool)>,
    /// Bon par bon: (label, value, strong) lines.
    pub balance: Vec<(String, String, bool)>,
}

impl DocData {
    pub fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }
}

/// The usual title of a column.
fn column_key(f: ItemField) -> &'static str {
    match f {
        ItemField::Index => "print.col.index",
        ItemField::Name => "print.col.item",
        ItemField::Sku => "print.col.sku",
        ItemField::Qty | ItemField::QtyUnit => "print.col.qty",
        ItemField::Unit => "print.col.unit",
        ItemField::Price => "print.col.unit_price",
        ItemField::Discount => "print.col.discount",
        ItemField::TaxRate => "print.col.tax",
        ItemField::Net => "print.col.net",
        ItemField::Amount => "print.col.amount",
        ItemField::Due => "print.schedule.due",
        ItemField::Paid => "print.schedule.paid",
        ItemField::State => "print.schedule.state",
    }
}

struct R<'a> {
    t: &'a Template,
    d: &'a DocData,
    lang: Lang,
    paper: Paper,
    /// Wide: a sheet or a roll of 80 mm and more.
    wide: bool,
    sheet: bool,
    accent: Rgb,
    base: f32,
}

impl R<'_> {
    fn text(&self, s: &str) -> String {
        let s = s.trim();
        if let Some(key) = s.strip_prefix('@') {
            return tr(self.lang, key).into_owned();
        }
        fill(s, |n| self.d.var(n)).trim().to_owned()
    }

    fn t(&self, key: &str) -> String {
        tr(self.lang, key).into_owned()
    }

    fn place(a: Align) -> Place {
        match a {
            Align::Start => Place::Start,
            Align::Center => Place::Center,
            Align::End => Place::End,
        }
    }

    fn accent_text(&self, blocks: Vec<Block>) -> Block {
        Block::Paint {
            color: self.accent,
            blocks,
        }
    }

    fn header(&self, legal: bool, out: &mut Vec<Block>) {
        let s = self.t.style.clone();
        let small = (self.base - 1.0).max(7.0);
        let mut shop = Vec::new();
        let place = if s.header == HeaderLayout::Split && self.wide {
            Place::Start
        } else {
            Place::Center
        };
        let name = self.d.var("shop.name").unwrap_or_default();
        shop.push(self.accent_text(vec![Block::text(
            name,
            self.base + if self.sheet { 7.0 } else { 4.0 },
            true,
            place,
        )]));
        for key in ["shop.tagline", "shop.address"] {
            if let Some(v) = self.d.var(key).filter(|v| !v.is_empty()) {
                shop.push(Block::text(v, small, false, place));
            }
        }
        if let Some(p) = self.d.var("shop.phone").filter(|v| !v.is_empty()) {
            shop.push(Block::text(
                format!(
                    "{} {}",
                    self.t("print.phone"),
                    nb_i18n::bidi::isolate_ltr(&p.replace(' ', "\u{a0}"))
                ),
                small,
                false,
                place,
            ));
        }
        if legal && !self.d.legal_ids.is_empty() {
            shop.push(Block::text(
                self.d.legal_ids.clone(),
                small - 0.5,
                false,
                place,
            ));
        }
        let logo = match (&self.d.logo, s.logo) {
            (Some(g), true) => Some(Block::Image {
                gray: g.px.clone(),
                w: g.w,
                h: g.h,
                width: f32::from(s.logo_width).min(g.w as f32 * 72.0 / 203.0 * 1.5),
            }),
            _ => None,
        };
        if s.header == HeaderLayout::Split && self.wide {
            // The shop at the start; the title, number and date at the end.
            let mut start = Vec::new();
            if let Some(l) = logo {
                start.push(Block::Columns {
                    cols: vec![(1.0, vec![l]), (2.0, vec![])],
                    gap: 0.0,
                });
                start.push(Block::Gap(4.0));
            }
            start.extend(shop);
            let mut end = vec![self.accent_text(vec![Block::text(
                self.d.var("doc.title").unwrap_or_default().to_uppercase(),
                self.base + 12.0,
                true,
                Place::End,
            )])];
            for key in ["doc.number", "doc.date"] {
                if let Some(v) = self.d.var(key).filter(|v| !v.is_empty()) {
                    end.push(Block::text(v, self.base, key == "doc.number", Place::End));
                }
            }
            out.push(Block::Columns {
                cols: vec![(0.58, start), (0.42, end)],
                gap: 12.0,
            });
        } else {
            if let Some(l) = logo {
                out.push(l);
            }
            out.extend(shop);
        }
    }

    fn fields(&self, fields: &[nb_domain::template::Field], out: &mut Vec<Block>) {
        let pairs: Vec<Block> = fields
            .iter()
            .map(|f| (self.text(&f.label), self.text(&f.value)))
            .filter(|(_, v)| !v.is_empty())
            .map(|(l, v)| {
                if l.is_empty() {
                    Block::text(v, self.base, false, Place::Start)
                } else {
                    Block::pair(l, v, self.base, false)
                }
            })
            .collect();
        if self.sheet && pairs.len() > 1 {
            let mut it = pairs.into_iter();
            while let Some(a) = it.next() {
                let b = it.next().map(|b| vec![b]).unwrap_or_default();
                out.push(Block::Columns {
                    cols: vec![(1.0, vec![a]), (1.0, b)],
                    gap: 24.0,
                });
            }
        } else {
            out.extend(pairs);
        }
    }

    fn parties(&self, details: bool, out: &mut Vec<Block>) {
        let small = (self.base - 1.0).max(7.0);
        // A split header already shows the number and the date.
        let split = self.t.style.header == HeaderLayout::Split && self.wide;
        let shown = |v: &str| {
            let plain = nb_i18n::bidi::strip_controls(v).replace('\u{a0}', " ");
            split
                && (Some(v.to_owned()) == self.d.var("doc.date")
                    || Some(plain) == self.d.var("doc.number"))
        };
        let detail_blocks: Vec<Block> = if details {
            self.d
                .details
                .iter()
                .filter(|(_, v)| !v.is_empty() && !shown(v))
                .map(|(l, v)| {
                    if self.sheet {
                        // Label and value side by side, the label quieter.
                        Block::Columns {
                            cols: vec![
                                (
                                    0.4,
                                    vec![Block::Paint {
                                        color: [110, 110, 110],
                                        blocks: vec![Block::text(
                                            l.clone(),
                                            small,
                                            false,
                                            Place::Start,
                                        )],
                                    }],
                                ),
                                (0.6, vec![Block::text(v.clone(), small, true, Place::Start)]),
                            ],
                            gap: 6.0,
                        }
                    } else {
                        Block::pair(l.clone(), v.clone(), small, false)
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let customer: Vec<Block> = self
            .d
            .customer
            .iter()
            .enumerate()
            .map(|(i, l)| {
                Block::text(
                    l.clone(),
                    if i == 0 { self.base + 1.0 } else { small },
                    i == 0,
                    Place::Start,
                )
            })
            .collect();
        if self.sheet {
            let bill = if customer.is_empty() {
                Vec::new()
            } else {
                let mut b = vec![self.accent_text(vec![Block::text(
                    self.t("print.bill_to").to_uppercase(),
                    small,
                    true,
                    Place::Start,
                )])];
                b.extend(customer);
                vec![Block::Panel {
                    blocks: b,
                    fill: Some(tint(self.accent, 18)),
                    border: None,
                    pad: 8.0,
                }]
            };
            out.push(Block::Columns {
                cols: vec![(0.55, bill), (0.45, detail_blocks)],
                gap: 24.0,
            });
            out.push(Block::Gap(8.0));
        } else {
            out.extend(detail_blocks);
            if let Some(first) = self.d.customer.first() {
                out.push(Block::text(
                    nb_i18n::tr_args(self.lang, "print.customer", &[("name", first)]),
                    small,
                    true,
                    Place::Start,
                ));
            }
        }
    }

    fn table(
        &self,
        heads: Vec<(String, f32, Place)>,
        rows: Vec<Vec<String>>,
        out: &mut Vec<Block>,
    ) {
        let style = if self.sheet {
            self.t.style.table
        } else {
            TableStyle::Lines
        };
        let size = if heads.len() > 6 {
            self.base - 1.5
        } else {
            self.base - 0.5
        };
        let head_row = Block::Row {
            cells: heads
                .iter()
                .map(|(t, s, p)| Cell::new(t.clone(), *s, *p))
                .collect(),
            size: (size - 0.5).max(7.0),
            bold: true,
        };
        match style {
            TableStyle::Shaded | TableStyle::Striped => out.push(Block::Panel {
                blocks: vec![Block::Paint {
                    color: [255, 255, 255],
                    blocks: vec![head_row],
                }],
                fill: Some(self.accent),
                border: None,
                pad: 4.0,
            }),
            TableStyle::Lines | TableStyle::Ruled => {
                out.push(Block::Line);
                out.push(head_row);
                out.push(Block::Line);
            }
        }
        for (i, r) in rows.into_iter().enumerate() {
            let row = Block::Row {
                cells: r
                    .into_iter()
                    .zip(&heads)
                    .map(|(v, (_, s, p))| Cell::new(v, *s, *p))
                    .collect(),
                size,
                bold: false,
            };
            match style {
                TableStyle::Striped if i % 2 == 1 => out.push(Block::Panel {
                    blocks: vec![row],
                    fill: Some(tint(self.accent, 14)),
                    border: None,
                    pad: 3.0,
                }),
                TableStyle::Striped | TableStyle::Shaded => out.push(Block::Panel {
                    blocks: vec![row],
                    fill: None,
                    border: None,
                    pad: 3.0,
                }),
                TableStyle::Ruled => {
                    out.push(row);
                    out.push(Block::Paint {
                        color: [200, 200, 200],
                        blocks: vec![Block::Line],
                    });
                }
                TableStyle::Lines => out.push(row),
            }
        }
        if matches!(style, TableStyle::Lines | TableStyle::Shaded) {
            out.push(Block::Line);
        }
    }

    /// Widens the columns whose title would wrap, taking from the widest:
    /// on a narrow roll « الكمية » or « Montant » must stay on one line.
    fn fit(&self, mut heads: Vec<(String, f32, Place)>) -> Vec<(String, f32, Place)> {
        let size = if heads.len() > 6 {
            self.base - 2.0
        } else {
            self.base - 1.0
        };
        let avail = self.paper.sheet().0 * if self.sheet { 0.84 } else { 0.94 };
        let need = |t: &str| ((t.chars().count() as f32 * size * 0.58 + 6.0) / avail).min(0.45);
        let Some(wide) = heads
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.1.total_cmp(&b.1.1))
            .map(|(i, _)| i)
        else {
            return heads;
        };
        for i in 0..heads.len() {
            if i == wide {
                continue;
            }
            let short = need(&heads[i].0) - heads[i].1;
            let spare = heads[wide].1 - 0.3;
            if short > 0.0 && spare > 0.0 {
                let take = short.min(spare);
                heads[i].1 += take;
                heads[wide].1 -= take;
            }
        }
        heads
    }

    fn items(&self, columns: &[ItemColumn], out: &mut Vec<Block>) {
        if let Some((heads, rows)) = &self.d.table {
            // A report: its own columns, the first wider.
            let n = heads.len().max(1);
            let first = if n == 1 {
                1.0
            } else {
                0.34_f32.max(1.0 / n as f32)
            };
            let rest = if n == 1 {
                0.0
            } else {
                (1.0 - first) / (n - 1) as f32
            };
            let heads = heads
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    (
                        h.clone(),
                        if i == 0 { first } else { rest },
                        if i == 0 { Place::Start } else { Place::End },
                    )
                })
                .collect();
            self.table(heads, rows.clone(), out);
            return;
        }
        // Too many columns for a narrow roll: name, quantity, amount.
        let mut cols: Vec<ItemColumn> = columns.to_vec();
        if !self.wide && cols.len() > 3 {
            cols.retain(|c| {
                matches!(
                    c.field,
                    ItemField::Name
                        | ItemField::Qty
                        | ItemField::QtyUnit
                        | ItemField::Amount
                        | ItemField::Due
                )
            });
        }
        if cols.is_empty() {
            return;
        }
        let total: f32 = cols
            .iter()
            .map(|c| f32::from(c.width))
            .sum::<f32>()
            .max(1.0);
        let heads = cols
            .iter()
            .map(|c| {
                let title = if c.title.trim().is_empty() {
                    self.t(column_key(c.field))
                } else {
                    self.text(&c.title)
                };
                let place = match c.field {
                    ItemField::Index => Place::Center,
                    f if f.is_figure() => Place::End,
                    _ => Place::Start,
                };
                (title, f32::from(c.width) / total, place)
            })
            .collect();
        let heads = self.fit(heads);
        let rows = self
            .d
            .rows
            .iter()
            .map(|r| {
                cols.iter()
                    .map(|c| r.get(&c.field).cloned().unwrap_or_default())
                    .collect()
            })
            .collect();
        self.table(heads, rows, out);
    }

    fn totals(&self, lines: &[TotalLine], out: &mut Vec<Block>) {
        let mut pairs = Vec::new();
        let mut grand = None;
        for want in lines {
            for (kind, label, value, strong) in &self.d.totals {
                if kind != want {
                    continue;
                }
                if *kind == TotalLine::Total {
                    grand = Some((label.clone(), value.clone()));
                    if !self.sheet {
                        pairs.push(Block::pair(
                            label.clone(),
                            value.clone(),
                            self.base + 3.0,
                            true,
                        ));
                    }
                } else {
                    pairs.push(Block::pair(
                        label.clone(),
                        value.clone(),
                        self.base,
                        *strong,
                    ));
                }
            }
        }
        if !self.sheet {
            out.extend(pairs);
            return;
        }
        // On a sheet: a column of figures at the end, the total in a band.
        let mut col = Vec::new();
        let total_at = lines.iter().position(|l| *l == TotalLine::Total);
        let before = pairs.len().min(
            total_at
                .map(|i| {
                    lines[..i]
                        .iter()
                        .map(|l| self.d.totals.iter().filter(|t| t.0 == *l).count())
                        .sum()
                })
                .unwrap_or(pairs.len()),
        );
        let mut rest = pairs;
        let after = rest.split_off(before);
        col.extend(rest);
        if let Some((l, v)) = grand {
            col.push(Block::Panel {
                blocks: vec![Block::Paint {
                    color: [255, 255, 255],
                    blocks: vec![Block::pair(l, v, self.base + 2.0, true)],
                }],
                fill: Some(self.accent),
                border: None,
                pad: 6.0,
            });
        }
        col.extend(after);
        out.push(Block::Gap(4.0));
        out.push(Block::Columns {
            cols: vec![(0.52, vec![]), (0.48, col)],
            gap: 0.0,
        });
    }

    fn barcode(&self, value: &str, out: &mut Vec<Block>) {
        let v = self.text(value);
        if v.is_empty() || v.contains('{') {
            return;
        }
        let modules = if v.len() == 13 && v.bytes().all(|b| b.is_ascii_digit()) {
            nb_domain::barcode::ean13_modules(&v).map(|m| {
                let mut q = vec![false; 9];
                q.extend(m);
                q.extend([false; 9]);
                q
            })
        } else {
            crate::barcode::code128(&v)
        };
        let Some(modules) = modules else {
            return;
        };
        let (w, h) = self.paper.sheet();
        let label = matches!(self.paper, Paper::Label { .. });
        out.push(Block::Bars {
            modules,
            width: if self.sheet { 170.0 } else { w * 0.8 },
            height: if label { (h * 0.28).max(12.0) } else { 26.0 },
            caption: Some(v),
        });
    }

    fn render(&self) -> Vec<Block> {
        let mut out = Vec::new();
        for s in &self.t.sections {
            match s {
                Section::Header { legal_ids } => self.header(*legal_ids, &mut out),
                Section::Title { text, align } => {
                    let v = self.text(text);
                    if !v.is_empty() {
                        out.push(self.accent_text(vec![Block::text(
                            v,
                            self.base + if self.sheet { 6.0 } else { 3.0 },
                            true,
                            Self::place(*align),
                        )]));
                    }
                }
                Section::Fields { fields } => self.fields(fields, &mut out),
                Section::Parties { details } => self.parties(*details, &mut out),
                Section::Items { columns } => self.items(columns, &mut out),
                Section::Totals { lines } => self.totals(lines, &mut out),
                Section::Balance => {
                    if !self.d.balance.is_empty() {
                        if !self.sheet {
                            out.push(Block::Rule);
                        }
                        let lines: Vec<Block> = self
                            .d
                            .balance
                            .iter()
                            .map(|(l, v, strong)| {
                                Block::pair(l.clone(), v.clone(), self.base, *strong)
                            })
                            .collect();
                        if self.sheet {
                            out.push(Block::Gap(8.0));
                            out.push(Block::Columns {
                                cols: vec![
                                    (0.52, vec![]),
                                    (
                                        0.48,
                                        vec![Block::Panel {
                                            blocks: lines,
                                            fill: None,
                                            border: Some(tint(self.accent, 40)),
                                            pad: 6.0,
                                        }],
                                    ),
                                ],
                                gap: 0.0,
                            });
                        } else {
                            out.extend(lines);
                        }
                    }
                }
                Section::Text {
                    text,
                    align,
                    size,
                    bold,
                } => {
                    let v = self.text(text);
                    if !v.is_empty() {
                        out.push(Block::text(
                            v,
                            (self.base + f32::from(*size)).max(6.0),
                            *bold,
                            Self::place(*align),
                        ));
                    }
                }
                Section::Barcode { value } => self.barcode(value, &mut out),
                Section::Signatures { start, end } => {
                    let label = |s: &str, key: &str| {
                        let v = self.text(s);
                        if v.is_empty() { self.t(key) } else { v }
                    };
                    out.push(Block::Gap(if self.sheet { 24.0 } else { 14.0 }));
                    out.push(Block::Columns {
                        cols: vec![
                            (
                                1.0,
                                vec![Block::text(
                                    label(start, "print.voucher.sign_customer"),
                                    self.base - 1.0,
                                    false,
                                    Place::Start,
                                )],
                            ),
                            (
                                1.0,
                                vec![Block::text(
                                    label(end, "print.voucher.sign_shop"),
                                    self.base - 1.0,
                                    false,
                                    Place::End,
                                )],
                            ),
                        ],
                        gap: 12.0,
                    });
                    out.push(Block::Gap(if self.sheet { 40.0 } else { 26.0 }));
                }
                Section::Divider { dashed } => {
                    out.push(if *dashed { Block::Rule } else { Block::Line })
                }
                Section::Spacer { height } => out.push(Block::Gap(f32::from(*height))),
            }
        }
        if self.paper.is_roll() {
            out.push(Block::Gap(6.0));
        }
        out
    }
}

/// `t` with `d` on `paper`, in `lang`. Thermal paper prints black.
pub fn render(t: &Template, d: &DocData, lang: Lang, paper: Paper) -> Doc {
    let thermal = paper.is_thermal();
    let sheet = !thermal;
    let wide = sheet || paper.dots() >= 500;
    let mut base = f32::from(t.style.size);
    if paper.is_narrow() {
        base = base.min(9.0);
    }
    if let Paper::Label { h_mm, .. } = paper {
        base = (f32::from(h_mm) * 72.0 / 25.4 / 8.0).clamp(7.0, 16.0);
    }
    let r = R {
        t,
        d,
        lang,
        paper,
        wide,
        sheet,
        accent: if thermal { INK } else { rgb(&t.style.accent) },
        base,
    };
    Doc {
        paper,
        rtl: lang == Lang::Ar,
        blocks: r.render(),
    }
}

/// The kind a template is for.
pub fn kind_of(t: &Template) -> DocKind {
    t.kind
}

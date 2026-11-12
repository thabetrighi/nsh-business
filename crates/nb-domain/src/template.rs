//! Document templates (M12.8): what a receipt, an invoice, a voucher, a
//! schedule, a label or a report shows, in which order, in which style —
//! as data the shop edits. Texts carry variables (`{doc.number}`), filled
//! when printed. A template is checked before it is kept; one that cannot
//! be used is never printed with: the built-in one is.

use serde::{Deserialize, Serialize};

use crate::RuleError;

/// The documents templates exist for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Receipt,
    Invoice,
    Voucher,
    Schedule,
    Label,
    Report,
}

impl DocKind {
    pub const ALL: [DocKind; 6] = [
        DocKind::Receipt,
        DocKind::Invoice,
        DocKind::Voucher,
        DocKind::Schedule,
        DocKind::Label,
        DocKind::Report,
    ];

    pub fn code(self) -> &'static str {
        match self {
            DocKind::Receipt => "receipt",
            DocKind::Invoice => "invoice",
            DocKind::Voucher => "voucher",
            DocKind::Schedule => "schedule",
            DocKind::Label => "label",
            DocKind::Report => "report",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.code() == code)
    }
}

/// Where the shop's identity sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HeaderLayout {
    /// Logo and shop centred, one above the other (receipts).
    #[default]
    Centered,
    /// The shop at the start, the document's title and number at the end
    /// (invoices on sheets).
    Split,
}

/// How the items table looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TableStyle {
    /// Lines above and below the header and the table.
    #[default]
    Lines,
    /// A header in the accent colour.
    Shaded,
    /// A shaded header and every other row tinted.
    Striped,
    /// A line under every row.
    Ruled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

/// The look of the whole document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    /// `#RRGGBB`: titles, the table's header (sheets; thermal prints black).
    pub accent: String,
    /// Body text in points (7–14); titles and totals scale from it.
    pub size: u8,
    pub header: HeaderLayout,
    pub table: TableStyle,
    pub logo: bool,
    /// The logo's width in points (30–200).
    pub logo_width: u16,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            accent: "#1F2937".into(),
            size: 10,
            header: HeaderLayout::Centered,
            table: TableStyle::Lines,
            logo: true,
            logo_width: 110,
        }
    }
}

/// What a column of the items table shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemField {
    Index,
    Name,
    Sku,
    Qty,
    Unit,
    /// Quantity and unit together.
    QtyUnit,
    Price,
    Discount,
    TaxRate,
    /// The line before tax.
    Net,
    Amount,
    /// Schedules: the due date, what was paid, the state.
    Due,
    Paid,
    State,
}

impl ItemField {
    pub const ALL: [ItemField; 14] = [
        ItemField::Index,
        ItemField::Name,
        ItemField::Sku,
        ItemField::Qty,
        ItemField::Unit,
        ItemField::QtyUnit,
        ItemField::Price,
        ItemField::Discount,
        ItemField::TaxRate,
        ItemField::Net,
        ItemField::Amount,
        ItemField::Due,
        ItemField::Paid,
        ItemField::State,
    ];

    pub fn code(self) -> &'static str {
        match self {
            ItemField::Index => "index",
            ItemField::Name => "name",
            ItemField::Sku => "sku",
            ItemField::Qty => "qty",
            ItemField::Unit => "unit",
            ItemField::QtyUnit => "qty_unit",
            ItemField::Price => "price",
            ItemField::Discount => "discount",
            ItemField::TaxRate => "tax_rate",
            ItemField::Net => "net",
            ItemField::Amount => "amount",
            ItemField::Due => "due",
            ItemField::Paid => "paid",
            ItemField::State => "state",
        }
    }

    /// Figures align to the end of their column.
    pub fn is_figure(self) -> bool {
        !matches!(
            self,
            ItemField::Name | ItemField::Sku | ItemField::Unit | ItemField::State | ItemField::Due
        )
    }

    /// The fields that make sense for a document's rows.
    pub fn of(kind: DocKind) -> &'static [ItemField] {
        match kind {
            DocKind::Receipt | DocKind::Invoice => &[
                ItemField::Index,
                ItemField::Name,
                ItemField::Sku,
                ItemField::Qty,
                ItemField::Unit,
                ItemField::QtyUnit,
                ItemField::Price,
                ItemField::Discount,
                ItemField::TaxRate,
                ItemField::Net,
                ItemField::Amount,
            ],
            DocKind::Voucher => &[ItemField::Index, ItemField::Name, ItemField::Amount],
            DocKind::Schedule => &[
                ItemField::Index,
                ItemField::Due,
                ItemField::Amount,
                ItemField::Paid,
                ItemField::State,
            ],
            DocKind::Label | DocKind::Report => &[],
        }
    }
}

/// A column of the items table: what, its title (empty: the usual one),
/// its share of the width in percent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemColumn {
    pub field: ItemField,
    #[serde(default)]
    pub title: String,
    pub width: u8,
}

impl ItemColumn {
    pub fn new(field: ItemField, width: u8) -> Self {
        Self {
            field,
            title: String::new(),
            width,
        }
    }
}

/// A line of the totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TotalLine {
    Discount,
    /// Before tax.
    Net,
    /// The VAT, one line per rate.
    Vat,
    Stamp,
    Total,
    /// What was handed over, per method.
    Payments,
    Change,
    /// Left on the customer's account.
    Owed,
}

impl TotalLine {
    pub const ALL: [TotalLine; 8] = [
        TotalLine::Discount,
        TotalLine::Net,
        TotalLine::Vat,
        TotalLine::Stamp,
        TotalLine::Total,
        TotalLine::Payments,
        TotalLine::Change,
        TotalLine::Owed,
    ];

    pub fn code(self) -> &'static str {
        match self {
            TotalLine::Discount => "discount",
            TotalLine::Net => "net",
            TotalLine::Vat => "vat",
            TotalLine::Stamp => "stamp",
            TotalLine::Total => "total",
            TotalLine::Payments => "payments",
            TotalLine::Change => "change",
            TotalLine::Owed => "owed",
        }
    }
}

/// A label and its value, both texts with variables. A label starting
/// with `@` is a translation key (`@print.voucher.method`): it follows the
/// document's language. A field whose value is empty is left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub label: String,
    pub value: String,
}

impl Field {
    pub fn new(label: &str, value: &str) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// A part of a document, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Section {
    /// The shop: logo, name, tagline, address, phone, legal identity —
    /// and, in a split header, the title and number at the end.
    Header {
        #[serde(default = "yes")]
        legal_ids: bool,
    },
    /// The document's title, large (`{doc.title}`, `{doc.number}`…).
    Title {
        text: String,
        #[serde(default)]
        align: Align,
    },
    /// Label: value pairs, two to a row on wide paper.
    Fields {
        fields: Vec<Field>,
    },
    /// The customer ("billed to") and the document's details, side by
    /// side on wide paper.
    Parties {
        #[serde(default = "yes")]
        details: bool,
    },
    /// The rows: items sold, what a receipt paid, instalments, a report.
    Items {
        columns: Vec<ItemColumn>,
    },
    Totals {
        lines: Vec<TotalLine>,
    },
    /// Bon par bon: balance before, this document, paid, balance after.
    Balance,
    Text {
        text: String,
        #[serde(default)]
        align: Align,
        /// Points relative to the body size (−3 to +8).
        #[serde(default)]
        size: i8,
        #[serde(default)]
        bold: bool,
    },
    /// A barcode of a value (`{doc.number}`, `{item.barcode}`).
    Barcode {
        value: String,
    },
    Signatures {
        start: String,
        end: String,
    },
    Divider {
        #[serde(default)]
        dashed: bool,
    },
    Spacer {
        height: u8,
    },
}

fn yes() -> bool {
    true
}

impl Section {
    /// Every kind of section, in the order the editor offers them.
    pub const CODES: [&'static str; 12] = [
        "header",
        "title",
        "fields",
        "parties",
        "items",
        "totals",
        "balance",
        "text",
        "barcode",
        "signatures",
        "divider",
        "spacer",
    ];

    /// A new section of `code` as the editor adds it to a `kind` document.
    pub fn fresh(code: &str, kind: DocKind) -> Option<Section> {
        use ItemField as F;
        Some(match code {
            "header" => Section::Header { legal_ids: true },
            "title" => Section::Title {
                text: match kind {
                    DocKind::Label => "{item.name}".into(),
                    DocKind::Report => "{report.title}".into(),
                    _ => "{doc.title}".into(),
                },
                align: Align::Center,
            },
            "fields" => Section::Fields {
                fields: vec![match kind {
                    DocKind::Label => Field::new("", "{item.sku}"),
                    DocKind::Report => Field::new("", "{report.period}"),
                    _ => Field::new("@print.label.date", "{doc.date}"),
                }],
            },
            "parties" => Section::Parties { details: true },
            "items" => Section::Items {
                columns: match kind {
                    DocKind::Receipt | DocKind::Invoice => vec![
                        ItemColumn::new(F::Name, 46),
                        ItemColumn::new(F::Qty, 14),
                        ItemColumn::new(F::Price, 18),
                        ItemColumn::new(F::Amount, 22),
                    ],
                    DocKind::Voucher => {
                        vec![ItemColumn::new(F::Name, 65), ItemColumn::new(F::Amount, 35)]
                    }
                    DocKind::Schedule => vec![
                        ItemColumn::new(F::Due, 40),
                        ItemColumn::new(F::Amount, 30),
                        ItemColumn::new(F::State, 30),
                    ],
                    DocKind::Label | DocKind::Report => vec![],
                },
            },
            "totals" => Section::Totals {
                lines: vec![TotalLine::Total],
            },
            "balance" => Section::Balance,
            "text" => Section::Text {
                text: String::new(),
                align: Align::Center,
                size: 0,
                bold: false,
            },
            "barcode" => Section::Barcode {
                value: match kind {
                    DocKind::Label => "{item.barcode}".into(),
                    DocKind::Report => "{report.title}".into(),
                    _ => "{doc.number}".into(),
                },
            },
            "signatures" => Section::Signatures {
                start: String::new(),
                end: String::new(),
            },
            "divider" => Section::Divider { dashed: true },
            "spacer" => Section::Spacer { height: 8 },
            _ => return None,
        })
    }

    pub fn code(&self) -> &'static str {
        match self {
            Section::Header { .. } => "header",
            Section::Title { .. } => "title",
            Section::Fields { .. } => "fields",
            Section::Parties { .. } => "parties",
            Section::Items { .. } => "items",
            Section::Totals { .. } => "totals",
            Section::Balance => "balance",
            Section::Text { .. } => "text",
            Section::Barcode { .. } => "barcode",
            Section::Signatures { .. } => "signatures",
            Section::Divider { .. } => "divider",
            Section::Spacer { .. } => "spacer",
        }
    }
}

/// A document's template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    #[serde(default = "first_version")]
    pub version: u32,
    pub kind: DocKind,
    #[serde(default)]
    pub style: Style,
    pub sections: Vec<Section>,
}

fn first_version() -> u32 {
    1
}

/// The variables a document of `kind` fills.
pub fn variables(kind: DocKind) -> Vec<&'static str> {
    let mut v = vec![
        "shop.name",
        "shop.tagline",
        "shop.address",
        "shop.phone",
        "shop.email",
        "shop.nif",
        "shop.nis",
        "shop.rc",
        "shop.ai",
        "doc.title",
        "doc.number",
        "doc.date",
        "doc.time",
        "doc.cashier",
        "notice",
        "footer",
    ];
    match kind {
        DocKind::Receipt | DocKind::Invoice => v.extend([
            "doc.register",
            "customer.name",
            "customer.ids",
            "customer.phone",
            "items.count",
            "total",
            "paid",
            "change",
            "owed",
            "balance.before",
            "balance.after",
        ]),
        DocKind::Voucher => v.extend([
            "customer.name",
            "customer.ids",
            "customer.phone",
            "amount",
            "method",
            "reference",
            "note",
        ]),
        DocKind::Schedule => v.extend([
            "customer.name",
            "customer.ids",
            "customer.phone",
            "doc.version",
            "sale.number",
            "sale.total",
            "down",
            "financed",
            "paid",
            "left",
        ]),
        DocKind::Label => v.extend([
            "item.name",
            "item.price",
            "item.unit",
            "item.barcode",
            "item.sku",
        ]),
        DocKind::Report => v.extend(["report.title", "report.period", "report.rows"]),
    }
    v
}

/// The `{name}` variables in a text.
pub fn names_in(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
                {
                    out.push(name);
                }
                rest = &after[close + 1..];
            }
            None => break,
        }
    }
    out
}

/// `text` with its `{name}` variables filled by `value`; an unknown name
/// stays as written.
pub fn fill(text: &str, value: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                match value(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

const MAX_TEXT: usize = 300;

fn valid_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl Template {
    /// Every texts of the template.
    fn texts(&self) -> Vec<&str> {
        let mut t = Vec::new();
        for s in &self.sections {
            match s {
                Section::Title { text, .. } | Section::Text { text, .. } => t.push(text.as_str()),
                Section::Fields { fields } => {
                    for f in fields {
                        t.push(f.label.as_str());
                        t.push(f.value.as_str());
                    }
                }
                Section::Items { columns } => t.extend(columns.iter().map(|c| c.title.as_str())),
                Section::Barcode { value } => t.push(value.as_str()),
                Section::Signatures { start, end } => {
                    t.push(start.as_str());
                    t.push(end.as_str());
                }
                _ => {}
            }
        }
        t
    }

    /// Whether the template can be kept: what is wrong, field by field.
    pub fn check(&self) -> Result<(), Vec<RuleError>> {
        let mut e = Vec::new();
        if self.version != 1 {
            e.push(RuleError::on("version", "template.version"));
        }
        if !valid_hex(&self.style.accent) {
            e.push(RuleError::on("accent", "template.color"));
        }
        if !(7..=14).contains(&self.style.size) {
            e.push(RuleError::on("size", "template.size"));
        }
        if !(30..=200).contains(&self.style.logo_width) {
            e.push(RuleError::on("logo_width", "template.size"));
        }
        if self.sections.is_empty() || self.sections.len() > 40 {
            e.push(RuleError::on("sections", "template.sections"));
        }
        let known = variables(self.kind);
        for t in self.texts() {
            if t.chars().count() > MAX_TEXT {
                e.push(RuleError::on("text", "template.text_long"));
            }
            if names_in(t).iter().any(|n| !known.contains(n)) {
                e.push(RuleError::on("text", "template.variable"));
            }
        }
        let items: Vec<&Vec<ItemColumn>> = self
            .sections
            .iter()
            .filter_map(|s| match s {
                Section::Items { columns } => Some(columns),
                _ => None,
            })
            .collect();
        if items.len() > 1 {
            e.push(RuleError::on("items", "template.items_once"));
        }
        let allowed = ItemField::of(self.kind);
        for cols in items {
            // A report's columns are its own.
            if self.kind == DocKind::Report {
                continue;
            }
            if cols.is_empty() || cols.len() > 10 {
                e.push(RuleError::on("columns", "template.columns"));
            }
            if cols
                .iter()
                .any(|c| c.width == 0 || !allowed.contains(&c.field))
            {
                e.push(RuleError::on("columns", "template.columns"));
            }
        }
        for s in &self.sections {
            match s {
                Section::Text { size, .. } if !(-3..=8).contains(size) => {
                    e.push(RuleError::on("size", "template.size"));
                }
                Section::Spacer { height } if *height > 80 => {
                    e.push(RuleError::on("height", "template.size"));
                }
                Section::Fields { fields } if fields.len() > 20 => {
                    e.push(RuleError::on("fields", "template.sections"));
                }
                _ => {}
            }
        }
        e.dedup();
        if e.is_empty() { Ok(()) } else { Err(e) }
    }

    /// Reads a kept template; `None` when it cannot be used (the caller
    /// prints with the built-in one).
    pub fn parse(json: &str) -> Option<Self> {
        let t: Template = serde_json::from_str(json).ok()?;
        t.check().ok()?;
        Some(t)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

// ------------------------------------------------------------- built-in

fn text(t: &str, align: Align, size: i8, bold: bool) -> Section {
    Section::Text {
        text: t.into(),
        align,
        size,
        bold,
    }
}

/// The templates that come with the app: (code, template). The first is
/// the default. Texts use variables only; labels come from the language.
pub fn builtin(kind: DocKind) -> Vec<(&'static str, Template)> {
    use ItemField as F;
    use TotalLine as T;
    let all_totals = vec![
        T::Discount,
        T::Net,
        T::Vat,
        T::Stamp,
        T::Total,
        T::Payments,
        T::Change,
        T::Owed,
    ];
    let t = |style: Style, sections: Vec<Section>| Template {
        version: 1,
        kind,
        style,
        sections,
    };
    match kind {
        DocKind::Receipt => vec![
            (
                "classic",
                t(
                    Style::default(),
                    vec![
                        Section::Header { legal_ids: true },
                        Section::Divider { dashed: true },
                        Section::Parties { details: true },
                        Section::Items {
                            columns: vec![
                                ItemColumn::new(F::Name, 42),
                                ItemColumn::new(F::Qty, 14),
                                ItemColumn::new(F::Price, 21),
                                ItemColumn::new(F::Amount, 23),
                            ],
                        },
                        text("{items.count}", Align::Start, -1, false),
                        Section::Totals {
                            lines: all_totals.clone(),
                        },
                        Section::Balance,
                        Section::Divider { dashed: true },
                        text("{footer}", Align::Center, 0, true),
                        text("{notice}", Align::Center, -1, false),
                        Section::Barcode {
                            value: "{doc.number}".into(),
                        },
                    ],
                ),
            ),
            (
                "compact",
                t(
                    Style {
                        size: 9,
                        logo: false,
                        ..Style::default()
                    },
                    vec![
                        Section::Header { legal_ids: false },
                        Section::Divider { dashed: true },
                        Section::Parties { details: true },
                        Section::Items {
                            columns: vec![
                                ItemColumn::new(F::Name, 50),
                                ItemColumn::new(F::Qty, 18),
                                ItemColumn::new(F::Amount, 32),
                            ],
                        },
                        Section::Totals {
                            lines: vec![T::Discount, T::Total, T::Payments, T::Change, T::Owed],
                        },
                        Section::Balance,
                        text("{footer}", Align::Center, 0, true),
                    ],
                ),
            ),
        ],
        DocKind::Invoice => vec![
            (
                "modern",
                t(
                    Style {
                        accent: "#4338CA".into(),
                        header: HeaderLayout::Split,
                        table: TableStyle::Striped,
                        ..Style::default()
                    },
                    vec![
                        Section::Header { legal_ids: true },
                        Section::Spacer { height: 10 },
                        Section::Parties { details: true },
                        Section::Spacer { height: 6 },
                        Section::Items {
                            columns: vec![
                                ItemColumn::new(F::Index, 6),
                                ItemColumn::new(F::Name, 36),
                                ItemColumn::new(F::QtyUnit, 13),
                                ItemColumn::new(F::Price, 15),
                                ItemColumn::new(F::TaxRate, 10),
                                ItemColumn::new(F::Amount, 20),
                            ],
                        },
                        Section::Totals {
                            lines: all_totals.clone(),
                        },
                        Section::Balance,
                        Section::Spacer { height: 12 },
                        text("{notice}", Align::Start, -1, false),
                        Section::Signatures {
                            start: String::new(),
                            end: String::new(),
                        },
                        text("{footer}", Align::Center, -1, false),
                    ],
                ),
            ),
            (
                "classic",
                t(
                    Style {
                        table: TableStyle::Ruled,
                        ..Style::default()
                    },
                    vec![
                        Section::Header { legal_ids: true },
                        Section::Divider { dashed: false },
                        Section::Title {
                            text: "{doc.title} {doc.number}".into(),
                            align: Align::Center,
                        },
                        Section::Parties { details: true },
                        Section::Items {
                            columns: vec![
                                ItemColumn::new(F::Index, 6),
                                ItemColumn::new(F::Name, 40),
                                ItemColumn::new(F::QtyUnit, 14),
                                ItemColumn::new(F::Price, 18),
                                ItemColumn::new(F::Amount, 22),
                            ],
                        },
                        Section::Totals { lines: all_totals },
                        Section::Spacer { height: 12 },
                        text("{notice}", Align::Start, -1, false),
                        Section::Signatures {
                            start: String::new(),
                            end: String::new(),
                        },
                    ],
                ),
            ),
        ],
        DocKind::Voucher => vec![(
            "standard",
            t(
                Style::default(),
                vec![
                    Section::Header { legal_ids: true },
                    Section::Divider { dashed: false },
                    Section::Title {
                        text: "{doc.title}".into(),
                        align: Align::Center,
                    },
                    Section::Parties { details: true },
                    text("{amount}", Align::Center, 8, true),
                    Section::Fields {
                        fields: vec![
                            Field::new("@print.voucher.method", "{method}"),
                            Field::new("@print.voucher.reference", "{reference}"),
                        ],
                    },
                    Section::Items {
                        columns: vec![ItemColumn::new(F::Name, 65), ItemColumn::new(F::Amount, 35)],
                    },
                    text("{note}", Align::Start, -1, false),
                    Section::Signatures {
                        start: String::new(),
                        end: String::new(),
                    },
                    Section::Barcode {
                        value: "{doc.number}".into(),
                    },
                ],
            ),
        )],
        DocKind::Schedule => vec![(
            "standard",
            t(
                Style {
                    table: TableStyle::Shaded,
                    ..Style::default()
                },
                vec![
                    Section::Header { legal_ids: true },
                    Section::Divider { dashed: false },
                    Section::Title {
                        text: "{doc.title}".into(),
                        align: Align::Center,
                    },
                    Section::Parties { details: true },
                    Section::Items {
                        columns: vec![
                            ItemColumn::new(F::Index, 8),
                            ItemColumn::new(F::Due, 26),
                            ItemColumn::new(F::Amount, 22),
                            ItemColumn::new(F::Paid, 22),
                            ItemColumn::new(F::State, 22),
                        ],
                    },
                    Section::Totals {
                        lines: vec![T::Total],
                    },
                    text("{notice}", Align::Start, -1, false),
                    Section::Signatures {
                        start: String::new(),
                        end: String::new(),
                    },
                ],
            ),
        )],
        DocKind::Label => vec![(
            "standard",
            t(
                Style {
                    logo: false,
                    ..Style::default()
                },
                vec![
                    text("{item.name}", Align::Center, 1, true),
                    text("{item.price}", Align::Center, 4, true),
                    Section::Barcode {
                        value: "{item.barcode}".into(),
                    },
                ],
            ),
        )],
        DocKind::Report => vec![(
            "standard",
            t(
                Style {
                    accent: "#4338CA".into(),
                    header: HeaderLayout::Split,
                    table: TableStyle::Striped,
                    size: 9,
                    logo: true,
                    logo_width: 70,
                },
                vec![
                    // The title and the period at the header's end.
                    Section::Header { legal_ids: false },
                    Section::Spacer { height: 12 },
                    Section::Items { columns: vec![] },
                ],
            ),
        )],
    }
}

/// The template to print `kind` with: the kept one if usable, else the
/// first built-in.
pub fn usable(kind: DocKind, kept: Option<&str>) -> Template {
    kept.and_then(Template::parse)
        .filter(|t| t.kind == kind)
        .unwrap_or_else(|| builtin(kind).remove(0).1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fresh_section_passes_the_check() {
        for kind in DocKind::ALL {
            for code in Section::CODES {
                let mut t = builtin(kind).remove(0).1;
                t.sections.retain(|s| !matches!(s, Section::Items { .. }));
                let s = Section::fresh(code, kind);
                assert_eq!(s.as_ref().map(Section::code), Some(code));
                t.sections.extend(s);
                if kind == DocKind::Label && code == "items" {
                    continue; // labels have no rows
                }
                assert_eq!(t.check(), Ok(()), "{kind:?} {code}");
            }
        }
        assert!(Section::fresh("nope", DocKind::Receipt).is_none());
    }

    #[test]
    fn every_builtin_is_valid_and_round_trips() {
        for k in DocKind::ALL {
            for (code, t) in builtin(k) {
                assert_eq!(t.check(), Ok(()), "{k:?} {code}");
                assert_eq!(Template::parse(&t.to_json()), Some(t), "{k:?} {code}");
            }
        }
    }

    #[test]
    fn variables_fill_and_unknown_ones_are_refused() {
        let text = "{doc.title} n° {doc.number} — {nope}";
        assert_eq!(names_in(text), ["doc.title", "doc.number", "nope"]);
        let filled = fill(text, |n| match n {
            "doc.title" => Some("Facture".into()),
            "doc.number" => Some("FV-1".into()),
            _ => None,
        });
        assert_eq!(filled, "Facture n° FV-1 — {nope}");
        assert_eq!(fill("{ open", |_| None), "{ open");
        let mut t = builtin(DocKind::Invoice).remove(0).1;
        t.sections.push(text_section("{customer.secret}"));
        assert!(
            t.check()
                .is_err_and(|e| e.iter().any(|r| r.code == "template.variable"))
        );
    }

    fn text_section(t: &str) -> Section {
        text(t, Align::Start, 0, false)
    }

    #[test]
    fn a_broken_template_is_never_printed_with() {
        // Not JSON, another kind, a wrong colour: the built-in is used.
        let first = builtin(DocKind::Receipt).remove(0).1;
        assert_eq!(usable(DocKind::Receipt, Some("{")), first);
        let invoice = builtin(DocKind::Invoice).remove(0).1.to_json();
        assert_eq!(usable(DocKind::Receipt, Some(&invoice)), first);
        let mut bad = builtin(DocKind::Receipt).remove(1).1;
        bad.style.accent = "blue".into();
        assert!(bad.check().is_err());
        assert_eq!(usable(DocKind::Receipt, Some(&bad.to_json())), first);
        // Columns from another document are refused.
        let mut cols = builtin(DocKind::Voucher).remove(0).1;
        cols.sections.push(Section::Items {
            columns: vec![ItemColumn::new(ItemField::Due, 50)],
        });
        assert!(cols.check().is_err());
        // An older file without what came later still reads.
        let minimal = r#"{"kind":"receipt","sections":[{"type":"header"},{"type":"items","columns":[{"field":"name","width":60},{"field":"amount","width":40}]}]}"#;
        assert!(Template::parse(minimal).is_some());
    }
}

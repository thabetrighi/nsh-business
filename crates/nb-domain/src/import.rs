//! The import wizard's rules (M14.3): a spreadsheet's text read as a table
//! (`,`, `;` or tab, quoted fields), the fields each kind of record takes,
//! a guess of which column feeds which field from the header, and the
//! rows as the importers read them once the person has confirmed the
//! mapping. Values are checked by each importer, line by line.

use std::collections::BTreeMap;

use crate::RuleError;

/// Most lines one import may have.
pub const MAX_ROWS: usize = 20_000;

/// What is being imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Products,
    Customers,
    Suppliers,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Kind::Products => "products",
            Kind::Customers => "customers",
            Kind::Suppliers => "suppliers",
        }
    }

    /// The fields of this kind, in the order the wizard shows them.
    pub fn fields(self) -> &'static [Field] {
        match self {
            Kind::Products => PRODUCT_FIELDS,
            Kind::Customers => CUSTOMER_FIELDS,
            Kind::Suppliers => SUPPLIER_FIELDS,
        }
    }
}

/// A field a record takes, and the header names that mean it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub key: &'static str,
    pub required: bool,
    pub aliases: &'static [&'static str],
}

const fn f(key: &'static str, required: bool, aliases: &'static [&'static str]) -> Field {
    Field {
        key,
        required,
        aliases,
    }
}

const NAME: &[&str] = &[
    "name",
    "nom",
    "raison sociale",
    "désignation",
    "designation",
    "الاسم",
    "المنتج",
];
const PHONE: &[&str] = &[
    "phone",
    "téléphone",
    "telephone",
    "tél",
    "tel",
    "mobile",
    "الهاتف",
];
const EMAIL: &[&str] = &["email", "e-mail", "courriel", "البريد"];
const CITY: &[&str] = &["city", "ville", "wilaya", "المدينة", "الولاية"];
const ADDRESS: &[&str] = &["address", "adresse", "العنوان"];
const NIF: &[&str] = &["nif", "tax id", "matricule fiscal", "الرقم الجبائي"];
const NIS: &[&str] = &["nis", "الرقم الإحصائي"];
const RC: &[&str] = &["rc", "registre de commerce", "السجل التجاري"];
const AI: &[&str] = &["ai", "article d'imposition", "art", "مادة الضريبة"];
const KIND: &[&str] = &["kind", "type", "النوع"];
const DAYS: &[&str] = &[
    "payment days",
    "délai",
    "delai",
    "délai de paiement",
    "أجل الدفع",
];
const BALANCE: &[&str] = &[
    "balance",
    "solde",
    "solde d'ouverture",
    "dette",
    "الرصيد",
    "الرصيد الافتتاحي",
    "الدين",
];

pub const PRODUCT_FIELDS: &[Field] = &[
    f("name", true, NAME),
    f(
        "price",
        true,
        &["price", "prix", "prix de vente", "السعر", "سعر البيع"],
    ),
    f(
        "sku",
        false,
        &["sku", "code", "référence", "reference", "réf", "الرمز"],
    ),
    f(
        "barcode",
        false,
        &["barcode", "code-barres", "code barres", "ean", "الباركود"],
    ),
    f(
        "cost",
        false,
        &[
            "cost",
            "coût",
            "cout",
            "prix d'achat",
            "التكلفة",
            "سعر الشراء",
        ],
    ),
    f("unit", false, &["unit", "unité", "unite", "الوحدة"]),
    f(
        "category",
        false,
        &[
            "category",
            "catégorie",
            "categorie",
            "famille",
            "الفئة",
            "التصنيف",
            "الصنف",
        ],
    ),
    f(
        "stock",
        false,
        &["stock", "quantité", "quantite", "qty", "الكمية", "المخزون"],
    ),
    f("tax", false, &["tax", "tva", "taxe", "الضريبة"]),
];

pub const CUSTOMER_FIELDS: &[Field] = &[
    f("name", true, NAME),
    f("kind", false, KIND),
    f("phone", false, PHONE),
    f("email", false, EMAIL),
    f("city", false, CITY),
    f("address", false, ADDRESS),
    f("nif", false, NIF),
    f("nis", false, NIS),
    f("rc", false, RC),
    f("ai", false, AI),
    f(
        "credit_limit",
        false,
        &[
            "credit limit",
            "plafond",
            "plafond de crédit",
            "سقف الدين",
            "حد الائتمان",
        ],
    ),
    f("payment_days", false, DAYS),
    f("balance", false, BALANCE),
];

pub const SUPPLIER_FIELDS: &[Field] = &[
    f("name", true, NAME),
    f("kind", false, KIND),
    f("phone", false, PHONE),
    f("email", false, EMAIL),
    f("city", false, CITY),
    f("address", false, ADDRESS),
    f("nif", false, NIF),
    f("nis", false, NIS),
    f("rc", false, RC),
    f("ai", false, AI),
    f("payment_days", false, DAYS),
    f("balance", false, BALANCE),
];

/// A spreadsheet's text as a table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub headers: Vec<String>,
    /// (line in the file, 1 = header; cells).
    pub rows: Vec<(usize, Vec<String>)>,
}

/// Splits one line: `sep`, double quotes around fields, `""` for a quote.
fn split_line(line: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => out.push(std::mem::take(&mut field)),
            c => field.push(c),
        }
    }
    out.push(field);
    out.into_iter().map(|f| f.trim().to_owned()).collect()
}

/// Reads a spreadsheet saved as CSV (or text with tabs).
pub fn parse(text: &str) -> Result<Table, RuleError> {
    let text = text.trim_start_matches('\u{FEFF}');
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let Some((_, header)) = lines.next() else {
        return Err(RuleError::on("file", "import.empty"));
    };
    let sep = [
        ('\t', header.matches('\t').count()),
        (';', header.matches(';').count()),
        (',', header.matches(',').count()),
    ]
    .into_iter()
    .max_by_key(|(_, n)| *n)
    .filter(|(_, n)| *n > 0)
    .map_or(',', |(c, _)| c);
    let headers = split_line(header, sep);
    let mut rows = Vec::new();
    for (i, line) in lines {
        if rows.len() == MAX_ROWS {
            return Err(RuleError::on("file", "import.too_long"));
        }
        rows.push((i + 1, split_line(line, sep)));
    }
    if rows.is_empty() {
        return Err(RuleError::on("file", "import.no_rows"));
    }
    Ok(Table { headers, rows })
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase().replace(['_', '.'], " ")
}

/// For each field of `kind`, the column whose header names it (each
/// column feeds one field at most).
pub fn guess(kind: Kind, headers: &[String]) -> Vec<Option<usize>> {
    let heads: Vec<String> = headers.iter().map(|h| norm(h)).collect();
    let mut taken = vec![false; heads.len()];
    kind.fields()
        .iter()
        .map(|field| {
            let at = heads.iter().enumerate().position(|(i, h)| {
                !taken[i] && (h == field.key || field.aliases.iter().any(|a| norm(a) == *h))
            });
            if let Some(i) = at {
                taken[i] = true;
            }
            at
        })
        .collect()
}

/// A row as an importer reads it: field → value (empty values left out).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    pub line: usize,
    pub fields: BTreeMap<&'static str, String>,
}

impl Row {
    pub fn get(&self, key: &str) -> &str {
        self.fields.get(key).map_or("", String::as_str)
    }
}

/// The rows of `table` through `mapping` (one column or none per field of
/// `kind`). Every required field needs a column.
pub fn rows(kind: Kind, table: &Table, mapping: &[Option<usize>]) -> Result<Vec<Row>, RuleError> {
    let fields = kind.fields();
    if mapping.len() != fields.len() {
        return Err(RuleError::on("mapping", "invalid_value"));
    }
    for (field, col) in fields.iter().zip(mapping) {
        if field.required && col.is_none() {
            return Err(RuleError::on(field.key, "import.missing_column"));
        }
        if col.is_some_and(|c| c >= table.headers.len()) {
            return Err(RuleError::on(field.key, "invalid_value"));
        }
    }
    Ok(table
        .rows
        .iter()
        .map(|(line, cells)| Row {
            line: *line,
            fields: fields
                .iter()
                .zip(mapping)
                .filter_map(|(field, col)| {
                    let v = cells.get((*col)?)?.trim();
                    (!v.is_empty()).then(|| (field.key, v.to_owned()))
                })
                .collect(),
        })
        .collect())
}

/// The rows as CSV text with the fields' own names as header (for the
/// product importer, which reads that).
pub fn to_csv(kind: Kind, rows: &[Row]) -> String {
    let quote = |v: &str| {
        if v.contains([',', '"', '\n', ';']) {
            format!("\"{}\"", v.replace('"', "\"\""))
        } else {
            v.to_owned()
        }
    };
    let keys: Vec<&str> = kind.fields().iter().map(|f| f.key).collect();
    let mut out = keys.join(",");
    out.push('\n');
    for r in rows {
        let line: Vec<String> = keys.iter().map(|k| quote(r.get(k))).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

/// A person or a company, from what a sheet says (`company`, `société`,
/// `entreprise`, `شركة`…); empty: a person.
pub fn kind_of(text: &str) -> Option<&'static str> {
    match norm(text).as_str() {
        "" | "person" | "personne" | "particulier" | "شخص" | "فرد" => Some("person"),
        "company" | "société" | "societe" | "entreprise" | "sarl" | "eurl" | "spa" | "شركة"
        | "مؤسسة" => Some("company"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_any_separator_and_quotes() -> Result<(), RuleError> {
        let t = parse(
            "\u{feff}Nom;Téléphone;Solde\n\"Benali; Ahmed\";0555 12 34 56;1200,50\n\nKarim;;\n",
        )?;
        assert_eq!(t.headers, ["Nom", "Téléphone", "Solde"]);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(
            t.rows[0],
            (
                2,
                vec![
                    "Benali; Ahmed".into(),
                    "0555 12 34 56".into(),
                    "1200,50".into()
                ]
            )
        );
        assert_eq!(t.rows[1].0, 4);
        let t = parse("name\tprice\nMilk\t11.90\n")?;
        assert_eq!(t.rows[0].1, ["Milk", "11.90"]);
        assert_eq!(parse("").map_err(|e| e.code), Err("import.empty"));
        assert_eq!(
            parse("name,price\n").map_err(|e| e.code),
            Err("import.no_rows")
        );
        Ok(())
    }

    #[test]
    fn guesses_columns_in_three_languages() -> Result<(), RuleError> {
        let t = parse("الاسم,الهاتف,الولاية,الرصيد\nأحمد,0555,الوادي,500\n")?;
        let m = guess(Kind::Customers, &t.headers);
        let at = |key: &str| {
            Kind::Customers
                .fields()
                .iter()
                .position(|f| f.key == key)
                .and_then(|i| m[i])
        };
        assert_eq!(
            (at("name"), at("phone"), at("city"), at("balance")),
            (Some(0), Some(1), Some(2), Some(3))
        );
        assert_eq!(at("email"), None);
        let rows = rows(Kind::Customers, &t, &m)?;
        assert_eq!(rows[0].get("city"), "الوادي");
        assert_eq!(rows[0].get("email"), "");
        Ok(())
    }

    #[test]
    fn a_required_field_needs_a_column() -> Result<(), RuleError> {
        let t = parse("Désignation,Qté\nLait,3\n")?;
        let m = guess(Kind::Products, &t.headers);
        assert_eq!(
            rows(Kind::Products, &t, &m).map_err(|e| (e.field, e.code)),
            Err((Some("price"), "import.missing_column"))
        );
        let mut m = m;
        m[1] = Some(1); // the person maps the price to the second column
        let r = rows(Kind::Products, &t, &m)?;
        assert_eq!(
            to_csv(Kind::Products, &r).lines().nth(1),
            Some("Lait,3,,,,,,,")
        );
        Ok(())
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of(""), Some("person"));
        assert_eq!(kind_of("SARL"), Some("company"));
        assert_eq!(kind_of("شركة"), Some("company"));
        assert_eq!(kind_of("robot"), None);
    }
}

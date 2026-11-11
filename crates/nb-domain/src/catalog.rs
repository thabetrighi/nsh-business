//! Catalog organization (PRD §6.3): category trees, variant matrices, and
//! the simple CSV product import (M5.10).

use std::collections::{BTreeMap, BTreeSet};

use crate::RuleError;

// ---------------------------------------------------------------------------
// Category trees

/// Whether giving `node` the parent `new_parent` would make a loop, given
/// the current `parents` (child → parent). A category cannot be its own
/// ancestor.
pub fn would_cycle(
    parents: &BTreeMap<String, Option<String>>,
    node: &str,
    new_parent: Option<&str>,
) -> bool {
    let mut at = new_parent.map(str::to_owned);
    let mut steps = 0;
    while let Some(p) = at {
        if p == node || steps > parents.len() {
            return true;
        }
        at = parents.get(&p).cloned().flatten();
        steps += 1;
    }
    false
}

/// Deepest nesting allowed (Family › Group › Subgroup …).
pub const MAX_DEPTH: usize = 5;

/// Depth of `node` (1 for a top category) if it took `parent`.
pub fn depth_with(parents: &BTreeMap<String, Option<String>>, parent: Option<&str>) -> usize {
    let mut d = 1;
    let mut at = parent.map(str::to_owned);
    while let Some(p) = at {
        d += 1;
        if d > MAX_DEPTH + 1 {
            break;
        }
        at = parents.get(&p).cloned().flatten();
    }
    d
}

/// The path of names from the top (`Boissons › Eaux › Gazeuses`).
pub fn path(
    parents: &BTreeMap<String, Option<String>>,
    names: &BTreeMap<String, String>,
    node: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = Some(node.to_owned());
    while let Some(id) = at {
        if out.len() > MAX_DEPTH + 1 {
            break;
        }
        out.push(names.get(&id).cloned().unwrap_or_default());
        at = parents.get(&id).cloned().flatten();
    }
    out.reverse();
    out
}

// ---------------------------------------------------------------------------
// Variants (M5.4)

/// Most variants one product may have (a matrix larger than this is almost
/// always a typing mistake).
pub const MAX_VARIANTS: usize = 200;

/// One attribute and its values (`Taille: S, M, L`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub values: Vec<String>,
}

/// One combination: (attribute, value) in attribute order.
pub type Combination = Vec<(String, String)>;

/// Every combination of the attributes' values (the variant matrix), in
/// order: the first attribute varies slowest. Names and values are
/// trimmed; empty ones, duplicates and oversized matrices are refused.
pub fn combinations(attrs: &[Attribute]) -> Result<Vec<Combination>, RuleError> {
    if attrs.is_empty() {
        return Err(RuleError::on("attributes", "variants.no_attribute"));
    }
    let mut names = BTreeSet::new();
    let mut clean = Vec::new();
    for a in attrs {
        let name = a.name.trim().to_owned();
        if name.is_empty() || !names.insert(name.to_lowercase()) {
            return Err(RuleError::on("attributes", "variants.attribute_name"));
        }
        let mut seen = BTreeSet::new();
        let values: Vec<String> = a
            .values
            .iter()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .filter(|v| seen.insert(v.to_lowercase()))
            .collect();
        if values.is_empty() {
            return Err(RuleError::on("attributes", "variants.no_value"));
        }
        clean.push((name, values));
    }
    let total = clean
        .iter()
        .try_fold(1usize, |n, (_, v)| n.checked_mul(v.len()))
        .unwrap_or(usize::MAX);
    if total > MAX_VARIANTS {
        return Err(RuleError::on("attributes", "variants.too_many"));
    }
    let mut out: Vec<Combination> = vec![vec![]];
    for (name, values) in &clean {
        out = out
            .into_iter()
            .flat_map(|c| {
                values.iter().map(move |v| {
                    let mut c = c.clone();
                    c.push((name.clone(), v.clone()));
                    c
                })
            })
            .collect();
    }
    Ok(out)
}

/// A variant's code: the parent's, then each value's letters and digits
/// (`TSH-01` + `Rouge`, `XL` → `TSH-01-ROUGE-XL`). Non-Latin values give
/// their position instead (`-1-3`), so codes stay ASCII.
pub fn variant_sku(parent: &str, combination: &Combination, attrs: &[Attribute]) -> String {
    let mut sku = parent.to_owned();
    for (i, (_, value)) in combination.iter().enumerate() {
        let ascii: String = value
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_uppercase())
            .take(8)
            .collect();
        let part = if ascii.is_empty() {
            attrs
                .get(i)
                .and_then(|a| a.values.iter().position(|v| v.trim() == value))
                .map_or_else(|| "X".to_owned(), |p| (p + 1).to_string())
        } else {
            ascii
        };
        sku.push('-');
        sku.push_str(&part);
    }
    sku
}

/// A variant's name: the parent's and its values (`T-shirt · Rouge · XL`).
pub fn variant_name(parent: &str, combination: &Combination) -> String {
    let mut name = parent.to_owned();
    for (_, v) in combination {
        name.push_str(" · ");
        name.push_str(v);
    }
    name
}

// ---------------------------------------------------------------------------
// CSV import (M5.10)

/// The columns an import file may have (header names, in any order, any of
/// the three languages' usual words). `name` and `price` are required.
pub const COLUMNS: &[(&str, &[&str])] = &[
    (
        "name",
        &[
            "name",
            "nom",
            "désignation",
            "designation",
            "الاسم",
            "المنتج",
        ],
    ),
    ("sku", &["sku", "code", "référence", "reference", "الرمز"]),
    (
        "barcode",
        &["barcode", "code-barres", "code barres", "ean", "الباركود"],
    ),
    ("price", &["price", "prix", "prix de vente", "السعر"]),
    (
        "cost",
        &[
            "cost",
            "coût",
            "cout",
            "prix d'achat",
            "التكلفة",
            "سعر الشراء",
        ],
    ),
    ("unit", &["unit", "unité", "unite", "الوحدة"]),
    (
        "category",
        &[
            "category",
            "catégorie",
            "categorie",
            "famille",
            "الفئة",
            "التصنيف",
        ],
    ),
    (
        "stock",
        &["stock", "quantité", "quantite", "qty", "الكمية", "المخزون"],
    ),
    ("tax", &["tax", "tva", "taxe", "الضريبة"]),
];

/// One line of an import file, as text (checked by the product rules).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportRow {
    /// Line number in the file (1 = header), for error messages.
    pub line: usize,
    pub fields: BTreeMap<&'static str, String>,
}

impl ImportRow {
    pub fn get(&self, column: &str) -> &str {
        self.fields.get(column).map_or("", String::as_str)
    }
}

/// Splits one CSV line: `,` or `;` (whichever the header uses), double
/// quotes around fields, `""` for a quote inside.
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

/// Most lines one import may have.
pub const MAX_IMPORT_ROWS: usize = 20_000;

/// Reads an import file. Errors are about the file itself (no header, no
/// name or price column, too long); rows are checked by the use case.
pub fn parse_csv(text: &str) -> Result<Vec<ImportRow>, RuleError> {
    let text = text.trim_start_matches('\u{FEFF}');
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let Some((_, header)) = lines.next() else {
        return Err(RuleError::on("file", "import.empty"));
    };
    let sep = if header.matches(';').count() > header.matches(',').count() {
        ';'
    } else {
        ','
    };
    let columns: Vec<Option<&'static str>> = split_line(header, sep)
        .iter()
        .map(|h| {
            let h = h.to_lowercase();
            COLUMNS
                .iter()
                .find(|(_, names)| names.contains(&h.as_str()))
                .map(|(c, _)| *c)
        })
        .collect();
    for required in ["name", "price"] {
        if !columns.contains(&Some(required)) {
            return Err(RuleError::on("file", "import.missing_column"));
        }
    }
    let mut rows = Vec::new();
    for (i, line) in lines {
        if rows.len() == MAX_IMPORT_ROWS {
            return Err(RuleError::on("file", "import.too_long"));
        }
        let mut fields = BTreeMap::new();
        for (col, value) in columns.iter().zip(split_line(line, sep)) {
            if let Some(c) = col
                && !value.is_empty()
            {
                fields.insert(*c, value);
            }
        }
        rows.push(ImportRow {
            line: i + 1,
            fields,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), b.map(str::to_owned)))
            .collect()
    }

    #[test]
    fn categories_never_loop() {
        let t = tree(&[
            ("food", None),
            ("dairy", Some("food")),
            ("milk", Some("dairy")),
        ]);
        assert!(would_cycle(&t, "food", Some("milk")));
        assert!(would_cycle(&t, "dairy", Some("dairy")));
        assert!(!would_cycle(&t, "milk", Some("food")));
        assert!(!would_cycle(&t, "milk", None));
        assert_eq!(depth_with(&t, Some("milk")), 4);
        assert_eq!(depth_with(&t, None), 1);
        let names: BTreeMap<String, String> = [
            ("food", "Alimentation"),
            ("dairy", "Laitiers"),
            ("milk", "Lait"),
        ]
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
        assert_eq!(
            path(&t, &names, "milk"),
            ["Alimentation", "Laitiers", "Lait"]
        );
    }

    /// Acceptance of M5.4: the matrix is generated.
    #[test]
    fn variant_matrix() -> Result<(), RuleError> {
        let attrs = vec![
            Attribute {
                name: "Couleur".into(),
                values: vec!["Rouge".into(), " Bleu ".into(), "rouge".into()],
            },
            Attribute {
                name: "Taille".into(),
                values: vec!["S".into(), "M".into(), "XL".into()],
            },
        ];
        let c = combinations(&attrs)?;
        assert_eq!(c.len(), 6, "duplicates ignored, 2 × 3");
        assert_eq!(
            c[0],
            vec![
                ("Couleur".into(), "Rouge".into()),
                ("Taille".into(), "S".into())
            ]
        );
        assert_eq!(
            c[5],
            vec![
                ("Couleur".into(), "Bleu".into()),
                ("Taille".into(), "XL".into())
            ]
        );
        assert_eq!(variant_sku("TSH-01", &c[5], &attrs), "TSH-01-BLEU-XL");
        assert_eq!(variant_name("T-shirt", &c[5]), "T-shirt · Bleu · XL");
        let ar = vec![Attribute {
            name: "اللون".into(),
            values: vec!["أحمر".into(), "أزرق".into()],
        }];
        let c = combinations(&ar)?;
        assert_eq!(variant_sku("TSH", &c[1], &ar), "TSH-2");
        Ok(())
    }

    #[test]
    fn bad_matrices_are_refused() {
        let a = |n: &str, v: &[&str]| Attribute {
            name: n.into(),
            values: v.iter().map(|s| (*s).to_owned()).collect(),
        };
        assert_eq!(
            combinations(&[]),
            Err(RuleError::on("attributes", "variants.no_attribute"))
        );
        assert_eq!(
            combinations(&[a("Taille", &[" "])]),
            Err(RuleError::on("attributes", "variants.no_value"))
        );
        assert_eq!(
            combinations(&[a("T", &["1"]), a("t", &["2"])]),
            Err(RuleError::on("attributes", "variants.attribute_name"))
        );
        let many: Vec<String> = (0..15).map(|i| i.to_string()).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            combinations(&[a("A", &many), a("B", &many)]),
            Err(RuleError::on("attributes", "variants.too_many"))
        );
    }

    #[test]
    fn csv_files_in_any_language() -> Result<(), RuleError> {
        let fr = "\u{FEFF}Désignation;Code-barres;Prix;Catégorie;Stock\n\"Huile \"\"Elio\"\" 5L\";6130000000017;950,00;Huiles;12\n\nLait;;120;;\n";
        let rows = parse_csv(fr)?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("name"), "Huile \"Elio\" 5L");
        assert_eq!(rows[0].get("price"), "950,00");
        assert_eq!(rows[0].get("barcode"), "6130000000017");
        assert_eq!(rows[1].line, 4);
        assert_eq!(rows[1].get("category"), "");
        let ar = "الاسم,السعر,الوحدة\nسكر,90,kg\n";
        assert_eq!(parse_csv(ar)?[0].get("unit"), "kg");
        assert_eq!(parse_csv(""), Err(RuleError::on("file", "import.empty")));
        assert_eq!(
            parse_csv("nom;stock\nx;1"),
            Err(RuleError::on("file", "import.missing_column"))
        );
        Ok(())
    }
}

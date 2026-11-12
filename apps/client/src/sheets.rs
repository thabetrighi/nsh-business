//! Spreadsheets in and out (owner notes): a file chosen for an import is
//! read whatever it is — Excel (`.xlsx`, `.xls`), OpenDocument (`.ods`) or
//! text (`.csv`, `.txt`) — into a table; exports and import templates are
//! written as Excel files (header in bold, numbers as numbers, the sheet
//! right to left in Arabic) or CSV.

use std::path::Path;

use nb_domain::import::{Table, parse};

/// Most lines read from a sheet (as for text).
const MAX_ROWS: usize = nb_domain::import::MAX_ROWS;

/// A cell as text: whole numbers without decimals, dates as Excel shows
/// them, nothing for an empty cell.
fn cell_text(c: &calamine::Data) -> String {
    use calamine::Data;
    match c {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_owned(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(d) => d.to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(_) => String::new(),
    }
}

/// The first sheet of a workbook as rows of text (empty rows left out).
fn workbook_rows(path: &Path) -> Result<Vec<Vec<String>>, String> {
    use calamine::{Reader, open_workbook_auto};
    let mut wb = open_workbook_auto(path).map_err(|e| e.to_string())?;
    let name = wb
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| "empty workbook".to_owned())?;
    let range = wb.worksheet_range(&name).map_err(|e| e.to_string())?;
    Ok(range
        .rows()
        .map(|r| r.iter().map(cell_text).collect::<Vec<_>>())
        .filter(|r| r.iter().any(|c| !c.is_empty()))
        .take(MAX_ROWS + 1)
        .collect())
}

/// Rows (the header first) as a table, lines numbered as in the sheet.
pub fn table_of(rows: Vec<Vec<String>>) -> Result<Table, nb_domain::RuleError> {
    let mut it = rows.into_iter();
    let Some(headers) = it.next() else {
        return Err(nb_domain::RuleError::on("file", "import.empty"));
    };
    let rows: Vec<(usize, Vec<String>)> = it.enumerate().map(|(i, r)| (i + 2, r)).collect();
    if rows.is_empty() {
        return Err(nb_domain::RuleError::on("file", "import.no_rows"));
    }
    if rows.len() > MAX_ROWS {
        return Err(nb_domain::RuleError::on("file", "import.too_long"));
    }
    Ok(Table { headers, rows })
}

/// Whether a file is a workbook (by its extension).
pub fn is_workbook(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("xlsx" | "xlsm" | "xlsb" | "xls" | "ods")
    )
}

/// Reads a file chosen for an import: a workbook's first sheet, or text.
pub fn read_file(path: &Path) -> Result<Table, String> {
    if is_workbook(path) {
        let rows = workbook_rows(path)?;
        return table_of(rows).map_err(|e| e.code.to_owned());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    // Text saved by Excel on Windows may be Windows-1252, not UTF-8.
    let text = String::from_utf8(bytes.clone())
        .unwrap_or_else(|_| bytes.iter().map(|b| char::from(*b)).collect());
    parse(&text).map_err(|e| e.code.to_owned())
}

/// A number as a spreadsheet takes it (`1 234,50` → 1234.5), if it is one.
fn as_number(s: &str) -> Option<f64> {
    let t: String = s
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}')
        .collect();
    if t.is_empty()
        || t.len() > 18
        || t.starts_with('0') && t.len() > 1 && !t.starts_with("0.") && !t.starts_with("0,")
    {
        // Codes with a leading zero (barcodes, phones) stay text.
        return None;
    }
    let t = t.replace(',', ".");
    if t.chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
        && t.matches('.').count() <= 1
    {
        t.parse().ok()
    } else {
        None
    }
}

/// Writes `rows` (the header first) as an Excel workbook.
pub fn write_xlsx(path: &Path, sheet: &str, rows: &[Vec<String>], rtl: bool) -> Result<(), String> {
    use rust_xlsxwriter::{Format, Workbook};
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    let name: String = sheet
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .take(31)
        .collect();
    if !name.trim().is_empty() {
        ws.set_name(name.trim()).map_err(|e| e.to_string())?;
    }
    if rtl {
        ws.set_right_to_left(true);
    }
    let head = Format::new().set_bold().set_background_color(0xEEF0F5);
    for (r, row) in rows.iter().enumerate() {
        let r32 = u32::try_from(r).map_err(|e| e.to_string())?;
        for (c, v) in row.iter().enumerate() {
            let c16 = u16::try_from(c).map_err(|e| e.to_string())?;
            if r == 0 {
                ws.write_string_with_format(r32, c16, v, &head)
                    .map_err(|e| e.to_string())?;
            } else if let Some(n) = as_number(v) {
                ws.write_number(r32, c16, n).map_err(|e| e.to_string())?;
            } else {
                ws.write_string(r32, c16, v).map_err(|e| e.to_string())?;
            }
        }
    }
    ws.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
    ws.autofit();
    wb.save(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workbook_written_reads_back_as_a_table() {
        let dir = std::env::temp_dir().join(format!("nb-sheets-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("t.xlsx");
        let rows = vec![
            vec!["Nom".to_owned(), "Prix".into(), "Code".into()],
            vec!["Lait".into(), "120,50".into(), "0613000123".into()],
            vec!["Pain".into(), "15".into(), "".into()],
        ];
        assert!(write_xlsx(&path, "Produits", &rows, true).is_ok());
        let t = read_file(&path).unwrap_or_default();
        assert_eq!(t.headers, ["Nom", "Prix", "Code"]);
        // Numbers come back as numbers; codes keep their leading zero.
        assert_eq!(
            t.rows[0],
            (
                2,
                vec!["Lait".to_owned(), "120.5".into(), "0613000123".into()]
            )
        );
        assert_eq!(t.rows[1].1[1], "15");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn numbers_and_codes_are_told_apart() {
        assert_eq!(as_number("1 234,50"), Some(1234.5));
        assert_eq!(as_number("-12"), Some(-12.0));
        assert_eq!(as_number("0.5"), Some(0.5));
        assert_eq!(as_number("0613000123"), None);
        assert_eq!(as_number("FV-01"), None);
        assert_eq!(as_number(""), None);
    }
}

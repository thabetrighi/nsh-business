//! Writes sample documents (receipts, invoices) to a folder, to look at:
//! `cargo run -p nb-docs --example sample -- <folder>`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use nb_core::Currency;
use nb_docs::templates::{Line, Options, Sale, Shop, invoice, receipt};
use nb_docs::{Paper, escpos, pdf, preview};
use nb_i18n::Lang;

fn shop() -> Shop {
    Shop {
        name: "متجر النور".into(),
        address: "الوادي — حي 400 مسكن".into(),
        phone: "0550 00 00 00".into(),
        nif: "000016001234567".into(),
        rc: "16/00-1234567B21".into(),
        ..Default::default()
    }
}

fn sale(lines: usize) -> Sale {
    let l = |i: usize| Line {
        name: format!("حليب كامل الدسم Soummam 1L #{i}"),
        qty_milli: 1_250,
        unit: "kg".into(),
        unit_price_minor: 18_000,
        amount_minor: 22_500,
        discount_minor: 0,
    };
    let n = lines as i64;
    Sale {
        number: "FV-01-26-A000123".into(),
        when: (2026, 11, 12, 9, 30),
        cashier: "أحمد".into(),
        register: "الصندوق 1".into(),
        customer: Some(("شركة الأمل".into(), "NIF 000016009876543".into())),
        currency: Currency::DZD,
        lines: (0..lines).map(l).collect(),
        discount_minor: 0,
        net_minor: 18_908 * n,
        taxes: vec![(1_900, 18_908 * n, 3_592 * n)],
        stamp_minor: 0,
        total_minor: 22_500 * n,
        payments: vec![("cash".into(), 30_000 * n)],
        change_minor: 7_500 * n,
        owed_minor: 0,
        balance: Some((120_000, 120_000)),
        settled_minor: 0,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "out".into()));
    std::fs::create_dir_all(&dir)?;
    for (name, doc) in [
        (
            "receipt-ar-80",
            receipt(
                &shop(),
                &sale(4),
                Lang::Ar,
                Paper::ROLL80,
                &Options::default(),
            ),
        ),
        (
            "receipt-fr-58",
            receipt(
                &shop(),
                &sale(3),
                Lang::Fr,
                Paper::ROLL58,
                &Options::default(),
            ),
        ),
        (
            "invoice-ar-a4",
            invoice(&shop(), &sale(6), Lang::Ar, Paper::A4, &Options::default()),
        ),
        (
            "invoice-fr-a4",
            invoice(&shop(), &sale(6), Lang::Fr, Paper::A4, &Options::default()),
        ),
    ] {
        std::fs::write(dir.join(format!("{name}.png")), preview(&doc)?.0)?;
        std::fs::write(dir.join(format!("{name}.pdf")), pdf(&doc)?)?;
        if doc.paper.is_roll() {
            std::fs::write(dir.join(format!("{name}.bin")), escpos(&doc, false)?.bytes)?;
        }
    }
    Ok(())
}

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
    let names = [
        "حليب كامل الدسم Soummam 1L",
        "زيت المائدة Elio 5L",
        "سكر أبيض 1 كغ",
        "قهوة Facto 250g",
        "ياغورت Danone ×8",
        "دقيق ممتاز 10 كغ",
    ];
    let l = |i: usize| Line {
        name: names[i % names.len()].into(),
        qty_milli: 1_250,
        unit: "kg".into(),
        unit_price_minor: 18_000,
        amount_minor: 22_500,
        discount_minor: if i == 1 { 500 } else { 0 },
        sku: format!("61300{i:02}"),
        tax_bps: 1_900,
        net_minor: 18_908,
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
            "voucher-ar-80",
            nb_docs::templates::voucher(
                &shop(),
                &nb_docs::templates::Voucher {
                    number: "RC-01-26-A000007".into(),
                    when: (2026, 11, 12, 10, 5),
                    cashier: "عدلان".into(),
                    customer: ("أبو القاسم".into(), "NIF: 000016001234567".into()),
                    currency: nb_core::Currency::DZD,
                    method: "cash".into(),
                    amount_minor: 122_494_358,
                    reference: String::new(),
                    note: String::new(),
                    invoices: vec![("FV-01-26-A000123".into(), 100_000_000)],
                    installments: vec![("TQ-01-26-000002".into(), 3, 22_494_358)],
                    unallocated_minor: 0,
                },
                Lang::Ar,
                Paper::ROLL80,
                &Options::default(),
            ),
        ),
        (
            "schedule-ar-a4",
            nb_docs::templates::schedule(
                &shop(),
                &nb_docs::templates::Schedule {
                    number: "TQ-01-26-000002".into(),
                    version: 1,
                    when: (2026, 11, 12, 10, 5),
                    customer: ("أبو القاسم".into(), String::new()),
                    currency: nb_core::Currency::DZD,
                    sale_number: "FV-01-26-A000123".into(),
                    total_minor: 35_700_000,
                    down_minor: 5_700_000,
                    financed_minor: 30_000_000,
                    dues: (1..=6)
                        .map(|i| nb_docs::templates::Due {
                            seq: i,
                            date: (2027, i, 12),
                            amount_minor: 5_000_000,
                            paid_minor: if i < 3 { 5_000_000 } else { 0 },
                            state: if i < 3 {
                                "paid".into()
                            } else {
                                "upcoming".into()
                            },
                        })
                        .collect(),
                },
                Lang::Ar,
                Paper::A4,
                &Options::default(),
            ),
        ),
        (
            "invoice-classic-ar-a4",
            nb_docs::render::render(
                &nb_domain::template::builtin(nb_domain::template::DocKind::Invoice)[1].1,
                &nb_docs::templates::sale_data(
                    &shop(),
                    &sale(6),
                    Lang::Ar,
                    &Options::default(),
                    nb_domain::template::DocKind::Invoice,
                ),
                Lang::Ar,
                Paper::A4,
            ),
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

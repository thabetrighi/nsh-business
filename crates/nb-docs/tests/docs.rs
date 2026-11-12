//! Documents (M12): a receipt prints on both rolls as a raster the bytes
//! encode exactly; an invoice is a PDF and a preview; long invoices flow
//! onto more pages.

use nb_core::Currency;
use nb_docs::templates::{Line, Sale, Shop, invoice, receipt};
use nb_docs::{Paper, decode_escpos, escpos, pdf, preview};
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
    }
}

fn ink(rows: &[Vec<bool>]) -> usize {
    rows.iter().flatten().filter(|&&b| b).count()
}

#[test]
fn a_receipt_prints_on_both_rolls() -> Result<(), nb_docs::DocError> {
    for (paper, dots) in [(Paper::Roll58, 384), (Paper::Roll80, 576)] {
        let doc = receipt(&shop(), &sale(3), Lang::Ar, paper);
        assert!(
            doc.plain_text().contains("675.00\u{a0}دج"),
            "{}",
            doc.plain_text()
        );
        let out = escpos(&doc, true)?;
        let (w, rows) = decode_escpos(&out.bytes)?;
        assert_eq!((w, &rows), (dots, &out.rows), "the bytes print the picture");
        assert!(ink(&rows) > 2_000, "something is printed");
        assert!(
            out.bytes.ends_with(&[0x1B, 0x70, 0x00, 0x19, 0xFA]),
            "the drawer opens"
        );
        let (png, pw, _) = preview(&doc)?;
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        assert_eq!(pw as usize, dots);
    }
    // Without the drawer.
    let doc = receipt(&shop(), &sale(1), Lang::Fr, Paper::Roll80);
    assert!(!escpos(&doc, false)?.bytes.ends_with(&[0xFA]));
    // A sheet is not a roll.
    assert!(escpos(&invoice(&shop(), &sale(1), Lang::Fr, Paper::A4), false).is_err());
    Ok(())
}

#[test]
fn an_invoice_is_a_pdf_that_flows_onto_pages() -> Result<(), nb_docs::DocError> {
    let short = pdf(&invoice(&shop(), &sale(3), Lang::Ar, Paper::A4))?;
    assert!(short.starts_with(b"%PDF"));
    let long = pdf(&invoice(&shop(), &sale(120), Lang::Fr, Paper::A4))?;
    assert!(long.len() > short.len());
    let doc = invoice(&shop(), &sale(3), Lang::Fr, Paper::A5);
    let text = doc.plain_text();
    assert!(text.contains("FV-01-26-A000123"), "{text}");
    let (png, w, h) = preview(&doc)?;
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    assert!(w > 500 && h > 300, "{w}×{h}");
    // A roll as a PDF too (to keep or send).
    assert!(pdf(&receipt(&shop(), &sale(2), Lang::Ar, Paper::Roll80))?.starts_with(b"%PDF"));
    Ok(())
}

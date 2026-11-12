//! Documents (M12): a receipt prints on both rolls as a raster the bytes
//! encode exactly; an invoice is a PDF and a preview; long invoices flow
//! onto more pages.

use nb_core::Currency;
use nb_docs::templates::{Line, Options, Sale, Shop, invoice, receipt};
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
        balance: Some((120_000, 120_000)),
        settled_minor: 0,
    }
}

fn ink(rows: &[Vec<bool>]) -> usize {
    rows.iter().flatten().filter(|&&b| b).count()
}

#[test]
fn a_receipt_prints_on_both_rolls() -> Result<(), nb_docs::DocError> {
    for (paper, dots) in [(Paper::ROLL58, 384), (Paper::ROLL80, 576)] {
        let doc = receipt(&shop(), &sale(3), Lang::Ar, paper, &Options::default());
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
    let doc = receipt(
        &shop(),
        &sale(1),
        Lang::Fr,
        Paper::ROLL80,
        &Options::default(),
    );
    assert!(!escpos(&doc, false)?.bytes.ends_with(&[0xFA]));
    // A sheet is not a roll.
    assert!(
        escpos(
            &invoice(&shop(), &sale(1), Lang::Fr, Paper::A4, &Options::default()),
            false
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn an_invoice_is_a_pdf_that_flows_onto_pages() -> Result<(), nb_docs::DocError> {
    let short = pdf(&invoice(
        &shop(),
        &sale(3),
        Lang::Ar,
        Paper::A4,
        &Options::default(),
    ))?;
    assert!(short.starts_with(b"%PDF"));
    let long = pdf(&invoice(
        &shop(),
        &sale(120),
        Lang::Fr,
        Paper::A4,
        &Options::default(),
    ))?;
    assert!(long.len() > short.len());
    let doc = invoice(&shop(), &sale(3), Lang::Fr, Paper::A5, &Options::default());
    let text = doc.plain_text();
    assert!(text.contains("FV-01-26-A000123"), "{text}");
    let (png, w, h) = preview(&doc)?;
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    assert!(w > 500 && h > 300, "{w}×{h}");
    // A roll as a PDF too (to keep or send).
    assert!(
        pdf(&receipt(
            &shop(),
            &sale(2),
            Lang::Ar,
            Paper::ROLL80,
            &Options::default()
        ))?
        .starts_with(b"%PDF")
    );
    Ok(())
}

/// What the shop chose shows, and only that: the balance ("bon par bon"),
/// the count, the barcode scanned back to the number, the logo.
#[test]
fn options_decide_what_a_receipt_shows() -> Result<(), nb_docs::DocError> {
    let all = Options::default();
    let text = receipt(&shop(), &sale(2), Lang::Fr, Paper::ROLL80, &all).plain_text();
    for want in [
        "Ancien solde",
        "Nouveau solde",
        "Articles",
        "[FV-01-26-A000123]",
    ] {
        assert!(text.contains(want), "{want} in {text}");
    }
    let none = Options {
        balance: false,
        count: false,
        barcode: false,
        vat: false,
        cashier: false,
        table: false,
        legal_ids: false,
        ..Options::default()
    };
    let text = receipt(&shop(), &sale(2), Lang::Fr, Paper::ROLL80, &none).plain_text();
    for gone in ["Ancien solde", "Articles", "[FV-", "TVA", "Caissier", "NIF"] {
        assert!(!text.contains(gone), "{gone} in {text}");
    }
    // The barcode prints and reads back.
    let doc = receipt(&shop(), &sale(1), Lang::Ar, Paper::ROLL58, &all);
    let bars = doc.blocks.iter().find_map(|b| match b {
        nb_docs::Block::Bars { modules, .. } => Some(modules.clone()),
        _ => None,
    });
    assert_eq!(
        bars.as_deref()
            .and_then(nb_docs::barcode::decode128)
            .as_deref(),
        Some("FV-01-26-A000123")
    );
    // A logo: a dark square on white, made a grey PNG no wider than 576.
    let (w, h) = (800usize, 200usize);
    let px: Vec<u8> = (0..w * h)
        .map(|i| if (i % w) < 400 { 0 } else { 255 })
        .collect();
    let mut png = Vec::new();
    {
        let mut e = png::Encoder::new(&mut png, w as u32, h as u32);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e
            .write_header()
            .map_err(|e| nb_docs::DocError(e.to_string()))?;
        wr.write_image_data(&px)
            .map_err(|e| nb_docs::DocError(e.to_string()))?;
    }
    let kept = nb_docs::logo::normalize(&png)?;
    let g = nb_docs::logo::decode(&kept)?;
    assert_eq!((g.w, g.h), (576, 144));
    assert!(nb_docs::logo::decode(b"GIF89a").is_err());
    let with_logo = Options {
        logo: Some(g),
        ..Options::default()
    };
    let plain = escpos(
        &receipt(&shop(), &sale(1), Lang::Ar, Paper::ROLL80, &all),
        false,
    )?;
    let logo = escpos(
        &receipt(&shop(), &sale(1), Lang::Ar, Paper::ROLL80, &with_logo),
        false,
    )?;
    assert!(
        logo.rows.len() > plain.rows.len() + 50,
        "the logo takes room"
    );
    assert!(ink(&logo.rows) > ink(&plain.rows) + 5_000, "and prints");
    assert!(
        pdf(&invoice(&shop(), &sale(2), Lang::Ar, Paper::A4, &with_logo))?.starts_with(b"%PDF")
    );
    Ok(())
}

/// A shelf label on a 40 × 30 mm label printer, as TSPL.
#[test]
fn a_shelf_label_prints_as_tspl() -> Result<(), nb_docs::DocError> {
    use nb_docs::templates::{ShelfLabel, shelf_label};
    let paper = Paper::Label { w_mm: 40, h_mm: 30 };
    let l = ShelfLabel {
        name: "زيتون أسود فراك".into(),
        price_minor: 65_000,
        unit: "kg".into(),
        currency: Currency::DZD,
        bars: nb_docs::barcode::code128("2000122").map(|m| (m, "2000122".into())),
    };
    let doc = shelf_label(&l, Lang::Ar, paper, Some("ساسي"));
    let bytes = nb_docs::label::tspl(&doc, 3, 2)?;
    assert!(bytes.starts_with(b"SIZE 40 mm,30 mm\r\nGAP 2 mm,0 mm"));
    let (copies, rows) = nb_docs::label::decode_tspl(&bytes)?;
    assert_eq!(copies, 3);
    assert_eq!((rows.len(), rows[0].len()), (240, 320));
    assert_eq!(rows, nb_docs::label::label_rows(&doc)?);
    assert!(ink(&rows) > 3_000);
    let (_, pw, ph) = preview(&doc)?;
    assert_eq!((pw, ph), (320, 240));
    assert!(pdf(&doc)?.starts_with(b"%PDF"));
    Ok(())
}

/// A receipt voucher and an instalment schedule, on a roll and a sheet.
#[test]
fn vouchers_and_schedules() -> Result<(), nb_docs::DocError> {
    use nb_docs::templates::{Due, Schedule, Voucher, schedule, voucher};
    let v = Voucher {
        number: "RC-01-26-A000007".into(),
        when: (2026, 11, 12, 10, 5),
        cashier: "Adlane".into(),
        customer: ("Abou El Kacem".into(), "NIF: 1234".into()),
        currency: Currency::DZD,
        method: "cash".into(),
        amount_minor: 122_494_358,
        reference: String::new(),
        note: String::new(),
        invoices: vec![("FV-01-26-A000123".into(), 100_000_000)],
        installments: vec![("TQ-01-26-000002".into(), 3, 22_494_358)],
        unallocated_minor: 0,
    };
    let text = voucher(&shop(), &v, Lang::Fr, Paper::ROLL80, &Options::default()).plain_text();
    for want in [
        "Bon de versement",
        "Abou El Kacem",
        "FV-01-26-A000123",
        "échéance",
        "Signature",
    ] {
        assert!(text.contains(want), "{want} in {text}");
    }
    assert!(
        escpos(
            &voucher(&shop(), &v, Lang::Ar, Paper::ROLL58, &Options::default()),
            false
        )
        .is_ok()
    );
    let s = Schedule {
        number: "TQ-01-26-000002".into(),
        version: 1,
        when: (2026, 11, 12, 10, 5),
        customer: ("Abou El Kacem".into(), String::new()),
        currency: Currency::DZD,
        sale_number: "FV-01-26-A000123".into(),
        total_minor: 35_700,
        down_minor: 5_700,
        financed_minor: 30_000,
        dues: (1..=3)
            .map(|i| Due {
                seq: i,
                date: (2026, 11 + i, 12),
                amount_minor: 10_000,
                paid_minor: if i == 1 { 10_000 } else { 0 },
                state: if i == 1 {
                    "paid".into()
                } else {
                    "upcoming".into()
                },
            })
            .collect(),
    };
    let doc = schedule(&shop(), &s, Lang::Ar, Paper::A4, &Options::default());
    let text = doc.plain_text();
    for want in ["جدول الأقساط", "الباقي", "دفوع"] {
        assert!(text.contains(want), "{want} in {text}");
    }
    assert!(pdf(&doc)?.starts_with(b"%PDF"));
    assert!(
        preview(&schedule(
            &shop(),
            &s,
            Lang::Fr,
            Paper::ROLL80,
            &Options::default()
        ))
        .is_ok()
    );
    Ok(())
}

/// A report's table flows onto as many pages as it needs.
#[test]
fn a_long_report_takes_several_pages() -> Result<(), nb_docs::DocError> {
    use nb_docs::templates::report_table;
    let mut rows = vec![vec!["Produit".to_owned(), "Qté".into(), "Montant".into()]];
    for i in 0..120 {
        rows.push(vec![
            format!("Article {i}"),
            format!("{i}"),
            format!("{}.00 DA", i * 100),
        ]);
    }
    let doc = report_table(
        "Épicerie",
        "Ventes",
        "Du 2026-11-01 au 2026-11-30",
        &rows,
        Lang::Fr,
        Paper::A4,
    );
    let text = doc.plain_text();
    assert!(text.contains("Article 119") && text.contains("Montant"));
    assert!(pdf(&doc)?.starts_with(b"%PDF"));
    let pages = nb_docs::page_count(&doc);
    assert!(pages >= 3, "{pages}");
    assert!(
        report_table("", "Vide", "", &[], Lang::Ar, Paper::A4)
            .blocks
            .len()
            >= 2
    );
    Ok(())
}

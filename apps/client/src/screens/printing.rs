//! Printing (M12.1–M12.3): a sale's receipt on the thermal printer (ESC/POS
//! over the network, port 9100) or its invoice as a PDF, with a preview
//! of exactly what comes out. The documents are drawn by `nb-docs`, off
//! the interface's thread. Devices with files and sockets only (not the
//! browser).

use std::io::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use nb_docs::templates::{Line, Sale, Shop, invoice, receipt};
use nb_docs::{Doc, OPEN_DRAWER, Paper, escpos, pdf, preview};
use nb_i18n::{Lang, tr_args};
use nb_store::usecases::printing::{PrintSale, PrintShop, print_sale, print_shop};
use nsh_core::element::View;
use nsh_core::prelude::*;

use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::icon::Icon;
use crate::ui::kit::{Size, Variant, button, label, segmented};
use crate::ui::media::data_url;
use crate::ui::overlay::{dialog_frame, show_toast};
use crate::ui::tokens::{Tone, ty};

/// What a document is drawn from: the sale and the shop.
#[derive(Debug, Clone, PartialEq)]
pub struct Printable {
    sale: PrintSale,
    shop: PrintShop,
    currency: nb_core::Currency,
}

/// Which document, on which paper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Receipt(Paper),
    Invoice(Paper),
}

/// The person's printing settings, read once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub addr: Option<String>,
    pub roll: Paper,
    pub sheet: Paper,
    pub copies: usize,
    pub show_tax: bool,
    pub drawer: bool,
}

impl Setup {
    pub fn of(st: AppState) -> Self {
        let b = st.biz;
        let addr = b.printer_addr.get_untracked().trim().to_owned();
        Self {
            addr: (b.printer.get_untracked() > 0 && !addr.is_empty()).then_some(addr),
            roll: if b.receipt_width.get_untracked() == 0 {
                Paper::Roll58
            } else {
                Paper::Roll80
            },
            sheet: if b.invoice_paper.get_untracked() == 1 {
                Paper::A5
            } else {
                Paper::A4
            },
            copies: b.copies.get_untracked() + 1,
            show_tax: b.tax_on_receipt.get_untracked(),
            drawer: b.drawer.get_untracked(),
        }
    }
}

fn currency(st: AppState) -> nb_core::Currency {
    st.session
        .get_untracked()
        .map_or(nb_core::Currency::DZD, |s| s.currency)
}

/// Local date and time of `ms` in the store's time zone (UTC+1).
fn local(ms: i64) -> (i32, u32, u32, u32, u32) {
    let offset = 60;
    let (y, m, d) = nb_store::time::civil_date(ms, offset);
    let mins = (ms.div_euclid(60_000) + i64::from(offset)).rem_euclid(24 * 60);
    (y, m, d, (mins / 60) as u32, (mins % 60) as u32)
}

/// The document `kind` of `p`, in the shop's language.
pub fn document(p: &Printable, kind: Kind, show_tax: bool) -> Doc {
    let s = &p.sale;
    let pr = &p.shop.profile;
    let shop = Shop {
        name: p.shop.name.clone(),
        address: pr.address.clone(),
        phone: pr.phone.clone(),
        nif: pr.nif.clone(),
        nis: pr.nis.clone(),
        rc: pr.rc.clone(),
        ai: pr.ai.clone(),
        footer: String::new(),
    };
    let receipt_kind = matches!(kind, Kind::Receipt(_));
    let sale = Sale {
        number: s.number.clone(),
        when: local(s.created_at),
        cashier: s.cashier.clone(),
        register: s.register.clone(),
        customer: s.customer.clone(),
        currency: p.currency,
        lines: s
            .lines
            .iter()
            .map(|l| Line {
                name: l.name.clone(),
                qty_milli: l.qty_milli,
                unit: l.unit.clone(),
                unit_price_minor: l.unit_price_minor,
                amount_minor: l.amount_minor,
                discount_minor: l.discount_minor,
            })
            .collect(),
        discount_minor: s.discount_minor,
        net_minor: s.net_minor,
        // An invoice always states its VAT; a receipt as the shop chose.
        taxes: if show_tax || !receipt_kind {
            s.taxes.clone()
        } else {
            Vec::new()
        },
        stamp_minor: s.stamp_minor,
        total_minor: s.total_minor,
        payments: s.payments.clone(),
        change_minor: s.change_minor,
        owed_minor: s.owed_minor,
    };
    let lang = Lang::from_tag(&p.shop.language).unwrap_or(Lang::Ar);
    match kind {
        Kind::Receipt(paper) => receipt(&shop, &sale, lang, paper),
        Kind::Invoice(paper) => invoice(&shop, &sale, lang, paper),
    }
}

/// Runs `f` on a thread of its own and waits for it without blocking the
/// interface.
async fn off_thread<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        // The receiver is gone only if the screen closed: nothing to tell.
        let _ = tx.send(f());
    });
    rx.await.ok()
}

/// Sends `bytes` to the printer at `addr` (`host` or `host:port`).
pub fn send(addr: &str, bytes: &[u8]) -> std::io::Result<()> {
    let addr = if addr.contains(':') {
        addr.to_owned()
    } else {
        format!("{addr}:9100")
    };
    let to = addr
        .to_socket_addrs()?
        .next()
        .ok_or(std::io::ErrorKind::NotFound)?;
    let mut s = TcpStream::connect_timeout(&to, Duration::from_secs(3))?;
    s.set_write_timeout(Some(Duration::from_secs(15)))?;
    s.write_all(bytes)?;
    s.flush()
}

/// The bytes of `copies` receipts, the drawer opened after the first.
pub fn receipt_bytes(doc: &Doc, copies: usize, drawer: bool) -> Result<Vec<u8>, String> {
    let one = escpos(doc, false).map_err(|e| e.0)?.bytes;
    let mut out = Vec::with_capacity(one.len() * copies.max(1) + OPEN_DRAWER.len());
    for i in 0..copies.max(1) {
        out.extend_from_slice(&one);
        if i == 0 && drawer {
            out.extend_from_slice(&OPEN_DRAWER);
        }
    }
    Ok(out)
}

async fn load(st: AppState, number: String) -> Result<Printable, String> {
    let currency = currency(st);
    st.db()
        .call(move |s| Ok((print_sale(s, &number)?, print_shop(s)?)))
        .await
        .map(|(sale, shop)| Printable {
            sale,
            shop,
            currency,
        })
        .map_err(|e| error_text(st, e.codes.first().map_or("internal", |c| c.1)))
}

/// Prints the receipt of sale `number` on the configured printer, quietly
/// (after a sale); a failure is a toast. `drawer`: open the cash drawer.
pub fn print_receipt(st: AppState, number: String, drawer: bool) {
    let setup = Setup::of(st);
    let Some(addr) = setup.addr.clone() else {
        return;
    };
    // Under the root: the payment window that asked is already closing.
    let task = st.root.run(|| {
        spawn(async move {
            let r = match load(st, number).await {
                Ok(p) => off_thread(move || {
                    let doc = document(&p, Kind::Receipt(setup.roll), setup.show_tax);
                    let bytes = receipt_bytes(&doc, setup.copies, drawer && setup.drawer)?;
                    send(&addr, &bytes).map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|| Err(String::new())),
                Err(e) => Err(e),
            };
            if r.is_err() {
                show_toast(st, st.tn("print.failed"));
            }
        })
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

/// Prints a test receipt on the printer at `addr`.
pub fn test_print(st: AppState, addr: String, roll: Paper) {
    let lang = st.lang();
    let title = st.tn("print.test.title");
    let body = st.tn("print.test.body");
    let task = spawn(async move {
        let r = off_thread(move || {
            let doc = Doc {
                paper: roll,
                rtl: lang == Lang::Ar,
                blocks: vec![
                    nb_docs::Block::text(title, 14.0, true, nb_docs::Place::Center),
                    nb_docs::Block::Rule,
                    nb_docs::Block::text(body, 10.0, false, nb_docs::Place::Center),
                    nb_docs::Block::Gap(6.0),
                ],
            };
            let bytes = receipt_bytes(&doc, 1, false)?;
            send(&addr, &bytes).map_err(|e| e.to_string())
        })
        .await
        .unwrap_or_else(|| Err(String::new()));
        show_toast(
            st,
            st.tn(if r.is_ok() {
                "print.sent"
            } else {
                "print.failed"
            }),
        );
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
}

/// The print dialog of sale `number`: receipt or invoice, the preview,
/// print (receipts, on the configured printer) or save as PDF.
pub fn print_dialog(st: AppState, number: String, close: impl Fn() + Clone + 'static) -> View {
    let setup = Setup::of(st);
    // 0 = receipt, 1 = invoice.
    let which = signal(0usize);
    let sheet = signal(if setup.sheet == Paper::A5 { 1usize } else { 0 });
    let busy = signal(false);
    let roll = setup.roll;
    let kind = move || {
        if which.get() == 0 {
            Kind::Receipt(roll)
        } else if sheet.get() == 1 {
            Kind::Invoice(Paper::A5)
        } else {
            Kind::Invoice(Paper::A4)
        }
    };
    let data = resource(
        {
            let number = number.clone();
            move || Some(number.clone())
        },
        move |n, _| load(st, n),
    );
    let show_tax = setup.show_tax;
    let picture = resource(
        move || Some((data.get()?, kind())),
        move |(p, k), _| async move {
            off_thread(move || preview(&document(&p, k, show_tax)).map_err(|e| e.0))
                .await
                .unwrap_or_else(|| Err(String::new()))
        },
    );
    let shown = dynamic(move || {
        if let Some(e) = data.error() {
            return label(move || e.clone(), ty::SM, FontWeight::NORMAL, Tone::Danger)
                .into_element();
        }
        let Some((bytes, w, h)) = picture.get() else {
            return label(
                st.t("print.preparing"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .into_element();
        };
        // A roll at about its paper's width on screen; a sheet fits the dialog.
        let shown_w = if w <= 600 { 300.0 } else { 480.0 };
        let shown_h = shown_w * h as f32 / w.max(1) as f32;
        view()
            .items_center()
            .child(
                view()
                    .border_1()
                    .border_color(Tone::Border.color())
                    .bg(rgb(0xFFFFFF))
                    .child(
                        image(data_url("image/png", &bytes))
                            .w(px(shown_w))
                            .h(px(shown_h)),
                    ),
            )
            .test_id("print-preview")
            .into_element()
    });
    let choices: Vec<(usize, Box<dyn Fn() -> String>)> = vec![
        (0, Box::new(st.t("print.kind.receipt"))),
        (1, Box::new(st.t("print.kind.invoice"))),
    ];
    let body = view()
        .gap(px(16.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(12.0))
                .child(segmented(choices, which).test_id("print-kind"))
                .child(when(
                    move || which.get() == 1,
                    move || {
                        let sheets: Vec<(usize, Box<dyn Fn() -> String>)> = vec![
                            (0, Box::new(|| "A4".to_owned())),
                            (1, Box::new(|| "A5".to_owned())),
                        ];
                        segmented(sheets, sheet).test_id("print-sheet")
                    },
                )),
        )
        .child(shown);
    let save_pdf = {
        move || {
            let Some(p) = data.get() else {
                return;
            };
            if busy.get_untracked() {
                return;
            }
            let k = kind();
            let name = format!("{}.pdf", p.sale.number.replace(['/', '\\'], "-"));
            busy.set(true);
            let task = spawn(async move {
                let dialog = nsh_core::file_dialog::SaveDialog::new()
                    .file_name(name)
                    .title(st.tn("print.save_pdf"));
                let path = pick_save_path(dialog).await;
                let ok = match path {
                    Ok(Some(path)) => off_thread(move || {
                        pdf(&document(&p, k, show_tax))
                            .map_err(|e| e.0)
                            .and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()))
                    })
                    .await
                    .map(|r| r.is_ok()),
                    Ok(None) => None,
                    Err(_) => Some(false),
                };
                busy.set(false);
                match ok {
                    Some(true) => show_toast(st, st.tn("print.saved")),
                    Some(false) => show_toast(st, st.tn("print.failed")),
                    None => {}
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let print_now = {
        let close = close.clone();
        let setup = setup.clone();
        move || {
            let (Some(p), Some(addr)) = (data.get(), setup.addr.clone()) else {
                return;
            };
            if busy.get_untracked() {
                return;
            }
            busy.set(true);
            let copies = setup.copies;
            let close = close.clone();
            let task = spawn(async move {
                let r = off_thread(move || {
                    let doc = document(&p, Kind::Receipt(roll), show_tax);
                    let bytes = receipt_bytes(&doc, copies, false)?;
                    send(&addr, &bytes).map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|| Err(String::new()));
                busy.set(false);
                if r.is_ok() {
                    show_toast(st, st.tn("print.sent"));
                    close();
                } else {
                    show_toast(st, st.tn("print.failed"));
                }
            });
            drop(task); // runs to completion: tasks end with their owner, not their handle
        }
    };
    let has_printer = setup.addr.is_some();
    let footer = view()
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::FileText),
                st.t("print.save_pdf"),
                save_pdf,
            )
            .test_id("print-pdf"),
        )
        .child(when(
            move || has_printer && which.get() == 0,
            move || {
                button(
                    Variant::Primary,
                    Size::Md,
                    Some(Icon::Printer),
                    st.t("print.print"),
                    print_now.clone(),
                )
                .test_id("print-send")
            },
        ));
    let title_number = number.clone();
    dialog_frame(
        st,
        move || tr_args(st.lang(), "print.title", &[("number", &title_number)]),
        move || {
            if has_printer {
                st.tn("print.subtitle")
            } else {
                st.tn("print.subtitle.no_printer")
            }
        },
        body,
        footer,
        close,
    )
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::net::TcpListener;

    use nb_store::usecases::org::BusinessProfile;
    use nb_store::usecases::printing::PrintLine;

    use super::*;

    fn sample() -> Printable {
        Printable {
            sale: PrintSale {
                number: "FV-01-000042".into(),
                created_at: 1_762_950_000_000,
                cashier: "Amina".into(),
                register: "Caisse 1".into(),
                customer: None,
                lines: vec![PrintLine {
                    name: "حليب".into(),
                    unit: "u".into(),
                    qty_milli: 3_000,
                    unit_price_minor: 1_190,
                    discount_minor: 0,
                    amount_minor: 3_570,
                }],
                discount_minor: 0,
                net_minor: 3_000,
                taxes: vec![(1_900, 3_000, 570)],
                stamp_minor: 0,
                total_minor: 3_570,
                payments: vec![("cash".into(), 5_000)],
                change_minor: 1_430,
                owed_minor: 0,
            },
            shop: PrintShop {
                name: "Épicerie El Baraka".into(),
                profile: BusinessProfile::default(),
                language: "ar".into(),
            },
            currency: nb_core::Currency::DZD,
        }
    }

    #[test]
    fn the_receipt_reaches_the_printer_with_copies_and_the_drawer() {
        let doc = document(&sample(), Kind::Receipt(Paper::Roll80), true);
        assert!(doc.rtl);
        assert!(doc.plain_text().contains("FV-01-000042"));
        let one = escpos(&doc, false).map(|e| e.bytes).unwrap_or_default();
        let bytes = receipt_bytes(&doc, 2, true).unwrap_or_default();
        assert_eq!(bytes.len(), 2 * one.len() + OPEN_DRAWER.len());
        // The drawer opens once, after the first copy.
        assert_eq!(
            &bytes[one.len()..one.len() + OPEN_DRAWER.len()],
            &OPEN_DRAWER
        );
        assert!(nb_docs::decode_escpos(&bytes).is_ok());

        let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
            return; // no loopback here: nothing to try
        };
        let Ok(addr) = listener.local_addr() else {
            return;
        };
        let got = std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Ok((mut s, _)) = listener.accept() {
                let _ = s.read_to_end(&mut out);
            }
            out
        });
        assert!(send(&addr.to_string(), &bytes).is_ok());
        assert_eq!(got.join().unwrap_or_default(), bytes);
        // Nobody listening: an error, not a hang.
        assert!(send("127.0.0.1:1", &bytes).is_err());
    }

    #[test]
    fn an_invoice_states_its_vat_even_when_receipts_do_not() {
        let p = sample();
        let receipt = document(&p, Kind::Receipt(Paper::Roll58), false).plain_text();
        let invoice = document(&p, Kind::Invoice(Paper::A4), false).plain_text();
        assert!(!receipt.contains("19%"));
        assert!(invoice.contains("19%"));
        assert!(pdf(&document(&p, Kind::Invoice(Paper::A5), false)).is_ok());
    }
}

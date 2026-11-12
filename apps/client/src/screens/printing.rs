//! Printing (M12): a sale's receipt or invoice, a product's label, on the
//! printers of this device ([`crate::printers`]) — ESC/POS rolls, TSPL
//! labels, sheets through the system — or saved as PDF, with a preview of
//! exactly what comes out. Documents are drawn by `nb-docs` off the
//! interface's thread. Devices with files and sockets only (not the
//! browser).

use nb_docs::templates::{Line, Options, Sale, ShelfLabel, Shop, invoice, receipt, shelf_label};
use nb_docs::{Doc, OPEN_DRAWER, Paper, escpos_with, pdf, preview};
use nb_i18n::{Lang, tr_args};
use nb_store::usecases::printing::{
    DocSettings, PrintSale, PrintShop, document_settings, print_sale, print_shop,
};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::printers::{Link, Printer, Role, Sheet, for_role, send};
use crate::screens::forms::error_text;
use crate::state::AppState;
use crate::ui::form::{field, labelled};
use crate::ui::icon::Icon;
use crate::ui::kit::{Size, Variant, button, label, segmented};
use crate::ui::media::data_url;
use crate::ui::overlay::{dialog_frame, show_toast};
use crate::ui::popup::dropdown;
use crate::ui::tokens::{Tone, ty};

/// What a sale's documents are drawn from.
#[derive(Debug, Clone, PartialEq)]
pub struct Printable {
    sale: PrintSale,
    shop: PrintShop,
    docs: DocSettings,
    currency: nb_core::Currency,
}

/// Which document of a sale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Receipt,
    Invoice,
}

impl Kind {
    fn role(self) -> Role {
        match self {
            Kind::Receipt => Role::Receipt,
            Kind::Invoice => Role::Invoice,
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

/// The shop's choices as the templates take them (the logo decoded).
pub fn options(d: &DocSettings) -> Options {
    Options {
        logo: d
            .logo
            .as_deref()
            .and_then(|b| nb_docs::logo::decode(b).ok()),
        tagline: d.tagline.clone(),
        legal_ids: d.legal_ids,
        vat: d.vat,
        cashier: d.cashier,
        table: d.table,
        count: d.count,
        balance: d.balance,
        barcode: d.barcode,
        notice: d.notice.clone(),
    }
}

fn lang_of(shop: &PrintShop) -> Lang {
    Lang::from_tag(&shop.language).unwrap_or(Lang::Ar)
}

fn shop_of(p: &PrintShop, footer: &str) -> Shop {
    let pr = &p.profile;
    Shop {
        name: p.name.clone(),
        address: pr.address.clone(),
        phone: pr.phone.clone(),
        nif: pr.nif.clone(),
        nis: pr.nis.clone(),
        rc: pr.rc.clone(),
        ai: pr.ai.clone(),
        footer: footer.to_owned(),
    }
}

/// The document `kind` of `p` on `paper`, in the shop's language.
pub fn document(p: &Printable, kind: Kind, paper: Paper) -> Doc {
    let s = &p.sale;
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
        taxes: s.taxes.clone(),
        stamp_minor: s.stamp_minor,
        total_minor: s.total_minor,
        payments: s.payments.clone(),
        change_minor: s.change_minor,
        owed_minor: s.owed_minor,
        balance: s.balance,
        settled_minor: s.settled_minor,
    };
    let shop = shop_of(&p.shop, &p.docs.footer);
    let o = options(&p.docs);
    let lang = lang_of(&p.shop);
    match kind {
        Kind::Receipt => receipt(&shop, &sale, lang, paper, &o),
        Kind::Invoice => invoice(&shop, &sale, lang, paper, &o),
    }
}

/// The paper a document goes on with `printer` (or as a PDF alone).
pub fn paper_for(kind: Kind, printer: Option<&Printer>, sheet: Paper) -> Paper {
    match (kind, printer.map(|p| p.sheet)) {
        (_, Some(Sheet::Roll { print_mm, .. })) => Paper::roll_mm(print_mm),
        (_, Some(Sheet::A4)) => Paper::A4,
        (_, Some(Sheet::A5)) => Paper::A5,
        (Kind::Receipt, _) => Paper::ROLL80,
        (Kind::Invoice, _) => sheet,
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

/// The bytes of `copies` thermal printouts, the drawer opened after the
/// first.
pub fn thermal_bytes(
    printer: &Printer,
    doc: &Doc,
    copies: u8,
    drawer: bool,
) -> Result<Vec<u8>, String> {
    let copies = copies.max(1);
    if let Paper::Label { .. } = doc.paper
        && printer.tspl
    {
        let gap = match printer.sheet {
            Sheet::Label { gap_mm, .. } => gap_mm,
            _ => 2,
        };
        return nb_docs::label::tspl(doc, u32::from(copies), gap).map_err(|e| e.0);
    }
    let one = escpos_with(doc, false, printer.cut).map_err(|e| e.0)?.bytes;
    let mut out = Vec::with_capacity(one.len() * usize::from(copies) + OPEN_DRAWER.len());
    for i in 0..copies {
        out.extend_from_slice(&one);
        if i == 0 && drawer {
            out.extend_from_slice(&OPEN_DRAWER);
        }
    }
    Ok(out)
}

/// Prints `doc` on `printer` (on the calling thread: plain IO).
pub fn print_on(printer: &Printer, doc: &Doc, copies: u8, drawer: bool) -> Result<(), String> {
    let drawer = drawer && printer.drawer;
    match (&printer.link, doc.paper.is_thermal()) {
        (Link::Network { addr }, true) => {
            send::network(addr, &thermal_bytes(printer, doc, copies, drawer)?)
                .map_err(|e| e.to_string())
        }
        (Link::Device { path }, true) => {
            send::device(path, &thermal_bytes(printer, doc, copies, drawer)?)
                .map_err(|e| e.to_string())
        }
        (Link::System { queue }, _) => {
            let bytes = pdf(doc).map_err(|e| e.0)?;
            send::system(queue, &bytes, copies)
        }
        // A sheet sent raw: network printers that print PDF on port 9100.
        (Link::Network { addr }, false) => {
            let bytes = pdf(doc).map_err(|e| e.0)?;
            for _ in 0..copies.max(1) {
                send::network(addr, &bytes).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        (Link::Device { path }, false) => {
            let bytes = pdf(doc).map_err(|e| e.0)?;
            send::device(path, &bytes).map_err(|e| e.to_string())
        }
    }
}

async fn load(st: AppState, number: String) -> Result<Printable, String> {
    let currency = currency(st);
    st.db()
        .call(move |s| {
            Ok((
                print_sale(s, &number)?,
                print_shop(s)?,
                document_settings(s)?,
            ))
        })
        .await
        .map(|(sale, shop, docs)| Printable {
            sale,
            shop,
            docs,
            currency,
        })
        .map_err(|e| error_text(st, e.codes.first().map_or("internal", |c| c.1)))
}

/// Prints the receipt of sale `number` on the receipt printer, quietly
/// (after a sale); a failure is a toast. `cash`: the drawer may open.
pub fn print_receipt(st: AppState, number: String, cash: bool) {
    let Some(printer) = for_role(&st.biz.printers.get_untracked(), Role::Receipt) else {
        return;
    };
    // Under the root: the payment window that asked is already closing.
    let task = st.root.run(|| {
        spawn(async move {
            let r = match load(st, number).await {
                Ok(p) => off_thread(move || {
                    let doc = document(
                        &p,
                        Kind::Receipt,
                        paper_for(Kind::Receipt, Some(&printer), Paper::A4),
                    );
                    print_on(&printer, &doc, printer.copies, cash)
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

/// A short test page on `printer`.
pub fn test_print(st: AppState, printer: Printer) {
    let lang = st.lang();
    let title = format!("{} — {}", st.tn("print.test.title"), printer.name);
    let body = st.tn("print.test.body");
    let task = spawn(async move {
        let r = off_thread(move || {
            let paper = printer.sheet.paper();
            let mut blocks = vec![
                nb_docs::Block::text(title, 14.0, true, nb_docs::Place::Center),
                nb_docs::Block::Line,
                nb_docs::Block::text(body, 10.0, false, nb_docs::Place::Center),
            ];
            if let Some(m) = nb_docs::barcode::code128("NSH-TEST") {
                blocks.push(nb_docs::Block::Bars {
                    modules: m,
                    width: 140.0,
                    height: 24.0,
                    caption: Some("NSH-TEST".into()),
                });
            }
            let doc = Doc {
                paper,
                rtl: lang == Lang::Ar,
                blocks,
            };
            print_on(&printer, &doc, 1, false)
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

/// Saves `doc` as a PDF the person names.
fn save_pdf(
    st: AppState,
    name: String,
    doc: impl FnOnce() -> Doc + Send + 'static,
    busy: Signal<bool>,
) {
    if busy.get_untracked() {
        return;
    }
    busy.set(true);
    let task = spawn(async move {
        let dialog = nsh_core::file_dialog::SaveDialog::new()
            .file_name(name)
            .title(st.tn("print.save_pdf"));
        let ok = match pick_save_path(dialog).await {
            Ok(Some(path)) => off_thread(move || {
                pdf(&doc())
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

/// The preview of `picture` (PNG bytes and size), at a readable size.
fn picture_view(
    st: AppState,
    picture: Option<(Vec<u8>, u32, u32)>,
    error: Option<String>,
) -> AnyElement {
    if let Some(e) = error {
        return label(move || e.clone(), ty::SM, FontWeight::NORMAL, Tone::Danger).into_element();
    }
    let Some((bytes, w, h)) = picture else {
        return label(
            st.t("print.preparing"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        )
        .into_element();
    };
    // A roll about its paper's width on screen; a sheet fits the dialog.
    let shown_w = if w <= 600 {
        (w as f32 * 0.75).clamp(160.0, 300.0)
    } else {
        480.0
    };
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
}

/// The printers to choose from: index in the list, or `NONE` for a PDF
/// only.
const NONE: usize = usize::MAX;

fn printer_choices(st: AppState) -> Vec<(usize, String)> {
    let mut v: Vec<(usize, String)> = st
        .biz
        .printers
        .get()
        .iter()
        .enumerate()
        .filter(|(_, p)| p.ready())
        .map(|(i, p)| (i, p.name.clone()))
        .collect();
    v.push((NONE, st.tn("print.pdf_only")));
    v
}

fn default_printer(st: AppState, role: Role) -> usize {
    let list = st.biz.printers.get_untracked();
    list.iter()
        .position(|p| p.roles.contains(&role) && p.ready())
        .unwrap_or(NONE)
}

fn copies_of(text: &str) -> u8 {
    text.trim().parse::<u8>().unwrap_or(1).clamp(1, 20)
}

/// The print dialog of sale `number`: receipt or invoice, on which printer,
/// how many copies, the preview; print, or save as PDF.
pub fn print_dialog(st: AppState, number: String, close: impl Fn() + Clone + 'static) -> View {
    let kind = signal(Kind::Receipt);
    let chosen = signal(default_printer(st, Role::Receipt));
    let sheet = signal(0usize); // 0 = A4, 1 = A5 (a PDF alone)
    let busy = signal(false);
    let printer = move || {
        let i = chosen.get();
        st.biz.printers.get().get(i).cloned()
    };
    let copies = signal(printer().map_or(1, |p| p.copies).to_string());
    // Another document: its own printer and copies.
    effect(move || {
        let k = kind.get();
        chosen.set(default_printer(st, k.role()));
    });
    effect(move || {
        let n = printer().map_or(1, |p| p.copies);
        copies.set(n.to_string());
    });
    let paper = move || {
        let a = if sheet.get() == 1 {
            Paper::A5
        } else {
            Paper::A4
        };
        paper_for(kind.get(), printer().as_ref(), a)
    };
    let data = resource(
        {
            let number = number.clone();
            move || Some(number.clone())
        },
        move |n, _| load(st, n),
    );
    let picture = resource(
        move || Some((data.get()?, kind.get(), paper())),
        move |(p, k, paper), _| async move {
            off_thread(move || preview(&document(&p, k, paper)).map_err(|e| e.0))
                .await
                .unwrap_or_else(|| Err(String::new()))
        },
    );
    let shown = dynamic(move || picture_view(st, picture.get(), data.error()));
    let kinds: Vec<(Kind, Box<dyn Fn() -> String>)> = vec![
        (Kind::Receipt, Box::new(st.t("print.kind.receipt"))),
        (Kind::Invoice, Box::new(st.t("print.kind.invoice"))),
    ];
    let controls = view()
        .flex_row()
        .flex_wrap()
        .items_end()
        .gap(px(12.0))
        .child(labelled(
            st.t("print.document"),
            segmented(kinds, kind).test_id("print-kind"),
            None,
        ))
        .child(
            view().min_w(px(200.0)).child(labelled(
                st.t("print.printer_used"),
                dropdown(
                    move || printer_choices(st),
                    chosen,
                    Some(Icon::Printer),
                    st.t("print.printer_used"),
                    "print-printer",
                )
                .w_full(),
                None,
            )),
        )
        .child(view().w(px(90.0)).child(field(
            st.t("print.copies"),
            copies,
            || "1".to_owned(),
            String::new,
            KeyboardType::Number,
            "print-copies",
        )))
        .child(when(
            move || kind.get() == Kind::Invoice && printer().is_none(),
            move || {
                let sheets: Vec<(usize, Box<dyn Fn() -> String>)> = vec![
                    (0, Box::new(|| "A4".to_owned())),
                    (1, Box::new(|| "A5".to_owned())),
                ];
                labelled(
                    st.t("print.paper"),
                    segmented(sheets, sheet).test_id("print-sheet"),
                    None,
                )
            },
        ));
    let body = view().gap(px(16.0)).child(controls).child(shown);
    let on_pdf = move || {
        let Some(p) = data.get() else {
            return;
        };
        let (k, paper) = (kind.get_untracked(), paper());
        let name = format!("{}.pdf", p.sale.number.replace(['/', '\\'], "-"));
        save_pdf(st, name, move || document(&p, k, paper), busy);
    };
    let print_now = {
        let close = close.clone();
        move || {
            let (Some(p), Some(pr)) = (data.get(), printer()) else {
                return;
            };
            if busy.get_untracked() {
                return;
            }
            busy.set(true);
            let (k, paper, n) = (
                kind.get_untracked(),
                paper(),
                copies_of(&copies.get_untracked()),
            );
            let close = close.clone();
            let task = spawn(async move {
                let r = off_thread(move || print_on(&pr, &document(&p, k, paper), n, false))
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
    let footer = view()
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::FileText),
                st.t("print.save_pdf"),
                on_pdf,
            )
            .test_id("print-pdf"),
        )
        .child(when(
            move || printer().is_some(),
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
            if st.biz.printers.get().iter().any(Printer::ready) {
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

// ----------------------------------------------------------------- labels

/// A product as its label shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelItem {
    pub name: String,
    pub price_minor: i64,
    pub unit: String,
    pub barcode: Option<String>,
}

fn shelf(st: AppState, item: &LabelItem) -> ShelfLabel {
    let bars = item.barcode.as_ref().and_then(|code| {
        let m = if code.len() == 13 && code.bytes().all(|b| b.is_ascii_digit()) {
            nb_domain::barcode::ean13_modules(code).map(|m| {
                // Quiet zones around the bars.
                let mut v = vec![false; 9];
                v.extend(m);
                v.extend([false; 9]);
                v
            })
        } else {
            nb_docs::barcode::code128(code)
        };
        m.map(|m| (m, code.clone()))
    });
    ShelfLabel {
        name: item.name.clone(),
        price_minor: item.price_minor,
        unit: item.unit.clone(),
        currency: currency(st),
        bars,
    }
}

/// The label dialog of a product: which printer, how many, the preview.
pub fn label_dialog(st: AppState, item: LabelItem, close: impl Fn() + Clone + 'static) -> View {
    let chosen = signal(default_printer(st, Role::Label));
    let busy = signal(false);
    let count = signal("1".to_owned());
    let size = signal(0usize); // a PDF alone: 0 = 40×30, 1 = 50×30, 2 = 58×40
    let printer = move || st.biz.printers.get().get(chosen.get()).cloned();
    let lang = st.lang();
    let shop = String::new();
    let paper = move || match printer().map(|p| p.sheet) {
        Some(Sheet::Label { w_mm, h_mm, .. }) => Paper::Label { w_mm, h_mm },
        Some(Sheet::Roll { print_mm, .. }) => Paper::Label {
            w_mm: print_mm,
            h_mm: 30,
        },
        _ => match size.get() {
            1 => Paper::Label { w_mm: 50, h_mm: 30 },
            2 => Paper::Label { w_mm: 58, h_mm: 40 },
            _ => Paper::Label { w_mm: 40, h_mm: 30 },
        },
    };
    let label_doc = move |l: &ShelfLabel, paper: Paper, shop: &str| {
        shelf_label(l, lang, paper, (!shop.is_empty()).then_some(shop))
    };
    let l = shelf(st, &item);
    let picture = resource(
        {
            let l = l.clone();
            move || Some((l.clone(), paper()))
        },
        {
            let shop = shop.clone();
            move |(l, paper), _| {
                let shop = shop.clone();
                async move {
                    off_thread(move || preview(&label_doc(&l, paper, &shop)).map_err(|e| e.0))
                        .await
                        .unwrap_or_else(|| Err(String::new()))
                }
            }
        },
    );
    let shown = dynamic(move || picture_view(st, picture.get(), None));
    let body = view()
        .gap(px(16.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap(px(12.0))
                .child(
                    view().min_w(px(200.0)).child(labelled(
                        st.t("print.printer_used"),
                        dropdown(
                            move || printer_choices(st),
                            chosen,
                            Some(Icon::Printer),
                            st.t("print.printer_used"),
                            "label-printer",
                        )
                        .w_full(),
                        None,
                    )),
                )
                .child(view().w(px(90.0)).child(field(
                    st.t("print.labels"),
                    count,
                    || "1".to_owned(),
                    String::new,
                    KeyboardType::Number,
                    "label-count",
                )))
                .child(when(
                    move || printer().is_none(),
                    move || {
                        let sizes: Vec<(usize, Box<dyn Fn() -> String>)> = vec![
                            (0, Box::new(|| "40 × 30".to_owned())),
                            (1, Box::new(|| "50 × 30".to_owned())),
                            (2, Box::new(|| "58 × 40".to_owned())),
                        ];
                        labelled(st.t("print.paper"), segmented(sizes, size), None)
                    },
                )),
        )
        .child(shown);
    let on_pdf = {
        let (l, shop) = (l.clone(), shop.clone());
        let name = format!("{}.pdf", item.name.replace(['/', '\\'], "-"));
        move || {
            let (l, paper, shop) = (l.clone(), paper(), shop.clone());
            save_pdf(st, name.clone(), move || label_doc(&l, paper, &shop), busy);
        }
    };
    let print_now = {
        let close = close.clone();
        move || {
            let Some(pr) = printer() else {
                return;
            };
            if busy.get_untracked() {
                return;
            }
            busy.set(true);
            let (l, paper, n, shop) = (
                l.clone(),
                paper(),
                copies_of(&count.get_untracked()),
                shop.clone(),
            );
            let close = close.clone();
            let task = spawn(async move {
                let r = off_thread(move || print_on(&pr, &label_doc(&l, paper, &shop), n, false))
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
    let footer = view()
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::FileText),
                st.t("print.save_pdf"),
                on_pdf,
            )
            .test_id("label-pdf"),
        )
        .child(when(
            move || printer().is_some(),
            move || {
                button(
                    Variant::Primary,
                    Size::Md,
                    Some(Icon::Printer),
                    st.t("print.print"),
                    print_now.clone(),
                )
                .test_id("label-send")
            },
        ));
    let title = item.name.clone();
    dialog_frame(
        st,
        move || tr_args(st.lang(), "print.label_title", &[("name", &title)]),
        st.t("print.label_subtitle"),
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
                customer: Some(("Abou El Kacem".into(), String::new())),
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
                balance: Some((2_000, 2_000)),
                settled_minor: 0,
            },
            shop: PrintShop {
                name: "Épicerie El Baraka".into(),
                profile: BusinessProfile::default(),
                language: "ar".into(),
            },
            docs: DocSettings::default(),
            currency: nb_core::Currency::DZD,
        }
    }

    fn network_printer(addr: String) -> Printer {
        Printer {
            id: 1,
            name: "Caisse".into(),
            link: Link::Network { addr },
            drawer: true,
            ..Printer::default()
        }
    }

    #[test]
    fn the_receipt_reaches_the_printer_with_copies_and_the_drawer() {
        let doc = document(&sample(), Kind::Receipt, Paper::ROLL80);
        assert!(doc.rtl);
        assert!(doc.plain_text().contains("FV-01-000042"));
        let pr = network_printer(String::new());
        let one = escpos_with(&doc, false, true)
            .map(|e| e.bytes)
            .unwrap_or_default();
        let bytes = thermal_bytes(&pr, &doc, 2, true).unwrap_or_default();
        assert_eq!(bytes.len(), 2 * one.len() + OPEN_DRAWER.len());
        // The drawer opens once, after the first copy.
        assert_eq!(
            &bytes[one.len()..one.len() + OPEN_DRAWER.len()],
            &OPEN_DRAWER
        );
        assert!(nb_docs::decode_escpos(&bytes).is_ok());
        // No cutter: no cut.
        let uncut = Printer {
            cut: false,
            ..pr.clone()
        };
        assert_eq!(
            thermal_bytes(&uncut, &doc, 1, false)
                .map(|b| b.len())
                .unwrap_or(0),
            one.len() - 4
        );

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
        let pr = network_printer(addr.to_string());
        assert!(print_on(&pr, &doc, 2, true).is_ok());
        assert_eq!(got.join().unwrap_or_default(), bytes);
        // Nobody listening: an error, not a hang.
        assert!(print_on(&network_printer("127.0.0.1:1".into()), &doc, 1, false).is_err());
    }

    #[test]
    fn labels_go_as_tspl_to_a_device() {
        let dir = std::env::temp_dir().join(format!("nb-label-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("lp1");
        let _ = std::fs::write(&path, b"");
        let pr = Printer {
            link: Link::Device {
                path: path.display().to_string(),
            },
            sheet: Sheet::Label {
                w_mm: 40,
                h_mm: 30,
                gap_mm: 2,
            },
            roles: vec![Role::Label],
            ..Printer::default()
        };
        let doc = shelf_label(
            &ShelfLabel {
                name: "Olive noir".into(),
                price_minor: 65_000,
                unit: "kg".into(),
                currency: nb_core::Currency::DZD,
                bars: nb_domain::barcode::ean13_modules("2000122003109")
                    .map(|m| (m, "2000122003109".into())),
            },
            Lang::Fr,
            pr.sheet.paper(),
            None,
        );
        assert!(print_on(&pr, &doc, 3, false).is_ok());
        let sent = std::fs::read(&path).unwrap_or_default();
        let (copies, rows) = nb_docs::label::decode_tspl(&sent).unwrap_or_default();
        assert_eq!((copies, rows.len()), (3, 240));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_invoice_states_its_vat_even_when_receipts_do_not() {
        let mut p = sample();
        p.docs.vat = false;
        let receipt = document(&p, Kind::Receipt, Paper::ROLL58).plain_text();
        let invoice = document(&p, Kind::Invoice, Paper::A4).plain_text();
        assert!(!receipt.contains("19%"));
        assert!(invoice.contains("19%"));
        assert!(pdf(&document(&p, Kind::Invoice, Paper::A5)).is_ok());
        // Paper: the printer's, else a roll or the sheet chosen.
        let roll58 = Printer {
            sheet: Sheet::Roll {
                paper_mm: 58,
                print_mm: 48,
            },
            ..Printer::default()
        };
        assert_eq!(
            paper_for(Kind::Invoice, Some(&roll58), Paper::A4),
            Paper::ROLL58
        );
        assert_eq!(paper_for(Kind::Invoice, None, Paper::A5), Paper::A5);
        assert_eq!(paper_for(Kind::Receipt, None, Paper::A4), Paper::ROLL80);
    }
}

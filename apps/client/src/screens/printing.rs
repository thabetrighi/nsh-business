//! Printing (M12): a sale's receipt or invoice, a product's label, on the
//! printers of this device ([`crate::printers`]) — ESC/POS rolls, TSPL
//! labels, sheets through the system — or saved as PDF, with a preview of
//! exactly what comes out. Documents are drawn by `nb-docs` off the
//! interface's thread. Devices with files and sockets only (not the
//! browser).

use nb_docs::templates::{Line, Options, Sale, ShelfLabel, Shop, apply, label_data, sale_data};
use nb_docs::{Doc, LabelSheet, OPEN_DRAWER, Paper, escpos_with, label_sheet_pdf, pdf, preview};
use nb_domain::template::{DocKind, Template, builtin};
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
    /// The layouts the shop chose for receipts and invoices.
    receipt_t: Template,
    invoice_t: Template,
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

pub fn shop_of(p: &PrintShop, footer: &str) -> Shop {
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
                sku: l.sku.clone(),
                tax_bps: l.tax_bps,
                net_minor: l.net_minor,
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
    let (dk, t) = match kind {
        Kind::Receipt => (DocKind::Receipt, &p.receipt_t),
        Kind::Invoice => (DocKind::Invoice, &p.invoice_t),
    };
    draw(t, &o, &sale_data(&shop, &sale, lang, &o, dk), lang, paper)
}

/// A document with the shop's layout, its switches applied.
pub fn draw(
    t: &Template,
    o: &Options,
    d: &nb_docs::render::DocData,
    lang: Lang,
    paper: Paper,
) -> Doc {
    nb_docs::render::render(&apply(t, o), d, lang, paper)
}

/// The layout a kind of document prints with (the built-in one if the
/// shop's cannot be read).
pub fn layout(s: &nb_store::Store, kind: DocKind) -> Template {
    nb_store::usecases::templates::template_for(s, kind)
        .unwrap_or_else(|_| nb_domain::template::usable(kind, None))
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
pub(crate) async fn off_thread<R: Send + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> Option<R> {
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
                (layout(s, DocKind::Receipt), layout(s, DocKind::Invoice)),
            ))
        })
        .await
        .map(|(sale, shop, docs, (receipt_t, invoice_t))| Printable {
            sale,
            shop,
            docs,
            currency,
            receipt_t,
            invoice_t,
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
pub(crate) fn picture_view(
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

// ------------------------------------------------- vouchers, schedules

/// Draws a document on a paper (made off the interface's thread).
pub type Maker = std::sync::Arc<dyn Fn(Paper) -> Doc + Send + Sync>;

async fn shop_and_docs(
    st: AppState,
    kind: DocKind,
) -> Result<(PrintShop, DocSettings, Template), String> {
    st.db()
        .call(move |s| Ok((print_shop(s)?, document_settings(s)?, layout(s, kind))))
        .await
        .map_err(|e| error_text(st, e.codes.first().map_or("internal", |c| c.1)))
}

fn local_date(iso: &str) -> (i32, u32, u32) {
    let mut it = iso.split('-').map(|p| p.parse::<u32>().unwrap_or(0));
    let y = it.next().unwrap_or(0);
    (
        i32::try_from(y).unwrap_or(0),
        it.next().unwrap_or(1),
        it.next().unwrap_or(1),
    )
}

/// The time now in the store's time zone.
pub fn now_local() -> (i32, u32, u32, u32, u32) {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
    local(ms)
}

/// A cart printed before it is paid, with the shop's receipt layout.
pub async fn prebill_maker(st: AppState, sale: Sale) -> Result<Maker, String> {
    let (shop, docs, t) = shop_and_docs(st, DocKind::Receipt).await?;
    let (sh, o, lang) = (shop_of(&shop, &docs.footer), options(&docs), lang_of(&shop));
    // No customer balance on a cart not yet sold.
    let o = Options {
        balance: false,
        ..o
    };
    Ok(std::sync::Arc::new(move |paper| {
        draw(
            &t,
            &o,
            &sale_data(&sh, &sale, lang, &o, DocKind::Receipt),
            lang,
            paper,
        )
    }))
}

/// A customer receipt's voucher.
pub async fn voucher_maker(st: AppState, number: String) -> Result<Maker, String> {
    use nb_docs::templates::{Voucher, voucher_data};
    let (shop, docs, t) = shop_and_docs(st, DocKind::Voucher).await?;
    let v = st
        .db()
        .call(move |s| nb_store::usecases::printing::print_voucher(s, &number))
        .await
        .map_err(|e| error_text(st, e.codes.first().map_or("internal", |c| c.1)))?;
    let v = Voucher {
        number: v.number,
        when: local(v.created_at),
        cashier: v.cashier,
        customer: v.customer,
        currency: currency(st),
        method: v.method,
        amount_minor: v.amount_minor,
        reference: v.reference,
        note: v.note,
        invoices: v.invoices,
        installments: v.installments,
        unallocated_minor: v.unallocated_minor,
    };
    let (sh, o, lang) = (shop_of(&shop, &docs.footer), options(&docs), lang_of(&shop));
    Ok(std::sync::Arc::new(move |paper| {
        draw(&t, &o, &voucher_data(&sh, &v, lang, &o), lang, paper)
    }))
}

/// An instalment plan's schedule.
pub async fn schedule_maker(
    st: AppState,
    plan: nb_store::usecases::installments::Plan,
) -> Result<Maker, String> {
    use nb_docs::templates::{Due, Schedule, schedule_data};
    let (shop, docs, t) = shop_and_docs(st, DocKind::Schedule).await?;
    let cid = plan.customer_id.clone();
    let ids = st
        .db()
        .call(move |s| nb_store::usecases::customers::get_customer(s, &cid))
        .await
        .map(|c| {
            [("NIF", c.nif), ("RC", c.rc)]
                .into_iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default();
    let s = Schedule {
        number: plan.number.clone(),
        version: plan.version,
        when: local(plan.created_at),
        customer: (plan.customer_name.clone(), ids),
        currency: currency(st),
        sale_number: plan.sale_number.clone(),
        total_minor: plan.total_minor,
        down_minor: plan.down_minor,
        financed_minor: plan.financed_minor,
        dues: plan
            .installments
            .iter()
            .map(|i| Due {
                seq: i.seq,
                date: local_date(&i.due_date),
                amount_minor: i.amount_minor,
                paid_minor: i.paid_minor,
                state: i.state.clone(),
            })
            .collect(),
    };
    let (sh, o, lang) = (shop_of(&shop, &docs.footer), options(&docs), lang_of(&shop));
    Ok(std::sync::Arc::new(move |paper| {
        draw(&t, &o, &schedule_data(&sh, &s, lang, &o), lang, paper)
    }))
}

/// A document's print dialog: which printer, copies, the preview; print,
/// or save as PDF. Without a printer: a sheet (A4/A5) or a roll.
pub fn document_dialog<F, Fut>(
    st: AppState,
    title: String,
    file_name: String,
    role: Role,
    load: F,
    close: impl Fn() + Clone + 'static,
) -> View
where
    F: Fn() -> Fut + 'static,
    Fut: std::future::Future<Output = Result<Maker, String>> + 'static,
{
    let chosen = signal(default_printer(st, role));
    let busy = signal(false);
    let printer = move || st.biz.printers.get().get(chosen.get()).cloned();
    let copies = signal(printer().map_or(1, |p| p.copies).to_string());
    let sheet = signal(0usize); // a PDF alone: 0 = A4, 1 = A5, 2 = 80 mm
    let paper = move || match printer().map(|p| p.sheet) {
        Some(Sheet::Roll { print_mm, .. }) => Paper::roll_mm(print_mm),
        Some(Sheet::A5) => Paper::A5,
        Some(Sheet::A4) => Paper::A4,
        _ => match sheet.get() {
            1 => Paper::A5,
            2 => Paper::ROLL80,
            _ => Paper::A4,
        },
    };
    let made = resource(move || Some(()), move |_, _| load());
    let picture = resource(
        move || Some((made.get()?, paper())),
        move |(m, paper): (Maker, Paper), _| async move {
            off_thread(move || preview(&m(paper)).map_err(|e| e.0))
                .await
                .unwrap_or_else(|| Err(String::new()))
        },
    );
    let shown = dynamic(move || picture_view(st, picture.get(), made.error()));
    let controls = view()
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
                    "doc-printer",
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
            "doc-copies",
        )))
        .child(when(
            move || printer().is_none(),
            move || {
                let sheets: Vec<(usize, Box<dyn Fn() -> String>)> = vec![
                    (0, Box::new(|| "A4".to_owned())),
                    (1, Box::new(|| "A5".to_owned())),
                    (2, Box::new(|| "80 mm".to_owned())),
                ];
                labelled(
                    st.t("print.paper"),
                    segmented(sheets, sheet).test_id("doc-sheet"),
                    None,
                )
            },
        ));
    let on_pdf = move || {
        let Some(m) = made.get() else {
            return;
        };
        let paper = paper();
        save_pdf(st, file_name.clone(), move || m(paper), busy);
    };
    let print_now = {
        let close = close.clone();
        move || {
            let (Some(m), Some(pr)) = (made.get(), printer()) else {
                return;
            };
            if busy.get_untracked() {
                return;
            }
            busy.set(true);
            let (paper, n) = (paper(), copies_of(&copies.get_untracked()));
            let close = close.clone();
            let task = spawn(async move {
                let r = off_thread(move || print_on(&pr, &m(paper), n, false))
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
            .test_id("doc-pdf"),
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
                .test_id("doc-send")
            },
        ));
    dialog_frame(
        st,
        move || title.clone(),
        st.t("print.subtitle"),
        view().gap(px(16.0)).child(controls).child(shown),
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
    pub sku: String,
}

fn shelf(st: AppState, item: &LabelItem) -> ShelfLabel {
    ShelfLabel {
        name: item.name.clone(),
        price_minor: item.price_minor,
        unit: item.unit.clone(),
        currency: currency(st),
        barcode: item.barcode.clone(),
        sku: item.sku.clone(),
    }
}

/// Single labels (label printers, or a PDF cut to size): width × height.
pub const SINGLES: [(u16, u16); 16] = [
    (25, 15),
    (30, 20),
    (32, 25),
    (38, 25),
    (40, 20),
    (40, 30),
    (50, 25),
    (50, 30),
    (57, 32),
    (58, 40),
    (60, 40),
    (70, 50),
    (100, 50),
    (100, 70),
    (100, 100),
    (100, 150),
];

/// A4 sheets of labels (office printers): the common Avery/Tico layouts.
pub fn sheets() -> [LabelSheet; 8] {
    let s = |cols, rows, w_mm, h_mm, top_mm, left_mm, gap_x_mm| LabelSheet {
        cols,
        rows,
        w_mm,
        h_mm,
        top_mm,
        left_mm,
        gap_x_mm,
        gap_y_mm: 0.0,
    };
    [
        s(2, 4, 105.0, 74.25, 0.0, 0.0, 0.0),
        s(2, 7, 99.1, 38.1, 15.15, 4.65, 2.5),
        s(3, 7, 63.5, 38.1, 15.15, 7.2, 2.5),
        s(3, 8, 70.0, 37.0, 0.5, 0.0, 0.0),
        s(3, 10, 70.0, 29.7, 0.0, 0.0, 0.0),
        s(4, 10, 52.5, 29.7, 0.0, 0.0, 0.0),
        s(4, 16, 48.5, 16.9, 13.0, 8.0, 0.0),
        s(5, 13, 38.1, 21.2, 10.7, 4.75, 2.5),
    ]
}

/// The label stock chosen: a single label, a sheet, or a size typed in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stock {
    Single(u16, u16),
    Sheet(LabelSheet),
}

/// The choice "a size of my own" (after the singles and the sheets).
pub const CUSTOM_STOCK: usize = SINGLES.len() + 8;

/// The stock at `at` in the list (custom: the typed size).
pub fn stock_at(at: usize, custom: (u16, u16)) -> Stock {
    if let Some(&(w, h)) = SINGLES.get(at) {
        return Stock::Single(w, h);
    }
    match sheets().get(at.wrapping_sub(SINGLES.len())) {
        Some(sh) => Stock::Sheet(*sh),
        None => Stock::Single(custom.0.clamp(15, 150), custom.1.clamp(10, 200)),
    }
}

/// The paper one label is drawn on.
pub fn stock_paper(stock: Stock) -> Paper {
    match stock {
        Stock::Single(w_mm, h_mm) => Paper::Label { w_mm, h_mm },
        Stock::Sheet(sh) => Paper::Label {
            w_mm: sh.w_mm as u16,
            h_mm: sh.h_mm as u16,
        },
    }
}

fn mm(v: f32) -> String {
    if v.fract().abs() < 0.01 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// What a label shows (each can be left out).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    pub name: bool,
    pub price: bool,
    pub barcode: bool,
    pub sku: bool,
    pub shop: bool,
}

/// The shop's label layout with only what is wanted: the parts left out
/// removed, the code and the shop's name added if wanted and missing.
pub fn label_template(t: &Template, shown: Shown) -> Template {
    use nb_domain::template::{Align, Section};
    let mut t = t.clone();
    let uses = |t: &Template, var: &str| {
        t.sections.iter().any(|s| match s {
            Section::Text { text, .. } => text.contains(var),
            Section::Barcode { value } => value.contains(var),
            _ => false,
        })
    };
    let gone = |text: &str| {
        (!shown.name && text.contains("{item.name}"))
            || (!shown.price && text.contains("{item.price}"))
            || (!shown.barcode && text.contains("{item.barcode}"))
            || (!shown.sku && text.contains("{item.sku}"))
            || (!shown.shop && text.contains("{shop.name}"))
    };
    t.sections.retain(|s| match s {
        Section::Text { text, .. } => !gone(text),
        Section::Barcode { value } => !gone(value),
        _ => true,
    });
    if shown.shop && !uses(&t, "{shop.name}") {
        t.sections.insert(
            0,
            Section::Text {
                text: "{shop.name}".into(),
                align: Align::Center,
                size: -1,
                bold: false,
            },
        );
    }
    if shown.sku && !uses(&t, "{item.sku}") {
        t.sections.push(Section::Text {
            text: "{item.sku}".into(),
            align: Align::Center,
            size: -2,
            bold: false,
        });
    }
    t
}

/// The places of a started sheet already used.
fn skipped(text: &str) -> usize {
    text.trim().parse::<usize>().unwrap_or(0).min(200)
}

fn labels_of(text: &str) -> usize {
    text.trim().parse::<usize>().unwrap_or(1).clamp(1, 999)
}

/// Saves the bytes `make` gives as a PDF the person names.
fn save_bytes(
    st: AppState,
    name: String,
    make: impl FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
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
                make().and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()))
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

/// A PDF (a sheet of labels) sent to an office printer.
fn send_pdf(printer: &Printer, bytes: &[u8]) -> Result<(), String> {
    match &printer.link {
        Link::System { queue } => send::system(queue, bytes, 1),
        Link::Network { addr } => send::network(addr, bytes).map_err(|e| e.to_string()),
        Link::Device { path } => send::device(path, bytes).map_err(|e| e.to_string()),
    }
}

/// The label dialog of a product: which printer, which labels (a roll's
/// size, an A4 sheet, a size of one's own), how many, what they show, the
/// preview.
pub fn label_dialog(st: AppState, item: LabelItem, close: impl Fn() + Clone + 'static) -> View {
    use crate::state::{persisted_flag, persisted_index, persisted_text};
    use crate::ui::kit::checkbox;
    let chosen = signal(default_printer(st, Role::Label));
    let busy = signal(false);
    let count = signal("1".to_owned());
    let skip = signal("0".to_owned());
    // The last stock and contents chosen are kept on the device.
    let at = persisted_index("nb.label.stock", 5);
    let custom_w = persisted_text("nb.label.custom_w");
    let custom_h = persisted_text("nb.label.custom_h");
    let show_name = persisted_flag("nb.label.name", true);
    let show_price = persisted_flag("nb.label.price", true);
    let show_barcode = persisted_flag("nb.label.barcode", true);
    let show_sku = persisted_flag("nb.label.sku", false);
    let show_shop = persisted_flag("nb.label.shop", false);
    let shown = move || Shown {
        name: show_name.get(),
        price: show_price.get(),
        barcode: show_barcode.get(),
        sku: show_sku.get(),
        shop: show_shop.get(),
    };
    let printer = move || st.biz.printers.get().get(chosen.get()).cloned();
    let lang = st.lang();
    let custom = move || {
        let n = |s: Signal<String>, d| s.get().trim().parse::<u16>().unwrap_or(d);
        (n(custom_w, 50), n(custom_h, 30))
    };
    // A label printer fixes the size; otherwise the stock chosen.
    let stock = move || match printer().map(|p| p.sheet) {
        Some(Sheet::Label { w_mm, h_mm, .. }) => Stock::Single(w_mm, h_mm),
        Some(Sheet::Roll { print_mm, .. }) => Stock::Single(print_mm, 30),
        _ => stock_at(at.get(), custom()),
    };
    let fixed = move || {
        matches!(
            printer().map(|p| p.sheet),
            Some(Sheet::Label { .. } | Sheet::Roll { .. })
        )
    };
    let label_doc = move |t: &Template, l: &ShelfLabel, paper: Paper, shop: &str| {
        let d = label_data(l, lang, (!shop.is_empty()).then_some(shop));
        draw(t, &Options::default(), &d, lang, paper)
    };
    // The shop's label layout and name, once read (the built-in one until
    // then).
    let tpl = signal(builtin(DocKind::Label).remove(0).1);
    let shop = signal(String::new());
    let task = spawn(async move {
        if let Ok((t, name)) = st
            .db()
            .call(|s| {
                Ok((
                    layout(s, DocKind::Label),
                    print_shop(s).map(|p| p.name).unwrap_or_default(),
                ))
            })
            .await
        {
            tpl.set(t);
            shop.set(name);
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
    let l = shelf(st, &item);
    let picture = resource(
        {
            let l = l.clone();
            move || {
                Some((
                    l.clone(),
                    stock_paper(stock()),
                    label_template(&tpl.get(), shown()),
                    shop.get(),
                ))
            }
        },
        move |(l, paper, t, shop), _| async move {
            off_thread(move || preview(&label_doc(&t, &l, paper, &shop)).map_err(|e| e.0))
                .await
                .unwrap_or_else(|| Err(String::new()))
        },
    );
    let size_choices = move || {
        let mut v: Vec<(usize, String)> = SINGLES
            .iter()
            .enumerate()
            .map(|(i, (w, h))| (i, format!("{w} × {h} mm")))
            .collect();
        for (i, sh) in sheets().iter().enumerate() {
            v.push((
                SINGLES.len() + i,
                format!(
                    "A4 · {} × {} — {} × {} mm",
                    sh.cols,
                    sh.rows,
                    mm(sh.w_mm),
                    mm(sh.h_mm)
                ),
            ));
        }
        v.push((CUSTOM_STOCK, st.tn("print.label_custom")));
        v
    };
    let is_sheet = move || matches!(stock(), Stock::Sheet(_));
    let tick = move |key: &'static str, on: Signal<bool>, id: &'static str| {
        view()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .cursor_pointer()
            .test_id(id)
            .on_press(move || on.update(|v| *v = !*v))
            .child(checkbox(
                move || Some(on.get()),
                move || on.update(|v| *v = !*v),
            ))
            .child(label(st.t(key), ty::SM, FontWeight::NORMAL, Tone::Ink))
    };
    let summary = move || match stock() {
        Stock::Sheet(sh) => {
            let used = skipped(&skip.get()).min(sh.per_page() - 1);
            let n = labels_of(&count.get()) + used - 1;
            tr_args(
                st.lang(),
                "print.label_sheets",
                &[
                    ("per", &sh.per_page().to_string()),
                    ("pages", &(n / sh.per_page() + 1).to_string()),
                ],
            )
        }
        Stock::Single(w, h) => tr_args(
            st.lang(),
            "print.label_single",
            &[(
                "size",
                &nb_i18n::bidi::isolate_ltr(&format!("{w} × {h} mm")),
            )],
        ),
    };
    let body = view()
        .gap(px(16.0))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap(px(12.0))
                .child(
                    view().min_w(px(200.0)).flex_1().child(labelled(
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
                .child(view().min_w(px(220.0)).flex_1().child(dynamic(move || {
                    if fixed() {
                        let Stock::Single(w, h) = stock() else {
                            return view().into_element();
                        };
                        return labelled(
                            st.t("print.paper"),
                            view().h(px(36.0)).justify_center().child(label(
                                move || format!("{w} × {h} mm"),
                                ty::SM,
                                FontWeight::MEDIUM,
                                Tone::Body,
                            )),
                            Some(Box::new(st.t("print.label_from_printer"))),
                        )
                        .into_element();
                    }
                    labelled(
                        st.t("print.paper"),
                        dropdown(
                            size_choices,
                            at,
                            Some(Icon::Tag),
                            st.t("print.paper"),
                            "label-size",
                        )
                        .w_full(),
                        None,
                    )
                    .into_element()
                }))),
        )
        .child(when(
            move || !fixed() && at.get() == CUSTOM_STOCK,
            move || {
                view()
                    .flex_row()
                    .gap(px(12.0))
                    .child(view().w(px(110.0)).child(field(
                        st.t("printers.label_w"),
                        custom_w,
                        || "50".to_owned(),
                        String::new,
                        KeyboardType::Number,
                        "label-custom-w",
                    )))
                    .child(view().w(px(110.0)).child(field(
                        st.t("printers.label_h"),
                        custom_h,
                        || "30".to_owned(),
                        String::new,
                        KeyboardType::Number,
                        "label-custom-h",
                    )))
            },
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap(px(12.0))
                .child(view().w(px(110.0)).child(field(
                    st.t("print.labels"),
                    count,
                    || "1".to_owned(),
                    String::new,
                    KeyboardType::Number,
                    "label-count",
                )))
                .child(when(is_sheet, move || {
                    view().w(px(150.0)).child(field(
                        st.t("print.label_skip"),
                        skip,
                        || "0".to_owned(),
                        String::new,
                        KeyboardType::Number,
                        "label-skip",
                    ))
                })),
        )
        .child(labelled(
            st.t("print.label_shows"),
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(16.0))
                .child(tick("print.show_name", show_name, "label-show-name"))
                .child(tick("print.show_price", show_price, "label-show-price"))
                .child(tick(
                    "print.show_barcode",
                    show_barcode,
                    "label-show-barcode",
                ))
                .child(tick("print.show_sku", show_sku, "label-show-sku"))
                .child(tick("print.show_shop", show_shop, "label-show-shop")),
            None,
        ))
        .child(
            view()
                .gap(px(8.0))
                .child(
                    label(summary, ty::SM, FontWeight::MEDIUM, Tone::Muted)
                        .test_id("label-summary"),
                )
                .child(dynamic(move || picture_view(st, picture.get(), None))),
        );
    // The labels as a PDF: one label a page, or the sheets.
    let make = {
        let l = l.clone();
        move || {
            let (l, stock, t, shop) = (
                l.clone(),
                stock(),
                label_template(&tpl.get_untracked(), shown()),
                shop.get_untracked(),
            );
            let (n, skip) = (
                labels_of(&count.get_untracked()),
                skipped(&skip.get_untracked()),
            );
            move || -> Result<Vec<u8>, String> {
                let doc = label_doc(&t, &l, stock_paper(stock), &shop);
                match stock {
                    Stock::Sheet(sh) => label_sheet_pdf(&doc, sh, n, skip).map_err(|e| e.0),
                    Stock::Single(..) => pdf(&doc).map_err(|e| e.0),
                }
            }
        }
    };
    let on_pdf = {
        let make = make.clone();
        let name = format!("{}.pdf", item.name.replace(['/', '\\'], "-"));
        move || save_bytes(st, name.clone(), make(), busy)
    };
    let print_now = {
        let close = close.clone();
        let l = l.clone();
        move || {
            let Some(pr) = printer() else {
                return;
            };
            if busy.get_untracked() {
                return;
            }
            busy.set(true);
            let sheet_bytes = is_sheet().then(|| make());
            let (l, paper, n, shop, t) = (
                l.clone(),
                stock_paper(stock()),
                labels_of(&count.get_untracked()),
                shop.get_untracked(),
                label_template(&tpl.get_untracked(), shown()),
            );
            let close = close.clone();
            let task = spawn(async move {
                let r = off_thread(move || match sheet_bytes {
                    Some(make) => make().and_then(|b| send_pdf(&pr, &b)),
                    None => {
                        // At most 200 a job (copies are a byte).
                        let doc = label_doc(&t, &l, paper, &shop);
                        let mut left = n;
                        while left > 0 {
                            let k = left.min(200);
                            print_on(&pr, &doc, k as u8, false)?;
                            left -= k;
                        }
                        Ok(())
                    }
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
                    sku: "613000".into(),
                    tax_bps: 1_900,
                    net_minor: 3_000,
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
            receipt_t: builtin(DocKind::Receipt).remove(0).1,
            invoice_t: builtin(DocKind::Invoice).remove(0).1,
        }
    }

    #[test]
    fn label_stocks_and_contents() {
        assert_eq!(stock_at(5, (0, 0)), Stock::Single(40, 30));
        assert!(
            matches!(stock_at(SINGLES.len() + 2, (0, 0)), Stock::Sheet(s) if s.per_page() == 21)
        );
        // A size of one's own, kept within what prints.
        assert_eq!(stock_at(CUSTOM_STOCK, (500, 5)), Stock::Single(150, 10));
        // Every sheet fits on A4.
        for sh in sheets() {
            let w =
                sh.left_mm + f32::from(sh.cols) * sh.w_mm + f32::from(sh.cols - 1) * sh.gap_x_mm;
            let h = sh.top_mm + f32::from(sh.rows) * sh.h_mm + f32::from(sh.rows - 1) * sh.gap_y_mm;
            assert!(w <= 210.1 && h <= 297.1, "{sh:?}");
        }
        let t = builtin(DocKind::Label).remove(0).1;
        let all = Shown {
            name: true,
            price: true,
            barcode: true,
            sku: true,
            shop: true,
        };
        let full = label_template(&t, all);
        assert_eq!(full.sections.len(), t.sections.len() + 2);
        let bare = label_template(
            &t,
            Shown {
                barcode: false,
                sku: false,
                shop: false,
                ..all
            },
        );
        assert_eq!(bare.sections.len(), t.sections.len() - 1);
        assert_eq!(skipped("x"), 0);
        assert_eq!(labels_of("5000"), 999);
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
        let doc = nb_docs::templates::shelf_label(
            &ShelfLabel {
                name: "Olive noir".into(),
                price_minor: 65_000,
                unit: "kg".into(),
                currency: nb_core::Currency::DZD,
                barcode: Some("2000122003109".into()),
                sku: String::new(),
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

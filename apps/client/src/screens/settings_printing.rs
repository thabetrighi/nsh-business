//! Settings › printing (M12): this device's printers — each with its
//! connection, paper, what it prints, copies, cutter and drawer, tested on
//! the spot — what happens after a sale, and what the shop's receipts and
//! invoices show (logo, legal identity, VAT, the customer's balance, the
//! barcode, the notice), with a live sample.

use nb_i18n::tr_args;
use nb_store::usecases::printing::{DocSettings, document_settings, save_document_settings};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::printers::{Link, Printer, Role, Sheet};
use crate::screens::forms::error_text;
use crate::screens::settings::{group_card, row};
use crate::state::AppState;
use crate::ui::form::{field, labelled};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, icon_button, label, label_line, segmented, switch,
};
use crate::ui::media::data_url;
use crate::ui::overlay::{dialog_frame, modal, show_toast};
use crate::ui::surface::{card, card_header_icon, empty_state};
use crate::ui::tokens::{Status, Tone, ty};

fn link_text(st: AppState, p: &Printer) -> String {
    match &p.link {
        Link::Network { addr } => tr_args(st.lang(), "printers.via.network", &[("addr", addr)]),
        Link::Device { path } => tr_args(st.lang(), "printers.via.device", &[("path", path)]),
        Link::System { queue } => tr_args(st.lang(), "printers.via.system", &[("name", queue)]),
    }
}

fn sheet_text(st: AppState, s: Sheet) -> String {
    match s {
        Sheet::Roll { paper_mm, .. } => {
            tr_args(st.lang(), "printers.roll", &[("mm", &paper_mm.to_string())])
        }
        Sheet::Label { w_mm, h_mm, .. } => tr_args(
            st.lang(),
            "printers.label_size",
            &[("w", &w_mm.to_string()), ("h", &h_mm.to_string())],
        ),
        Sheet::A4 => "A4".into(),
        Sheet::A5 => "A5".into(),
    }
}

/// The settings tab.
pub fn printing_settings(st: AppState, compact: bool) -> View {
    let b = st.biz;
    // The printer being edited: its index, or `usize::MAX` for a new one.
    let editing: Signal<Option<usize>> = signal(None);
    let editing_open = signal(false);
    let list = dynamic(move || {
        let printers = b.printers.get();
        if printers.is_empty() {
            return empty_state(
                Icon::Printer,
                st.t("printers.empty"),
                st.t("printers.empty.body"),
                None,
            )
            .into_element();
        }
        let mut v = view();
        for (i, p) in printers.into_iter().enumerate() {
            let roles = p
                .roles
                .iter()
                .map(|r| st.tn(r.key()))
                .collect::<Vec<_>>()
                .join(" · ");
            let detail = format!("{} · {}", link_text(st, &p), sheet_text(st, p.sheet));
            let name = p.name.clone();
            let test = p.clone();
            let ready = p.ready();
            v = v.child(
                view()
                    .test_id(&format!("printer-row-{i}"))
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(20.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .child(icon(Icon::Printer, 20.0, || Tone::Muted))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(180.0))
                            .gap(px(2.0))
                            .child(label_line(
                                move || name.clone(),
                                ty::MD,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            ))
                            .child(label(
                                move || detail.clone(),
                                ty::SM,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(badge(Status::Info, None, move || roles.clone()))
                    .child(
                        button(
                            Variant::Secondary,
                            Size::Sm,
                            Some(Icon::Printer),
                            st.t("printers.test"),
                            move || {
                                if ready {
                                    crate::screens::printing::test_print(st, test.clone());
                                } else {
                                    show_toast(st, st.tn("print.no_address"));
                                }
                            },
                        )
                        .test_id(&format!("printer-test-{i}")),
                    )
                    .child(
                        icon_button(
                            || Icon::Pencil,
                            st.t("action.edit"),
                            Variant::Ghost,
                            move || {
                                editing.set(Some(i));
                                editing_open.set(true);
                            },
                        )
                        .test_id(&format!("printer-edit-{i}")),
                    )
                    .child(
                        icon_button(
                            || Icon::Trash,
                            st.t("action.delete"),
                            Variant::Ghost,
                            move || {
                                b.printers.update(|l| {
                                    if i < l.len() {
                                        l.remove(i);
                                    }
                                });
                            },
                        )
                        .test_id(&format!("printer-delete-{i}")),
                    ),
            );
        }
        v.into_element()
    });
    let printers_card = card()
        .child(card_header_icon(
            Some(Icon::Printer),
            st.t("printers.title"),
            Some(Box::new(st.t("printers.title.help"))),
            button(
                Variant::Primary,
                Size::Sm,
                Some(Icon::Plus),
                st.t("printers.add"),
                move || {
                    editing.set(Some(usize::MAX));
                    editing_open.set(true);
                },
            )
            .test_id("printer-add"),
        ))
        .child(list);
    let after_sale = group_card(
        st,
        Some((Icon::Receipt, "printers.after_sale")),
        vec![
            row(
                st,
                "print.auto",
                "print.auto.help",
                switch(b.auto_print, st.t("print.auto")).test_id("print-auto"),
                compact,
            ),
            row(
                st,
                "print.preview_first",
                "print.preview_first.help",
                switch(b.preview_first, st.t("print.preview_first")).test_id("print-preview-first"),
                compact,
            ),
        ],
    );
    view()
        .gap(px(20.0))
        .child(printers_card)
        .child(after_sale)
        .child(documents_card(st, compact))
        .child(modal(editing_open, compact, move || {
            let Some(i) = editing.get_untracked() else {
                return view().into_element();
            };
            let current = b.printers.get_untracked().get(i).cloned();
            printer_dialog(
                st,
                current,
                move |p| {
                    b.printers.update(|l| match l.get_mut(i) {
                        Some(slot) => *slot = p,
                        None => {
                            let id = l.iter().map(|x| x.id).max().unwrap_or(0) + 1;
                            l.push(Printer { id, ..p });
                        }
                    });
                    editing_open.set(false);
                },
                move || editing_open.set(false),
            )
            .into_element()
        }))
}

// ------------------------------------------------------------ the printer

#[derive(Clone, Copy, PartialEq, Eq)]
enum LinkKind {
    Network,
    Device,
    System,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SheetKind {
    Roll58,
    Roll80,
    Custom,
    Label,
    A4,
    A5,
}

fn num(s: &Signal<String>, min: u16, max: u16) -> Option<u16> {
    s.get_untracked()
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|v| (min..=max).contains(v))
}

/// Adds or edits a printer.
fn printer_dialog(
    st: AppState,
    current: Option<Printer>,
    save: impl Fn(Printer) + 'static,
    close: impl Fn() + Clone + 'static,
) -> View {
    let p = current.clone().unwrap_or_default();
    let name = signal(p.name.clone());
    let (lk, target) = match &p.link {
        Link::Network { addr } => (LinkKind::Network, addr.clone()),
        Link::Device { path } => (LinkKind::Device, path.clone()),
        Link::System { queue } => (LinkKind::System, queue.clone()),
    };
    let link = signal(lk);
    let target = signal(target);
    let (sk, paper_mm, print_mm, w_mm, h_mm, gap_mm) = match p.sheet {
        Sheet::Roll { paper_mm: 58, .. } => (SheetKind::Roll58, 58, 48, 40, 30, 2),
        Sheet::Roll { paper_mm: 80, .. } => (SheetKind::Roll80, 80, 72, 40, 30, 2),
        Sheet::Roll { paper_mm, print_mm } => (SheetKind::Custom, paper_mm, print_mm, 40, 30, 2),
        Sheet::Label { w_mm, h_mm, gap_mm } => (SheetKind::Label, 80, 72, w_mm, h_mm, gap_mm),
        Sheet::A4 => (SheetKind::A4, 80, 72, 40, 30, 2),
        Sheet::A5 => (SheetKind::A5, 80, 72, 40, 30, 2),
    };
    let sheet = signal(sk);
    let paper_mm = signal(paper_mm.to_string());
    let print_mm = signal(print_mm.to_string());
    let w_mm = signal(w_mm.to_string());
    let h_mm = signal(h_mm.to_string());
    let gap_mm = signal(gap_mm.to_string());
    let receipts = signal(p.roles.contains(&Role::Receipt));
    let invoices = signal(p.roles.contains(&Role::Invoice));
    let labels = signal(p.roles.contains(&Role::Label));
    let copies = signal(p.copies.max(1).to_string());
    let cut = signal(p.cut);
    let drawer = signal(p.drawer);
    let tspl = signal(p.tspl);
    let error = signal(String::new());
    // The paper says what it is for, as a start.
    effect(move || match sheet.get() {
        SheetKind::Label => {
            labels.set(true);
            receipts.set(false);
        }
        SheetKind::A4 | SheetKind::A5 => {
            invoices.set(true);
            if link.get_untracked() != LinkKind::System {
                link.set(LinkKind::System);
            }
        }
        _ => {}
    });
    // System printers, found off the interface's thread.
    let system: Signal<Vec<String>> = signal(Vec::new());
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::printers::send::system_printers());
        });
        let task = spawn(async move {
            if let Ok(list) = rx.await {
                system.set(list);
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    }
    let links: Vec<(LinkKind, Box<dyn Fn() -> String>)> = vec![
        (LinkKind::Network, Box::new(st.t("printers.link.network"))),
        (LinkKind::Device, Box::new(st.t("printers.link.device"))),
        (LinkKind::System, Box::new(st.t("printers.link.system"))),
    ];
    let sheets: Vec<(SheetKind, Box<dyn Fn() -> String>)> = vec![
        (SheetKind::Roll58, Box::new(|| "58 mm".to_owned())),
        (SheetKind::Roll80, Box::new(|| "80 mm".to_owned())),
        (SheetKind::Custom, Box::new(st.t("printers.sheet.custom"))),
        (SheetKind::Label, Box::new(st.t("printers.sheet.label"))),
        (SheetKind::A4, Box::new(|| "A4".to_owned())),
        (SheetKind::A5, Box::new(|| "A5".to_owned())),
    ];
    let small = move |caption: &'static str, value: Signal<String>, id: &'static str| {
        view().w(px(110.0)).child(field(
            st.t(caption),
            value,
            String::new,
            String::new,
            KeyboardType::Number,
            id,
        ))
    };
    let target_field = dynamic(move || {
        let (caption, hint, id) = match link.get() {
            LinkKind::Network => ("printers.addr", "192.168.1.50:9100", "printer-addr"),
            LinkKind::Device => (
                "printers.path",
                if cfg!(target_os = "windows") {
                    "\\\\localhost\\XP58"
                } else {
                    "/dev/usb/lp0"
                },
                "printer-path",
            ),
            LinkKind::System => ("printers.queue", "", "printer-queue"),
        };
        let mut v = view().gap(px(8.0)).child(field(
            st.t(caption),
            target,
            move || hint.to_owned(),
            String::new,
            KeyboardType::Default,
            id,
        ));
        let help = match link.get() {
            LinkKind::Network => "printers.addr.help",
            LinkKind::Device => "printers.path.help",
            LinkKind::System => "printers.queue.help",
        };
        v = v.child(label(st.t(help), ty::SM, FontWeight::NORMAL, Tone::Muted));
        if link.get() == LinkKind::System {
            let found = system.get();
            if !found.is_empty() {
                let mut chips = view().flex_row().flex_wrap().gap(px(6.0));
                for (i, q) in found.into_iter().enumerate() {
                    let pick = q.clone();
                    chips = chips.child(
                        button(
                            Variant::Secondary,
                            Size::Sm,
                            None,
                            move || q.clone(),
                            move || target.set(pick.clone()),
                        )
                        .test_id(&format!("printer-queue-{i}")),
                    );
                }
                v = v.child(chips);
            }
        }
        v.into_element()
    });
    let sizes = dynamic(move || match sheet.get() {
        SheetKind::Custom => view()
            .flex_row()
            .gap(px(12.0))
            .child(small("printers.paper_mm", paper_mm, "printer-paper-mm"))
            .child(small("printers.print_mm", print_mm, "printer-print-mm"))
            .into_element(),
        SheetKind::Label => view()
            .flex_row()
            .flex_wrap()
            .gap(px(12.0))
            .child(small("printers.label_w", w_mm, "printer-label-w"))
            .child(small("printers.label_h", h_mm, "printer-label-h"))
            .child(small("printers.label_gap", gap_mm, "printer-label-gap"))
            .into_element(),
        _ => view().into_element(),
    });
    let toggle = move |caption: &'static str, on: Signal<bool>, id: &'static str| {
        view()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(switch(on, st.t(caption)).test_id(id))
            .child(label(st.t(caption), ty::SM, FontWeight::NORMAL, Tone::Ink))
    };
    let body = view()
        .gap(px(16.0))
        .child(field(
            st.t("printers.name"),
            name,
            st.t("printers.name.hint"),
            String::new,
            KeyboardType::Default,
            "printer-name",
        ))
        .child(labelled(
            st.t("printers.link"),
            segmented(links, link).test_id("printer-link"),
            None,
        ))
        .child(target_field)
        .child(labelled(
            st.t("printers.sheet"),
            segmented(sheets, sheet).test_id("printer-sheet"),
            None,
        ))
        .child(sizes)
        .child(labelled(
            st.t("printers.roles"),
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(16.0))
                .child(toggle(
                    "printers.role.receipt",
                    receipts,
                    "printer-role-receipt",
                ))
                .child(toggle(
                    "printers.role.invoice",
                    invoices,
                    "printer-role-invoice",
                ))
                .child(toggle("printers.role.label", labels, "printer-role-label")),
            None,
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap(px(16.0))
                .child(small("print.copies", copies, "printer-copies"))
                .child(when(
                    move || !matches!(sheet.get(), SheetKind::A4 | SheetKind::A5),
                    move || {
                        view()
                            .flex_row()
                            .flex_wrap()
                            .gap(px(16.0))
                            .child(toggle("printers.cut", cut, "printer-cut"))
                            .child(toggle("printers.drawer", drawer, "printer-drawer"))
                    },
                ))
                .child(when(
                    move || sheet.get() == SheetKind::Label,
                    move || toggle("printers.tspl", tspl, "printer-tspl"),
                )),
        )
        .child(dynamic(move || {
            let e = error.get();
            if e.is_empty() {
                view().into_element()
            } else {
                label(move || e.clone(), ty::SM, FontWeight::MEDIUM, Tone::Danger).into_element()
            }
        }));
    let id = p.id;
    let on_save = move || {
        let n = name.get_untracked().trim().to_owned();
        if n.is_empty() {
            error.set(st.tn("printers.name_required"));
            return;
        }
        let t = target.get_untracked().trim().to_owned();
        let link = match link.get_untracked() {
            LinkKind::Network => Link::Network { addr: t },
            LinkKind::Device => Link::Device { path: t },
            LinkKind::System => Link::System { queue: t },
        };
        let sheet = match sheet.get_untracked() {
            SheetKind::Roll58 => Sheet::Roll {
                paper_mm: 58,
                print_mm: 48,
            },
            SheetKind::Roll80 => Sheet::Roll {
                paper_mm: 80,
                print_mm: 72,
            },
            SheetKind::Custom => {
                let (Some(pw), Some(pr)) = (num(&paper_mm, 30, 120), num(&print_mm, 20, 112))
                else {
                    error.set(st.tn("printers.size_invalid"));
                    return;
                };
                if pr > pw {
                    error.set(st.tn("printers.size_invalid"));
                    return;
                }
                Sheet::Roll {
                    paper_mm: pw,
                    print_mm: pr,
                }
            }
            SheetKind::Label => {
                let (Some(w), Some(h), Some(g)) = (
                    num(&w_mm, 15, 112),
                    num(&h_mm, 10, 200),
                    num(&gap_mm, 0, 10),
                ) else {
                    error.set(st.tn("printers.size_invalid"));
                    return;
                };
                Sheet::Label {
                    w_mm: w,
                    h_mm: h,
                    gap_mm: g,
                }
            }
            SheetKind::A4 => Sheet::A4,
            SheetKind::A5 => Sheet::A5,
        };
        let roles: Vec<Role> = [
            (Role::Receipt, receipts.get_untracked()),
            (Role::Invoice, invoices.get_untracked()),
            (Role::Label, labels.get_untracked()),
        ]
        .into_iter()
        .filter_map(|(r, on)| on.then_some(r))
        .collect();
        if roles.is_empty() {
            error.set(st.tn("printers.roles_required"));
            return;
        }
        save(Printer {
            id,
            name: n,
            link,
            sheet,
            roles,
            copies: copies
                .get_untracked()
                .trim()
                .parse::<u8>()
                .unwrap_or(1)
                .clamp(1, 20),
            cut: cut.get_untracked(),
            drawer: drawer.get_untracked(),
            tspl: tspl.get_untracked(),
        });
    };
    let footer = view()
        .child(button(
            Variant::Secondary,
            Size::Md,
            None,
            st.t("form.cancel"),
            close.clone(),
        ))
        .child(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Check),
                st.t("form.save"),
                on_save,
            )
            .test_id("printer-save"),
        );
    let new = current.is_none();
    dialog_frame(
        st,
        move || st.tn(if new { "printers.add" } else { "printers.edit" }),
        st.t("printers.dialog.help"),
        body,
        footer,
        close,
    )
}

// ------------------------------------------------------------- documents

/// A sale to show the choices on (never written).
fn sample_doc(st: AppState, d: &DocSettings) -> nb_docs::Doc {
    use nb_docs::templates::{Line, Sale, Shop, receipt};
    let lang = st.lang();
    let line = |name: &str, qty: i64, price: i64| Line {
        name: name.into(),
        qty_milli: qty,
        unit: "u".into(),
        unit_price_minor: price,
        amount_minor: price * qty / 1000,
        discount_minor: 0,
    };
    let sale = Sale {
        number: "FV-01-26-A000123".into(),
        when: (2026, 11, 8, 19, 57),
        cashier: st.tn("print.sample.cashier"),
        register: st.tn("print.sample.register"),
        customer: Some((st.tn("print.sample.customer"), String::new())),
        currency: st
            .session
            .get_untracked()
            .map_or(nb_core::Currency::DZD, |s| s.currency),
        lines: vec![
            line(&st.tn("print.sample.item1"), 1_000, 39_000),
            line(&st.tn("print.sample.item2"), 310, 65_000),
            line(&st.tn("print.sample.item3"), 2_000, 21_000),
        ],
        discount_minor: 0,
        net_minor: 98_084,
        taxes: vec![(1_900, 98_084, 18_636)],
        stamp_minor: 0,
        total_minor: 116_150,
        payments: vec![("cash".into(), 120_000)],
        change_minor: 3_850,
        owed_minor: 0,
        balance: Some((50_000, 50_000)),
        settled_minor: 0,
    };
    let shop = Shop {
        name: st.tn("print.sample.shop"),
        phone: "0799 39 36 39".into(),
        nif: "000016001234567".into(),
        footer: d.footer.clone(),
        ..Shop::default()
    };
    receipt(
        &shop,
        &sale,
        lang,
        nb_docs::Paper::ROLL80,
        &crate::screens::printing::options(d),
    )
}

fn documents_card(st: AppState, compact: bool) -> View {
    let loaded: Signal<Option<DocSettings>> = signal(None);
    let tagline = signal(String::new());
    let footer = signal(String::new());
    let notice = signal(String::new());
    let legal_ids = signal(true);
    let vat = signal(true);
    let cashier = signal(true);
    let table = signal(true);
    let count = signal(true);
    let balance = signal(true);
    let barcode = signal(true);
    let logo: Signal<Option<Vec<u8>>> = signal(None);
    let logo_changed = signal(false);
    let busy = signal(false);
    let task = spawn({
        let db = st.db();
        async move {
            if let Ok(d) = db.call(|s| document_settings(s)).await {
                tagline.set(d.tagline.clone());
                footer.set(d.footer.clone());
                notice.set(d.notice.clone());
                legal_ids.set(d.legal_ids);
                vat.set(d.vat);
                cashier.set(d.cashier);
                table.set(d.table);
                count.set(d.count);
                balance.set(d.balance);
                barcode.set(d.barcode);
                logo.set(d.logo.clone());
                loaded.set(Some(d));
            }
        }
    });
    drop(task); // runs to completion: tasks end with their owner, not their handle
    let current = move || DocSettings {
        tagline: tagline.get(),
        footer: footer.get(),
        notice: notice.get(),
        legal_ids: legal_ids.get(),
        vat: vat.get(),
        cashier: cashier.get(),
        table: table.get(),
        count: count.get(),
        balance: balance.get(),
        barcode: barcode.get(),
        logo: logo.get(),
    };
    // The sample receipt follows every choice.
    let sample = resource(
        move || {
            loaded.get()?;
            Some(current())
        },
        move |d, _| async move {
            let doc = sample_doc(st, &d);
            let (tx, rx) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(nb_docs::preview(&doc).map_err(|e| e.0));
            });
            rx.await.unwrap_or_else(|_| Err(String::new()))
        },
    );
    let choose_logo = move || {
        let task = spawn(async move {
            let dialog =
                nsh_core::file_dialog::OpenDialog::files().title(st.tn("documents.logo.choose"));
            let Ok(Some(paths)) = pick_paths(dialog).await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let (tx, rx) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let r = std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| nb_docs::logo::normalize(&b).map_err(|e| e.0));
                let _ = tx.send(r);
            });
            match rx.await {
                Ok(Ok(png)) => {
                    logo.set(Some(png));
                    logo_changed.set(true);
                }
                _ => show_toast(st, error_text(st, "documents.logo")),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let on_save = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        let d = DocSettings {
            logo: None,
            ..current()
        };
        let new_logo = logo_changed.get_untracked().then(|| logo.get_untracked());
        let Some(meta) = st.session.get_untracked().map(|s| s.meta("documents.save")) else {
            busy.set(false);
            return;
        };
        let db = st.db();
        let task = spawn(async move {
            let r = db
                .call(move |s| save_document_settings(s, meta, d, new_logo))
                .await;
            busy.set(false);
            match r {
                Ok(_) => {
                    logo_changed.set(false);
                    show_toast(st, st.tn("documents.saved"));
                }
                Err(e) => show_toast(
                    st,
                    error_text(st, e.codes.first().map_or("internal", |c| c.1)),
                ),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let toggle = |key: &'static str, help: &'static str, on: Signal<bool>, id: &'static str| {
        row(st, key, help, switch(on, st.t(key)).test_id(id), compact)
    };
    let text = |key: &'static str, help: &'static str, value: Signal<String>, id: &'static str| {
        row(
            st,
            key,
            help,
            field(
                String::new,
                value,
                String::new,
                String::new,
                KeyboardType::Default,
                id,
            )
            .w_full(),
            compact,
        )
    };
    let logo_row = row(
        st,
        "documents.logo",
        "documents.logo.help",
        view()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(dynamic(move || match logo.get() {
                Some(bytes) => view()
                    .h(px(48.0))
                    .max_w(px(160.0))
                    .child(
                        image(data_url("image/png", &bytes))
                            .h(px(48.0))
                            .w(px(120.0)),
                    )
                    .into_element(),
                None => label(
                    st.t("documents.logo.none"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )
                .into_element(),
            }))
            .child(
                button(
                    Variant::Secondary,
                    Size::Sm,
                    Some(Icon::ImagePlus),
                    st.t("documents.logo.choose"),
                    choose_logo,
                )
                .test_id("logo-choose"),
            )
            .child(when(
                move || logo.get().is_some(),
                move || {
                    icon_button(
                        || Icon::Trash,
                        st.t("action.delete"),
                        Variant::Ghost,
                        move || {
                            logo.set(None);
                            logo_changed.set(true);
                        },
                    )
                    .test_id("logo-remove")
                },
            )),
        compact,
    );
    let options = group_card(
        st,
        Some((Icon::FileText, "documents.title")),
        vec![
            logo_row,
            text(
                "documents.tagline",
                "documents.tagline.help",
                tagline,
                "doc-tagline",
            ),
            toggle(
                "documents.legal_ids",
                "documents.legal_ids.help",
                legal_ids,
                "doc-legal",
            ),
            toggle("documents.vat", "documents.vat.help", vat, "doc-vat"),
            toggle(
                "documents.cashier",
                "documents.cashier.help",
                cashier,
                "doc-cashier",
            ),
            toggle(
                "documents.table",
                "documents.table.help",
                table,
                "doc-table",
            ),
            toggle(
                "documents.count",
                "documents.count.help",
                count,
                "doc-count",
            ),
            toggle(
                "documents.balance",
                "documents.balance.help",
                balance,
                "doc-balance",
            ),
            toggle(
                "documents.barcode",
                "documents.barcode.help",
                barcode,
                "doc-barcode",
            ),
            text(
                "documents.footer",
                "documents.footer.help",
                footer,
                "doc-footer",
            ),
            text(
                "documents.notice",
                "documents.notice.help",
                notice,
                "doc-notice",
            ),
            view()
                .px(px(20.0))
                .py(px(14.0))
                .flex_row()
                .justify_end()
                .child(
                    button(
                        Variant::Primary,
                        Size::Md,
                        Some(Icon::Check),
                        st.t("form.save"),
                        on_save,
                    )
                    .test_id("doc-save"),
                ),
        ],
    );
    let preview_card = card()
        .child(card_header_icon(
            Some(Icon::Eye),
            st.t("documents.sample"),
            Some(Box::new(st.t("documents.sample.help"))),
            view(),
        ))
        .child(view().p(px(16.0)).items_center().child(dynamic(move || {
            match sample.get() {
                Some((bytes, w, h)) => {
                    let shown_w = 300.0_f32;
                    let shown_h = shown_w * h as f32 / w.max(1) as f32;
                    view()
                        .border_1()
                        .border_color(Tone::Border.color())
                        .bg(rgb(0xFFFFFF))
                        .child(
                            image(data_url("image/png", &bytes))
                                .w(px(shown_w))
                                .h(px(shown_h)),
                        )
                        .test_id("doc-sample")
                        .into_element()
                }
                None => label(
                    st.t("print.preparing"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )
                .into_element(),
            }
        })));
    // The settings panel is too narrow for both side by side: the
    // choices, then the receipt they make.
    view().gap(px(20.0)).child(options).child(preview_card)
}

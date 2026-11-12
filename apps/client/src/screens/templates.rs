//! Settings › printing › templates (M12.11): every kind of document —
//! receipt, invoice, customer receipt, schedule, label, report — has
//! built-in layouts and the shop's own. A layout is edited as a form, not
//! drawn: its look (colour, type size, header, table style, logo) and its
//! sections in order (each with its own settings and variables such as
//! `{shop.name}`), with the document it makes shown live on sample data.
//! What the shop switched off in « what documents show » stays off
//! whatever the layout says; a layout that cannot be read never prints
//! (the built-in one does).

use nb_docs::Paper;
use nb_docs::templates::{Options, Shop, sample_data};
use nb_domain::template::{
    Align, DocKind, Field, HeaderLayout, ItemColumn, ItemField, Section, TableStyle, Template,
    TotalLine, builtin, variables,
};
use nb_store::usecases::printing::{DocSettings, PrintShop, document_settings, print_shop};
use nb_store::usecases::templates::{
    TemplateInput, TemplateRow, choose_template, delete_template, list_templates, save_template,
};
use nsh_core::element::{KeyboardType, View};
use nsh_core::prelude::*;

use crate::screens::forms::error_text;
use crate::screens::printing::{draw, off_thread, options, shop_of};
use crate::state::AppState;
use crate::ui::form::{field, labelled, text_area};
use crate::ui::icon::{Icon, icon};
use crate::ui::kit::{
    Size, Variant, badge, button, icon_button, label, label_line, segmented, switch,
};
use crate::ui::media::data_url;
use crate::ui::overlay::{dialog_frame, show_toast, wide_modal};
use crate::ui::popup::dropdown;
use crate::ui::surface::{card, card_header_icon};
use crate::ui::tokens::{Status, Tone, ty};

/// The colours offered for the accent (sheets; rolls print black).
const ACCENTS: [&str; 8] = [
    "#4338CA", "#1D4ED8", "#0F766E", "#047857", "#B45309", "#BE123C", "#7C3AED", "#111827",
];

/// What a template's sections may hold: at most this many.
const MAX_SECTIONS: usize = 40;

fn kind_name(st: AppState, k: DocKind) -> String {
    st.tn(&format!("templates.kind.{}", k.code()))
}

fn row_name(st: AppState, kind: DocKind, r: &TemplateRow) -> String {
    if r.builtin {
        st.tn(&format!("templates.builtin.{}.{}", kind.code(), r.name))
    } else {
        r.name.clone()
    }
}

/// The paper a kind is usually shown on, and the others offered.
fn papers(kind: DocKind) -> Vec<(usize, Paper, &'static str)> {
    match kind {
        DocKind::Label => vec![
            (0, Paper::Label { w_mm: 40, h_mm: 30 }, "40×30"),
            (1, Paper::Label { w_mm: 58, h_mm: 40 }, "58×40"),
        ],
        DocKind::Invoice | DocKind::Schedule | DocKind::Report => vec![
            (0, Paper::A4, "A4"),
            (1, Paper::A5, "A5"),
            (2, Paper::ROLL80, "80 mm"),
        ],
        DocKind::Receipt | DocKind::Voucher => vec![
            (0, Paper::ROLL80, "80 mm"),
            (1, Paper::ROLL58, "58 mm"),
            (2, Paper::A4, "A4"),
        ],
    }
}

/// What samples are drawn with: the shop's identity and its choices.
#[derive(Clone, PartialEq)]
struct Base {
    shop: Shop,
    options: Options,
    lang: nb_i18n::Lang,
    currency: nb_core::Currency,
}

fn base_of(st: AppState, shop: Option<PrintShop>, docs: Option<DocSettings>) -> Base {
    let docs = docs.unwrap_or_default();
    let mut s = shop
        .as_ref()
        .map(|p| shop_of(p, &docs.footer))
        .unwrap_or_default();
    if s.name.trim().is_empty() {
        s.name = st.tn("print.sample.shop");
    }
    if s.phone.is_empty() {
        s.phone = "0799 39 36 39".into();
    }
    if s.address.is_empty() {
        s.address = st.tn("templates.sample.address");
    }
    if s.nif.is_empty() {
        s.nif = "000016001234567".into();
    }
    Base {
        shop: s,
        options: options(&docs),
        lang: st.lang(),
        currency: st
            .session
            .get_untracked()
            .map_or(nb_core::Currency::DZD, |s| s.currency),
    }
}

async fn picture(t: Template, paper: Paper, b: Base) -> Result<(Vec<u8>, u32, u32), String> {
    off_thread(move || {
        let d = sample_data(t.kind, b.lang, b.currency, &b.shop, &b.options);
        nb_docs::preview(&draw(&t, &b.options, &d, b.lang, paper)).map_err(|e| e.0)
    })
    .await
    .unwrap_or_else(|| Err(String::new()))
}

/// A picture of a document at most `max_w` wide (a roll about its paper's
/// width, a sheet as wide as there is room).
fn picture_fit(
    st: AppState,
    picture: Option<(Vec<u8>, u32, u32)>,
    error: Option<String>,
    max_w: f32,
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
    let shown_w = if w <= 600 {
        (w as f32 * 0.75).clamp(160.0, 300.0).min(max_w)
    } else {
        max_w
    };
    let shown_h = shown_w * h as f32 / w.max(1) as f32;
    view()
        .items_center()
        .child(
            view()
                .border_1()
                .border_color(Tone::Border.color())
                .bg(rgb(0xFFFFFF))
                .shadow_xs()
                .child(
                    image(data_url("image/png", &bytes))
                        .w(px(shown_w))
                        .h(px(shown_h)),
                ),
        )
        .test_id("tpl-preview")
        .into_element()
}

/// A change to the shop's templates, run on the database.
type Change =
    Box<dyn FnOnce(&mut nb_store::Store, nb_store::OpMeta) -> Result<(), nb_store::Error> + Send>;

/// What is being edited: the shop's template (`id`) or a new one.
#[derive(Clone, PartialEq)]
struct Draft {
    id: Option<String>,
    name: String,
    template: Template,
    /// The built-in layout it started from (reset goes back to it).
    origin: Template,
}

// ------------------------------------------------------------- the list

/// The templates card of Settings › printing.
pub fn templates_card(st: AppState, compact: bool) -> View {
    let kind_at = signal(0usize);
    let kind = move || {
        DocKind::ALL
            .get(kind_at.get())
            .copied()
            .unwrap_or(DocKind::Receipt)
    };
    let tick = signal(0u32);
    let rows = resource(
        move || Some((kind(), tick.get())),
        move |(k, _), _| async move {
            st.db()
                .call(move |s| Ok((k, list_templates(s, k)?)))
                .await
                .map_err(|e| error_text(st, e.codes.first().map_or("internal", |c| c.1)))
        },
    );
    // The last list read stays on screen while the next one loads (the
    // page does not jump after a save).
    let kept: Signal<Option<(DocKind, Vec<TemplateRow>)>> = signal(None);
    effect(move || {
        if let Some(r) = rows.get() {
            kept.set(Some(r));
        }
    });
    let base = resource(
        move || Some(()),
        move |_, _| async move {
            let r = st
                .db()
                .call(|s| Ok((print_shop(s).ok(), document_settings(s).ok())))
                .await
                .unwrap_or((None, None));
            Ok::<_, String>(base_of(st, r.0, r.1))
        },
    );
    // The row shown in the preview: the one in use until another is pressed.
    let looked: Signal<Option<String>> = signal(None);
    let draft: Signal<Option<Draft>> = signal(None);
    let editing = signal(false);
    let edit = move |d: Draft| {
        draft.set(Some(d));
        editing.set(true);
    };
    let run = move |what: &'static str, f: Change| {
        let Some(meta) = st.session.get_untracked().map(|s| s.meta(what)) else {
            return;
        };
        let task = spawn(async move {
            match st.db().call(move |s| f(s, meta)).await {
                Ok(()) => {
                    tick.update(|t| *t += 1);
                    show_toast(st, st.tn("templates.saved"));
                }
                Err(e) => show_toast(
                    st,
                    error_text(st, e.codes.first().map_or("internal", |c| c.1)),
                ),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let list = dynamic(move || {
        let Some((k, list)) = kept.get() else {
            return label(
                st.t("print.preparing"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .px(px(20.0))
            .py(px(12.0))
            .into_element();
        };
        let mut v = view();
        for (i, r) in list.into_iter().enumerate() {
            let name = row_name(st, k, &r);
            let shown = looked.get_untracked().map_or(r.chosen, |l| l == r.id);
            let detail = if r.builtin {
                st.tn("templates.builtin")
            } else {
                st.tn("templates.own")
            };
            let id = r.id.clone();
            let mut actions = view().flex_row().items_center().gap(px(4.0));
            if r.chosen {
                actions = actions.child(badge(
                    Status::Success,
                    Some(Icon::Check),
                    st.t("templates.in_use"),
                ));
            } else {
                let id = r.id.clone();
                actions = actions.child(
                    button(
                        Variant::Secondary,
                        Size::Sm,
                        None,
                        st.t("templates.use"),
                        move || {
                            let id = id.clone();
                            run(
                                "templates.choose",
                                Box::new(move |s, meta| choose_template(s, meta, k, &id)),
                            );
                        },
                    )
                    .test_id(&format!("tpl-use-{i}")),
                );
            }
            let copy = {
                let (t, name) = (r.template.clone(), name.clone());
                move || {
                    edit(Draft {
                        id: None,
                        name: tr_copy(st, &name),
                        template: t.clone(),
                        origin: builtin(t.kind).remove(0).1,
                    });
                }
            };
            actions = actions.child(
                icon_button(|| Icon::Copy, st.t("templates.copy"), Variant::Ghost, copy)
                    .test_id(&format!("tpl-copy-{i}")),
            );
            if !r.builtin {
                let (t, n, id) = (r.template.clone(), r.name.clone(), r.id.clone());
                actions = actions.child(
                    icon_button(
                        || Icon::Pencil,
                        st.t("action.edit"),
                        Variant::Ghost,
                        move || {
                            edit(Draft {
                                id: Some(id.clone()),
                                name: n.clone(),
                                template: t.clone(),
                                origin: builtin(t.kind).remove(0).1,
                            });
                        },
                    )
                    .test_id(&format!("tpl-edit-{i}")),
                );
                let id = r.id.clone();
                actions = actions.child(
                    icon_button(
                        || Icon::Trash,
                        st.t("action.delete"),
                        Variant::Ghost,
                        move || {
                            let id = id.clone();
                            run(
                                "templates.delete",
                                Box::new(move |s, meta| delete_template(s, meta, &id)),
                            );
                        },
                    )
                    .test_id(&format!("tpl-delete-{i}")),
                );
            }
            v = v.child(
                view()
                    .test_id(&format!("tpl-row-{i}"))
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(20.0))
                    .py(px(10.0))
                    .border_t_1()
                    .border_color(Tone::Border.color())
                    .bg(if shown {
                        Tone::PrimarySoft.color()
                    } else {
                        Tone::Surface.color()
                    })
                    .cursor_pointer()
                    .on_press(move || looked.set(Some(id.clone())))
                    .child(icon(Icon::FileText, 18.0, move || {
                        if shown { Tone::Primary } else { Tone::Muted }
                    }))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(140.0))
                            .gap(px(2.0))
                            .child(label_line(
                                move || name.clone(),
                                ty::MD,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            ))
                            .child(label(
                                move || detail.clone(),
                                ty::XS,
                                FontWeight::NORMAL,
                                Tone::Muted,
                            )),
                    )
                    .child(actions),
            );
        }
        v.into_element()
    });
    let preview = resource(
        move || {
            let (_, list) = kept.get()?;
            let l = looked.get();
            let t = list
                .iter()
                .find(|r| l.as_ref().map_or(r.chosen, |l| *l == r.id))
                .or_else(|| list.first())
                .map(|r| r.template.clone())?;
            Some((t, base.get()?))
        },
        move |(t, b): (Template, Base), _| async move {
            let paper = papers(t.kind)[0].1;
            picture(t, paper, b).await
        },
    );
    let kinds: Vec<(usize, Box<dyn Fn() -> String>)> = DocKind::ALL
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let k = *k;
            (
                i,
                Box::new(move || kind_name(st, k)) as Box<dyn Fn() -> String>,
            )
        })
        .collect();
    let new_one = move || {
        let k = kind();
        let t = builtin(k).remove(0).1;
        edit(Draft {
            id: None,
            name: tr_copy(
                st,
                &st.tn(&format!(
                    "templates.builtin.{}.{}",
                    k.code(),
                    builtin(k)[0].0
                )),
            ),
            template: t.clone(),
            origin: t,
        });
    };
    let shown = dynamic(move || {
        picture_fit(
            st,
            preview.get(),
            preview.error(),
            if compact { 300.0 } else { 320.0 },
        )
    });
    let body = if compact {
        view()
            .gap(px(12.0))
            .child(list)
            .child(view().p(px(16.0)).child(shown))
    } else {
        view()
            .flex_row()
            .items_start()
            .child(view().flex_1().min_w(px(0.0)).child(list))
            .child(
                view()
                    .w(px(360.0))
                    .p(px(16.0))
                    .border_s(px(1.0))
                    .border_s_color(Tone::Border.color())
                    .child(shown),
            )
    };
    card()
        .child(card_header_icon(
            Some(Icon::Layers),
            st.t("templates.title"),
            Some(Box::new(st.t("templates.title.help"))),
            button(
                Variant::Primary,
                Size::Sm,
                Some(Icon::Plus),
                st.t("templates.new"),
                new_one,
            )
            .test_id("tpl-new"),
        ))
        .child(
            view()
                .px(px(20.0))
                .pb(px(12.0))
                .child(scroll_x(segmented(kinds, kind_at).test_id("tpl-kind"))),
        )
        .child(body)
        .child(wide_modal(editing, compact, move || {
            let Some(d) = draft.get_untracked() else {
                return view().into_element();
            };
            editor(
                st,
                d,
                compact,
                move || {
                    editing.set(false);
                    tick.update(|t| *t += 1);
                },
                move || editing.set(false),
            )
            .into_element()
        }))
}

/// A control that may be wider than a phone: it scrolls sideways.
fn scroll_x(v: View) -> View {
    view().child(scroll().horizontal().child(v))
}

fn tr_copy(st: AppState, name: &str) -> String {
    nb_i18n::tr_args(st.lang(), "templates.copy_of", &[("name", name)])
        .chars()
        .take(60)
        .collect()
}

// ------------------------------------------------------------ the editor

fn section_name(st: AppState, code: &str) -> String {
    st.tn(&format!("templates.section.{code}"))
}

/// One line on what a section shows (its text, its columns…).
fn summary(st: AppState, s: &Section) -> String {
    let short = |t: &str| {
        let t = t.replace('\n', " ");
        if t.chars().count() > 40 {
            format!("{}…", t.chars().take(40).collect::<String>())
        } else {
            t
        }
    };
    match s {
        Section::Title { text, .. } | Section::Text { text, .. } => short(text),
        Section::Barcode { value } => short(value),
        Section::Fields { fields } => short(
            &fields
                .iter()
                .map(|f| f.value.as_str())
                .collect::<Vec<_>>()
                .join(" · "),
        ),
        Section::Items { columns } => columns
            .iter()
            .map(|c| st.tn(&format!("templates.col.{}", c.field.code())))
            .collect::<Vec<_>>()
            .join(" · "),
        Section::Totals { lines } => lines
            .iter()
            .map(|l| st.tn(&format!("templates.total.{}", l.code())))
            .collect::<Vec<_>>()
            .join(" · "),
        Section::Spacer { height } => format!("{height} pt"),
        _ => String::new(),
    }
}

fn editor(
    st: AppState,
    d: Draft,
    compact: bool,
    on_saved: impl Fn() + Clone + 'static,
    close: impl Fn() + Clone + 'static,
) -> View {
    let kind = d.template.kind;
    let tpl = signal(d.template.clone());
    let name = signal(d.name.clone());
    let use_it = signal(true);
    let selected: Signal<Option<usize>> = signal(None);
    let busy = signal(false);
    // The look, written into the template as it changes.
    let accent = signal(d.template.style.accent.clone());
    let size = signal(d.template.style.size);
    let header = signal(d.template.style.header);
    let table = signal(d.template.style.table);
    let logo = signal(d.template.style.logo);
    effect(move || {
        let (a, s, h, t, l) = (
            accent.get(),
            size.get(),
            header.get(),
            table.get(),
            logo.get(),
        );
        tpl.update(|x| {
            x.style.accent = a;
            x.style.size = s;
            x.style.header = h;
            x.style.table = t;
            x.style.logo = l;
        });
    });
    let paper_at = signal(0usize);
    let base = resource(
        move || Some(()),
        move |_, _| async move {
            let r = st
                .db()
                .call(|s| Ok((print_shop(s).ok(), document_settings(s).ok())))
                .await
                .unwrap_or((None, None));
            Ok::<_, String>(base_of(st, r.0, r.1))
        },
    );
    let shown = resource(
        move || Some((tpl.get(), paper_at.get(), base.get()?)),
        move |(t, at, b): (Template, usize, Base), _| async move {
            let paper = papers(t.kind)
                .into_iter()
                .find(|p| p.0 == at)
                .map_or(Paper::A4, |p| p.1);
            picture(t, paper, b).await
        },
    );

    // The look.
    let swatches = {
        let mut row = view().flex_row().flex_wrap().gap(px(8.0));
        for (i, hex) in ACCENTS.iter().enumerate() {
            let hex = (*hex).to_owned();
            let c = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
            let h2 = hex.clone();
            let on = computed(move || accent.get().eq_ignore_ascii_case(&h2));
            row = row.child(
                view()
                    .test_id(&format!("tpl-accent-{i}"))
                    .w(px(28.0))
                    .h(px(28.0))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(rgb(c))
                    .border_2()
                    .border_color(move || {
                        if on.get() {
                            Tone::Ink.color()
                        } else {
                            Tone::Surface.color()
                        }
                    })
                    .on_press(move || accent.set(hex.clone())),
            );
        }
        row
    };
    let sizes: Vec<(u8, Box<dyn Fn() -> String>)> = (8..=12u8)
        .map(|n| {
            (
                n,
                Box::new(move || n.to_string()) as Box<dyn Fn() -> String>,
            )
        })
        .collect();
    let headers: Vec<(HeaderLayout, Box<dyn Fn() -> String>)> = vec![
        (
            HeaderLayout::Centered,
            Box::new(st.t("templates.header.centered")),
        ),
        (
            HeaderLayout::Split,
            Box::new(st.t("templates.header.split")),
        ),
    ];
    let tables: Vec<(TableStyle, Box<dyn Fn() -> String>)> = vec![
        (TableStyle::Lines, Box::new(st.t("templates.table.lines"))),
        (TableStyle::Shaded, Box::new(st.t("templates.table.shaded"))),
        (
            TableStyle::Striped,
            Box::new(st.t("templates.table.striped")),
        ),
        (TableStyle::Ruled, Box::new(st.t("templates.table.ruled"))),
    ];
    let look = view()
        .gap(px(14.0))
        .child(field(
            st.t("templates.name"),
            name,
            String::new,
            String::new,
            KeyboardType::Default,
            "tpl-name",
        ))
        .child(labelled(
            st.t("templates.accent"),
            swatches,
            Some(Box::new(st.t("templates.accent.help"))),
        ))
        .child(labelled(
            st.t("templates.size"),
            segmented(sizes, size).test_id("tpl-size"),
            None,
        ))
        .child(labelled(
            st.t("templates.header"),
            segmented(headers, header).test_id("tpl-header"),
            None,
        ))
        .child(labelled(
            st.t("templates.table"),
            scroll_x(segmented(tables, table).test_id("tpl-table")),
            None,
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(switch(logo, st.t("templates.logo")).test_id("tpl-logo"))
                .child(label(
                    st.t("templates.logo"),
                    ty::SM,
                    FontWeight::MEDIUM,
                    Tone::Body,
                )),
        );

    // The sections, in order; the settings of the selected one under it.
    // One slot per place, so a field never shows another section's text.
    let mut list = view().gap(px(6.0));
    for i in 0..MAX_SECTIONS {
        list = list
            .child(when(
                move || i < tpl.get().sections.len(),
                move || dynamic(move || section_row(st, tpl, selected, i)),
            ))
            .child(when(
                move || selected.get() == Some(i),
                move || section_editor(st, tpl, i, kind),
            ));
    }
    let add_at = signal(7usize); // text
    let codes: Vec<(usize, String)> = Section::CODES
        .iter()
        .enumerate()
        .map(|(i, c)| (i, section_name(st, c)))
        .collect();
    let add = view()
        .flex_row()
        .items_end()
        .gap(px(8.0))
        .child(view().flex_1().child(dropdown(
            move || codes.clone(),
            add_at,
            Some(Icon::Plus),
            st.t("templates.add_section"),
            "tpl-add-kind",
        )))
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::Plus),
                st.t("templates.add_section"),
                move || {
                    let Some(s) = Section::CODES
                        .get(add_at.get_untracked())
                        .and_then(|c| Section::fresh(c, kind))
                    else {
                        return;
                    };
                    if matches!(s, Section::Items { .. })
                        && tpl
                            .get_untracked()
                            .sections
                            .iter()
                            .any(|x| matches!(x, Section::Items { .. }))
                    {
                        show_toast(st, error_text(st, "template.items_once"));
                        return;
                    }
                    let mut at = None;
                    tpl.update(|t| {
                        if t.sections.len() < MAX_SECTIONS {
                            t.sections.push(s);
                            at = Some(t.sections.len() - 1);
                        }
                    });
                    selected.set(at);
                },
            )
            .test_id("tpl-add"),
        );
    let sections = view()
        .gap(px(10.0))
        .child(label(
            st.t("templates.sections"),
            ty::MD,
            FontWeight::SEMIBOLD,
            Tone::Ink,
        ))
        .child(label(
            st.t("templates.sections.help"),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(list)
        .child(add);

    let paper_choice: Vec<(usize, Box<dyn Fn() -> String>)> = papers(kind)
        .into_iter()
        .map(|(i, _, n)| (i, Box::new(move || n.to_owned()) as Box<dyn Fn() -> String>))
        .collect();
    let preview = view()
        .gap(px(12.0))
        .items_center()
        .child(segmented(paper_choice, paper_at).test_id("tpl-paper"))
        .child(dynamic(move || {
            picture_fit(
                st,
                shown.get(),
                shown.error(),
                if compact { 320.0 } else { 600.0 },
            )
        }))
        .child(label(
            st.t("templates.preview.help"),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        ));

    let body = if compact {
        view()
            .gap(px(20.0))
            .child(look)
            .child(sections)
            .child(preview)
    } else {
        // Each side scrolls alone: the document stays in sight while the
        // sections are edited.
        view()
            .flex_row()
            .flex_1()
            .min_h(px(0.0))
            .child(
                scroll().w(px(440.0)).min_h(px(0.0)).child(
                    view()
                        .gap(px(24.0))
                        .px(px(24.0))
                        .py(px(20.0))
                        .child(look)
                        .child(sections),
                ),
            )
            .child(
                scroll()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .bg(Tone::Subtle.color())
                    .child(view().p(px(20.0)).child(preview)),
            )
    };

    let origin = d.origin.clone();
    let reset = move || {
        let o = origin.clone();
        selected.set(None);
        accent.set(o.style.accent.clone());
        size.set(o.style.size);
        header.set(o.style.header);
        table.set(o.style.table);
        logo.set(o.style.logo);
        tpl.set(o);
    };
    let id = d.id.clone();
    let on_save = move || {
        if busy.get_untracked() {
            return;
        }
        let Some(session) = st.session.get_untracked() else {
            return;
        };
        busy.set(true);
        let input = TemplateInput {
            id: id.clone(),
            name: name.get_untracked(),
            template: tpl.get_untracked(),
        };
        let (m1, m2) = (
            session.meta("templates.save"),
            session.meta("templates.choose"),
        );
        let chosen = use_it.get_untracked();
        let done = on_saved.clone();
        let task = spawn(async move {
            let r = st
                .db()
                .call(move |s| {
                    let id = save_template(s, m1, input)?;
                    if chosen {
                        choose_template(s, m2, kind, &id)?;
                    }
                    Ok(())
                })
                .await;
            busy.set(false);
            match r {
                Ok(()) => {
                    show_toast(st, st.tn("templates.saved"));
                    done();
                }
                Err(e) => show_toast(
                    st,
                    error_text(st, e.codes.first().map_or("internal", |c| c.1)),
                ),
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    };
    let footer = view()
        .child(
            button(
                Variant::Ghost,
                Size::Md,
                Some(Icon::Undo),
                st.t("templates.reset"),
                reset,
            )
            .test_id("tpl-reset"),
        )
        .child(view().flex_1())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(switch(use_it, st.t("templates.use_after")).test_id("tpl-use-after"))
                .child(label(
                    st.t("templates.use_after"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Body,
                )),
        )
        .child(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Check),
                st.t("form.save"),
                on_save,
            )
            .test_id("tpl-save"),
        );
    let title = nb_i18n::tr_args(
        st.lang(),
        "templates.editor",
        &[("kind", &kind_name(st, kind))],
    );
    if compact {
        return dialog_frame(
            st,
            move || title.clone(),
            st.t("templates.editor.help"),
            body,
            footer,
            close,
        );
    }
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(
            view()
                .flex_row()
                .items_start()
                .gap(px(12.0))
                .px(px(24.0))
                .py(px(16.0))
                .border_b_1()
                .border_color(Tone::Border.color())
                .child(
                    view()
                        .flex_1()
                        .gap(px(4.0))
                        .child(label(
                            move || title.clone(),
                            ty::XL,
                            FontWeight::BOLD,
                            Tone::Ink,
                        ))
                        .child(label(
                            st.t("templates.editor.help"),
                            ty::SM,
                            FontWeight::NORMAL,
                            Tone::Muted,
                        )),
                )
                .child(
                    icon_button(|| Icon::Close, st.t("action.close"), Variant::Ghost, close)
                        .test_id("dialog-close"),
                ),
        )
        .child(body)
        .child(
            footer
                .flex_row()
                .items_center()
                .justify_end()
                .gap(px(8.0))
                .px(px(24.0))
                .py(px(12.0))
                .bg(Tone::Subtle.color())
                .border_t_1()
                .border_color(Tone::Border.color()),
        )
}

/// The line of section `i` in the list: what it is, what it shows, and
/// moving or removing it.
fn section_row(
    st: AppState,
    tpl: Signal<Template>,
    selected: Signal<Option<usize>>,
    i: usize,
) -> AnyElement {
    let t = tpl.get();
    let n = t.sections.len();
    let Some(s) = t.sections.get(i) else {
        return view().into_element();
    };
    let title = section_name(st, s.code());
    let sub = summary(st, s);
    let on = selected.get() == Some(i);
    let mv = move |to: usize| {
        selected.set(None);
        tpl.update(|t| {
            if to < t.sections.len() {
                t.sections.swap(i, to);
            }
        });
    };
    let mut row = view()
        .test_id(&format!("tpl-sec-{i}"))
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(if on {
            Tone::Primary.color()
        } else {
            Tone::Border.color()
        })
        .bg(if on {
            Tone::PrimarySoft.color()
        } else {
            Tone::Surface.color()
        })
        .cursor_pointer()
        .on_press(move || selected.set(if on { None } else { Some(i) }))
        .child(icon(
            if on { Icon::ChevronDown } else { Icon::Sliders },
            16.0,
            move || if on { Tone::Primary } else { Tone::Muted },
        ))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(label_line(
                    move || title.clone(),
                    ty::SM,
                    FontWeight::SEMIBOLD,
                    Tone::Ink,
                ))
                .child(label_line(
                    move || sub.clone(),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )),
        );
    if i > 0 {
        row = row.child(
            icon_button(
                || Icon::ArrowUp,
                st.t("templates.up"),
                Variant::Ghost,
                move || mv(i - 1),
            )
            .test_id(&format!("tpl-up-{i}")),
        );
    }
    if i + 1 < n {
        row = row.child(
            icon_button(
                || Icon::ArrowDown,
                st.t("templates.down"),
                Variant::Ghost,
                move || mv(i + 1),
            )
            .test_id(&format!("tpl-down-{i}")),
        );
    }
    row.child(
        icon_button(
            || Icon::Trash,
            st.t("action.delete"),
            Variant::Ghost,
            move || {
                selected.set(None);
                tpl.update(|t| {
                    if t.sections.len() > 1 && i < t.sections.len() {
                        t.sections.remove(i);
                    }
                });
            },
        )
        .test_id(&format!("tpl-remove-{i}")),
    )
    .into_element()
}

/// Chips that add a variable to a text.
fn variable_chips(st: AppState, kind: DocKind, into: Signal<String>) -> View {
    let mut row = view().flex_row().flex_wrap().gap(px(6.0));
    for name in variables(kind) {
        let caption = st.tn(&format!("templates.var.{name}"));
        row = row.child(
            view()
                .test_id(&format!("tpl-var-{name}"))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .bg(Tone::Subtle.color())
                .border_1()
                .border_color(Tone::Border.color())
                .on_press(move || {
                    into.update(|t| {
                        if !t.is_empty() && !t.ends_with([' ', '\n']) {
                            t.push(' ');
                        }
                        t.push('{');
                        t.push_str(name);
                        t.push('}');
                    })
                })
                .child(label(
                    move || caption.clone(),
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Body,
                )),
        );
    }
    labelled(
        st.t("templates.variables"),
        row,
        Some(Box::new(st.t("templates.variables.help"))),
    )
}

fn aligns(st: AppState) -> Vec<(Align, Box<dyn Fn() -> String>)> {
    vec![
        (Align::Start, Box::new(st.t("templates.align.start"))),
        (Align::Center, Box::new(st.t("templates.align.center"))),
        (Align::End, Box::new(st.t("templates.align.end"))),
    ]
}

fn toggle_row(_st: AppState, on: Signal<bool>, caption: String, id: String) -> View {
    let c2 = caption.clone();
    view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(switch(on, move || c2.clone()).test_id(&id))
        .child(label(
            move || caption.clone(),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Body,
        ))
        .test_id(&format!("{id}-row"))
}

/// Lines of « label: value » as fields, and back.
fn fields_text(fields: &[Field]) -> String {
    fields
        .iter()
        .map(|f| {
            if f.label.is_empty() {
                f.value.clone()
            } else {
                format!("{}: {}", f.label, f.value)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_fields(text: &str) -> Vec<Field> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(20)
        .map(|l| match l.split_once(": ") {
            Some((a, b)) if !a.contains('{') => Field::new(a.trim(), b.trim()),
            _ => Field::new("", l),
        })
        .collect()
}

/// The settings of section `i`.
fn section_editor(st: AppState, tpl: Signal<Template>, i: usize, kind: DocKind) -> View {
    let Some(s) = tpl.get_untracked().sections.get(i).cloned() else {
        return view();
    };
    let set = move |f: Box<dyn FnOnce(&mut Section)>| {
        tpl.update(|t| {
            if let Some(s) = t.sections.get_mut(i) {
                f(s);
            }
        });
    };
    let panel = view()
        .test_id("tpl-section-editor")
        .gap(px(12.0))
        .p(px(14.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(Tone::Primary.color());
    match s {
        Section::Header { legal_ids } => {
            let on = signal(legal_ids);
            effect(move || {
                let v = on.get();
                set(Box::new(move |s| {
                    if let Section::Header { legal_ids } = s {
                        *legal_ids = v;
                    }
                }));
            });
            panel
                .child(toggle_row(
                    st,
                    on,
                    st.tn("templates.legal_ids"),
                    "tpl-legal".into(),
                ))
                .child(label(
                    st.t("templates.header.help"),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
        }
        Section::Parties { details } => {
            let on = signal(details);
            effect(move || {
                let v = on.get();
                set(Box::new(move |s| {
                    if let Section::Parties { details } = s {
                        *details = v;
                    }
                }));
            });
            panel.child(toggle_row(
                st,
                on,
                st.tn("templates.details"),
                "tpl-details".into(),
            ))
        }
        Section::Title { text, align } | Section::Text { text, align, .. } => {
            let is_text = matches!(
                tpl.get_untracked().sections.get(i),
                Some(Section::Text { .. })
            );
            let (size0, bold0) = match tpl.get_untracked().sections.get(i) {
                Some(Section::Text { size, bold, .. }) => (*size, *bold),
                _ => (0, false),
            };
            let value = signal(text);
            let al = signal(align);
            let sz = signal(size0);
            let bd = signal(bold0);
            effect(move || {
                let (v, a, z, b) = (value.get(), al.get(), sz.get(), bd.get());
                set(Box::new(move |s| match s {
                    Section::Title { text, align } => {
                        *text = v;
                        *align = a;
                    }
                    Section::Text {
                        text,
                        align,
                        size,
                        bold,
                    } => {
                        *text = v;
                        *align = a;
                        *size = z;
                        *bold = b;
                    }
                    _ => {}
                }));
            });
            let mut p = panel
                .child(text_area(
                    st.t("templates.text"),
                    value,
                    st.t("templates.text.placeholder"),
                    90.0,
                    "tpl-text",
                ))
                .child(labelled(
                    st.t("templates.align"),
                    segmented(aligns(st), al).test_id("tpl-align"),
                    None,
                ));
            if is_text {
                let sizes: Vec<(i8, Box<dyn Fn() -> String>)> = [-2i8, 0, 2, 4, 6]
                    .into_iter()
                    .map(|n| {
                        let c = if n > 0 {
                            format!("+{n}")
                        } else {
                            n.to_string()
                        };
                        (n, Box::new(move || c.clone()) as Box<dyn Fn() -> String>)
                    })
                    .collect();
                p = p
                    .child(labelled(
                        st.t("templates.text_size"),
                        segmented(sizes, sz).test_id("tpl-text-size"),
                        None,
                    ))
                    .child(toggle_row(
                        st,
                        bd,
                        st.tn("templates.bold"),
                        "tpl-bold".into(),
                    ));
            }
            p.child(variable_chips(st, kind, value))
        }
        Section::Fields { fields } => {
            let value = signal(fields_text(&fields));
            effect(move || {
                let f = parse_fields(&value.get());
                set(Box::new(move |s| {
                    if let Section::Fields { fields } = s {
                        *fields = f;
                    }
                }));
            });
            panel
                .child(text_area(
                    st.t("templates.fields"),
                    value,
                    st.t("templates.fields.placeholder"),
                    110.0,
                    "tpl-fields",
                ))
                .child(label(
                    st.t("templates.fields.help"),
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ))
                .child(variable_chips(st, kind, value))
        }
        Section::Barcode { value } => {
            let v = signal(value);
            effect(move || {
                let x = v.get();
                set(Box::new(move |s| {
                    if let Section::Barcode { value } = s {
                        *value = x;
                    }
                }));
            });
            panel
                .child(field(
                    st.t("templates.barcode"),
                    v,
                    String::new,
                    String::new,
                    KeyboardType::Default,
                    "tpl-barcode",
                ))
                .child(variable_chips(st, kind, v))
        }
        Section::Signatures { start, end } => {
            let a = signal(start);
            let b = signal(end);
            effect(move || {
                let (x, y) = (a.get(), b.get());
                set(Box::new(move |s| {
                    if let Section::Signatures { start, end } = s {
                        *start = x;
                        *end = y;
                    }
                }));
            });
            panel
                .child(field(
                    st.t("templates.sign_start"),
                    a,
                    st.t("print.voucher.sign_customer"),
                    String::new,
                    KeyboardType::Default,
                    "tpl-sign-start",
                ))
                .child(field(
                    st.t("templates.sign_end"),
                    b,
                    st.t("print.voucher.sign_shop"),
                    String::new,
                    KeyboardType::Default,
                    "tpl-sign-end",
                ))
        }
        Section::Divider { dashed } => {
            let on = signal(dashed);
            effect(move || {
                let v = on.get();
                set(Box::new(move |s| {
                    if let Section::Divider { dashed } = s {
                        *dashed = v;
                    }
                }));
            });
            panel.child(toggle_row(
                st,
                on,
                st.tn("templates.dashed"),
                "tpl-dashed".into(),
            ))
        }
        Section::Spacer { height } => {
            let h = signal(height);
            effect(move || {
                let v = h.get();
                set(Box::new(move |s| {
                    if let Section::Spacer { height } = s {
                        *height = v;
                    }
                }));
            });
            let heights: Vec<(u8, Box<dyn Fn() -> String>)> = [4u8, 6, 8, 10, 12, 16, 24, 40]
                .into_iter()
                .map(|n| {
                    (
                        n,
                        Box::new(move || n.to_string()) as Box<dyn Fn() -> String>,
                    )
                })
                .collect();
            panel.child(labelled(
                st.t("templates.height"),
                segmented(heights, h).test_id("tpl-height"),
                None,
            ))
        }
        Section::Balance => panel.child(label(
            st.t("templates.balance.help"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        )),
        Section::Items { columns } => {
            if kind == DocKind::Report || kind == DocKind::Label {
                return panel.child(label(
                    st.t("templates.items.report"),
                    ty::SM,
                    FontWeight::NORMAL,
                    Tone::Muted,
                ));
            }
            let mut p = panel.child(label(
                st.t("templates.items.help"),
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            ));
            for f in ItemField::of(kind) {
                let f = *f;
                let on = signal(columns.iter().any(|c| c.field == f));
                effect(move || {
                    let v = on.get();
                    set(Box::new(move |s| {
                        if let Section::Items { columns } = s {
                            let has = columns.iter().any(|c| c.field == f);
                            if v && !has && columns.len() < 10 {
                                columns.push(ItemColumn::new(f, width_of(f)));
                                // Columns keep the usual order.
                                let order = ItemField::of(kind);
                                columns.sort_by_key(|c| order.iter().position(|x| *x == c.field));
                            } else if !v && has && columns.len() > 1 {
                                columns.retain(|c| c.field != f);
                            }
                        }
                    }));
                });
                p = p.child(toggle_row(
                    st,
                    on,
                    st.tn(&format!("templates.col.{}", f.code())),
                    format!("tpl-col-{}", f.code()),
                ));
            }
            p
        }
        Section::Totals { lines } => {
            let offered: &[TotalLine] = match kind {
                DocKind::Receipt | DocKind::Invoice => &TotalLine::ALL,
                _ => &[TotalLine::Total],
            };
            let mut p = panel;
            for l in offered {
                let l = *l;
                let on = signal(lines.contains(&l));
                effect(move || {
                    let v = on.get();
                    set(Box::new(move |s| {
                        if let Section::Totals { lines } = s {
                            if v && !lines.contains(&l) {
                                lines.push(l);
                                lines.sort_by_key(|x| TotalLine::ALL.iter().position(|y| y == x));
                            } else if !v {
                                lines.retain(|x| *x != l);
                            }
                        }
                    }));
                });
                p = p.child(toggle_row(
                    st,
                    on,
                    st.tn(&format!("templates.total.{}", l.code())),
                    format!("tpl-total-{}", l.code()),
                ));
            }
            p
        }
    }
}

/// A column's usual share of the width.
fn width_of(f: ItemField) -> u8 {
    match f {
        ItemField::Name => 36,
        ItemField::Index => 6,
        ItemField::Sku | ItemField::Due | ItemField::State => 16,
        ItemField::TaxRate | ItemField::Unit => 9,
        _ => 14,
    }
}

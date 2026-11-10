//! The data table pattern (DESIGN.md v2 §Tables): sticky header with
//! sortable columns, virtualized rows, optional selection checkboxes, hover
//! highlight, and a row-actions column. Built on flex rows, so it mirrors
//! with the language like everything else (no manual RTL handling).

use std::collections::BTreeSet;
use std::rc::Rc;

use nsh_core::element::{AnyElement, View};
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::kit::{checkbox, label_line};
use super::tokens::{Tone, sp, ty};

/// How wide a column is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Width {
    Fixed(f32),
    /// A share of the room left by fixed columns, with a minimum.
    Flex(f32, f32),
}

/// Where a column's content sits: text at the reading start, numbers at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    End,
}

pub struct Column {
    pub key: &'static str,
    pub title: Rc<dyn Fn() -> String>,
    pub width: Width,
    pub align: Align,
    pub sortable: bool,
}

impl Column {
    pub fn new(key: &'static str, title: impl Fn() -> String + 'static, width: Width) -> Self {
        Self {
            key,
            title: Rc::new(title),
            width,
            align: Align::Start,
            sortable: false,
        }
    }
    pub fn end(mut self) -> Self {
        self.align = Align::End;
        self
    }
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }
}

/// The sort the user picked: a column and its direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub key: &'static str,
    pub desc: bool,
}

/// Selection of rows by their identity.
#[derive(Clone, Copy)]
pub struct Selection {
    pub ids: Signal<BTreeSet<u64>>,
}

impl Selection {
    pub fn new() -> Self {
        Self {
            ids: signal(BTreeSet::new()),
        }
    }
    pub fn count(&self) -> usize {
        self.ids.with(BTreeSet::len)
    }
    pub fn clear(&self) {
        self.ids.set(BTreeSet::new());
    }
}

fn sized(v: View, w: Width) -> View {
    match w {
        Width::Fixed(p) => v.w(px(p)).flex_shrink_0(),
        Width::Flex(grow, min) => v.flex_grow(grow).basis(px(0.0)).min_w(px(min)),
    }
}

fn cell_frame(c: &Column) -> View {
    let v = sized(view().h_full().justify_center().px(px(12.0)), c.width);
    match c.align {
        Align::Start => v.items_start(),
        Align::End => v.items_end(),
    }
}

const CHECK_W: f32 = 44.0;

/// The table. `cell(at, key)` builds a cell once; it reads `at.index()` in
/// its bindings to follow the row it shows. `id_of(index)` identifies rows
/// for the selection.
#[allow(clippy::too_many_arguments)]
pub fn table(
    columns: Vec<Column>,
    count: impl Fn() -> usize + Clone + 'static,
    sort: Signal<Option<Sort>>,
    selection: Option<Selection>,
    id_of: impl Fn(usize) -> Option<u64> + Clone + 'static,
    cell: impl Fn(Position, &'static str) -> AnyElement + 'static,
    test_id: &'static str,
) -> View {
    let columns = Rc::new(columns);
    let row_h = sp(48.0);

    // Header.
    let mut header = view()
        .flex_row()
        .items_center()
        .h(px(sp(40.0)))
        .bg(Tone::Subtle.color())
        .border_b_1()
        .border_color(Tone::Border.color());
    if let Some(sel) = selection {
        let count_all = count.clone();
        let ids_all = id_of.clone();
        let state = move || {
            let n = sel.count();
            if n == 0 {
                Some(false)
            } else if n >= count_all() {
                Some(true)
            } else {
                None
            }
        };
        let count_t = count.clone();
        let ids_t = ids_all.clone();
        header = header.child(view().w(px(CHECK_W)).items_center().justify_center().child(
            checkbox(state, move || {
                if sel.count() > 0 {
                    sel.clear();
                } else {
                    let all: BTreeSet<u64> = (0..count_t()).filter_map(&ids_t).collect();
                    sel.ids.set(all);
                }
            }),
        ));
    }
    for c in columns.iter() {
        let key = c.key;
        let title = c.title.clone();
        let mut inner = view()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .child(label_line(
                move || title(),
                ty::XS,
                FontWeight::SEMIBOLD,
                Tone::Muted,
            ));
        if c.sortable {
            let active = computed(move || sort.get().map(|s| s.key) == Some(key));
            inner = inner
                .cursor_pointer()
                .on_press(move || {
                    sort.set(match sort.get_untracked() {
                        Some(s) if s.key == key && !s.desc => Some(Sort { key, desc: true }),
                        Some(s) if s.key == key && s.desc => None,
                        _ => Some(Sort { key, desc: false }),
                    })
                })
                .child(dynamic(move || {
                    let which = match sort.get() {
                        Some(s) if s.key == key && s.desc => Icon::ArrowDown,
                        Some(s) if s.key == key => Icon::ArrowUp,
                        _ => Icon::ChevronsUpDown,
                    };
                    icon(which, 12.0, move || {
                        if active.get() { Tone::Ink } else { Tone::Faint }
                    })
                    .into_element()
                }));
        }
        header = header.child(cell_frame(c).child(inner));
    }

    // Rows.
    let cols = columns.clone();
    let cell = Rc::new(cell);
    let rows = virtual_list(count.clone(), move |at: Position| {
        let selected = selection.map(|sel| {
            let id_of = id_of.clone();
            computed(move || id_of(at.index()).is_some_and(|id| sel.ids.with(|s| s.contains(&id))))
        });
        let mut row = view()
            .flex_row()
            .items_center()
            .h(px(row_h))
            .border_b_1()
            .border_color(Tone::Border.color())
            .hover(|s| s.bg(Tone::Subtle.color()));
        if let (Some(sel), Some(is_sel)) = (selection, selected) {
            let id_of = id_of.clone();
            row = row
                .bg(move || {
                    if is_sel.get() {
                        Tone::PrimarySoft.color()
                    } else {
                        Tone::Surface.color()
                    }
                })
                .child(
                    view()
                        .w(px(CHECK_W))
                        .items_center()
                        .justify_center()
                        .child(checkbox(
                            move || Some(is_sel.get()),
                            move || {
                                if let Some(id) = id_of(at.index()) {
                                    sel.ids.update(|s| {
                                        if !s.remove(&id) {
                                            s.insert(id);
                                        }
                                    });
                                }
                            },
                        )),
                );
        }
        for c in cols.iter() {
            row = row.child(cell_frame(c).child(cell(at, c.key)));
        }
        row
    })
    .flex_1();

    view()
        .test_id(test_id)
        .flex_1()
        .min_h(px(240.0))
        .child(header)
        .child(rows)
}

/// The bar under a table: a count at the start, paging at the end.
pub fn table_footer(info: impl Fn() -> String + 'static, end: View) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(sp(48.0)))
        .px(px(16.0))
        .border_t_1()
        .border_color(Tone::Border.color())
        .child(
            view()
                .flex_1()
                .child(label_line(info, ty::SM, FontWeight::NORMAL, Tone::Muted)),
        )
        .child(end.flex_row().items_center().gap(px(6.0)))
}

//! The data table pattern (DESIGN.md v2 §Tables): sticky header with
//! sortable columns, virtualized rows, optional selection checkboxes, hover
//! highlight, and a row-actions column. Built on flex rows, so it mirrors
//! with the language like everything else (no manual RTL handling).

use std::collections::BTreeSet;
use std::rc::Rc;

use nsh_core::element::{AnyElement, View};
use nsh_core::prelude::*;

use nb_i18n::tr_args;

use super::icon::{Icon, icon};
use super::kit::{checkbox, label_line};
use super::popup::dropdown;
use super::tokens::radius;
use super::tokens::{Tone, sp, ty};
use crate::state::AppState;

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
    // As tall as its rows (a page of them): the page around it scrolls.
    .h(move || px(count() as f32 * row_h));

    view().test_id(test_id).child(header).child(rows)
}

/// The rows per page a reader can pick.
pub const PAGE_SIZES: [u32; 4] = [10, 25, 50, 100];

/// Which page of a list shows, and how many rows a page holds.
#[derive(Clone, Copy)]
pub struct Paging {
    /// From 0.
    pub page: Signal<u32>,
    pub size: Signal<u32>,
}

impl Paging {
    pub fn new(size: u32) -> Self {
        Self {
            page: signal(0),
            size: signal(size),
        }
    }
    /// The first row of the page (tracked).
    pub fn offset(self) -> u32 {
        self.page.get().saturating_mul(self.size.get())
    }
    /// Back to the first page (filters changed).
    pub fn reset(self) {
        if self.page.get_untracked() != 0 {
            self.page.set(0);
        }
    }
}

fn pages(total: u32, size: u32) -> u32 {
    total.div_ceil(size.max(1)).max(1)
}

/// The pages to offer around `current` (0-based): the first, the last, and
/// two on each side; `None` is a gap.
fn page_window(current: u32, count: u32) -> Vec<Option<u32>> {
    let mut out = Vec::new();
    let mut last: Option<u32> = None;
    for p in 0..count {
        let near = p + 2 >= current && p <= current + 2;
        if p == 0 || p + 1 == count || near {
            if last.is_some_and(|l| p > l + 1) {
                out.push(None);
            }
            out.push(Some(p));
            last = Some(p);
        }
    }
    out
}

fn page_button(
    content: AnyElement,
    active: bool,
    enabled: bool,
    on_press: impl Fn() + 'static,
    test_id: String,
) -> View {
    let mut b = view()
        .test_id(&test_id)
        .min_w(px(32.0))
        .h(px(32.0))
        .px(px(6.0))
        .items_center()
        .justify_center()
        .rounded(px(radius::md()))
        .border_1()
        .border_color(if active {
            Tone::Primary.color()
        } else {
            Tone::Border.color()
        })
        .bg(if active {
            Tone::PrimarySoft.color()
        } else {
            Tone::Surface.color()
        })
        .child(content);
    if enabled && !active {
        b = b
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Hover.color()))
            .on_press(on_press);
    } else if !enabled {
        b = b.opacity(0.45);
    }
    b
}

/// The bar under a paged table: rows per page and the range shown at the
/// start, the pages at the end (on phones: previous, page of pages, next).
pub fn pager(
    st: AppState,
    paging: Paging,
    total: impl Fn() -> u32 + Clone + 'static,
    compact: bool,
) -> View {
    // Another page size starts again from the first page; fewer rows than
    // the page shown bring it back to the last one.
    let size_seen = std::cell::Cell::new(paging.size.get_untracked());
    effect(move || {
        let size = paging.size.get();
        if size_seen.replace(size) != size {
            paging.page.set(0);
        }
    });
    let t = total.clone();
    effect(move || {
        let last = pages(t(), paging.size.get()) - 1;
        if paging.page.get() > last {
            paging.page.set(last);
        }
    });
    let t = total.clone();
    let range = move || {
        let (n, size, page) = (t(), paging.size.get(), paging.page.get());
        if n == 0 {
            return st.tn("table.none");
        }
        let from = page * size + 1;
        let to = (from + size - 1).min(n);
        let f = st.fmt();
        tr_args(
            st.lang(),
            "table.range",
            &[
                ("from", &f.integer(i64::from(from))),
                ("to", &f.integer(i64::from(to))),
                ("total", &f.integer(i64::from(n))),
            ],
        )
    };
    let size_choice = view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(label_line(
            st.t("table.per_page"),
            ty::SM,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
        .child(view().w(px(84.0)).child(dropdown(
            || PAGE_SIZES.iter().map(|n| (*n, n.to_string())).collect(),
            paging.size,
            None,
            st.t("table.per_page"),
            "page-size",
        )));
    let nav = dynamic(move || {
        let count = pages(total(), paging.size.get());
        let current = paging.page.get().min(count - 1);
        let (prev_icon, next_icon) = if st.rtl() {
            (Icon::ChevronRight, Icon::ChevronLeft)
        } else {
            (Icon::ChevronLeft, Icon::ChevronRight)
        };
        let mut v = view().flex_row().items_center().gap(px(4.0));
        v = v.child(page_button(
            icon(prev_icon, 16.0, || Tone::Ink).into_element(),
            false,
            current > 0,
            move || paging.page.set(current.saturating_sub(1)),
            "page-prev".into(),
        ));
        if compact {
            v = v.child(view().px(px(8.0)).child(label_line(
                move || {
                    tr_args(
                        st.lang(),
                        "table.page_of",
                        &[
                            ("page", &st.fmt().integer(i64::from(current + 1))),
                            ("pages", &st.fmt().integer(i64::from(count))),
                        ],
                    )
                },
                ty::SM,
                FontWeight::MEDIUM,
                Tone::Ink,
            )));
        } else {
            for p in page_window(current, count) {
                v = v.child(match p {
                    Some(p) => page_button(
                        label_line(
                            move || st.fmt().integer(i64::from(p + 1)),
                            ty::SM,
                            FontWeight::MEDIUM,
                            if p == current {
                                Tone::Primary
                            } else {
                                Tone::Ink
                            },
                        )
                        .into_element(),
                        p == current,
                        true,
                        move || paging.page.set(p),
                        format!("page-{}", p + 1),
                    ),
                    None => view().w(px(20.0)).items_center().child(label_line(
                        || "…".to_owned(),
                        ty::SM,
                        FontWeight::NORMAL,
                        Tone::Muted,
                    )),
                });
            }
        }
        v.child(page_button(
            icon(next_icon, 16.0, || Tone::Ink).into_element(),
            false,
            current + 1 < count,
            move || paging.page.set(current + 1),
            "page-next".into(),
        ))
        .into_element()
    });
    let mut bar = view()
        .test_id("pager")
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(12.0))
        .px(px(16.0))
        .py(px(10.0))
        .min_h(px(sp(52.0)))
        .border_t_1()
        .border_color(Tone::Border.color());
    if !compact {
        bar = bar.child(size_choice);
    }
    bar.child(
        view()
            .flex_1()
            .child(label_line(range, ty::SM, FontWeight::NORMAL, Tone::Muted)),
    )
    .child(nav)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_around_the_current_one() {
        let w = |c, n| page_window(c, n);
        assert_eq!(w(0, 1), [Some(0)]);
        assert_eq!(w(0, 5), [Some(0), Some(1), Some(2), None, Some(4)]);
        assert_eq!(
            w(5, 80),
            [
                Some(0),
                None,
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                None,
                Some(79)
            ]
        );
        assert_eq!(w(79, 80), [Some(0), None, Some(77), Some(78), Some(79)]);
        assert_eq!(pages(0, 25), 1);
        assert_eq!(pages(2_000, 25), 80);
        assert_eq!(pages(2_001, 25), 81);
    }
}

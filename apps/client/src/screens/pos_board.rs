//! The till's screen built from its layout ([`crate::screens::pos_layout`])
//! and edited in place: a panel is dragged by its handle to a slot (between
//! panels, or a new column), columns are resized by dragging the bar
//! between them, panels by their bottom edge; buttons do the same for touch
//! screens (move, taller/shorter, wider/narrower, remove, add, style).

use std::rc::Rc;

use crate::screens::pos_layout::{
    BUILTIN, Kept, Layout, MAX_COLUMNS, MAX_PANELS, Panel, Widget, builtin,
};

use nsh_core::element::desktop::{MenuItem, popup_menu};
use nsh_core::element::{ResponderEvent, TouchClaim};

use super::*;

/// Where a dragged panel would go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Slot {
    /// Column, place before which it goes.
    At(usize, usize),
    /// A column of its own, at this place.
    NewColumn(usize),
}

/// What a drag resizes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Resize {
    /// The bar after column `c` (between it and the next).
    Column(usize),
    Panel(usize, usize),
}

/// The till's layout as shown and as edited.
#[derive(Clone, Copy)]
pub(super) struct Board {
    pub editing: Signal<bool>,
    pub draft: Signal<Layout>,
    pub drag: Signal<Option<(usize, usize)>>,
    pub over: Signal<Option<Slot>>,
    pub resizing: Signal<Option<Resize>>,
    /// Where the pointer was when the resize last stepped.
    pub start: Signal<(f32, f32)>,
    /// The board's width (shares of it are columns).
    pub width: Signal<f32>,
    pub layouts_open: Signal<bool>,
    pub name: Signal<String>,
}

/// No colour (a slot that is not needed now).
fn clear() -> AdaptiveColor {
    AdaptiveColor::new(rgba(0x0000_0000), rgba(0x0000_0000))
}

/// Points of height per share when a panel is dragged taller.
const SHARE_H: f32 = 48.0;

impl Board {
    pub fn new() -> Self {
        Board {
            editing: signal(false),
            draft: signal(builtin().remove(0)),
            drag: signal(None),
            over: signal(None),
            resizing: signal(None),
            start: signal((0.0, 0.0)),
            width: signal(1000.0),
            layouts_open: signal(false),
            name: signal(String::new()),
        }
    }

    pub fn kept(st: AppState) -> Kept {
        Kept::parse(&st.biz.pos.layouts.get())
    }

    /// What shows: the draft while editing, else the chosen layout.
    pub fn shown(self, st: AppState) -> Layout {
        if self.editing.get() {
            self.draft.get()
        } else {
            Self::kept(st).current()
        }
    }

    pub fn edit(self, l: Layout, name: String) {
        self.name.set(name);
        self.draft.set(l);
        self.editing.set(true);
    }

    /// Keeps the draft under its name and shows it.
    pub fn save(self, st: AppState) {
        let mut l = self.draft.get_untracked();
        let name = self.name.get_untracked().trim().to_owned();
        if name.is_empty() {
            show_toast(st, error_text(st, "required"));
            return;
        }
        l.name = name;
        let mut k = Self::kept(st);
        if k.keep(l) {
            st.biz.pos.layouts.set(k.to_json());
            self.editing.set(false);
            show_toast(st, st.tn("pos.layouts.saved"));
        } else {
            show_toast(st, st.tn("pos.layouts.invalid"));
        }
    }

    fn change(self, f: impl FnOnce(&mut Layout)) {
        self.draft.update(f);
    }

    /// Puts the dragged panel where the pointer is.
    fn drop_now(self) {
        let (Some(from), over) = (self.drag.get_untracked(), self.over.get_untracked()) else {
            return;
        };
        self.drag.set(None);
        self.over.set(None);
        match over {
            Some(Slot::At(c, i)) => self.change(|l| {
                l.move_panel(from, (c, i));
            }),
            Some(Slot::NewColumn(at)) => self.change(|l| {
                l.split_off_column(from, at);
            }),
            None => {}
        }
    }

    /// Follows the pointer: a resize steps by shares; a drag ends when
    /// the button is up.
    fn pointer(self, pos: Point<Points>, down: bool, rtl: bool) {
        // Some platforms report no button during a drag: the release
        // (on the handle) ends it; a move reported without a button where
        // one is known to be down is ignored.
        let _ = down;
        let Some(r) = self.resizing.get_untracked() else {
            return;
        };
        let (x0, y0) = self.start.get_untracked();
        let (x, y) = (pos.x.0, pos.y.0);
        match r {
            Resize::Column(c) => {
                let total: f32 = self
                    .draft
                    .with_untracked(|l| l.columns.iter().map(|c| f32::from(c.width)).sum());
                let per = self.width.get_untracked() / total.max(1.0);
                let dx = (x - x0) * if rtl { -1.0 } else { 1.0 };
                let steps = (dx / per.max(1.0)).trunc() as i32;
                if steps != 0 {
                    self.change(|l| {
                        let n = l.columns.len();
                        if c + 1 < n {
                            let a = i32::from(l.columns[c].width) + steps;
                            let b = i32::from(l.columns[c + 1].width) - steps;
                            if a >= 1 && b >= 1 {
                                l.resize_column(c, steps);
                                l.resize_column(c + 1, -steps);
                            }
                        }
                    });
                    self.start
                        .set((x0 + steps as f32 * per * if rtl { -1.0 } else { 1.0 }, y0));
                }
            }
            Resize::Panel(c, i) => {
                let steps = ((y - y0) / SHARE_H).trunc() as i32;
                if steps != 0 {
                    self.change(|l| {
                        // From "as tall as its content": start from a few shares.
                        if let Some(p) = l.columns.get_mut(c).and_then(|c| c.panels.get_mut(i))
                            && p.height == 0
                        {
                            p.height = 3;
                        }
                        l.resize_panel((c, i), steps);
                    });
                    self.start.set((x0, y0 + steps as f32 * SHARE_H));
                }
            }
        }
    }
}

/// What the board needs from the selling view (built there, with the
/// search's text and the products found).
#[derive(Clone)]
pub(super) struct BoardParts {
    /// The search; `true`: what is found shows under it.
    pub search: Rc<dyn Fn(bool) -> View>,
    /// The products in a style (0 cards, 1 list, 2 tiles).
    pub products: Rc<dyn Fn(u8) -> View>,
}

/// A panel's content.
fn widget(st: AppState, t: Till, l: &Layout, p: &Panel, parts: &BoardParts) -> View {
    let has = |w: Widget| {
        l.columns
            .iter()
            .any(|c| c.panels.iter().any(|p| p.widget == w))
    };
    match p.widget {
        Widget::Search => (parts.search)(!has(Widget::Products)),
        Widget::Categories => {
            if p.style == 1 {
                category_column(st, t)
            } else {
                category_chips(st, t)
            }
        }
        Widget::Products => (parts.products)(p.style),
        Widget::Cart => cart_box(
            st,
            t,
            false,
            CartParts {
                lines: Some(p.style == 1),
                customer: !has(Widget::Customer),
                sums: false,
                pay: false,
            },
            false,
        ),
        Widget::Totals => {
            card()
                .pb(px(12.0))
                .child(cart_sums(st, t, false, false, !has(Widget::BigTotal)))
        }
        Widget::BigTotal => total_band(st, t),
        Widget::Pay => pay_buttons(st, t, false),
        Widget::Customer => card().child(customer_chip(st, t, false)),
        Widget::Actions => card().child(side_actions(st, t)),
        Widget::Keypad => card().p(px(12.0)).child(line_keypad(st, t)),
    }
}

fn widget_name(st: AppState, w: Widget) -> String {
    st.tn(&format!("pos.widget.{}", w.code()))
}

/// The till's screen from its layout; while editing, with the handles.
pub(super) fn board_view(st: AppState, t: Till, parts: BoardParts) -> View {
    let b = t.board;
    let rtl = st.lang().direction() == nb_i18n::Direction::Rtl;
    let screen = dynamic(move || {
        let l = b.shown(st);
        let editing = b.editing.get();
        // What the products show follows the layout.
        let pr = st.biz.pos;
        if pr.tile.get_untracked() != usize::from(l.tile) {
            pr.tile.set(usize::from(l.tile));
        }
        if pr.stock.get_untracked() != l.stock {
            pr.stock.set(l.stock);
        }
        if pr.sku.get_untracked() != l.sku {
            pr.sku.set(l.sku);
        }
        // While editing the title stays (its tools edit the layout).
        let head = l.head || editing;
        if pr.head.get_untracked() != head {
            pr.head.set(head);
        }
        if pr.big_lines.get_untracked() != l.big_lines {
            pr.big_lines.set(l.big_lines);
        }
        let gap = f32::from(l.gap);
        let n = l.columns.len();
        let mut row = view()
            .flex_1()
            .min_h(px(0.0))
            .flex_row()
            .gap(px(if editing { 0.0 } else { gap }));
        for (c, col) in l.columns.iter().enumerate() {
            if editing {
                row = row.child(new_column_slot(st, b, c));
            }
            let mut stack = view()
                .flex_grow(f32::from(col.width))
                .basis(px(0.0))
                .min_w(px(180.0))
                .min_h(px(0.0))
                .gap(px(if editing { 0.0 } else { gap }));
            if editing {
                stack = stack.child(column_bar(st, b, c, col.width));
            }
            for (i, p) in col.panels.iter().enumerate() {
                if editing {
                    stack = stack.child(slot(b, c, i));
                }
                let content = widget(st, t, &l, p, &parts);
                let sized = if editing && p.height > 0 {
                    // While editing the board scrolls: a share is a fixed height.
                    view()
                        .h(px(f32::from(p.height) * SHARE_H + 40.0))
                        .child(content.flex_1().min_h(px(0.0)))
                } else if p.height > 0 {
                    view()
                        .flex_grow(f32::from(p.height))
                        .basis(px(0.0))
                        .min_h(px(72.0))
                        .child(content.flex_1().min_h(px(0.0)))
                } else {
                    view().child(content)
                };
                stack = stack.child(if editing {
                    edit_frame(st, b, c, i, p.clone(), n, col.panels.len(), sized)
                } else {
                    sized
                });
            }
            if editing {
                stack = stack.child(slot(b, c, col.panels.len()).flex_1());
            }
            row = row.child(stack);
            if editing && c + 1 < n {
                row = row.child(splitter(b, c, rtl));
            }
        }
        if editing {
            row = row.child(new_column_slot(st, b, n));
        }
        let mut v = view().flex_1().min_h(px(0.0)).gap(px(10.0));
        if editing {
            v = v.child(edit_bar(st, b)).child(
                scroll()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(row.flex_1().pb(px(24.0))),
            );
        } else {
            v = v.child(row);
        }
        v.into_element()
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .on_layout(move |r: &Bounds<Points>| b.width.set(r.size.width.0))
        // A drag or resize follows the pointer over the whole board.
        .on_mouse_move(move |e: &MouseEvent| {
            if b.drag.get_untracked().is_some() || b.resizing.get_untracked().is_some() {
                b.pointer(e.page_position, e.pressed_button.is_some(), rtl);
            }
        })
        .child(screen)
}

/// Where a dragged panel can go: a thin line, wider while dragging.
fn slot(b: Board, c: usize, i: usize) -> View {
    let here = computed(move || b.over.get() == Some(Slot::At(c, i)));
    view()
        .test_id(&format!("pos-slot-{c}-{i}"))
        .min_h(move || px(if b.drag.get().is_some() { 26.0 } else { 8.0 }))
        .my(px(2.0))
        .rounded(px(6.0))
        .border_2()
        .border_color(move || {
            if here.get() {
                Tone::Primary.color()
            } else if b.drag.get().is_some() {
                Tone::Border.color()
            } else {
                clear()
            }
        })
        .bg(move || {
            if here.get() {
                Tone::PrimarySoft.color()
            } else {
                clear()
            }
        })
        .on_mouse_enter(move |_: &MouseEvent| {
            if b.drag.get_untracked().is_some() {
                b.over.set(Some(Slot::At(c, i)));
            }
        })
        .on_press_up(move || b.drop_now())
        // Touch: a press on the slot puts the chosen panel there.
        .on_press(move || {
            if b.drag.get_untracked().is_some() {
                b.over.set(Some(Slot::At(c, i)));
                b.drop_now();
            }
        })
}

/// A new column at `at`: a strip at the side of a column.
fn new_column_slot(st: AppState, b: Board, at: usize) -> View {
    let here = computed(move || b.over.get() == Some(Slot::NewColumn(at)));
    let full = computed(move || b.draft.with(|l| l.columns.len() >= MAX_COLUMNS));
    view()
        .test_id(&format!("pos-newcol-{at}"))
        .w(move || {
            px(if b.drag.get().is_some() && !full.get() {
                44.0
            } else {
                10.0
            })
        })
        .mx(px(2.0))
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .border_2()
        .border_color(move || {
            if here.get() {
                Tone::Primary.color()
            } else if b.drag.get().is_some() && !full.get() {
                Tone::Border.color()
            } else {
                clear()
            }
        })
        .bg(move || {
            if here.get() {
                Tone::PrimarySoft.color()
            } else {
                clear()
            }
        })
        .on_mouse_enter(move |_: &MouseEvent| {
            if b.drag.get_untracked().is_some() && !full.get_untracked() {
                b.over.set(Some(Slot::NewColumn(at)));
            }
        })
        .on_press_up(move || b.drop_now())
        .on_press(move || {
            if b.drag.get_untracked().is_some() && !full.get_untracked() {
                b.over.set(Some(Slot::NewColumn(at)));
                b.drop_now();
            }
        })
        .child(when(
            move || b.drag.get().is_some() && !full.get(),
            move || icon(Icon::Plus, 16.0, || Tone::Primary).into_element(),
        ))
        .label(move || SharedString::from(st.tn("pos.layouts.new_column")))
}

/// The bar between two columns: drag it to share the width differently.
fn splitter(b: Board, c: usize, rtl: bool) -> View {
    let on = computed(move || b.resizing.get() == Some(Resize::Column(c)));
    view()
        .test_id(&format!("pos-split-{c}"))
        .w(px(12.0))
        .items_center()
        .justify_center()
        .cursor_pointer()
        // The drag is the bar's alone (mouse or finger) while it lasts.
        .claim_touches(TouchClaim::on_start())
        .on_responder_grant(move |e: &ResponderEvent| {
            b.start.set((e.page.x.0, e.page.y.0));
            b.resizing.set(Some(Resize::Column(c)));
        })
        .on_responder_move(move |e: &ResponderEvent| b.pointer(e.page, true, rtl))
        .on_responder_release(move |_: &ResponderEvent| b.resizing.set(None))
        .child(view().w(px(4.0)).h(px(64.0)).rounded_full().bg(move || {
            if on.get() {
                Tone::Primary.color()
            } else {
                Tone::BorderStrong.color()
            }
        }))
}

/// Above a column while editing: its width, narrower or wider, and what
/// can be added to it.
fn column_bar(st: AppState, b: Board, c: usize, width: u8) -> View {
    let add = move || {
        let items: Vec<MenuItem> = b.draft.with_untracked(|l| {
            l.missing()
                .into_iter()
                .map(|w| {
                    MenuItem::new(widget_name(st, w)).on_select(move || {
                        b.change(|l| {
                            l.add(c, w);
                        })
                    })
                })
                .collect()
        });
        drop(popup_menu(items));
    };
    view()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .mb(px(6.0))
        .px(px(6.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .bg(Tone::Subtle.color())
        .child(icon(Icon::Columns, 14.0, || Tone::Muted))
        .child(label_line(
            move || format!("{} · {width}", st.tn("pos.layouts.width")),
            ty::XS,
            FontWeight::SEMIBOLD,
            Tone::Muted,
        ))
        .child(
            icon_button(
                || Icon::Minus,
                st.t("pos.layouts.narrower"),
                Variant::Ghost,
                move || b.change(|l| l.resize_column(c, -1)),
            )
            .test_id(&format!("pos-col-narrow-{c}")),
        )
        .child(
            icon_button(
                || Icon::Plus,
                st.t("pos.layouts.wider"),
                Variant::Ghost,
                move || b.change(|l| l.resize_column(c, 1)),
            )
            .test_id(&format!("pos-col-wide-{c}")),
        )
        .child(view().flex_1())
        .child(when(
            move || {
                b.draft.with(|l| {
                    !l.missing().is_empty()
                        && l.columns
                            .get(c)
                            .is_some_and(|x| x.panels.len() < MAX_PANELS)
                })
            },
            move || {
                button(
                    Variant::Secondary,
                    Size::Sm,
                    Some(Icon::Plus),
                    st.t("pos.layouts.add"),
                    add,
                )
                .test_id(&format!("pos-col-add-{c}"))
                .into_element()
            },
        ))
}

/// A panel while editing: its handle and name, what may be done to it,
/// and its bottom edge to drag it taller.
#[allow(clippy::too_many_arguments)]
fn edit_frame(
    st: AppState,
    b: Board,
    c: usize,
    i: usize,
    p: Panel,
    columns: usize,
    panels: usize,
    content: View,
) -> View {
    let dragged = computed(move || b.drag.get() == Some((c, i)));
    let w = p.widget;
    let tool = |ic: Icon, key: &'static str, id: String, f: Box<dyn Fn()>| {
        icon_button(move || ic, st.t(key), Variant::Ghost, f).test_id(&id)
    };
    let mut bar = view()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .px(px(6.0))
        .py(px(2.0))
        .bg(Tone::PrimarySoft.color())
        .child(
            view()
                .test_id(&format!("pos-grip-{c}-{i}"))
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(6.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(Tone::Surface.color()))
                // Drag it (mouse) or press it, then a slot (touch).
                .on_press_down(move || {
                    b.drag.set(Some((c, i)));
                    b.over.set(None);
                })
                .on_press_up(move || b.drop_now())
                .child(icon(Icon::Grip, 16.0, || Tone::Primary))
                .child(label_line(
                    move || widget_name(st, w),
                    ty::SM,
                    FontWeight::BOLD,
                    Tone::Primary,
                )),
        )
        .child(view().flex_1());
    // Its style (products: cards, list, tiles…).
    if !w.styles().is_empty() {
        let n = w.styles().len() as u8;
        let code = p.style_code();
        bar = bar.child(
            button(
                Variant::Ghost,
                Size::Sm,
                Some(Icon::Sliders),
                move || st.tn(&format!("pos.style.{code}")),
                move || {
                    b.change(|l| {
                        if let Some(p) = l.columns.get_mut(c).and_then(|c| c.panels.get_mut(i)) {
                            p.style = (p.style + 1) % n;
                        }
                    })
                },
            )
            .test_id(&format!("pos-style-{c}-{i}")),
        );
    }
    let height = p.height;
    bar = bar
        .child(label_line(
            move || {
                if height == 0 {
                    st.tn("pos.layouts.auto")
                } else {
                    format!("↕ {height}")
                }
            },
            ty::XS,
            FontWeight::MEDIUM,
            Tone::Muted,
        ))
        .child(tool(
            Icon::Minus,
            "pos.layouts.shorter",
            format!("pos-short-{c}-{i}"),
            Box::new(move || b.change(|l| l.resize_panel((c, i), -1))),
        ))
        .child(tool(
            Icon::Plus,
            "pos.layouts.taller",
            format!("pos-tall-{c}-{i}"),
            Box::new(move || {
                b.change(|l| {
                    if let Some(p) = l.columns.get_mut(c).and_then(|c| c.panels.get_mut(i))
                        && p.height == 0
                    {
                        p.height = 2;
                    }
                    l.resize_panel((c, i), 1)
                })
            }),
        ));
    if i > 0 {
        bar = bar.child(tool(
            Icon::ArrowUp,
            "templates.up",
            format!("pos-up-{c}-{i}"),
            Box::new(move || {
                b.change(|l| {
                    l.move_panel((c, i), (c, i - 1));
                })
            }),
        ));
    }
    if i + 1 < panels {
        bar = bar.child(tool(
            Icon::ArrowDown,
            "templates.down",
            format!("pos-down-{c}-{i}"),
            Box::new(move || {
                b.change(|l| {
                    l.move_panel((c, i), (c, i + 2));
                })
            }),
        ));
    }
    // To the column before or after (reading order).
    if c > 0 {
        bar = bar.child(tool(
            Icon::ChevronRight,
            "pos.layouts.col_before",
            format!("pos-prev-{c}-{i}"),
            Box::new(move || {
                b.change(|l| {
                    let at = l.columns[c - 1].panels.len();
                    l.move_panel((c, i), (c - 1, at));
                })
            }),
        ));
    }
    if c + 1 < columns {
        bar = bar.child(tool(
            Icon::ChevronLeft,
            "pos.layouts.col_after",
            format!("pos-next-{c}-{i}"),
            Box::new(move || {
                b.change(|l| {
                    let at = l.columns[c + 1].panels.len();
                    l.move_panel((c, i), (c + 1, at));
                })
            }),
        ));
    }
    if !matches!(w, Widget::Cart | Widget::Pay) {
        bar = bar.child(tool(
            Icon::Close,
            "action.delete",
            format!("pos-remove-{c}-{i}"),
            Box::new(move || {
                b.change(|l| {
                    l.remove((c, i));
                })
            }),
        ));
    }
    let resizing = computed(move || b.resizing.get() == Some(Resize::Panel(c, i)));
    let grows = p.height > 0;
    let frame = view()
        .test_id(&format!("pos-panel-{c}-{i}"))
        .rounded(px(10.0))
        .border_2()
        .opacity(move || if dragged.get() { 0.45_f32 } else { 1.0_f32 })
        // Over a panel: the dragged one goes before it.
        .on_mouse_enter(move |_: &MouseEvent| {
            if b.drag.get_untracked().is_some_and(|d| d != (c, i)) {
                b.over.set(Some(Slot::At(c, i)));
            }
        })
        .border_color(move || {
            if (b.over.get() == Some(Slot::At(c, i)) && b.drag.get().is_some()) || dragged.get() {
                Tone::Primary.color()
            } else {
                Tone::BorderStrong.color()
            }
        })
        .overflow_hidden()
        .child(bar)
        .child(
            view()
                .p(px(6.0))
                .flex_1()
                .min_h(px(0.0))
                .child(content.flex_1()),
        )
        // Its bottom edge: drag it taller or shorter.
        .child(
            view()
                .test_id(&format!("pos-edge-{c}-{i}"))
                .h(px(10.0))
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(move || {
                    if resizing.get() {
                        Tone::PrimarySoft.color()
                    } else {
                        Tone::Subtle.color()
                    }
                })
                .claim_touches(TouchClaim::on_start())
                .on_responder_grant(move |e: &ResponderEvent| {
                    b.start.set((e.page.x.0, e.page.y.0));
                    b.resizing.set(Some(Resize::Panel(c, i)));
                })
                .on_responder_move(move |e: &ResponderEvent| b.pointer(e.page, true, false))
                .on_responder_release(move |_: &ResponderEvent| b.resizing.set(None))
                .child(
                    view()
                        .w(px(40.0))
                        .h(px(3.0))
                        .rounded_full()
                        .bg(Tone::BorderStrong.color()),
                ),
        );
    let _ = grows;
    frame
}

/// While editing: the layout's name, its look, the ready-made layouts to
/// start from, cancel and save.
fn edit_bar(st: AppState, b: Board) -> View {
    let tiles: Vec<(u8, Box<dyn Fn() -> String>)> = vec![
        (0, Box::new(st.t("pos.tile.s"))),
        (1, Box::new(st.t("pos.tile.m"))),
        (2, Box::new(st.t("pos.tile.l"))),
    ];
    let tile = signal(b.draft.get_untracked().tile);
    let stock = signal(b.draft.get_untracked().stock);
    let sku = signal(b.draft.get_untracked().sku);
    let gap = signal(b.draft.get_untracked().gap);
    let head = signal(b.draft.get_untracked().head);
    let big = signal(b.draft.get_untracked().big_lines);
    effect(move || {
        let now = (
            tile.get(),
            stock.get(),
            sku.get(),
            gap.get(),
            head.get(),
            big.get(),
        );
        // Only a change rebuilds the board (which builds this bar again).
        let same = b
            .draft
            .with_untracked(|l| (l.tile, l.stock, l.sku, l.gap, l.head, l.big_lines) == now);
        if !same {
            b.draft.update(|l| {
                (l.tile, l.stock, l.sku, l.gap, l.head, l.big_lines) = now;
            });
        }
    });
    let gaps: Vec<(u8, Box<dyn Fn() -> String>)> = [6u8, 10, 14, 20]
        .into_iter()
        .map(|g| {
            (
                g,
                Box::new(move || g.to_string()) as Box<dyn Fn() -> String>,
            )
        })
        .collect();
    let start_from = move || {
        let items: Vec<MenuItem> = builtin()
            .into_iter()
            .map(|l| {
                let label = st.tn(&format!("pos.layout.{}", l.name));
                MenuItem::new(label).on_select(move || {
                    tile.set(l.tile);
                    stock.set(l.stock);
                    sku.set(l.sku);
                    gap.set(l.gap);
                    head.set(l.head);
                    big.set(l.big_lines);
                    b.draft.set(l.clone());
                })
            })
            .collect();
        drop(popup_menu(items));
    };
    let toggle = |key: &'static str, on: Signal<bool>, id: &'static str| {
        view()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(switch(on, st.t(key)).test_id(id))
            .child(label_line(
                st.t(key),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Body,
            ))
    };
    view()
        .test_id("pos-edit-bar")
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(12.0))
        .p(px(12.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(Tone::Primary.color())
        .bg(Tone::PrimarySoft.color())
        .child(icon(Icon::LayoutTemplate, 18.0, || Tone::Primary))
        .child(view().w(px(200.0)).child(field(
            String::new,
            b.name,
            st.t("pos.layouts.name"),
            String::new,
            KeyboardType::Default,
            "pos-layout-name",
        )))
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                Some(Icon::LayoutTemplate),
                st.t("pos.layouts.start_from"),
                start_from,
            )
            .test_id("pos-start-from"),
        )
        .child(segmented(tiles, tile).test_id("pos-edit-tile"))
        .child(segmented(gaps, gap).test_id("pos-edit-gap"))
        .child(toggle("pos.show.stock", stock, "pos-edit-stock"))
        .child(toggle("pos.show.sku", sku, "pos-edit-sku"))
        .child(toggle("pos.show.head", head, "pos-edit-head"))
        .child(toggle("pos.show.big_lines", big, "pos-edit-big-lines"))
        .child(view().flex_1())
        .child(
            label(
                st.t("pos.layouts.hint"),
                ty::XS,
                FontWeight::NORMAL,
                Tone::Muted,
            )
            .max_w(px(260.0)),
        )
        .child(
            button(
                Variant::Secondary,
                Size::Md,
                None,
                st.t("form.cancel"),
                move || b.editing.set(false),
            )
            .test_id("pos-edit-cancel"),
        )
        .child(
            button(
                Variant::Primary,
                Size::Md,
                Some(Icon::Check),
                st.t("form.save"),
                move || b.save(st),
            )
            .test_id("pos-edit-save"),
        )
}

/// The layouts: the ready-made ones and the cashier's, which one shows,
/// edit one (a copy of a ready-made one), delete one.
pub(super) fn layouts_dialog(st: AppState, t: Till, close: impl Fn() + Clone + 'static) -> View {
    let b = t.board;
    let shut = close.clone();
    let list = dynamic(move || {
        let close = shut.clone();
        let k = Board::kept(st);
        let current = k.current();
        let mut rows: Vec<(String, String, Layout, bool)> = builtin()
            .into_iter()
            .map(|l| {
                (
                    format!("{BUILTIN}{}", l.name),
                    st.tn(&format!("pos.layout.{}", l.name)),
                    l,
                    true,
                )
            })
            .collect();
        rows.extend(
            k.own
                .iter()
                .map(|l| (l.name.clone(), l.name.clone(), l.clone(), false)),
        );
        let mut v = view().gap(px(8.0));
        for (i, (id, name, l, ready)) in rows.into_iter().enumerate() {
            let on = if ready {
                k.chosen.is_empty() && i == 0 || k.chosen == id
            } else {
                current == l && k.chosen == id
            };
            let help = if ready {
                st.tn(&format!("pos.layout.{}.help", l.name))
            } else {
                st.tn("templates.own")
            };
            let (id2, l2, name2) = (id.clone(), l.clone(), name.clone());
            let close2 = close.clone();
            let mut actions = view().flex_row().items_center().gap(px(4.0));
            if on {
                actions = actions.child(badge(
                    Status::Success,
                    Some(Icon::Check),
                    st.t("templates.in_use"),
                ));
            } else {
                let id = id.clone();
                actions = actions.child(
                    button(
                        Variant::Secondary,
                        Size::Sm,
                        None,
                        st.t("templates.use"),
                        move || {
                            let mut k = Board::kept(st);
                            k.chosen = id.clone();
                            st.biz.pos.layouts.set(k.to_json());
                        },
                    )
                    .test_id(&format!("pos-layout-use-{i}")),
                );
            }
            actions = actions.child(
                icon_button(
                    || Icon::Pencil,
                    st.t("pos.layouts.edit"),
                    Variant::Ghost,
                    move || {
                        // A ready-made layout is edited as a copy of its own.
                        let name = if ready {
                            nb_i18n::tr_args(st.lang(), "templates.copy_of", &[("name", &name2)])
                        } else {
                            name2.clone()
                        };
                        b.edit(l2.clone(), name);
                        close2();
                    },
                )
                .test_id(&format!("pos-layout-edit-{i}")),
            );
            if !ready {
                actions = actions.child(
                    icon_button(
                        || Icon::Trash,
                        st.t("action.delete"),
                        Variant::Ghost,
                        move || {
                            let mut k = Board::kept(st);
                            k.forget(&id2);
                            st.biz.pos.layouts.set(k.to_json());
                        },
                    )
                    .test_id(&format!("pos-layout-delete-{i}")),
                );
            }
            v = v.child(
                view()
                    .test_id(&format!("pos-layout-{i}"))
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .p(px(12.0))
                    .rounded(px(10.0))
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
                    .child(miniature(&l))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap(px(2.0))
                            .child(label_line(
                                move || name.clone(),
                                ty::MD,
                                FontWeight::SEMIBOLD,
                                Tone::Ink,
                            ))
                            .child(label(
                                move || help.clone(),
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
    let body = view().gap(px(14.0)).child(list).child(
        view()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(
                switch(st.biz.pos.big_total, st.t("pos.show.big_total")).test_id("pos-big-total"),
            )
            .child(label(
                st.t("pos.show.big_total.phone"),
                ty::SM,
                FontWeight::NORMAL,
                Tone::Body,
            )),
    );
    let footer = view().child(
        button(
            Variant::Primary,
            Size::Md,
            Some(Icon::Check),
            st.t("action.done"),
            close.clone(),
        )
        .test_id("pos-customize-done"),
    );
    dialog_frame(
        st,
        st.t("pos.layouts"),
        st.t("pos.layouts.help"),
        body,
        footer,
        close,
    )
}

/// A small drawing of a layout: its columns and panels at their sizes.
fn miniature(l: &Layout) -> View {
    let mut row = view()
        .w(px(96.0))
        .h(px(60.0))
        .flex_row()
        .gap(px(3.0))
        .p(px(4.0))
        .rounded(px(6.0))
        .bg(Tone::Subtle.color())
        .border_1()
        .border_color(Tone::Border.color());
    for c in &l.columns {
        let mut col = view()
            .flex_grow(f32::from(c.width))
            .basis(px(0.0))
            .gap(px(3.0));
        for p in &c.panels {
            let tone = match p.widget {
                Widget::Products => Tone::Primary,
                Widget::Cart => Tone::Ink,
                Widget::Pay | Widget::BigTotal => Tone::Success,
                _ => Tone::BorderStrong,
            };
            let grow = if p.height == 0 {
                1.0
            } else {
                f32::from(p.height)
            };
            col = col.child(
                view()
                    .flex_grow(grow)
                    .basis(px(0.0))
                    .min_h(px(4.0))
                    .rounded(px(2.0))
                    .bg(tone.color()),
            );
        }
        row = row.child(col);
    }
    row
}

/// The categories as a column of buttons (beside tiles).
pub(super) fn category_column(st: AppState, t: Till) -> View {
    let cats = resource(
        move || st.session.get().map(|_| st.data_rev.get()),
        move |_, _| {
            let db = st.db();
            async move {
                db.call(|s| nb_store::usecases::catalog::list_categories(s))
                    .await
            }
        },
    );
    let item = move |id: Option<String>, name: String, test: String| {
        let on = {
            let id = id.clone();
            computed(move || t.category.get() == id)
        };
        view()
            .test_id(&test)
            .min_h(px(48.0))
            .px(px(12.0))
            .justify_center()
            .rounded(px(radius::md()))
            .border_1()
            .border_color(move || {
                if on.get() {
                    Tone::Primary.color()
                } else {
                    Tone::Border.color()
                }
            })
            .bg(move || {
                if on.get() {
                    Tone::PrimarySoft.color()
                } else {
                    Tone::Surface.color()
                }
            })
            .cursor_pointer()
            .on_press(move || t.category.set(id.clone()))
            .child(label(
                move || name.clone(),
                ty::SM,
                FontWeight::SEMIBOLD,
                Tone::Ink,
            ))
    };
    card()
        .p(px(8.0))
        .child(scroll().flex_1().min_h(px(0.0)).child(dynamic(move || {
            let list = cats.get().unwrap_or_default();
            let mut v = view().gap(px(6.0));
            v = v.child(item(
                None,
                st.tn("pos.categories.all"),
                "pos-catcol-all".into(),
            ));
            for (i, c) in list
                .into_iter()
                .filter(|c| c.parent_id.is_none())
                .enumerate()
            {
                v = v.child(item(Some(c.id), c.name, format!("pos-catcol-{i}")));
            }
            v.into_element()
        })))
}

/// Digits for the selected line: type a number, then set it as the
/// quantity or the price.
pub(super) fn line_keypad(st: AppState, t: Till) -> View {
    let value = signal(String::new());
    let key = move |k: &'static str| {
        view()
            .test_id(&format!("pad-{k}"))
            .flex_1()
            .h(px(46.0))
            .items_center()
            .justify_center()
            .rounded(px(radius::md()))
            .border_1()
            .border_color(Tone::Border.color())
            .bg(Tone::Surface.color())
            .cursor_pointer()
            .hover(|s| s.bg(Tone::Subtle.color()))
            .on_press(move || {
                value.update(|v| match k {
                    "⌫" => {
                        v.pop();
                    }
                    "C" => v.clear(),
                    _ => {
                        if k != "." || !v.contains('.') {
                            v.push_str(k);
                        }
                    }
                })
            })
            .child(figure(
                move || k.to_owned(),
                ty::LG,
                FontWeight::SEMIBOLD,
                || Tone::Ink,
            ))
    };
    let set = move |price: bool| {
        move || {
            let Some(i) = t.selected.get_untracked() else {
                show_toast(st, st.tn("pos.keypad.select"));
                return;
            };
            let v = value.get_untracked();
            let c = currency(st);
            let ok = if price {
                parse_price(&v, c).ok().map(|p| {
                    t.cart.update(|x| {
                        if let Some(l) = x.get_mut(i) {
                            l.price_minor = p.minor();
                        }
                    })
                })
            } else {
                parse_qty(&v).filter(|q| *q > 0).map(|q| {
                    t.cart.update(|x| {
                        if let Some(l) = x.get_mut(i) {
                            l.qty_milli = q;
                        }
                    })
                })
            };
            if ok.is_some() {
                value.set(String::new());
            } else {
                show_toast(st, error_text(st, "invalid_value"));
            }
        }
    };
    let mut pad = view().gap(px(6.0)).direction(LayoutDirection::Ltr);
    for row in [
        ["7", "8", "9", "⌫"],
        ["4", "5", "6", "C"],
        ["1", "2", "3", "."],
    ] {
        let mut r = view().flex_row().gap(px(6.0));
        for k in row {
            r = r.child(key(k));
        }
        pad = pad.child(r);
    }
    pad = pad.child(
        view()
            .flex_row()
            .gap(px(6.0))
            .child(key("0"))
            .child(key("00")),
    );
    view()
        .gap(px(8.0))
        .child(
            view()
                .h(px(40.0))
                .px(px(12.0))
                .justify_center()
                .items_end()
                .rounded(px(radius::md()))
                .bg(Tone::Subtle.color())
                .child(figure(
                    move || value.get(),
                    ty::XL,
                    FontWeight::BOLD,
                    || Tone::Ink,
                )),
        )
        .child(pad)
        .child(
            view()
                .flex_row()
                .gap(px(6.0))
                .child(
                    button(
                        Variant::Secondary,
                        Size::Md,
                        None,
                        st.t("pos.keypad.qty"),
                        set(false),
                    )
                    .test_id("pad-qty")
                    .flex_1(),
                )
                .child(
                    button(
                        Variant::Secondary,
                        Size::Md,
                        None,
                        st.t("pos.keypad.price"),
                        set(true),
                    )
                    .test_id("pad-price")
                    .flex_1(),
                ),
        )
}

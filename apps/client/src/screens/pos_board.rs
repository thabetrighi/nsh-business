//! The till's screen built from its layout ([`crate::screens::pos_layout`])
//! and edited in place: a panel is dragged by its handle to a slot (between
//! panels, or a new column), columns are resized by dragging the bar
//! between them, panels by their bottom edge; buttons do the same for touch
//! screens (move, taller/shorter, wider/narrower, remove, add, style).

use std::rc::Rc;

use crate::screens::pos_layout::{
    BUILTIN, Kept, Layout, MAX_COLUMNS, MAX_PANELS, Panel, Widget, builtin,
};

use nsh_core::element::{ResponderEvent, TouchClaim};

use crate::screens::pos_layout::{ALIGNS, COLORS, LABELS, Place, Spot, TEXT_SIZES};
use crate::ui::menu::{Act, menu_on};
use nsh_core::element::ElementRef;

use super::*;

/// What a drag can be dropped on: a place (a slot between panels, a new
/// column, between strips), or a panel (before it or after it, by which
/// half the pointer is over).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Place(Place),
    Panel(Spot),
}

/// A target where it is on the screen: (target, left, top, width, height).
type Rect = (Target, f32, f32, f32, f32);

thread_local! {
    /// The drop targets on the board now (built again with it).
    static TARGETS: std::cell::RefCell<Vec<(Target, ElementRef)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Where they are on the screen, measured when a drag starts: (target,
    /// left, top, width, height).
    static RECTS: std::cell::RefCell<Vec<Rect>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Makes `v` a drop target.
fn target(v: View, t: Target) -> View {
    let r = element_ref();
    TARGETS.with(|x| x.borrow_mut().push((t, r)));
    v.element_ref(&r)
}

/// Where a target sends what is dropped at `y` over it.
fn place_of(t: Target, top: f32, h: f32, y: f32) -> Option<Place> {
    let after = y > top + h / 2.0;
    match t {
        Target::Place(p) => Some(p),
        Target::Panel(Spot::Col(c, i)) => Some(Place::Col(c, i + usize::from(after))),
        Target::Panel(Spot::Top(i)) => Some(Place::Strip(true, i + usize::from(after))),
        Target::Panel(Spot::Bottom(i)) => Some(Place::Strip(false, i + usize::from(after))),
    }
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
    /// The panel being dragged (or picked, on a touch screen).
    pub drag: Signal<Option<Spot>>,
    pub over: Signal<Option<Place>>,
    /// Where the pointer is while dragging, in the board.
    pub ghost: Signal<Option<(f32, f32)>>,
    /// The board on the screen (where the ghost is drawn from).
    pub root: ElementRef,
    pub origin: Signal<(f32, f32)>,
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
            ghost: signal(None),
            root: element_ref(),
            origin: signal((0.0, 0.0)),
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
        self.ghost.set(None);
        if let Some(to) = over {
            self.change(|l| {
                l.move_to(from, to);
            });
        }
    }

    /// A drag starts at `page` (a panel's handle): where the targets are
    /// is measured once, then the pointer is followed.
    fn start_drag(self, from: Spot, page: Point<Points>) {
        self.drag.set(Some(from));
        self.over.set(None);
        self.start.set((page.x.0, page.y.0));
        let refs = TARGETS.with(|t| t.borrow().clone());
        let root = self.root;
        let task = spawn(async move {
            let mut rects = Vec::with_capacity(refs.len());
            for (t, r) in refs {
                if let Some(m) = r.measure().await {
                    rects.push((
                        t,
                        m.page.x.0,
                        m.page.y.0,
                        m.frame.size.width.0,
                        m.frame.size.height.0,
                    ));
                }
            }
            RECTS.with(|x| *x.borrow_mut() = rects);
            if let Some(m) = root.measure().await {
                self.origin.set((m.page.x.0, m.page.y.0));
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    }

    /// The pointer moved while dragging: the ghost follows, the place
    /// under it lights.
    fn follow(self, page: Point<Points>) {
        if self.drag.get_untracked().is_none() {
            return;
        }
        let (x, y) = (page.x.0, page.y.0);
        let (x0, y0) = self.start.get_untracked();
        // A press without a move picks the panel (touch: then a slot).
        if self.ghost.get_untracked().is_none() && (x - x0).abs() + (y - y0).abs() < 6.0 {
            return;
        }
        self.ghost.set(Some((x, y)));
        // Places first (they lie between and beside the panels).
        let found = RECTS.with(|r| {
            let r = r.borrow();
            let inside = |&&(_, l, t, w, h): &&Rect| x >= l && x <= l + w && y >= t && y <= t + h;
            r.iter()
                .filter(|e| matches!(e.0, Target::Place(_)))
                .find(inside)
                .or_else(|| r.iter().find(inside))
                .and_then(|&(t, _, top, _, h)| place_of(t, top, h, y))
        });
        if self.over.get_untracked() != found {
            self.over.set(found);
        }
    }

    /// The press ended: a drag drops; a press that did not move keeps the
    /// panel picked for a slot to be pressed (touch).
    fn release(self) {
        if self.ghost.get_untracked().is_some() {
            self.drop_now();
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
    /// The favourite products, in a style.
    pub favorites: Rc<dyn Fn(u8) -> View>,
}

/// A panel's content at its type size: a larger or smaller panel has its
/// scale provided above it, which its text, icons and controls follow
/// (also when parts of it are made again later).
fn panel_view(st: AppState, t: Till, l: &Layout, p: &Panel, parts: &BoardParts, n: usize) -> View {
    let k = p.scale();
    if (k - 1.0).abs() < f32::EPSILON {
        return widget(st, t, l, p, parts, n);
    }
    // Only a panel that takes the height it is given fills its place.
    let grows = p.widget.grows() || p.height > 0;
    let (l, p, parts) = (l.clone(), p.clone(), parts.clone());
    let wrap = view().min_h(px(0.0));
    let wrap = if grows { wrap.flex_1() } else { wrap };
    wrap.child(dynamic(move || {
        provide_context(crate::ui::tokens::Scale(k));
        let v = untracked(|| widget(st, t, &l, &p, &parts, n));
        let v = if grows { v.flex_1().min_h(px(0.0)) } else { v };
        v.into_element()
    }))
}

/// A panel's content (`n`: which of its kind, for buttons in several
/// places).
fn widget(st: AppState, t: Till, l: &Layout, p: &Panel, parts: &BoardParts, n: usize) -> View {
    let has = |w: Widget| l.shows(w);
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
            {
                let items = p.shown_items();
                let on = |c: &str| items.iter().any(|i| i == c);
                CartParts {
                    head: on("head"),
                    actions: on("actions"),
                    cols: Cols::of(&items),
                    sticky: on("sticky"),
                    ..CartParts::of(
                        Some(p.style == 1),
                        on("customer") && !has(Widget::Customer),
                        false,
                        false,
                    )
                }
            },
            false,
        ),
        Widget::Totals => {
            card()
                .pb(px(12.0))
                .child(cart_sums(st, t, false, false, !has(Widget::BigTotal)))
        }
        Widget::BigTotal => big_total(st, t, p),
        Widget::Pay => pay_panel(st, t, p, false),
        Widget::Customer => card().child(customer_chip(st, t, false)),
        Widget::Actions => card().child(actions_panel(st, t, p, n)),
        Widget::Keypad => card().p(px(12.0)).child(line_keypad(st, t)),
        Widget::Session => session_panel(st, t, p),
        Widget::CartBar => bottom_bar(st, t, p),
        Widget::HeldBar => held_bar(st, t, p),
        Widget::Favorites => (parts.favorites)(p.style),
        Widget::LastSale => last_sale(st, t, p),
        Widget::Clock => clock_panel(st, t, p),
    }
}

fn widget_name(st: AppState, w: Widget) -> String {
    st.tn(&format!("pos.widget.{}", w.code()))
}

fn widget_icon(w: Widget) -> Icon {
    match w {
        Widget::Search => Icon::Search,
        Widget::Categories => Icon::Layers,
        Widget::Products => Icon::Package,
        Widget::Cart => Icon::Cart,
        Widget::Totals => Icon::Receipt,
        Widget::BigTotal => Icon::Dollar,
        Widget::Pay => Icon::Banknote,
        Widget::Customer => Icon::User,
        Widget::Actions => Icon::Zap,
        Widget::Keypad => Icon::Keyboard,
        Widget::Session => Icon::Wallet,
        Widget::CartBar => Icon::PanelBottom,
        Widget::HeldBar => Icon::History,
        Widget::Favorites => Icon::Star,
        Widget::LastSale => Icon::Receipt,
        Widget::Clock => Icon::Clock,
    }
}

/// What may be set on a panel (`at`: where it is): its type size, how its
/// buttons read, what it shows and in what order, its style, colour and
/// alignment, whether it shows at all.
fn panel_acts(st: AppState, b: Board, at: Spot, p: &Panel) -> Vec<Act> {
    let set = move |f: Rc<dyn Fn(&mut Panel)>| {
        move || {
            let f = f.clone();
            b.change(move |l| {
                if let Some(p) = l.panel_mut(at) {
                    f(p);
                }
            })
        }
    };
    let w = p.widget;
    let mut acts = vec![Act::sub(
        Icon::TextSize,
        st.tn("pos.panel.text"),
        TEXT_SIZES
            .iter()
            .enumerate()
            .map(|(k, code)| {
                let k = u8::try_from(k).unwrap_or(0);
                Act::new(
                    st.tn(&format!("pos.panel.text.{code}")),
                    set(Rc::new(move |p: &mut Panel| p.text = k)),
                )
                .checked(p.text == k)
            })
            .collect(),
    )];
    if w.has_buttons() {
        acts.push(Act::sub(
            Icon::Type,
            st.tn("pos.panel.labels"),
            LABELS
                .iter()
                .enumerate()
                .map(|(k, code)| {
                    let k = u8::try_from(k).unwrap_or(0);
                    Act::new(
                        st.tn(&format!("pos.panel.labels.{code}")),
                        set(Rc::new(move |p: &mut Panel| p.labels = k)),
                    )
                    .checked(p.labels == k)
                })
                .collect(),
        ));
    }
    if !w.items().is_empty() {
        let shown = p.shown_items();
        acts.push(Act::sub(
            Icon::ClipboardCheck,
            st.tn("pos.panel.items"),
            w.items()
                .iter()
                .map(|code| {
                    let code: &'static str = code;
                    Act::new(
                        st.tn(&format!("pos.item.{}.{code}", w.code())),
                        set(Rc::new(move |p: &mut Panel| p.toggle_item(code))),
                    )
                    .checked(shown.iter().any(|s| s == code))
                })
                .collect(),
        ));
    }
    if !w.styles().is_empty() {
        acts.push(Act::sub(
            Icon::Sliders,
            st.tn("pos.panel.style"),
            w.styles()
                .iter()
                .enumerate()
                .map(|(k, code)| {
                    let k = u8::try_from(k).unwrap_or(0);
                    Act::new(
                        st.tn(&format!("pos.style.{code}")),
                        set(Rc::new(move |p: &mut Panel| p.style = k)),
                    )
                    .checked(p.style == k)
                })
                .collect(),
        ));
    }
    // The order of what it shows: each one earlier.
    if p.shown_items().len() > 1 {
        let shown = p.shown_items();
        acts.push(Act::sub(
            Icon::ArrowUpDown,
            st.tn("pos.panel.order"),
            shown
                .iter()
                .skip(1)
                .map(|code| {
                    let c = code.clone();
                    Act::new(
                        tr_args(
                            st.lang(),
                            "pos.panel.earlier",
                            &[("name", &st.tn(&format!("pos.item.{}.{code}", w.code())))],
                        ),
                        set(Rc::new(move |p: &mut Panel| p.move_item_up(&c))),
                    )
                })
                .collect(),
        ));
    }
    if w.colored() {
        acts.push(Act::sub(
            Icon::Palette,
            st.tn("pos.panel.color"),
            COLORS
                .iter()
                .enumerate()
                .map(|(k, code)| {
                    let k = u8::try_from(k).unwrap_or(0);
                    Act::new(
                        st.tn(&format!("pos.color.{code}")),
                        set(Rc::new(move |p: &mut Panel| p.color = k)),
                    )
                    .checked(p.color == k)
                })
                .collect(),
        ));
        acts.push(Act::sub(
            Icon::AlignCenter,
            st.tn("pos.panel.align"),
            ALIGNS
                .iter()
                .enumerate()
                .map(|(k, code)| {
                    let k = u8::try_from(k).unwrap_or(0);
                    Act::new(
                        st.tn(&format!("pos.align.{code}")),
                        set(Rc::new(move |p: &mut Panel| p.align = k)),
                    )
                    .checked(p.align == k)
                })
                .collect(),
        ));
    }
    // Its height in a column: its content's, what is left, or a share.
    if let Spot::Col(..) = at {
        let mut hs = vec![
            Act::new(
                st.tn("pos.panel.height.content"),
                set(Rc::new(|p: &mut Panel| {
                    p.height = 0;
                    p.fill = false;
                })),
            )
            .checked(p.height == 0 && !p.fill),
            Act::new(
                st.tn("pos.panel.height.fill"),
                set(Rc::new(|p: &mut Panel| {
                    p.height = 0;
                    p.fill = true;
                })),
            )
            .checked(p.fill),
            Act::Line,
        ];
        for h in [2u8, 3, 4, 6, 8, 10, 12] {
            hs.push(
                Act::new(
                    tr_args(
                        st.lang(),
                        "pos.panel.height.share",
                        &[("n", &h.to_string())],
                    ),
                    set(Rc::new(move |p: &mut Panel| {
                        p.height = h;
                        p.fill = false;
                    })),
                )
                .checked(!p.fill && p.height == h),
            );
        }
        acts.push(Act::sub(Icon::ArrowUpDown, st.tn("pos.panel.height"), hs));
    }
    acts.push(Act::Line);
    acts.push(Act::with(
        if p.hidden { Icon::Eye } else { Icon::EyeOff },
        st.tn(if p.hidden {
            "pos.panel.show"
        } else {
            "pos.panel.hide"
        }),
        set(Rc::new(|p: &mut Panel| p.hidden = !p.hidden)),
    ));
    if matches!(at, Spot::Top(_) | Spot::Bottom(_)) {
        acts.push(Act::with(Icon::ArrowUp, st.tn("pos.strip.up"), move || {
            b.change(|l| {
                l.move_strip(at, true);
            })
        }));
        acts.push(Act::with(
            Icon::ArrowDown,
            st.tn("pos.strip.down"),
            move || {
                b.change(|l| {
                    l.move_strip(at, false);
                })
            },
        ));
    }
    acts.push(
        Act::with(Icon::Trash, st.tn("pos.panel.remove"), move || {
            b.change(|l| {
                l.remove_at(at);
            })
        })
        .danger(),
    );
    acts
}

/// The options' button of a panel being edited.
fn options_button(st: AppState, b: Board, at: Spot, p: &Panel, id: String) -> View {
    menu_on(
        icon_button(
            || Icon::Settings,
            st.t("pos.panel.options"),
            Variant::Ghost,
            || {},
        )
        .test_id(&id),
        panel_acts(st, b, at, p),
    )
}

/// A strip while editing: where it is, its name, its options.
fn strip_frame(st: AppState, b: Board, at: Spot, p: Panel, content: View) -> View {
    let w = p.widget;
    let (id, place, ic) = match at {
        Spot::Top(i) => (format!("top-{i}"), "pos.strip.top", Icon::ArrowUp),
        Spot::Bottom(i) => (format!("bottom-{i}"), "pos.strip.bottom", Icon::PanelBottom),
        Spot::Col(c, i) => (format!("{c}-{i}"), "pos.strip.bottom", Icon::PanelBottom),
    };
    let hidden = p.hidden;
    let dragged = computed(move || b.drag.get() == Some(at));
    let arrow = |ic: Icon, key: &'static str, up: bool, tid: String| {
        icon_button(
            move || ic,
            st.t(key),
            Variant::Ghost,
            move || {
                b.change(|l| {
                    l.move_strip(at, up);
                })
            },
        )
        .test_id(&tid)
    };
    let frame = view()
        .test_id(&format!("pos-strip-{id}"))
        .opacity(move || if dragged.get() { 0.45_f32 } else { 1.0_f32 })
        .rounded(px(10.0))
        .border_2()
        .border_color(Tone::BorderStrong.color())
        .overflow_hidden()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(10.0))
                .py(px(4.0))
                .bg(Tone::PrimarySoft.color())
                .child(grip(st, b, at, w, format!("pos-strip-grip-{id}")))
                .child(icon(ic, 14.0, || Tone::Muted))
                .child(label_line(
                    move || {
                        if hidden {
                            format!("{} · {}", st.tn(place), st.tn("pos.panel.hidden"))
                        } else {
                            st.tn(place)
                        }
                    },
                    ty::XS,
                    FontWeight::MEDIUM,
                    Tone::Muted,
                ))
                .child(view().flex_1())
                .child(arrow(
                    Icon::ArrowUp,
                    "pos.strip.up",
                    true,
                    format!("pos-strip-up-{id}"),
                ))
                .child(arrow(
                    Icon::ArrowDown,
                    "pos.strip.down",
                    false,
                    format!("pos-strip-down-{id}"),
                ))
                .child(options_button(st, b, at, &p, format!("pos-opts-{id}"))),
        )
        .child(
            view()
                .p(px(6.0))
                .opacity(if hidden { 0.4 } else { 1.0 })
                .child(content),
        );
    target(frame, Target::Panel(at))
}

/// The till's screen from its layout; while editing, with the handles.
pub(super) fn board_view(st: AppState, t: Till, parts: BoardParts) -> View {
    let b = t.board;
    let rtl = st.lang().direction() == nb_i18n::Direction::Rtl;
    let screen = dynamic(move || {
        let l = b.shown(st);
        let editing = b.editing.get();
        // The targets are those of this board.
        TARGETS.with(|t| t.borrow_mut().clear());
        // What the products show follows the layout.
        let pr = st.biz.pos;
        if pr.tile.get_untracked() != usize::from(l.tile) {
            pr.tile.set(usize::from(l.tile));
        }
        if pr.cols.get_untracked() != usize::from(l.tile_cols) {
            pr.cols.set(usize::from(l.tile_cols));
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
        // The app's sidebar beside the till: as the layout says, and gone
        // in full screen.
        let focus = !l.sidebar || t.full.get();
        if st.focus.get_untracked() != focus {
            st.focus.set(focus);
        }
        // The app's bar at the top: as the layout says, and gone in full
        // screen (the till's quick menu reaches the pages).
        let bare = !l.navbar || t.full.get();
        if st.bare.get_untracked() != bare {
            st.bare.set(bare);
        }
        let gap = f32::from(l.gap);
        let n = l.columns.len();
        // Buttons may be in several places: each has its own test ids.
        let mut actions_seen = 0usize;
        // While editing the board scrolls: the columns are as tall as their
        // panels (nothing under the strips below them).
        let mut row = if editing {
            view().flex_row().flex_shrink(0.0)
        } else {
            view().flex_1().min_h(px(0.0)).flex_row()
        }
        .gap(px(if editing { 0.0 } else { gap }));
        for (c, col) in l.columns.iter().enumerate() {
            // A column with nothing shown is not there (but while editing).
            if !editing && col.panels.iter().all(|p| p.hidden) {
                continue;
            }
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
                let which = if p.widget == Widget::Actions {
                    actions_seen += 1;
                    actions_seen - 1
                } else {
                    0
                };
                if p.hidden && !editing {
                    continue;
                }
                let content = panel_view(st, t, &l, p, &parts, which);
                let content = if p.hidden {
                    content.opacity(0.4)
                } else {
                    content
                };
                let sized = if editing && (p.height > 0 || p.fill) {
                    // While editing the board scrolls: a share is a fixed height.
                    let h = if p.fill { 6 } else { p.height };
                    view()
                        .h(px(f32::from(h) * SHARE_H + 40.0))
                        .child(content.flex_1().min_h(px(0.0)))
                } else if p.fill {
                    // The height the others leave.
                    view()
                        .flex_1()
                        .basis(px(0.0))
                        .min_h(px(72.0))
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
                // What the search finds opens over the panels after it.
                let sized = if p.widget == Widget::Search {
                    sized.relative().z_index(30)
                } else {
                    sized
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
        // Strips across the whole width, above and below the columns.
        let mut strip = |at: Spot, p: &Panel| -> Option<View> {
            if p.hidden && !editing {
                return None;
            }
            let which = if p.widget == Widget::Actions {
                actions_seen += 1;
                actions_seen - 1
            } else {
                0
            };
            let content = panel_view(st, t, &l, p, &parts, which);
            let content = if p.widget == Widget::Search {
                content.relative().z_index(30)
            } else {
                content
            };
            Some(if editing {
                strip_frame(st, b, at, p.clone(), content)
            } else {
                content
            })
        };
        let mut tops: Vec<View> = Vec::new();
        for (i, p) in l.top.iter().enumerate() {
            if editing {
                tops.push(place_slot(
                    b,
                    Place::Strip(true, i),
                    format!("pos-slot-top-{i}"),
                ));
            }
            tops.extend(strip(Spot::Top(i), p));
        }
        if editing {
            let n = l.top.len();
            tops.push(place_slot(
                b,
                Place::Strip(true, n),
                format!("pos-slot-top-{n}"),
            ));
        }
        let mut bottoms: Vec<View> = Vec::new();
        for (i, p) in l.bottom.iter().enumerate() {
            if editing {
                bottoms.push(place_slot(
                    b,
                    Place::Strip(false, i),
                    format!("pos-slot-bottom-{i}"),
                ));
            }
            bottoms.extend(strip(Spot::Bottom(i), p));
        }
        if editing {
            let n = l.bottom.len();
            bottoms.push(place_slot(
                b,
                Place::Strip(false, n),
                format!("pos-slot-bottom-{n}"),
            ));
        }
        let mut v = view()
            .flex_1()
            .min_h(px(0.0))
            .gap(px(if editing { 10.0 } else { gap }));
        if editing {
            let mut inner = view().gap(px(4.0)).pb(px(24.0));
            for s in tops {
                inner = inner.child(s);
            }
            inner = inner.child(row);
            for s in bottoms {
                inner = inner.child(s);
            }
            v = v
                .child(edit_bar(st, b))
                .child(scroll().flex_1().min_h(px(0.0)).child(inner));
        } else {
            // The strips keep their height; the columns take the rest
            // (what does not fit scrolls inside its panel, under nothing).
            for s in tops {
                v = v.child(s.flex_shrink(0.0));
            }
            v = v.child(row.overflow_hidden());
            for s in bottoms {
                v = v.child(s.flex_shrink(0.0));
            }
        }
        v.into_element()
    });
    // What is being dragged, under the pointer.
    let ghost = dynamic(move || {
        let (Some((x, y)), Some(from)) = (b.ghost.get(), b.drag.get_untracked()) else {
            return view().into_element();
        };
        let (ox, oy) = b.origin.get();
        let Some(w) = b.draft.with_untracked(|l| l.panel(from).map(|p| p.widget)) else {
            return view().into_element();
        };
        view()
            .test_id("pos-drag-ghost")
            .absolute()
            .left(px(x - ox + 14.0))
            .top(px(y - oy + 14.0))
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(8.0))
            .bg(Tone::Primary.color())
            .shadow_lg()
            .opacity(0.92)
            .child(icon(widget_icon(w), 16.0, || Tone::OnPrimary))
            .child(label_line(
                move || widget_name(st, w),
                ty::SM,
                FontWeight::BOLD,
                Tone::OnPrimary,
            ))
            .into_element()
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .relative()
        .element_ref(&b.root)
        .on_layout(move |r: &Bounds<Points>| b.width.set(r.size.width.0))
        // A drag or resize follows the pointer over the whole board.
        .on_mouse_move(move |e: &MouseEvent| {
            if b.drag.get_untracked().is_some() || b.resizing.get_untracked().is_some() {
                b.pointer(e.page_position, e.pressed_button.is_some(), rtl);
            }
        })
        .child(screen)
        .child(ghost)
}

/// Where a dragged panel can go: a thin line, wider while dragging.
fn slot(b: Board, c: usize, i: usize) -> View {
    place_slot(b, Place::Col(c, i), format!("pos-slot-{c}-{i}"))
}

/// A place a panel can be dropped on, lit while the pointer is over it;
/// on a touch screen, pressed after the panel's handle.
fn place_slot(b: Board, place: Place, id: String) -> View {
    let here = computed(move || b.over.get() == Some(place));
    let v = view()
        .test_id(&id)
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
        // Touch: a press on the slot puts the chosen panel there.
        .on_press(move || {
            if b.drag.get_untracked().is_some() {
                b.over.set(Some(place));
                b.drop_now();
            }
        });
    target(v, Target::Place(place))
}

/// A new column at `at`: a strip at the side of a column.
fn new_column_slot(st: AppState, b: Board, at: usize) -> View {
    let here = computed(move || b.over.get() == Some(Place::NewColumn(at)));
    let full = computed(move || b.draft.with(|l| l.columns.len() >= MAX_COLUMNS));
    let v = view()
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
        .on_press(move || {
            if b.drag.get_untracked().is_some() && !full.get_untracked() {
                b.over.set(Some(Place::NewColumn(at)));
                b.drop_now();
            }
        })
        .child(when(
            move || b.drag.get().is_some() && !full.get(),
            move || icon(Icon::Plus, 16.0, || Tone::Primary).into_element(),
        ))
        .label(move || SharedString::from(st.tn("pos.layouts.new_column")));
    target(v, Target::Place(Place::NewColumn(at)))
}

/// A panel's handle: dragged (mouse or finger) it carries the panel; a
/// press without a move picks it for a slot to be pressed (touch).
fn grip(st: AppState, b: Board, from: Spot, w: Widget, id: String) -> View {
    let picked = computed(move || b.drag.get() == Some(from));
    view()
        .test_id(&id)
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(6.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(move || {
            if picked.get() {
                Tone::Surface.color()
            } else {
                clear()
            }
        })
        .hover(|s| s.bg(Tone::Surface.color()))
        .claim_touches(TouchClaim::on_start())
        .on_responder_grant(move |e: &ResponderEvent| {
            // A second press on a picked panel puts it down.
            if b.drag.get_untracked() == Some(from) && b.ghost.get_untracked().is_none() {
                b.drag.set(None);
                return;
            }
            b.start_drag(from, e.page);
        })
        .on_responder_move(move |e: &ResponderEvent| b.follow(e.page))
        .on_responder_release(move |_: &ResponderEvent| b.release())
        .on_responder_terminate(move |_: &ResponderEvent| {
            b.ghost.set(None);
            b.drag.set(None);
            b.over.set(None);
        })
        .child(icon(Icon::Grip, 16.0, || Tone::Primary))
        .child(label_line(
            move || widget_name(st, w),
            ty::SM,
            FontWeight::BOLD,
            Tone::Primary,
        ))
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
    let add_acts = move || -> Vec<Act> {
        b.draft.with_untracked(|l| {
            l.missing()
                .into_iter()
                .map(|w| {
                    Act::with(widget_icon(w), widget_name(st, w), move || {
                        b.change(|l| {
                            l.add(c, w);
                        })
                    })
                })
                .collect()
        })
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
                menu_on(
                    button(
                        Variant::Secondary,
                        Size::Sm,
                        Some(Icon::Plus),
                        st.t("pos.layouts.add"),
                        || {},
                    )
                    .test_id(&format!("pos-col-add-{c}")),
                    add_acts(),
                )
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
    let dragged = computed(move || b.drag.get() == Some(Spot::Col(c, i)));
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
        .child(grip(st, b, Spot::Col(c, i), w, format!("pos-grip-{c}-{i}")))
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
    bar = bar.child(options_button(
        st,
        b,
        Spot::Col(c, i),
        &p,
        format!("pos-opts-{c}-{i}"),
    ));
    let height = p.height;
    let fill = p.fill;
    bar = bar
        .child(label_line(
            move || {
                if fill {
                    st.tn("pos.panel.height.fill")
                } else if height == 0 {
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
        .border_color(move || {
            if dragged.get() {
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
    target(frame, Target::Panel(Spot::Col(c, i)))
}

/// While editing: the layout's name, its look, the ready-made layouts to
/// start from, cancel and save.
fn edit_bar(st: AppState, b: Board) -> View {
    use crate::screens::pos_layout::{HEAD_ITEMS, PADS, TILES, tile_key};
    let l = b.draft.get_untracked();
    let set = move |f: Rc<dyn Fn(&mut Layout)>| {
        move || {
            let f = f.clone();
            b.change(move |l| f(l))
        }
    };
    let flag = |key: &'static str, on: bool, f: fn(&mut Layout)| {
        Act::new(st.tn(key), set(Rc::new(move |l: &mut Layout| f(l)))).checked(on)
    };
    // The products' cards: size, how many across, what they show.
    let products = vec![
        Act::sub(
            Icon::Scaling,
            st.tn("pos.edit.card_size"),
            TILES
                .iter()
                .map(|&k| {
                    Act::new(
                        st.tn(tile_key(k)),
                        set(Rc::new(move |l: &mut Layout| l.tile = k)),
                    )
                    .checked(l.tile == k)
                })
                .collect(),
        ),
        Act::sub(
            Icon::Columns,
            st.tn("pos.edit.card_cols"),
            (0u8..=8)
                .filter(|n| *n != 1)
                .map(|n| {
                    Act::new(
                        if n == 0 {
                            st.tn("pos.layouts.auto")
                        } else {
                            n.to_string()
                        },
                        set(Rc::new(move |l: &mut Layout| l.tile_cols = n)),
                    )
                    .checked(l.tile_cols == n)
                })
                .collect(),
        ),
        Act::Line,
        flag("pos.show.stock", l.stock, |l| l.stock = !l.stock),
        flag("pos.show.sku", l.sku, |l| l.sku = !l.sku),
    ];
    // The page around the till.
    let pads = std::iter::once(None)
        .chain(PADS.iter().map(|p| Some(*p)))
        .map(|p| {
            Act::new(
                p.map_or_else(|| st.tn("pos.edit.pad_default"), |p| p.to_string()),
                set(Rc::new(move |l: &mut Layout| l.pad = p)),
            )
            .checked(l.pad == p)
        })
        .collect();
    let gaps = [4u8, 6, 10, 14, 20, 28]
        .into_iter()
        .map(|g| {
            Act::new(g.to_string(), set(Rc::new(move |l: &mut Layout| l.gap = g)))
                .checked(l.gap == g)
        })
        .collect();
    let page = vec![
        flag("pos.show.head", l.head, |l| l.head = !l.head),
        flag("pos.show.navbar", l.navbar, |l| l.navbar = !l.navbar),
        flag("pos.show.sidebar", l.sidebar, |l| l.sidebar = !l.sidebar),
        flag("pos.show.big_lines", l.big_lines, |l| {
            l.big_lines = !l.big_lines
        }),
        Act::Line,
        Act::sub(Icon::Frame, st.tn("pos.edit.pad"), pads),
        Act::sub(Icon::Columns, st.tn("pos.edit.gap"), gaps),
    ];
    // The head's buttons: which, in what order, how they read.
    let shown = l.head_shown();
    let mut head: Vec<Act> = HEAD_ITEMS
        .iter()
        .map(|&code| {
            Act::new(
                st.tn(&format!("pos.head.item.{code}")),
                set(Rc::new(move |l: &mut Layout| l.toggle_head(code))),
            )
            .checked(shown.iter().any(|s| s == code))
        })
        .collect();
    if shown.len() > 1 {
        head.push(Act::Line);
        head.push(Act::sub(
            Icon::ArrowUpDown,
            st.tn("pos.panel.order"),
            shown
                .iter()
                .skip(1)
                .map(|code| {
                    let c = code.clone();
                    Act::new(
                        tr_args(
                            st.lang(),
                            "pos.panel.earlier",
                            &[("name", &st.tn(&format!("pos.head.item.{code}")))],
                        ),
                        set(Rc::new(move |l: &mut Layout| l.head_up(&c))),
                    )
                })
                .collect(),
        ));
    }
    head.push(Act::sub(
        Icon::Type,
        st.tn("pos.panel.labels"),
        [
            "pos.head.labels.auto",
            "pos.panel.labels.icon",
            "pos.panel.labels.both",
        ]
        .into_iter()
        .enumerate()
        .map(|(k, key)| {
            let k = u8::try_from(k).unwrap_or(0);
            Act::new(
                st.tn(key),
                set(Rc::new(move |l: &mut Layout| l.head_labels = k)),
            )
            .checked(l.head_labels == k)
        })
        .collect(),
    ));
    // Strips to add, above or below the columns.
    let strip_acts = move |top: bool| -> Vec<Act> {
        b.draft.with_untracked(|l| {
            l.missing()
                .into_iter()
                .filter(|w| w.strip())
                .map(|w| {
                    Act::with(widget_icon(w), widget_name(st, w), move || {
                        b.change(|l| {
                            l.add_strip(top, w);
                        })
                    })
                })
                .collect()
        })
    };
    let start_acts: Vec<Act> = builtin()
        .into_iter()
        .map(|l| {
            let label = st.tn(&format!("pos.layout.{}", l.name));
            Act::with(Icon::LayoutTemplate, label, move || b.draft.set(l.clone()))
        })
        .collect();
    let menu = |ic: Icon, key: &'static str, id: &'static str, acts: Vec<Act>| {
        menu_on(
            button(Variant::Secondary, Size::Md, Some(ic), st.t(key), || {}).test_id(id),
            acts,
        )
    };
    view()
        .test_id("pos-edit-bar")
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .p(px(10.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(Tone::Primary.color())
        .bg(Tone::PrimarySoft.color())
        .child(icon(Icon::LayoutTemplate, 18.0, || Tone::Primary))
        .child(view().w(px(190.0)).child(field(
            String::new,
            b.name,
            st.t("pos.layouts.name"),
            String::new,
            KeyboardType::Default,
            "pos-layout-name",
        )))
        .child(menu(
            Icon::LayoutTemplate,
            "pos.layouts.start_from",
            "pos-start-from",
            start_acts,
        ))
        .child(menu(
            Icon::Package,
            "pos.edit.products",
            "pos-edit-products",
            products,
        ))
        .child(menu(Icon::Frame, "pos.edit.page", "pos-edit-page", page))
        .child(menu(
            Icon::PanelTop,
            "pos.edit.head",
            "pos-edit-head-items",
            head,
        ))
        .child(menu(
            Icon::PanelBottom,
            "pos.strip.add",
            "pos-add-strip",
            vec![
                Act::sub(Icon::ArrowUp, st.tn("pos.strip.add_top"), strip_acts(true)),
                Act::sub(
                    Icon::PanelBottom,
                    st.tn("pos.strip.add_bottom"),
                    strip_acts(false),
                ),
            ],
        ))
        .child(view().flex_1())
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
        .child(view().w_full().child(label(
            st.t("pos.layouts.hint"),
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        )))
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

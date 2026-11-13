//! The till's screen as a layout (owner notes): columns side by side, each
//! a stack of panels — the search, the categories, the products, the cart,
//! the totals, payment, the customer, quick actions, a keypad — with their
//! widths and heights. Ready-made layouts for a shop, a supermarket, a
//! restaurant, a touch screen; the cashier's own, edited on the screen
//! itself (drag a panel to another place, resize columns and panels),
//! kept on the device. A layout that cannot be read falls back to the
//! first ready-made one: the till always opens.

use serde::{Deserialize, Serialize};

/// What a panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Widget {
    /// Scan or search, with what is found under it.
    Search,
    Categories,
    Products,
    /// The cart's lines (with its header and the customer).
    Cart,
    /// Net, VAT, discounts and total.
    Totals,
    /// The total, large.
    BigTotal,
    /// Pay, and pay and print.
    Pay,
    Customer,
    /// Large buttons: discount, customer, hold, held, free item, clear.
    Actions,
    /// Digits for the selected line's quantity or price.
    Keypad,
    /// The till: its number and name, the shift, the cashier, what was
    /// sold, and closing it.
    Session,
    /// A strip: the cart as a badge (open it, clear it), its total, the
    /// customer, the carts on hold, paying at once.
    CartBar,
    /// The carts on hold as badges (each with its total; take it back with
    /// a press, drop it with its ×), and a button to hold this one.
    HeldBar,
    /// The products marked as favourites, to sell with a press.
    Favorites,
    /// The last sale: its number, total, the change given, its receipt.
    LastSale,
    /// The time, the date, the till.
    Clock,
}

impl Widget {
    pub const ALL: [Widget; 16] = [
        Widget::Search,
        Widget::Categories,
        Widget::Products,
        Widget::Cart,
        Widget::Totals,
        Widget::BigTotal,
        Widget::Pay,
        Widget::Customer,
        Widget::Actions,
        Widget::Keypad,
        Widget::Session,
        Widget::CartBar,
        Widget::HeldBar,
        Widget::Favorites,
        Widget::LastSale,
        Widget::Clock,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Widget::Search => "search",
            Widget::Categories => "categories",
            Widget::Products => "products",
            Widget::Cart => "cart",
            Widget::Totals => "totals",
            Widget::BigTotal => "big_total",
            Widget::Pay => "pay",
            Widget::Customer => "customer",
            Widget::Actions => "actions",
            Widget::Keypad => "keypad",
            Widget::Session => "session",
            Widget::CartBar => "cart_bar",
            Widget::HeldBar => "held_bar",
            Widget::Favorites => "favorites",
            Widget::LastSale => "last_sale",
            Widget::Clock => "clock",
        }
    }

    /// May be on the screen more than once (buttons in several places).
    pub fn repeatable(self) -> bool {
        matches!(self, Widget::Actions)
    }

    /// What it may show, each on or off, in order (empty: nothing to choose).
    pub fn items(self) -> &'static [&'static str] {
        match self {
            Widget::Actions => &[
                "discount",
                "customer",
                "hold",
                "held",
                "misc",
                "return",
                "clear",
                "sales",
                "prebill",
                "keys",
                "close",
                "fullscreen",
                "layouts",
            ],
            Widget::Pay => &["direct", "dialog", "print"],
            Widget::CartBar => &["count", "customer", "held", "till", "total", "clear", "pay"],
            Widget::Session => &["info", "totals", "sales", "close"],
            Widget::Cart => &[
                "head", "customer", "actions", "num", "sku", "qty", "price", "discount", "total",
                "sticky",
            ],
            Widget::BigTotal => &["label", "count", "customer"],
            Widget::HeldBar => &["title", "lines", "total", "hold"],
            Widget::LastSale => &["number", "total", "change", "print"],
            Widget::Clock => &["time", "date", "till", "cashier"],
            _ => &[],
        }
    }

    /// What it shows unless chosen otherwise.
    pub fn default_items(self) -> &'static [&'static str] {
        match self {
            Widget::Actions => &["discount", "customer", "hold", "held", "misc", "clear"],
            Widget::CartBar => &["count", "customer", "held", "total", "clear", "pay"],
            Widget::BigTotal => &["label"],
            Widget::Clock => &["time", "date"],
            other => other.items(),
        }
    }

    /// Whether it shows buttons (icon, text or both).
    pub fn has_buttons(self) -> bool {
        matches!(
            self,
            Widget::Actions
                | Widget::Pay
                | Widget::CartBar
                | Widget::Session
                | Widget::HeldBar
                | Widget::LastSale
        )
    }

    /// May be a strip across the screen, above or below the columns.
    pub fn strip(self) -> bool {
        !self.grows()
    }

    /// Takes a colour of its own (its background).
    pub fn colored(self) -> bool {
        !matches!(self, Widget::Search | Widget::Keypad)
    }

    /// Whether it scrolls its content (it takes the height it is given);
    /// the others are as tall as what they show unless sized.
    pub fn grows(self) -> bool {
        matches!(self, Widget::Products | Widget::Cart | Widget::Favorites)
    }

    /// The styles it offers (products: cards, list, tiles; cart: lines or
    /// a table).
    pub fn styles(self) -> &'static [&'static str] {
        match self {
            Widget::Products | Widget::Favorites => &["cards", "list", "tiles"],
            Widget::Actions => &["tiles", "row", "list"],
            Widget::Cart => &["lines", "table"],
            Widget::Categories => &["chips", "column"],
            _ => &[],
        }
    }
}

/// A panel: what it shows, its height (0: as tall as its content; else a
/// share of the column's height), its style, its type size, how its
/// buttons read and what it shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    pub widget: Widget,
    #[serde(default)]
    pub height: u8,
    #[serde(default)]
    pub style: u8,
    /// 0 normal, 1 small, 2 large, 3 larger.
    #[serde(default)]
    pub text: u8,
    /// Buttons: 0 icon and text, 1 icon only, 2 text only.
    #[serde(default)]
    pub labels: u8,
    /// What it shows (empty: its defaults), in its order.
    #[serde(default)]
    pub items: Vec<String>,
    /// Its colour (`COLORS`; 0: its own).
    #[serde(default)]
    pub color: u8,
    /// Where its content sits along it (`ALIGNS`).
    #[serde(default)]
    pub align: u8,
    /// Kept in the layout but not shown.
    #[serde(default)]
    pub hidden: bool,
    /// Takes the column's height left by the others.
    #[serde(default)]
    pub fill: bool,
}

/// The colours a panel may take.
pub const COLORS: [&str; 8] = [
    "auto", "primary", "dark", "success", "warning", "danger", "soft", "plain",
];
/// Where a panel's content may sit.
pub const ALIGNS: [&str; 3] = ["start", "center", "end"];

/// The type sizes a panel may have.
pub const TEXT_SIZES: [&str; 4] = ["normal", "small", "large", "larger"];
/// How a panel's buttons may read.
pub const LABELS: [&str; 3] = ["both", "icon", "text"];

impl Panel {
    pub fn new(widget: Widget) -> Self {
        Panel::sized(widget, if widget.grows() { 6 } else { 0 }, 0)
    }

    pub fn sized(widget: Widget, height: u8, style: u8) -> Self {
        Panel {
            widget,
            height,
            style,
            text: 0,
            labels: 0,
            items: Vec::new(),
            color: 0,
            align: 0,
            hidden: false,
            fill: false,
        }
    }

    /// Taking the height its column leaves.
    pub fn filling(mut self) -> Self {
        self.fill = true;
        self.height = 0;
        self
    }

    /// In this style (`Widget::styles`).
    pub fn style_of(mut self, style: u8) -> Self {
        self.style = style;
        self
    }

    /// In this colour (`COLORS`).
    pub fn tint(mut self, color: u8) -> Self {
        self.color = color;
        self
    }

    /// Its content placed (`ALIGNS`).
    pub fn placed(mut self, align: u8) -> Self {
        self.align = align;
        self
    }

    /// Moves `item` (shown) one place earlier.
    pub fn move_item_up(&mut self, item: &str) {
        let mut now = self.shown_items();
        if let Some(at) = now.iter().position(|i| i == item)
            && at > 0
        {
            now.swap(at, at - 1);
            self.items = now;
        }
    }

    /// With these type size and labels.
    pub fn look(mut self, text: u8, labels: u8) -> Self {
        self.text = text;
        self.labels = labels;
        self
    }

    /// Showing `items` (in this order).
    pub fn showing(mut self, items: &[&str]) -> Self {
        self.items = items.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    /// How much larger than the rest of the screen its type is.
    pub fn scale(&self) -> f32 {
        match self.text {
            1 => 0.88,
            2 => 1.2,
            3 => 1.45,
            _ => 1.0,
        }
    }

    /// What it shows, in order: its own choice, or its widget's defaults.
    pub fn shown_items(&self) -> Vec<String> {
        if self.items.is_empty() {
            self.widget
                .default_items()
                .iter()
                .map(|s| (*s).to_owned())
                .collect()
        } else {
            self.items
                .iter()
                .filter(|i| self.widget.items().contains(&i.as_str()))
                .cloned()
                .collect()
        }
    }

    /// Shows `item` or not (kept in the widget's order).
    pub fn toggle_item(&mut self, item: &str) {
        let mut now = self.shown_items();
        if let Some(at) = now.iter().position(|i| i == item) {
            // One thing at least stays.
            if now.len() > 1 {
                now.remove(at);
            }
        } else if self.widget.items().contains(&item) {
            // Back at its place among those shown.
            let order = self.widget.items();
            let rank = |x: &str| order.iter().position(|o| *o == x).unwrap_or(usize::MAX);
            let at = now
                .iter()
                .position(|i| rank(i) > rank(item))
                .unwrap_or(now.len());
            now.insert(at, item.to_owned());
        }
        self.items = now;
    }

    pub fn style_code(&self) -> &'static str {
        let s = self.widget.styles();
        s.get(usize::from(self.style))
            .or(s.first())
            .copied()
            .unwrap_or("")
    }
}

/// A column: its width (a share of the screen's) and its panels from the
/// top.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub width: u8,
    pub panels: Vec<Panel>,
}

/// A layout of the till.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// The cashier's name for it; a ready-made one's code.
    pub name: String,
    pub columns: Vec<Column>,
    /// Products' cards: 0 small, 1 medium, 2 large.
    #[serde(default = "medium")]
    pub tile: u8,
    #[serde(default = "yes")]
    pub stock: bool,
    #[serde(default)]
    pub sku: bool,
    /// Space between panels, in points.
    #[serde(default = "gap")]
    pub gap: u8,
    /// The page's title above the till.
    #[serde(default = "yes")]
    pub head: bool,
    /// Strips across the whole width above the columns (a total, the
    /// carts on hold, buttons), from the top.
    #[serde(default)]
    pub top: Vec<Panel>,
    /// Strips across the whole width below the columns.
    #[serde(default)]
    pub bottom: Vec<Panel>,
    /// An older layout's one strip at the bottom (read, then moved to
    /// `bottom`).
    #[serde(default, skip_serializing)]
    pub footer: Option<Panel>,
    /// The app's sidebar beside the till (off: the till takes the screen).
    #[serde(default = "yes")]
    pub sidebar: bool,
    /// The cart's lines in large type (touch screens, a customer display).
    #[serde(default)]
    pub big_lines: bool,
    /// The app's bar at the top (search, alerts, the user) beside the till.
    #[serde(default = "yes")]
    pub navbar: bool,
    /// The buttons beside the till's chip, in order (empty: the defaults).
    #[serde(default)]
    pub head_items: Vec<String>,
    /// How they read: 0 icons (closing the till in words), 1 icons, 2
    /// icons and words.
    #[serde(default)]
    pub head_labels: u8,
    /// The space around the till, in points (none: the app's own).
    #[serde(default)]
    pub pad: Option<u8>,
    /// Products' cards across (0: as many as fit).
    #[serde(default)]
    pub tile_cols: u8,
}

/// What the head of the till may show beside the till's chip, in order.
pub const HEAD_ITEMS: [&str; 16] = [
    "quick",
    "held",
    "sales",
    "return",
    "misc",
    "discount",
    "customer",
    "hold",
    "clear",
    "prebill",
    "layouts",
    "edit",
    "fullscreen",
    "keys",
    "theme",
    "close",
];

/// What it shows unless chosen otherwise.
pub const HEAD_DEFAULT: [&str; 10] = [
    "quick",
    "held",
    "sales",
    "return",
    "misc",
    "layouts",
    "edit",
    "fullscreen",
    "keys",
    "close",
];

/// The space around the till a layout may choose.
pub const PADS: [u8; 6] = [0, 8, 12, 16, 24, 32];

/// Cards' sizes, smallest first (the stored codes: 4 extra small, 0 small,
/// 1 medium, 2 large, 3 extra large).
pub const TILES: [u8; 5] = [4, 0, 1, 2, 3];

/// Card sizes' names (by stored code).
pub fn tile_key(tile: u8) -> &'static str {
    match tile {
        4 => "pos.tile.xs",
        0 => "pos.tile.s",
        2 => "pos.tile.l",
        3 => "pos.tile.xl",
        _ => "pos.tile.m",
    }
}

fn medium() -> u8 {
    1
}
fn yes() -> bool {
    true
}
fn gap() -> u8 {
    14
}

/// Where a dragged panel may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Column, place before which it goes.
    Col(usize, usize),
    /// A column of its own, at this place.
    NewColumn(usize),
    /// Above (`true`) or below the columns, at this place.
    Strip(bool, usize),
}

/// Where a panel is: a column and its place, or a strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spot {
    Col(usize, usize),
    Top(usize),
    Bottom(usize),
}

/// Limits that keep any layout usable.
pub const MAX_STRIPS: usize = 4;
pub const MAX_COLUMNS: usize = 4;
pub const MAX_PANELS: usize = 8;
pub const MAX_SHARE: u8 = 12;

impl Layout {
    fn of(name: &str, columns: Vec<Column>) -> Self {
        Layout {
            name: name.into(),
            columns,
            tile: 1,
            stock: true,
            sku: false,
            gap: 14,
            head: true,
            big_lines: false,
            top: Vec::new(),
            bottom: Vec::new(),
            footer: None,
            sidebar: true,
            navbar: true,
            head_items: Vec::new(),
            head_labels: 0,
            pad: None,
            tile_cols: 0,
        }
    }

    /// The head's buttons, in order.
    pub fn head_shown(&self) -> Vec<String> {
        if self.head_items.is_empty() {
            HEAD_DEFAULT.iter().map(|s| (*s).to_owned()).collect()
        } else {
            self.head_items
                .iter()
                .filter(|i| HEAD_ITEMS.contains(&i.as_str()))
                .cloned()
                .collect()
        }
    }

    /// Shows a head button or not (back at its place among those shown).
    pub fn toggle_head(&mut self, item: &str) {
        let mut now = self.head_shown();
        if let Some(at) = now.iter().position(|i| i == item) {
            now.remove(at);
        } else if HEAD_ITEMS.contains(&item) {
            let rank = |x: &str| {
                HEAD_ITEMS
                    .iter()
                    .position(|o| *o == x)
                    .unwrap_or(usize::MAX)
            };
            let at = now
                .iter()
                .position(|i| rank(i) > rank(item))
                .unwrap_or(now.len());
            now.insert(at, item.to_owned());
        }
        // An empty list means the defaults: none at all is kept apart.
        if now.is_empty() {
            now.push("none".into());
        }
        self.head_items = now;
    }

    /// A head button one place earlier.
    pub fn head_up(&mut self, item: &str) {
        let mut now = self.head_shown();
        if let Some(at) = now.iter().position(|i| i == item)
            && at > 0
        {
            now.swap(at, at - 1);
            self.head_items = now;
        }
    }

    /// Takes the panel at `at` out (none if it is not there).
    fn take(&mut self, at: Spot) -> Option<Panel> {
        match at {
            Spot::Col(c, i) => {
                let col = self.columns.get_mut(c)?;
                (i < col.panels.len()).then(|| col.panels.remove(i))
            }
            Spot::Top(i) => (i < self.top.len()).then(|| self.top.remove(i)),
            Spot::Bottom(i) => (i < self.bottom.len()).then(|| self.bottom.remove(i)),
        }
    }

    /// Moves a panel or a strip anywhere: into a column (before a panel),
    /// a new column, or the strips above or below. What the layout cannot
    /// hold (a list as a strip, too many) leaves it as it was.
    pub fn move_to(&mut self, from: Spot, to: Place) -> bool {
        let before = self.clone();
        let Some(p) = self.take(from) else {
            return false;
        };
        // Places after the one taken out move back by one.
        let shift = |same: bool, i: usize, taken: usize| if same && i > taken { i - 1 } else { i };
        match to {
            Place::Col(c, i) => {
                let i = match from {
                    Spot::Col(fc, fi) => shift(fc == c, i, fi),
                    _ => i,
                };
                match self.columns.get_mut(c) {
                    Some(col) => col.panels.insert(i.min(col.panels.len()), p),
                    None => {
                        *self = before;
                        return false;
                    }
                }
            }
            Place::NewColumn(at) => self.columns.insert(
                at.min(self.columns.len()),
                Column {
                    width: 4,
                    panels: vec![p],
                },
            ),
            Place::Strip(top, i) => {
                let i = match from {
                    Spot::Top(fi) if top => shift(true, i, fi),
                    Spot::Bottom(fi) if !top => shift(true, i, fi),
                    _ => i,
                };
                let list = if top { &mut self.top } else { &mut self.bottom };
                list.insert(i.min(list.len()), p);
            }
        }
        // A column left empty goes; its new column's place counts it.
        self.drop_empty();
        if self.check() {
            true
        } else {
            *self = before;
            false
        }
    }

    /// As it is used now (an older layout's footer becomes a strip).
    pub fn normalized(mut self) -> Self {
        if let Some(f) = self.footer.take()
            && self.bottom.is_empty()
        {
            self.bottom.push(f);
        }
        self
    }

    /// Every panel, the strips' too.
    fn panels(&self) -> impl Iterator<Item = &Panel> {
        self.top
            .iter()
            .chain(self.columns.iter().flat_map(|c| c.panels.iter()))
            .chain(self.bottom.iter())
    }

    /// Whether `w` is on the screen (shown).
    pub fn shows(&self, w: Widget) -> bool {
        self.panels().any(|p| p.widget == w && !p.hidden)
    }

    /// Whether it can be used: columns, strips and panels in number,
    /// shares in range, each widget once (but buttons), something shown.
    /// Nothing else is required: the cart and payment may go too (the keys
    /// still pay and the phone's sheet still shows the cart).
    pub fn check(&self) -> bool {
        let all: Vec<Widget> = self.panels().map(|p| p.widget).collect();
        let once = Widget::ALL
            .iter()
            .filter(|w| !w.repeatable())
            .all(|w| all.iter().filter(|x| *x == w).count() <= 1);
        let strips_ok = self.top.len() <= MAX_STRIPS
            && self.bottom.len() <= MAX_STRIPS
            && self
                .top
                .iter()
                .chain(&self.bottom)
                .all(|p| p.widget.strip());
        self.columns.len() <= MAX_COLUMNS
            && (!self.columns.is_empty() || !self.top.is_empty() || !self.bottom.is_empty())
            && self.columns.iter().all(|c| {
                (1..=MAX_SHARE).contains(&c.width)
                    && !c.panels.is_empty()
                    && c.panels.len() <= MAX_PANELS
                    && c.panels.iter().all(|p| p.height <= MAX_SHARE)
            })
            && self.panels().all(|p| {
                p.text <= 3
                    && p.labels <= 2
                    && usize::from(p.color) < COLORS.len()
                    && usize::from(p.align) < ALIGNS.len()
            })
            && strips_ok
            && once
            && self.tile <= 4
            && self.gap <= 32
            && self.head_labels <= 2
            && self.pad.is_none_or(|p| p <= 48)
            && self.tile_cols <= 8
    }

    /// The widgets that may be added (not on the screen yet, or that may
    /// be there more than once).
    pub fn missing(&self) -> Vec<Widget> {
        Widget::ALL
            .into_iter()
            .filter(|w| w.repeatable() || !self.panels().any(|p| p.widget == *w))
            .collect()
    }

    /// The panel at `at`.
    pub fn panel(&self, at: Spot) -> Option<&Panel> {
        match at {
            Spot::Col(c, i) => self.columns.get(c).and_then(|c| c.panels.get(i)),
            Spot::Top(i) => self.top.get(i),
            Spot::Bottom(i) => self.bottom.get(i),
        }
    }

    /// The panel at `at`.
    pub fn panel_mut(&mut self, at: Spot) -> Option<&mut Panel> {
        match at {
            Spot::Col(c, i) => self.columns.get_mut(c).and_then(|c| c.panels.get_mut(i)),
            Spot::Top(i) => self.top.get_mut(i),
            Spot::Bottom(i) => self.bottom.get_mut(i),
        }
    }

    /// Adds a strip showing `w` (above the columns or below them).
    pub fn add_strip(&mut self, top: bool, w: Widget) -> bool {
        let list = if top { &self.top } else { &self.bottom };
        if !w.strip() || list.len() >= MAX_STRIPS || !self.missing().contains(&w) {
            return false;
        }
        let p = Panel::new(w);
        if top {
            self.top.push(p);
        } else {
            self.bottom.push(p);
        }
        true
    }

    /// Moves a strip one place up or down, over the columns if needs be
    /// (the last at the top goes first at the bottom, and back).
    pub fn move_strip(&mut self, at: Spot, up: bool) -> bool {
        match (at, up) {
            (Spot::Top(i), true) if i > 0 => self.top.swap(i, i - 1),
            (Spot::Top(i), false) if i + 1 < self.top.len() => self.top.swap(i, i + 1),
            (Spot::Top(i), false) if i < self.top.len() && self.bottom.len() < MAX_STRIPS => {
                let p = self.top.remove(i);
                self.bottom.insert(0, p);
            }
            (Spot::Bottom(i), false) if i + 1 < self.bottom.len() => self.bottom.swap(i, i + 1),
            (Spot::Bottom(i), true) if i > 0 => self.bottom.swap(i, i - 1),
            (Spot::Bottom(0), true) if !self.bottom.is_empty() && self.top.len() < MAX_STRIPS => {
                let p = self.bottom.remove(0);
                self.top.push(p);
            }
            _ => return false,
        }
        true
    }

    /// Removes a panel or a strip; the screen keeps something.
    pub fn remove_at(&mut self, at: Spot) -> bool {
        let before = self.clone();
        let gone = match at {
            Spot::Col(c, i) => {
                if self.columns.get(c).is_none_or(|col| i >= col.panels.len()) {
                    return false;
                }
                self.columns[c].panels.remove(i);
                self.drop_empty();
                true
            }
            Spot::Top(i) if i < self.top.len() => {
                self.top.remove(i);
                true
            }
            Spot::Bottom(i) if i < self.bottom.len() => {
                self.bottom.remove(i);
                true
            }
            _ => false,
        };
        if gone && !self.check() {
            *self = before;
            return false;
        }
        gone
    }

    /// Moves the panel at `from` (column, index) to `to` (column, index
    /// before which it goes); an emptied column goes.
    pub fn move_panel(&mut self, from: (usize, usize), to: (usize, usize)) -> bool {
        let Some(col) = self.columns.get_mut(from.0) else {
            return false;
        };
        if from.1 >= col.panels.len() || to.0 >= self.columns.len() {
            return false;
        }
        if self.columns[to.0].panels.len() >= MAX_PANELS && to.0 != from.0 {
            return false;
        }
        let p = self.columns[from.0].panels.remove(from.1);
        let mut at = to.1;
        if to.0 == from.0 && at > from.1 {
            at -= 1;
        }
        let target = &mut self.columns[to.0].panels;
        target.insert(at.min(target.len()), p);
        self.drop_empty();
        true
    }

    fn drop_empty(&mut self) {
        self.columns.retain(|c| !c.panels.is_empty());
    }

    /// Removes the panel at (column, index).
    pub fn remove(&mut self, at: (usize, usize)) -> bool {
        self.remove_at(Spot::Col(at.0, at.1))
    }

    /// Adds `w` at the bottom of column `c`.
    pub fn add(&mut self, c: usize, w: Widget) -> bool {
        if !self.missing().contains(&w) {
            return false;
        }
        match self.columns.get_mut(c) {
            Some(col) if col.panels.len() < MAX_PANELS => {
                col.panels.push(Panel::new(w));
                true
            }
            _ => false,
        }
    }

    /// Widens or narrows column `c` by `delta` shares.
    pub fn resize_column(&mut self, c: usize, delta: i32) {
        if let Some(col) = self.columns.get_mut(c) {
            col.width = clamp_share(i32::from(col.width) + delta, 1);
        }
    }

    /// Makes a panel taller or shorter (0: as tall as its content).
    pub fn resize_panel(&mut self, at: (usize, usize), delta: i32) {
        if let Some(p) = self
            .columns
            .get_mut(at.0)
            .and_then(|c| c.panels.get_mut(at.1))
        {
            p.height = clamp_share(i32::from(p.height) + delta, 0);
        }
    }
}

fn clamp_share(v: i32, min: i32) -> u8 {
    u8::try_from(v.clamp(min, i32::from(MAX_SHARE))).unwrap_or(1)
}

fn col(width: u8, panels: Vec<Panel>) -> Column {
    Column { width, panels }
}

/// The ready-made layouts: (code, layout).
pub fn builtin() -> Vec<Layout> {
    use Widget as W;
    let p = Panel::new;
    let bar = |items: &[&str]| Panel::new(W::CartBar).showing(items);
    let big = |items: &[&str]| Panel::new(W::BigTotal).showing(items);
    vec![
        // A shop: products to pick, the cart beside them.
        Layout::of(
            "shop",
            vec![
                col(8, vec![p(W::Search), p(W::Categories), p(W::Products)]),
                col(4, vec![p(W::Cart), p(W::Totals), p(W::BigTotal), p(W::Pay)]),
            ],
        ),
        // A supermarket: scanning first, the cart as a table.
        Layout {
            stock: false,
            head: false,
            ..Layout::of(
                "supermarket",
                vec![
                    col(8, vec![p(W::Search), Panel::sized(W::Cart, 10, 1)]),
                    col(
                        4,
                        vec![
                            p(W::BigTotal),
                            p(W::Customer),
                            p(W::Totals),
                            p(W::Actions),
                            p(W::Pay).look(2, 0),
                        ],
                    ),
                ],
            )
        },
        // A restaurant or a café: large tiles by category.
        Layout {
            tile: 2,
            stock: false,
            ..Layout::of(
                "restaurant",
                vec![
                    col(2, vec![Panel::sized(W::Categories, 8, 1)]),
                    col(7, vec![p(W::Search), Panel::sized(W::Products, 8, 2)]),
                    col(4, vec![p(W::Cart), p(W::BigTotal), p(W::Pay)]),
                ],
            )
        },
        // A touch screen: large tiles, a keypad beside the cart.
        Layout {
            tile: 1,
            head: false,
            big_lines: true,
            ..Layout::of(
                "touch",
                vec![
                    col(5, vec![p(W::Categories), Panel::sized(W::Products, 6, 2)]),
                    col(4, vec![p(W::Search), p(W::Cart)]),
                    col(
                        3,
                        vec![
                            p(W::Keypad).look(2, 0),
                            p(W::BigTotal),
                            p(W::Pay).look(2, 0),
                        ],
                    ),
                ],
            )
        },
        // Compact: a list of products and a narrow cart.
        Layout {
            tile: 0,
            sku: true,
            ..Layout::of(
                "compact",
                vec![
                    col(7, vec![p(W::Search), Panel::sized(W::Products, 8, 1)]),
                    col(4, vec![p(W::Customer), p(W::Cart), p(W::Totals), p(W::Pay)]),
                ],
            )
        },
        // A café counter: tiles, the order beside them, a bar to pay at once.
        Layout {
            tile: 2,
            stock: false,
            head: false,
            bottom: vec![bar(&["count", "held", "total", "clear", "pay"])],
            ..Layout::of(
                "cafe",
                vec![
                    col(8, vec![p(W::Categories), Panel::sized(W::Products, 8, 2)]),
                    col(
                        4,
                        vec![
                            p(W::Cart).showing(&["head", "actions"]),
                            p(W::Actions)
                                .look(0, 1)
                                .showing(&["discount", "hold", "held", "misc", "clear"]),
                        ],
                    ),
                ],
            )
        },
        // Fast scanning: the cart's table across the screen, the buttons
        // below.
        Layout {
            stock: false,
            head: false,
            bottom: vec![Panel::new(W::Actions).style_of(1).showing(&[
                "customer", "discount", "hold", "held", "return", "misc", "clear",
            ])],
            ..Layout::of(
                "scanner",
                vec![
                    col(9, vec![p(W::Search), Panel::sized(W::Cart, 10, 1)]),
                    col(
                        3,
                        vec![
                            p(W::BigTotal),
                            p(W::Totals),
                            p(W::Keypad),
                            p(W::Pay).look(2, 0),
                        ],
                    ),
                ],
            )
        },
        // A pharmacy or a parts shop: codes in a list, the customer first.
        Layout {
            tile: 0,
            sku: true,
            ..Layout::of(
                "pharmacy",
                vec![
                    col(7, vec![p(W::Search), Panel::sized(W::Products, 8, 1)]),
                    col(
                        5,
                        vec![
                            p(W::Customer),
                            Panel::sized(W::Cart, 8, 1),
                            p(W::Totals),
                            p(W::Actions)
                                .look(1, 0)
                                .showing(&["discount", "hold", "held", "return", "prebill"]),
                            p(W::Pay),
                        ],
                    ),
                ],
            )
        },
        // A kiosk or a large screen: everything larger, the whole screen.
        Layout {
            tile: 2,
            stock: false,
            head: false,
            big_lines: true,
            sidebar: false,
            bottom: vec![bar(&["count", "total", "clear", "pay"]).look(2, 0)],
            ..Layout::of(
                "kiosk",
                vec![
                    col(
                        8,
                        vec![
                            p(W::Categories).look(2, 0),
                            Panel::sized(W::Products, 8, 2).look(2, 0),
                        ],
                    ),
                    col(
                        4,
                        vec![
                            p(W::Cart).look(2, 0).showing(&["head", "actions"]),
                            p(W::BigTotal),
                        ],
                    ),
                ],
            )
        },
        // A manager's till: the shift beside the cart.
        Layout::of(
            "manager",
            vec![
                col(7, vec![p(W::Search), p(W::Categories), p(W::Products)]),
                col(
                    5,
                    vec![
                        p(W::Session).look(1, 0),
                        p(W::Customer),
                        p(W::Cart),
                        p(W::Totals),
                        p(W::Pay),
                    ],
                ),
            ],
        ),
        // The total across the top, large, for the customer to read; the
        // carts on hold along the bottom.
        Layout {
            head: false,
            top: vec![
                big(&["label", "count", "customer"])
                    .look(3, 0)
                    .tint(2)
                    .placed(1),
            ],
            bottom: vec![p(W::HeldBar)],
            ..Layout::of(
                "total_top",
                vec![
                    col(8, vec![p(W::Search), p(W::Categories), p(W::Products)]),
                    col(4, vec![p(W::Cart), p(W::Totals), p(W::Pay)]),
                ],
            )
        },
        // Express: the whole screen, scanning into a table with a fixed
        // header, the total above, the buttons below.
        Layout {
            head: false,
            sidebar: false,
            stock: false,
            top: vec![big(&["label", "count"]).look(2, 0).tint(1).placed(2)],
            bottom: vec![Panel::new(W::Actions).style_of(1).look(0, 0).showing(&[
                "customer", "discount", "hold", "held", "return", "misc", "clear", "close",
            ])],
            ..Layout::of(
                "express",
                vec![
                    col(9, vec![p(W::Search), Panel::sized(W::Cart, 10, 1)]),
                    col(
                        3,
                        vec![
                            p(W::LastSale),
                            p(W::Keypad).look(2, 0),
                            p(W::Pay).look(2, 0),
                        ],
                    ),
                ],
            )
        },
        // A boutique: favourites first, the last sale at hand.
        Layout {
            tile: 2,
            ..Layout::of(
                "boutique",
                vec![
                    col(
                        7,
                        vec![
                            p(W::Search),
                            Panel::sized(W::Favorites, 5, 2),
                            Panel::sized(W::Products, 5, 0),
                        ],
                    ),
                    col(
                        5,
                        vec![
                            p(W::Customer),
                            p(W::Cart),
                            p(W::Totals),
                            p(W::LastSale),
                            p(W::Pay),
                        ],
                    ),
                ],
            )
        },
        // A bakery: categories down the side, favourites as tiles, the cart
        // bar along the bottom.
        Layout {
            tile: 2,
            stock: false,
            head: false,
            top: vec![p(W::HeldBar).showing(&["lines", "total", "hold"])],
            bottom: vec![bar(&["count", "customer", "total", "clear", "pay"])],
            ..Layout::of(
                "bakery",
                vec![
                    col(2, vec![Panel::sized(W::Categories, 8, 1)]),
                    col(6, vec![Panel::sized(W::Favorites, 8, 2)]),
                    col(4, vec![p(W::Cart).showing(&["head", "actions"])]),
                ],
            )
        },
        // Minimal: a scanner, the cart and the total; nothing else.
        Layout {
            head: false,
            sku: true,
            ..Layout::of(
                "minimal",
                vec![
                    col(
                        7,
                        vec![
                            p(W::Search),
                            Panel::sized(W::Cart, 10, 1)
                                .showing(&["num", "qty", "price", "total", "sticky"]),
                        ],
                    ),
                    col(
                        5,
                        vec![big(&["label", "count"]).look(3, 0), p(W::Pay).look(2, 0)],
                    ),
                ],
            )
        },
        // A counter with everything at hand: the time and the till above,
        // the carts on hold and the buttons below.
        Layout {
            head: false,
            sidebar: false,
            tile: 1,
            top: vec![
                p(W::Clock)
                    .showing(&["time", "date", "till", "cashier"])
                    .tint(6),
            ],
            bottom: vec![
                p(W::HeldBar),
                Panel::new(W::Actions).style_of(1).look(0, 1).showing(&[
                    "customer",
                    "discount",
                    "misc",
                    "return",
                    "sales",
                    "prebill",
                    "fullscreen",
                    "close",
                ]),
            ],
            ..Layout::of(
                "counter",
                vec![
                    col(7, vec![p(W::Search), p(W::Categories), p(W::Products)]),
                    col(5, vec![p(W::Cart), p(W::BigTotal), p(W::Pay)]),
                ],
            )
        },
        // A grocery: categories down the side, small cards, a keypad for
        // quantities.
        Layout {
            tile: 4,
            tile_cols: 0,
            head: false,
            ..Layout::of(
                "grocery",
                vec![
                    col(2, vec![Panel::sized(W::Categories, 0, 1).filling()]),
                    col(6, vec![p(W::Search), p(W::Products).filling()]),
                    col(
                        4,
                        vec![
                            p(W::Cart).filling(),
                            p(W::Keypad),
                            p(W::BigTotal),
                            p(W::Pay),
                        ],
                    ),
                ],
            )
        },
        // Fast food: very large tiles, the order number and total above.
        Layout {
            tile: 3,
            stock: false,
            head: false,
            sidebar: false,
            big_lines: true,
            top: vec![big(&["label", "count"]).look(2, 0).tint(1).placed(1)],
            ..Layout::of(
                "fast_food",
                vec![
                    col(
                        8,
                        vec![
                            p(W::Categories).look(2, 0),
                            Panel::sized(W::Products, 0, 2).filling(),
                        ],
                    ),
                    col(
                        4,
                        vec![
                            p(W::Cart).filling().showing(&["head", "actions"]),
                            p(W::Pay).look(2, 0),
                        ],
                    ),
                ],
            )
        },
        // Wholesale: the customer and their account first, the cart as a
        // table, quantities typed.
        Layout {
            sku: true,
            head: false,
            ..Layout::of(
                "wholesale",
                vec![
                    col(
                        8,
                        vec![
                            p(W::Customer),
                            p(W::Search),
                            Panel::sized(W::Cart, 0, 1).filling(),
                        ],
                    ),
                    col(
                        4,
                        vec![
                            p(W::Totals),
                            p(W::Keypad),
                            p(W::Actions)
                                .look(1, 0)
                                .showing(&["discount", "prebill", "hold", "held"]),
                            p(W::BigTotal),
                            p(W::Pay),
                        ],
                    ),
                ],
            )
        },
        // Electronics and phones: references in a list, the last sale and
        // the customer at hand.
        Layout {
            tile: 0,
            sku: true,
            ..Layout::of(
                "electronics",
                vec![
                    col(
                        7,
                        vec![p(W::Search), Panel::sized(W::Products, 0, 1).filling()],
                    ),
                    col(
                        5,
                        vec![
                            p(W::Customer),
                            p(W::Cart).filling(),
                            p(W::Totals),
                            p(W::LastSale),
                            p(W::Pay),
                        ],
                    ),
                ],
            )
        },
        // A butcher or a vegetable stall: weighed goods, a large keypad.
        Layout {
            tile: 2,
            stock: false,
            head: false,
            big_lines: true,
            ..Layout::of(
                "butcher",
                vec![
                    col(
                        6,
                        vec![p(W::Categories), Panel::sized(W::Products, 0, 2).filling()],
                    ),
                    col(3, vec![p(W::Keypad).look(3, 0).filling()]),
                    col(
                        3,
                        vec![p(W::Cart).filling(), p(W::BigTotal), p(W::Pay).look(2, 0)],
                    ),
                ],
            )
        },
        // A tablet: two columns of cards, the cart bar along the bottom.
        Layout {
            tile: 2,
            tile_cols: 2,
            head: false,
            sidebar: false,
            navbar: false,
            bottom: vec![bar(&["count", "customer", "total", "pay"]).look(2, 0)],
            ..Layout::of(
                "tablet",
                vec![
                    col(
                        7,
                        vec![p(W::Search), p(W::Categories), p(W::Products).filling()],
                    ),
                    col(5, vec![p(W::Cart).filling().showing(&["head", "actions"])]),
                ],
            )
        },
        // A bookshop or a stationer: names and references in a list.
        Layout {
            tile: 0,
            sku: true,
            ..Layout::of(
                "bookshop",
                vec![
                    col(
                        6,
                        vec![p(W::Search), Panel::sized(W::Products, 0, 1).filling()],
                    ),
                    col(
                        6,
                        vec![
                            Panel::sized(W::Cart, 0, 1).filling(),
                            p(W::Totals),
                            p(W::Pay),
                        ],
                    ),
                ],
            )
        },
        // A beauty shop: favourites as tiles, the customer and loyalty at
        // hand.
        Layout {
            tile: 1,
            ..Layout::of(
                "beauty",
                vec![
                    col(
                        7,
                        vec![
                            p(W::Search),
                            Panel::sized(W::Favorites, 4, 2),
                            p(W::Products).filling(),
                        ],
                    ),
                    col(
                        5,
                        vec![
                            p(W::Customer),
                            p(W::Cart).filling(),
                            p(W::Totals),
                            p(W::Pay),
                        ],
                    ),
                ],
            )
        },
        // A pro cashier: the search across the top, the cart in the
        // middle, every button along the bottom.
        Layout {
            head: false,
            stock: false,
            top: vec![p(W::Search)],
            bottom: vec![Panel::new(W::Actions).style_of(1).look(0, 0).showing(&[
                "customer", "discount", "hold", "held", "return", "misc", "prebill", "clear",
                "sales", "close",
            ])],
            ..Layout::of(
                "pro",
                vec![
                    col(8, vec![Panel::sized(W::Cart, 0, 1).filling()]),
                    col(
                        4,
                        vec![
                            p(W::BigTotal),
                            p(W::Totals),
                            p(W::Keypad).filling(),
                            p(W::Pay).look(2, 0),
                        ],
                    ),
                ],
            )
        },
        // A mini-market: the shop's cards with a bar to pay at once.
        Layout {
            tile: 0,
            bottom: vec![bar(&["count", "held", "customer", "total", "clear", "pay"])],
            ..Layout::of(
                "minimarket",
                vec![
                    col(
                        8,
                        vec![p(W::Search), p(W::Categories), p(W::Products).filling()],
                    ),
                    col(4, vec![p(W::Cart).filling(), p(W::Totals)]),
                ],
            )
        },
        // A wide screen: three columns — categories, products, the cart.
        Layout {
            tile: 1,
            ..Layout::of(
                "wide",
                vec![
                    col(2, vec![Panel::sized(W::Categories, 0, 1).filling()]),
                    col(6, vec![p(W::Search), p(W::Products).filling()]),
                    col(
                        4,
                        vec![
                            p(W::Customer),
                            p(W::Cart).filling(),
                            p(W::Totals),
                            p(W::BigTotal),
                            p(W::Pay),
                        ],
                    ),
                ],
            )
        },
        // A night shift: the clock and the session above, a quick scanner.
        Layout {
            head: false,
            stock: false,
            top: vec![p(W::Clock).showing(&["time", "till", "cashier"]).tint(4)],
            ..Layout::of(
                "night",
                vec![
                    col(8, vec![p(W::Search), Panel::sized(W::Cart, 0, 1).filling()]),
                    col(4, vec![p(W::Session), p(W::BigTotal), p(W::Pay).look(2, 0)]),
                ],
            )
        },
        // Self-service: very large type and buttons, nothing to arrange.
        Layout {
            tile: 3,
            stock: false,
            head: false,
            sidebar: false,
            navbar: false,
            big_lines: true,
            bottom: vec![bar(&["count", "total", "pay"]).look(3, 0)],
            head_items: vec!["fullscreen".into(), "keys".into()],
            ..Layout::of(
                "self_service",
                vec![
                    col(
                        8,
                        vec![
                            p(W::Search).look(2, 0),
                            Panel::sized(W::Products, 0, 2).filling(),
                        ],
                    ),
                    col(4, vec![p(W::Cart).look(2, 0).filling().showing(&["head"])]),
                ],
            )
        },
        // A delivery desk: the customer first, the carts on hold below.
        Layout {
            tile: 0,
            bottom: vec![p(W::HeldBar)],
            ..Layout::of(
                "delivery",
                vec![
                    col(
                        7,
                        vec![p(W::Search), p(W::Categories), p(W::Products).filling()],
                    ),
                    col(
                        5,
                        vec![
                            p(W::Customer),
                            p(W::Cart).filling(),
                            p(W::Actions)
                                .look(1, 0)
                                .showing(&["hold", "held", "prebill", "discount"]),
                            p(W::Pay),
                        ],
                    ),
                ],
            )
        },
    ]
}

/// The layouts the cashier keeps (theirs first, then the ready-made ones
/// they have not replaced) and which one shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Kept {
    #[serde(default)]
    pub own: Vec<Layout>,
    /// `builtin:<code>` or the name of one of theirs.
    #[serde(default)]
    pub chosen: String,
}

pub const BUILTIN: &str = "builtin:";

impl Kept {
    /// The layout the till shows: the chosen one if it reads, else the
    /// first ready-made one.
    pub fn current(&self) -> Layout {
        let all = builtin();
        let fallback = all[0].clone();
        if let Some(code) = self.chosen.strip_prefix(BUILTIN) {
            return all.into_iter().find(|l| l.name == code).unwrap_or(fallback);
        }
        self.own
            .iter()
            .map(|l| l.clone().normalized())
            .find(|l| l.name == self.chosen && l.check())
            .unwrap_or(fallback)
    }

    /// Keeps `l` under its name (replacing one of that name) and shows it.
    pub fn keep(&mut self, l: Layout) -> bool {
        if !l.check() || l.name.trim().is_empty() || l.name.starts_with(BUILTIN) {
            return false;
        }
        let name = l.name.clone();
        match self.own.iter_mut().find(|x| x.name == name) {
            Some(x) => *x = l,
            None => self.own.push(l),
        }
        self.chosen = name;
        true
    }

    pub fn forget(&mut self, name: &str) {
        self.own.retain(|l| l.name != name);
        if self.chosen == name {
            self.chosen.clear();
        }
    }

    pub fn parse(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ready_made_layout_can_be_used() {
        let all = builtin();
        assert!(all.len() >= 16);
        let names: std::collections::HashSet<&str> = all.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names.len(), all.len());
        for l in &all {
            assert!(l.check(), "{}", l.name);
            let back: Layout = serde_json::from_str(&serde_json::to_string(l).unwrap_or_default())
                .unwrap_or_else(|_| builtin().remove(1));
            assert_eq!(&back, l);
        }
    }

    #[test]
    fn panels_show_what_is_chosen_in_their_order() {
        let mut p = Panel::new(Widget::Actions);
        assert_eq!(
            p.shown_items(),
            ["discount", "customer", "hold", "held", "misc", "clear"]
        );
        p.toggle_item("return");
        p.toggle_item("customer");
        assert_eq!(
            p.shown_items(),
            ["discount", "hold", "held", "misc", "return", "clear"]
        );
        // Never nothing.
        let mut one = Panel::new(Widget::Pay).showing(&["direct"]);
        one.toggle_item("direct");
        assert_eq!(one.shown_items(), ["direct"]);
        assert!((Panel::new(Widget::Cart).look(3, 0).scale() - 1.45).abs() < 1e-6);
        // Buttons may be in several places; anything may go, the cart too,
        // so long as something shows.
        let mut l = builtin().remove(0);
        assert!(l.add(0, Widget::Actions) && l.add(1, Widget::Actions));
        assert!(l.check());
        let cart = l.columns[1]
            .panels
            .iter()
            .position(|p| p.widget == Widget::Cart);
        assert!(l.remove((1, cart.unwrap_or(0))));
        assert!(l.check() && !l.shows(Widget::Cart));
        // Strips above and below; one place up moves the first at the
        // bottom to the top.
        assert!(!l.add_strip(true, Widget::BigTotal));
        assert!(l.add_strip(true, Widget::HeldBar));
        assert!(l.add_strip(false, Widget::CartBar));
        assert!(!l.add_strip(false, Widget::Products));
        assert!(l.move_strip(Spot::Bottom(0), true));
        assert_eq!(l.top.len(), 2);
        assert_eq!(l.top[1].widget, Widget::CartBar);
        assert!(l.remove_at(Spot::Top(0)));
        assert!(l.check());
        // Items: order and place.
        let mut a = Panel::new(Widget::Actions);
        a.move_item_up("held");
        assert_eq!(a.shown_items()[2], "held");
        // An old layout (before these options) still reads.
        let old =
            r#"{"name":"x","columns":[{"width":4,"panels":[{"widget":"cart"},{"widget":"pay"}]}]}"#;
        let back: Layout = serde_json::from_str(old).unwrap_or_else(|_| builtin().remove(1));
        assert!(back.check() && back.bottom.is_empty() && back.columns[0].panels[0].text == 0);
        assert!(back.sidebar);
        // One kept with a footer (before strips) has it at the bottom.
        let footed = r#"{"name":"y","columns":[{"width":4,"panels":[{"widget":"cart"}]}],"footer":{"widget":"cart_bar"}}"#;
        let back: Layout = serde_json::from_str(footed).unwrap_or_else(|_| builtin().remove(1));
        let back = back.normalized();
        assert_eq!(back.bottom.len(), 1);
        assert!(back.footer.is_none());
    }

    #[test]
    fn panels_and_strips_go_anywhere_the_head_is_chosen() {
        let mut l = builtin().remove(0);
        let cols = l.columns.len();
        // A column's search above the columns, then below, then back.
        let at = l.columns[0]
            .panels
            .iter()
            .position(|p| p.widget == Widget::Search)
            .unwrap_or(0);
        assert!(l.move_to(Spot::Col(0, at), Place::Strip(true, 0)));
        assert_eq!(l.top[0].widget, Widget::Search);
        assert!(l.move_to(Spot::Top(0), Place::Strip(false, 0)));
        assert_eq!(l.bottom[0].widget, Widget::Search);
        assert!(l.move_to(Spot::Bottom(0), Place::Col(0, 0)));
        assert_eq!(l.columns[0].panels[0].widget, Widget::Search);
        assert_eq!(l.columns.len(), cols);
        // A list cannot be a strip: nothing moves.
        let before = l.clone();
        let list = l.columns.iter().enumerate().find_map(|(c, col)| {
            col.panels
                .iter()
                .position(|p| p.widget.grows())
                .map(|i| (c, i))
        });
        if let Some((c, i)) = list {
            assert!(!l.move_to(Spot::Col(c, i), Place::Strip(true, 0)));
            assert_eq!(l, before);
        }
        // The head: the defaults, one hidden, one moved, none at all.
        assert_eq!(l.head_shown().len(), HEAD_DEFAULT.len());
        l.toggle_head("keys");
        assert!(!l.head_shown().iter().any(|i| i == "keys"));
        l.toggle_head("theme");
        assert!(l.head_shown().iter().any(|i| i == "theme"));
        l.head_up("sales");
        assert_eq!(l.head_shown()[1], "sales");
        for i in l.head_shown() {
            l.toggle_head(&i);
        }
        assert!(l.head_shown().is_empty());
        assert!(l.check());
        let back: Layout = serde_json::from_str(&serde_json::to_string(&l).unwrap_or_default())
            .unwrap_or_else(|_| builtin().remove(1));
        assert_eq!(back, l);
    }

    #[test]
    fn panels_move_resize_and_the_till_keeps_its_cart() {
        let mut l = builtin().remove(0);
        // The categories (column 0, second) to the top of column 1.
        assert!(l.move_panel((0, 1), (1, 0)));
        assert_eq!(l.columns[1].panels[0].widget, Widget::Categories);
        // Within a column, after itself: one place down.
        assert!(l.move_panel((1, 0), (1, 2)));
        assert_eq!(l.columns[1].panels[1].widget, Widget::Categories);
        // Its own column, then back: the emptied column goes.
        assert!(l.move_to(Spot::Col(1, 1), Place::NewColumn(2)));
        assert_eq!(l.columns.len(), 3);
        assert!(l.move_panel((2, 0), (0, 0)));
        assert_eq!(l.columns.len(), 2);
        // Panels go and come back.
        assert!(l.remove((0, 0)));
        assert!(l.missing().contains(&Widget::Categories));
        assert!(l.add(0, Widget::Categories));
        assert!(!l.add(0, Widget::Categories));
        // Sizes stay in range.
        l.resize_column(0, 40);
        assert_eq!(l.columns[0].width, MAX_SHARE);
        l.resize_panel((0, 0), -5);
        assert_eq!(l.columns[0].panels[0].height, 0);
        assert!(l.check());
    }

    #[test]
    fn the_layout_shown_is_kept_or_the_first_ready_made_one() {
        let mut k = Kept::default();
        assert_eq!(k.current().name, "shop");
        k.chosen = format!("{BUILTIN}restaurant");
        assert_eq!(k.current().name, "restaurant");
        let mut mine = builtin().remove(1);
        mine.name = "Ma caisse".into();
        mine.columns.reverse();
        assert!(k.keep(mine.clone()));
        assert_eq!(k.current(), mine);
        let again = Kept::parse(&k.to_json());
        assert_eq!(again, k);
        // One with nothing on it cannot be kept; a damaged one is not shown.
        let mut bad = mine.clone();
        bad.columns.clear();
        assert!(!k.keep(bad));
        let damaged = k.to_json().replace("\"cart\"", "\"nope\"");
        assert_eq!(Kept::parse(&damaged).current().name, "shop");
        k.forget("Ma caisse");
        assert_eq!(k.current().name, "shop");
    }
}

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
}

impl Widget {
    pub const ALL: [Widget; 12] = [
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
                "discount", "customer", "hold", "held", "misc", "return", "clear", "sales",
                "prebill", "keys",
            ],
            Widget::Pay => &["direct", "dialog", "print"],
            Widget::CartBar => &["count", "customer", "held", "till", "total", "clear", "pay"],
            Widget::Session => &["info", "totals", "sales", "close"],
            _ => &[],
        }
    }

    /// What it shows unless chosen otherwise.
    pub fn default_items(self) -> &'static [&'static str] {
        match self {
            Widget::Actions => &["discount", "customer", "hold", "held", "misc", "clear"],
            Widget::CartBar => &["count", "customer", "held", "total", "clear", "pay"],
            other => other.items(),
        }
    }

    /// Whether it shows buttons (icon, text or both).
    pub fn has_buttons(self) -> bool {
        matches!(
            self,
            Widget::Actions | Widget::Pay | Widget::CartBar | Widget::Session
        )
    }

    /// Whether it scrolls its content (it takes the height it is given);
    /// the others are as tall as what they show unless sized.
    pub fn grows(self) -> bool {
        matches!(self, Widget::Products | Widget::Cart)
    }

    /// The styles it offers (products: cards, list, tiles; cart: lines or
    /// a table).
    pub fn styles(self) -> &'static [&'static str] {
        match self {
            Widget::Products => &["cards", "list", "tiles"],
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
}

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
            now.push(item.to_owned());
            let order = self.widget.items();
            now.sort_by_key(|i| order.iter().position(|o| o == i));
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
    /// A strip along the bottom, the whole width (the cart bar, buttons).
    #[serde(default)]
    pub footer: Option<Panel>,
    /// The cart's lines in large type (touch screens, a customer display).
    #[serde(default)]
    pub big_lines: bool,
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

/// Limits that keep any layout usable.
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
            footer: None,
        }
    }

    /// Every panel, the footer's too.
    fn panels(&self) -> impl Iterator<Item = &Panel> {
        self.columns
            .iter()
            .flat_map(|c| c.panels.iter())
            .chain(self.footer.iter())
    }

    /// Whether it can be used: columns and panels in number, shares in
    /// range, a cart and a way to pay somewhere, each widget once.
    pub fn check(&self) -> bool {
        let all: Vec<Widget> = self.panels().map(|p| p.widget).collect();
        let once = Widget::ALL
            .iter()
            .filter(|w| !w.repeatable())
            .all(|w| all.iter().filter(|x| *x == w).count() <= 1);
        let footer_ok = self
            .footer
            .as_ref()
            .is_none_or(|p| !p.widget.grows() && p.widget != Widget::Categories);
        !self.columns.is_empty()
            && self.columns.len() <= MAX_COLUMNS
            && self.columns.iter().all(|c| {
                (1..=MAX_SHARE).contains(&c.width)
                    && !c.panels.is_empty()
                    && c.panels.len() <= MAX_PANELS
                    && c.panels.iter().all(|p| p.height <= MAX_SHARE)
            })
            && self.panels().all(|p| p.text <= 3 && p.labels <= 2)
            && footer_ok
            && all.contains(&Widget::Cart)
            && (all.contains(&Widget::Pay) || self.pays_in_footer())
            && once
            && self.tile <= 2
            && self.gap <= 32
    }

    /// The widgets that may be added (not on the screen yet, or that may
    /// be there more than once).
    pub fn missing(&self) -> Vec<Widget> {
        Widget::ALL
            .into_iter()
            .filter(|w| w.repeatable() || !self.panels().any(|p| p.widget == *w))
            .collect()
    }

    /// The footer's cart bar pays (so the pay panel may go).
    pub fn pays_in_footer(&self) -> bool {
        self.footer.as_ref().is_some_and(|f| {
            f.widget == Widget::CartBar && f.shown_items().iter().any(|i| i == "pay")
        })
    }

    /// The panel at `at` (`None` column: the footer).
    pub fn panel_mut(&mut self, at: (Option<usize>, usize)) -> Option<&mut Panel> {
        match at.0 {
            Some(c) => self.columns.get_mut(c).and_then(|c| c.panels.get_mut(at.1)),
            None => self.footer.as_mut(),
        }
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

    /// A panel into a new column at `at` (its own column).
    pub fn split_off_column(&mut self, from: (usize, usize), at: usize) -> bool {
        if self.columns.len() >= MAX_COLUMNS {
            return false;
        }
        let Some(col) = self.columns.get_mut(from.0) else {
            return false;
        };
        if from.1 >= col.panels.len() {
            return false;
        }
        let p = col.panels.remove(from.1);
        self.columns.insert(
            at.min(self.columns.len()),
            Column {
                width: 4,
                panels: vec![p],
            },
        );
        self.drop_empty();
        true
    }

    fn drop_empty(&mut self) {
        self.columns.retain(|c| !c.panels.is_empty());
    }

    /// Removes a panel (never the cart or payment: the till needs them).
    pub fn remove(&mut self, at: (usize, usize)) -> bool {
        let Some(p) = self.columns.get(at.0).and_then(|c| c.panels.get(at.1)) else {
            return false;
        };
        if p.widget == Widget::Cart || (p.widget == Widget::Pay && !self.pays_in_footer()) {
            return false;
        }
        self.columns[at.0].panels.remove(at.1);
        self.drop_empty();
        true
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
    let bar = |items: &[&str]| Some(Panel::new(W::CartBar).showing(items));
    vec![
        // A shop: products to pick, the cart beside them.
        Layout::of(
            "shop",
            vec![
                col(8, vec![p(W::Search), p(W::Categories), p(W::Products)]),
                col(
                    4,
                    vec![
                        p(W::Cart),
                        p(W::Totals),
                        Panel::sized(W::BigTotal, 0, 0),
                        p(W::Pay),
                    ],
                ),
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
            footer: bar(&["count", "held", "total", "clear", "pay"]),
            ..Layout::of(
                "cafe",
                vec![
                    col(8, vec![p(W::Categories), Panel::sized(W::Products, 8, 2)]),
                    col(
                        4,
                        vec![
                            p(W::Cart),
                            p(W::Actions)
                                .look(0, 1)
                                .showing(&["discount", "hold", "held", "misc", "clear"]),
                        ],
                    ),
                ],
            )
        },
        // Fast scanning: the cart's table across the screen, the buttons
        // and the total below.
        Layout {
            stock: false,
            head: false,
            footer: Some(Panel::new(W::Actions).look(0, 0).showing(&[
                "customer", "discount", "hold", "held", "return", "misc", "clear",
            ])),
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
        // A kiosk or a large screen: everything larger.
        Layout {
            tile: 2,
            stock: false,
            head: false,
            big_lines: true,
            footer: bar(&["count", "total", "clear", "pay"]),
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
                    col(4, vec![p(W::Cart).look(2, 0), p(W::BigTotal)]),
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
            .find(|l| l.name == self.chosen && l.check())
            .cloned()
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
        assert!(all.len() >= 10);
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
        // Buttons may be in several places; a cart bar that pays stands
        // for the pay panel.
        let mut l = builtin().remove(0);
        assert!(l.add(0, Widget::Actions) && l.add(1, Widget::Actions));
        assert!(l.check());
        let pay = l.columns[1]
            .panels
            .iter()
            .position(|p| p.widget == Widget::Pay);
        assert!(!l.remove((1, pay.unwrap_or(0))));
        l.footer = Some(Panel::new(Widget::CartBar));
        assert!(l.remove((1, pay.unwrap_or(0))));
        assert!(l.check());
        // An old layout (before these options) still reads.
        let old =
            r#"{"name":"x","columns":[{"width":4,"panels":[{"widget":"cart"},{"widget":"pay"}]}]}"#;
        let back: Layout = serde_json::from_str(old).unwrap_or_else(|_| builtin().remove(1));
        assert!(back.check() && back.footer.is_none() && back.columns[0].panels[0].text == 0);
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
        assert!(l.split_off_column((1, 1), 2));
        assert_eq!(l.columns.len(), 3);
        assert!(l.move_panel((2, 0), (0, 0)));
        assert_eq!(l.columns.len(), 2);
        // The cart and payment stay; others go and come back.
        let cart = l.columns[1]
            .panels
            .iter()
            .position(|p| p.widget == Widget::Cart);
        assert!(!l.remove((1, cart.unwrap_or(0))));
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
        // One without a cart cannot be kept; a damaged one is not shown.
        let mut bad = mine.clone();
        bad.columns
            .iter_mut()
            .for_each(|c| c.panels.retain(|p| p.widget != Widget::Cart));
        assert!(!k.keep(bad));
        let damaged = k.to_json().replace("\"cart\"", "\"nope\"");
        assert_eq!(Kept::parse(&damaged).current().name, "shop");
        k.forget("Ma caisse");
        assert_eq!(k.current().name, "shop");
    }
}

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
}

impl Widget {
    pub const ALL: [Widget; 10] = [
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
        }
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
/// share of the column's height) and its style.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    pub widget: Widget,
    #[serde(default)]
    pub height: u8,
    #[serde(default)]
    pub style: u8,
}

impl Panel {
    pub fn new(widget: Widget) -> Self {
        Panel {
            widget,
            height: if widget.grows() { 6 } else { 0 },
            style: 0,
        }
    }

    pub fn sized(widget: Widget, height: u8, style: u8) -> Self {
        Panel {
            widget,
            height,
            style,
        }
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
        }
    }

    /// Whether it can be used: columns and panels in number, shares in
    /// range, a cart and a way to pay somewhere, each widget once.
    pub fn check(&self) -> bool {
        let all: Vec<Widget> = self
            .columns
            .iter()
            .flat_map(|c| c.panels.iter().map(|p| p.widget))
            .collect();
        let once = Widget::ALL
            .iter()
            .all(|w| all.iter().filter(|x| *x == w).count() <= 1);
        !self.columns.is_empty()
            && self.columns.len() <= MAX_COLUMNS
            && self.columns.iter().all(|c| {
                (1..=MAX_SHARE).contains(&c.width)
                    && !c.panels.is_empty()
                    && c.panels.len() <= MAX_PANELS
                    && c.panels.iter().all(|p| p.height <= MAX_SHARE)
            })
            && all.contains(&Widget::Cart)
            && all.contains(&Widget::Pay)
            && once
            && self.tile <= 2
            && self.gap <= 32
    }

    /// The widgets not on the screen yet (to add).
    pub fn missing(&self) -> Vec<Widget> {
        Widget::ALL
            .into_iter()
            .filter(|w| {
                !self
                    .columns
                    .iter()
                    .any(|c| c.panels.iter().any(|p| p.widget == *w))
            })
            .collect()
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
        if matches!(p.widget, Widget::Cart | Widget::Pay) {
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
                            p(W::Pay),
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
                    col(3, vec![p(W::Keypad), p(W::BigTotal), p(W::Pay)]),
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
        assert_eq!(all.len(), 5);
        for l in &all {
            assert!(l.check(), "{}", l.name);
            let back: Layout = serde_json::from_str(&serde_json::to_string(l).unwrap_or_default())
                .unwrap_or_else(|_| builtin().remove(1));
            assert_eq!(&back, l);
        }
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

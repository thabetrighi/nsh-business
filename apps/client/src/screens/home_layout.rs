//! The home dashboard as the person arranges it (owner notes): its cards in
//! order, each as wide as chosen (a share of twelve), shown or hidden;
//! cards added and removed; kept on the device. A layout that cannot be
//! read is the default one: home always opens.

use serde::{Deserialize, Serialize};

/// A card of the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Card {
    Revenue,
    Orders,
    Basket,
    Credit,
    /// Sales over the period, against the one before.
    Sales,
    Payments,
    Invoices,
    Categories,
    LowStock,
    /// What wants attention (the bell's list).
    Alerts,
    /// Ways to start: a sale, a product, a purchase, a customer…
    Quick,
    /// The customers who owe most, and how late.
    Debts,
    /// The suppliers owed most.
    Payables,
}

impl Card {
    pub const ALL: [Card; 13] = [
        Card::Revenue,
        Card::Orders,
        Card::Basket,
        Card::Credit,
        Card::Sales,
        Card::Payments,
        Card::Invoices,
        Card::Categories,
        Card::LowStock,
        Card::Alerts,
        Card::Quick,
        Card::Debts,
        Card::Payables,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Card::Revenue => "revenue",
            Card::Orders => "orders",
            Card::Basket => "basket",
            Card::Credit => "credit",
            Card::Sales => "sales",
            Card::Payments => "payments",
            Card::Invoices => "invoices",
            Card::Categories => "categories",
            Card::LowStock => "low_stock",
            Card::Alerts => "alerts",
            Card::Quick => "quick",
            Card::Debts => "debts",
            Card::Payables => "payables",
        }
    }

    /// Its width when added.
    pub fn width(self) -> u8 {
        match self {
            Card::Revenue | Card::Orders | Card::Basket | Card::Credit => 3,
            Card::Sales | Card::Invoices => 8,
            Card::Debts | Card::Payables => 6,
            _ => 4,
        }
    }
}

/// The widths a card may have (shares of twelve).
pub const WIDTHS: [u8; 5] = [3, 4, 6, 8, 12];

/// A card in its place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub card: Card,
    pub width: u8,
    #[serde(default)]
    pub hidden: bool,
}

impl Item {
    pub fn new(card: Card) -> Self {
        Item {
            card,
            width: card.width(),
            hidden: false,
        }
    }
}

/// The dashboard's cards, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Home {
    pub items: Vec<Item>,
}

impl Default for Home {
    fn default() -> Self {
        use Card as C;
        Home {
            items: [
                C::Revenue,
                C::Orders,
                C::Basket,
                C::Credit,
                C::Sales,
                C::Payments,
                C::Invoices,
                C::Categories,
                C::LowStock,
                C::Quick,
                C::Alerts,
            ]
            .into_iter()
            .map(Item::new)
            .collect(),
        }
    }
}

impl Home {
    /// Each card once, widths known.
    pub fn check(&self) -> bool {
        Card::ALL
            .iter()
            .all(|c| self.items.iter().filter(|i| i.card == *c).count() <= 1)
            && self.items.iter().all(|i| WIDTHS.contains(&i.width))
    }

    pub fn parse(json: &str) -> Self {
        serde_json::from_str::<Home>(json)
            .ok()
            .filter(Home::check)
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The cards not on it (to add).
    pub fn missing(&self) -> Vec<Card> {
        Card::ALL
            .into_iter()
            .filter(|c| !self.items.iter().any(|i| i.card == *c))
            .collect()
    }

    pub fn add(&mut self, c: Card) {
        if self.missing().contains(&c) {
            self.items.push(Item::new(c));
        }
    }

    pub fn remove(&mut self, at: usize) {
        if at < self.items.len() {
            self.items.remove(at);
        }
    }

    /// One place earlier (`up`) or later.
    pub fn shift(&mut self, at: usize, up: bool) {
        let to = if up { at.checked_sub(1) } else { Some(at + 1) };
        if let Some(to) = to
            && to < self.items.len()
            && at < self.items.len()
        {
            self.items.swap(at, to);
        }
    }

    pub fn set_width(&mut self, at: usize, w: u8) {
        if let Some(i) = self.items.get_mut(at)
            && WIDTHS.contains(&w)
        {
            i.width = w;
        }
    }

    pub fn toggle(&mut self, at: usize) {
        if let Some(i) = self.items.get_mut(at) {
            i.hidden = !i.hidden;
        }
    }

    /// The cards shown (all while editing) in rows of twelve: (place, item).
    pub fn rows(&self, editing: bool) -> Vec<Vec<(usize, Item)>> {
        let mut rows: Vec<Vec<(usize, Item)>> = Vec::new();
        let mut used = 0u8;
        for (at, i) in self.items.iter().enumerate() {
            if i.hidden && !editing {
                continue;
            }
            if used + i.width > 12 || rows.is_empty() {
                rows.push(Vec::new());
                used = 0;
            }
            used += i.width;
            if let Some(r) = rows.last_mut() {
                r.push((at, i.clone()));
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_flow_in_rows_of_twelve_and_are_arranged() {
        let mut h = Home::default();
        assert!(h.check());
        let rows = h.rows(false);
        // Four figures, then the sales and payments, then the invoices…
        assert_eq!(rows[0].len(), 4);
        assert_eq!(
            rows[1].iter().map(|(_, i)| i.card).collect::<Vec<_>>(),
            [Card::Sales, Card::Payments]
        );
        h.toggle(0);
        assert_eq!(h.rows(false)[0].len(), 3);
        assert_eq!(h.rows(true)[0].len(), 4);
        h.shift(1, true);
        assert_eq!(h.items[0].card, Card::Orders);
        h.set_width(0, 12);
        h.set_width(0, 5);
        assert_eq!(h.items[0].width, 12);
        assert!(h.missing().contains(&Card::Debts));
        h.add(Card::Debts);
        h.add(Card::Debts);
        assert!(h.check());
        let back = Home::parse(&h.to_json());
        assert_eq!(back, h);
        // A damaged one reads as the default.
        assert_eq!(
            Home::parse("{\"items\":[{\"card\":\"nope\"}]}"),
            Home::default()
        );
        h.remove(0);
        assert!(!h.items.iter().any(|i| i.card == Card::Orders));
    }
}

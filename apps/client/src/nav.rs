//! The app's map (PRD §13.4): every page, the section it belongs to, its
//! icon and its name. The sidebar, the phone's "More" sheet, the page tabs,
//! the breadcrumbs and the command palette are all built from this table.

use nb_domain::features::Feature;

use crate::ui::icon::Icon;

macro_rules! routes {
    ($($name:ident => $code:literal, $icon:ident;)*) => {
        /// A page of the app.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Route { $($name),* }

        impl Route {
            pub const ALL: &'static [Route] = &[$(Route::$name),*];

            /// Stable identifier (`NB_ROUTE`, palette ids).
            pub fn code(self) -> &'static str {
                match self { $(Route::$name => $code),* }
            }

            /// Message key of its name.
            pub fn key(self) -> &'static str {
                match self { $(Route::$name => concat!("nav.", $code)),* }
            }

            /// Message key of the line under its title.
            pub fn desc_key(self) -> &'static str {
                match self { $(Route::$name => concat!("nav.", $code, ".desc")),* }
            }

            pub fn icon(self) -> Icon {
                match self { $(Route::$name => Icon::$icon),* }
            }
        }
    };
}

routes! {
    Home => "home", Dashboard;
    Sell => "sell", Cart;
    // Sales
    Sales => "sales", Receipt;
    Quotes => "quotes", FileText;
    Returns => "returns", Undo;
    Promotions => "promotions", Percent;
    // Purchases
    Purchases => "purchases", Truck;
    PurchaseOrders => "purchase_orders", ClipboardList;
    Suppliers => "suppliers", Building;
    // Catalog
    Products => "products", Package;
    Categories => "categories", Layers;
    Units => "units", Ruler;
    PriceLists => "price_lists", Tags;
    // Inventory
    Inventory => "inventory", Boxes;
    Movements => "movements", ArrowsLR;
    Transfers => "transfers", Truck;
    StockCounts => "stock_counts", ClipboardCheck;
    Warehouses => "warehouses", Warehouse;
    // Customers and credit
    Customers => "customers", Users;
    Debts => "debts", Coins;
    Installments => "installments", CalendarClock;
    // Finance
    Cash => "cash", Wallet;
    Banks => "banks", Landmark;
    Expenses => "expenses", Banknote;
    Shifts => "shifts", Clock;
    // Accounting
    Journal => "journal", NotebookPen;
    Accounts => "accounts", ListTree;
    Ledger => "ledger", Book;
    TrialBalance => "trial_balance", Scale;
    // Distribution
    Tours => "tours", MapPin;
    Vans => "vans", Truck;
    FieldOrders => "field_orders", ClipboardList;
    // Reports
    Reports => "reports", ChartLine;
    StockReports => "stock_reports", ChartPie;
    FinanceReports => "finance_reports", FileChart;
    PartyReports => "party_reports", Users;
    // System
    Settings => "settings", Settings;
    Design => "design", Palette;
}

/// A group of pages in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    /// Message key of its name.
    pub key: &'static str,
    pub icon: Icon,
    pub routes: &'static [Route],
}

pub const SECTIONS: &[Section] = &[
    Section {
        key: "nav.sec.sales",
        icon: Icon::Receipt,
        routes: &[
            Route::Sales,
            Route::Quotes,
            Route::Returns,
            Route::Promotions,
        ],
    },
    Section {
        key: "nav.sec.purchases",
        icon: Icon::Truck,
        routes: &[Route::Purchases, Route::PurchaseOrders, Route::Suppliers],
    },
    Section {
        key: "nav.sec.catalog",
        icon: Icon::Package,
        routes: &[
            Route::Products,
            Route::Categories,
            Route::Units,
            Route::PriceLists,
        ],
    },
    Section {
        key: "nav.sec.inventory",
        icon: Icon::Boxes,
        routes: &[
            Route::Inventory,
            Route::Movements,
            Route::Transfers,
            Route::StockCounts,
            Route::Warehouses,
        ],
    },
    Section {
        key: "nav.sec.customers",
        icon: Icon::Users,
        routes: &[Route::Customers, Route::Debts, Route::Installments],
    },
    Section {
        key: "nav.sec.finance",
        icon: Icon::Wallet,
        routes: &[Route::Cash, Route::Banks, Route::Expenses, Route::Shifts],
    },
    Section {
        key: "nav.sec.accounting",
        icon: Icon::BookOpen,
        routes: &[
            Route::Journal,
            Route::Accounts,
            Route::Ledger,
            Route::TrialBalance,
        ],
    },
    Section {
        key: "nav.sec.distribution",
        icon: Icon::Map,
        routes: &[Route::Tours, Route::Vans, Route::FieldOrders],
    },
    Section {
        key: "nav.sec.reports",
        icon: Icon::Chart,
        routes: &[
            Route::Reports,
            Route::StockReports,
            Route::FinanceReports,
            Route::PartyReports,
        ],
    },
];

/// Pages above the sections.
pub const TOP: &[Route] = &[Route::Home, Route::Sell];
/// Pages at the bottom of the sidebar.
pub const BOTTOM: &[Route] = &[Route::Design, Route::Settings];

impl Route {
    /// The feature a page belongs to: it shows only when the organization's
    /// activity profile (or a switch) turns that feature on (M4.3).
    pub fn feature(self) -> Option<Feature> {
        use Feature as F;
        Some(match self {
            Route::Sell => F::Pos,
            Route::Sales | Route::Quotes | Route::Returns => F::Sales,
            Route::Promotions => F::Promotions,
            Route::Purchases | Route::PurchaseOrders | Route::Suppliers => F::Purchasing,
            Route::Products | Route::Categories | Route::Units => F::Catalog,
            Route::PriceLists => F::PriceLists,
            Route::Inventory | Route::Movements | Route::StockCounts | Route::Warehouses => {
                F::Inventory
            }
            Route::Transfers => F::MultiWarehouse,
            Route::Customers => F::Customers,
            Route::Debts => F::Debts,
            Route::Installments => F::Installments,
            Route::Cash | Route::Banks | Route::Expenses | Route::Shifts => F::Cash,
            Route::Journal | Route::Accounts | Route::Ledger | Route::TrialBalance => F::Accounting,
            Route::Tours => F::Routes,
            Route::Vans => F::Vehicles,
            Route::FieldOrders => F::SalesReps,
            Route::Reports | Route::StockReports | Route::FinanceReports | Route::PartyReports => {
                F::Reports
            }
            Route::Home | Route::Settings | Route::Design => return None,
        })
    }

    /// The permission needed to see a page at all (others show to everyone
    /// and protect their actions).
    pub fn permission(self) -> Option<&'static str> {
        Some(match self {
            Route::Journal | Route::Accounts | Route::Ledger | Route::TrialBalance => {
                "accounting.view"
            }
            Route::Reports | Route::StockReports | Route::FinanceReports | Route::PartyReports => {
                "reports.view"
            }
            Route::Purchases | Route::PurchaseOrders => "purchases.order",
            _ => return None,
        })
    }

    /// The section it belongs to, with its index.
    pub fn section(self) -> Option<(usize, &'static Section)> {
        SECTIONS
            .iter()
            .enumerate()
            .find(|(_, s)| s.routes.contains(&self))
    }

    pub fn from_code(code: &str) -> Option<Route> {
        Route::ALL.iter().copied().find(|r| r.code() == code)
    }

    /// NB_ROUTE (development): start on a given page, e.g. for screenshots.
    pub fn from_env() -> Option<Self> {
        Route::from_code(&std::env::var("NB_ROUTE").ok()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_is_reachable_once() {
        for &r in Route::ALL {
            let in_sections = SECTIONS.iter().filter(|s| s.routes.contains(&r)).count();
            let elsewhere = usize::from(TOP.contains(&r)) + usize::from(BOTTOM.contains(&r));
            assert_eq!(in_sections + elsewhere, 1, "{r:?}");
        }
    }

    #[test]
    fn every_page_and_section_is_named_in_every_language() {
        for lang in nb_i18n::Lang::ALL {
            for &r in Route::ALL {
                assert_ne!(nb_i18n::tr(lang, r.key()), r.key(), "{lang:?} {r:?}");
                assert_ne!(
                    nb_i18n::tr(lang, r.desc_key()),
                    r.desc_key(),
                    "{lang:?} {r:?}"
                );
            }
            for s in SECTIONS {
                assert_ne!(nb_i18n::tr(lang, s.key), s.key, "{lang:?} {}", s.key);
            }
        }
    }
}

#[cfg(test)]
mod key_tests {
    /// Every message key written in the screens exists in every language
    /// (a missing one would show as its key).
    #[test]
    fn every_literal_key_is_translated() {
        let mut missing = Vec::new();
        let mut seen = 0;
        let mut files = vec![std::path::PathBuf::from("src")];
        while let Some(p) = files.pop() {
            if p.is_dir() {
                for e in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                    files.push(e.path());
                }
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            // Formatting may break a call over lines: read it as one line.
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            for pat in [
                "st.t(\"",
                "st.t( \"",
                "st.tn(\"",
                "st.tn( \"",
                "tr_args(st.lang(), \"",
                "tr_args( st.lang(), \"",
            ] {
                for (i, _) in text.match_indices(pat) {
                    let rest = &text[i + pat.len()..];
                    let Some(end) = rest.find('"') else { continue };
                    let key = &rest[..end];
                    if key.contains('{') || key.ends_with('.') {
                        continue;
                    }
                    seen += 1;
                    for lang in nb_i18n::Lang::ALL {
                        if nb_i18n::tr(lang, key) == key {
                            missing.push(format!("{} {key} ({})", p.display(), lang.code()));
                        }
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(seen > 300, "only {seen} keys found: wrong directory?");
        assert!(missing.is_empty(), "{missing:#?}");
    }
}

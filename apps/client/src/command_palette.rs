//! Command palette (Ctrl/Cmd+K): go to any section or run a common action
//! by typing. Labels are in the current language; every item also matches
//! its names in the other languages and a few local synonyms, so typing
//! "produits" or "كريدي" works whatever the interface language.

use nb_i18n::{Lang, tr};
use nsh_base::{CommandItem, command};
use nsh_core::element::SharedString;
use nsh_core::prelude::*;

use crate::shell::Route;
use crate::state::{AppState, Intent};
use crate::ui::overlay::modal;

fn all_langs(key: &str) -> Vec<String> {
    Lang::ALL.iter().map(|&l| tr(l, key).into_owned()).collect()
}

const SECTIONS: [(Route, &str, &[&str]); 10] = [
    (Route::Home, "home", &["dashboard", "accueil", "لوحة"]),
    (Route::Sales, "sales", &["factures", "invoices", "فواتير"]),
    (
        Route::Customers,
        "customers",
        &["كريدي", "ديون", "crédit", "dettes", "debts"],
    ),
    (
        Route::Products,
        "products",
        &["articles", "catalogue", "سلع"],
    ),
    (
        Route::Inventory,
        "inventory",
        &["stock", "مستودع", "entrepôt"],
    ),
    (
        Route::Purchases,
        "purchases",
        &["fournisseurs", "suppliers", "موردين"],
    ),
    (Route::Cash, "cash", &["caisse", "خزينة"]),
    (Route::Reports, "reports", &["statistiques", "إحصائيات"]),
    (
        Route::Settings,
        "settings",
        &["préférences", "تفضيلات", "langue", "اللغة", "thème"],
    ),
    (
        Route::Design,
        "design",
        &["composants", "components", "مكونات"],
    ),
];

fn items(lang: Lang) -> Vec<CommandItem> {
    let t = |k: &str| tr(lang, k).into_owned();
    let go = t("command.group.go");
    let act = t("command.group.actions");
    let mut v = vec![
        CommandItem::new("act:sell", t("dashboard.start_sale"))
            .group(act.clone())
            .keywords(all_langs("dashboard.start_sale"))
            .shortcut("mod+n"),
        CommandItem::new("act:add_product", t("products.add"))
            .group(act.clone())
            .keywords(all_langs("products.add")),
        CommandItem::new("act:theme", t("command.toggle_theme"))
            .group(act.clone())
            .keywords(all_langs("command.toggle_theme")),
    ];
    for l in Lang::ALL {
        v.push(
            CommandItem::new(
                format!("lang:{}", l.code()),
                format!("{} — {}", t("settings.language"), l.native_name()),
            )
            .group(act.clone())
            .keywords([l.code().to_owned(), l.native_name().to_owned()]),
        );
    }
    for (route, id, synonyms) in SECTIONS {
        let key = format!("nav.{id}");
        let mut keywords = all_langs(&key);
        keywords.extend(synonyms.iter().map(|s| (*s).to_owned()));
        v.push(
            CommandItem::new(format!("go:{id}"), t(&key))
                .group(go.clone())
                .keywords(keywords),
        );
        let _ = route;
    }
    v
}

fn run(st: AppState, id: &str) {
    let route = st.route;
    match id {
        "act:sell" => route.set(Route::Sell),
        "act:add_product" => {
            route.set(Route::Products);
            st.intent.set(Some(Intent::AddProduct));
        }
        "act:theme" => st.toggle_appearance(),
        _ => {
            if let Some(code) = id.strip_prefix("lang:") {
                if let Some(l) = Lang::from_tag(code) {
                    st.lang.set(l);
                }
            } else if let Some(section) = id.strip_prefix("go:")
                && let Some((r, _, _)) = SECTIONS.iter().find(|(_, s, _)| *s == section)
            {
                route.set(*r);
            }
        }
    }
}

/// The palette and its shortcut. Placed once in the shell.
pub fn command_palette(st: AppState, compact: bool) -> impl IntoElement {
    let route = st.route;
    // Ctrl+K (Cmd+K on Apple platforms) toggles it; desktop platforms only fire keys.
    on_key("mod+k", move |_| st.palette.update(|o| *o = !*o));
    on_key("mod+n", move |_| route.set(Route::Sell));
    let open = st.palette;
    modal(open, compact, move || {
        command()
            .items(move || items(st.lang()))
            .placeholder(move || SharedString::from(tr(st.lang(), "command.placeholder").into_owned()))
            .search_accessibility_label(move || SharedString::from(tr(st.lang(), "command.search_label").into_owned()))
            .auto_focus(true)
            .on_select(move |id: &SharedString| {
                open.set(false);
                run(st, id);
            })
            .on_cancel(move || open.set(false))
            .search_bar_class("flex-row items-center h-12 px-4 border-b border-border")
            .search_class("flex-1 text-sm text-foreground")
            .list_class("h-[360px] p-1.5")
            .group_class("px-2.5 pt-3 pb-1 text-xs font-semibold text-muted-foreground")
            .item_class("flex-row items-center h-9 px-2.5 rounded-md text-sm text-foreground focus:bg-accent focus:text-accent-foreground")
            .into_element()
    })
}

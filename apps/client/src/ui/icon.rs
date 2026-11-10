//! Lucide icons (ISC license, `licenses/Lucide-ISC.txt`), recolored per
//! appearance so they render the same on native and web (where
//! `currentColor` would be black).

use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

use super::palette::Tone;

/// The app's icon set. Some icons are reserved for screens still being built.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Home,
    Cart,
    Receipt,
    Package,
    Boxes,
    Users,
    Truck,
    Wallet,
    Chart,
    Settings,
    Search,
    Offline,
    Synced,
    Refresh,
    Sun,
    Moon,
    Languages,
    Plus,
    In,
    Out,
    Coins,
    Alert,
    Store,
    More,
    User,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Banknote,
}

impl Icon {
    fn markup(self) -> &'static str {
        match self {
            Icon::Home => include_str!("../../icons/house.svg"),
            Icon::Cart => include_str!("../../icons/shopping-cart.svg"),
            Icon::Receipt => include_str!("../../icons/receipt-text.svg"),
            Icon::Package => include_str!("../../icons/package.svg"),
            Icon::Boxes => include_str!("../../icons/boxes.svg"),
            Icon::Users => include_str!("../../icons/users.svg"),
            Icon::Truck => include_str!("../../icons/truck.svg"),
            Icon::Wallet => include_str!("../../icons/wallet.svg"),
            Icon::Chart => include_str!("../../icons/chart-column.svg"),
            Icon::Settings => include_str!("../../icons/settings.svg"),
            Icon::Search => include_str!("../../icons/search.svg"),
            Icon::Offline => include_str!("../../icons/wifi-off.svg"),
            Icon::Synced => include_str!("../../icons/cloud-check.svg"),
            Icon::Refresh => include_str!("../../icons/refresh-cw.svg"),
            Icon::Sun => include_str!("../../icons/sun.svg"),
            Icon::Moon => include_str!("../../icons/moon.svg"),
            Icon::Languages => include_str!("../../icons/languages.svg"),
            Icon::Plus => include_str!("../../icons/plus.svg"),
            Icon::In => include_str!("../../icons/arrow-down-left.svg"),
            Icon::Out => include_str!("../../icons/arrow-up-right.svg"),
            Icon::Coins => include_str!("../../icons/hand-coins.svg"),
            Icon::Alert => include_str!("../../icons/triangle-alert.svg"),
            Icon::Store => include_str!("../../icons/store.svg"),
            Icon::More => include_str!("../../icons/ellipsis.svg"),
            Icon::User => include_str!("../../icons/user-round.svg"),
            Icon::ChevronDown => include_str!("../../icons/chevron-down.svg"),
            Icon::ChevronLeft => include_str!("../../icons/chevron-left.svg"),
            Icon::ChevronRight => include_str!("../../icons/chevron-right.svg"),
            Icon::Banknote => include_str!("../../icons/banknote.svg"),
        }
    }
}

/// An icon `size` points square, stroked in `tone`.
pub fn icon(which: Icon, size: f32, tone: impl Fn() -> Tone + 'static) -> impl IntoElement {
    let src = move || {
        let markup = which
            .markup()
            .replace("currentColor", &tone().hex())
            .replace("stroke-width=\"2\"", "stroke-width=\"1.75\"");
        ImageSrc::svg_markup(&markup)
    };
    svg(src).w(px(size)).h(px(size))
}

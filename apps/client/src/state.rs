//! App-wide UI state: language, appearance settings, navigation. Changing a
//! setting updates the whole interface at once, without a restart, and is
//! remembered on the device.

use nb_i18n::{Digits, Direction, Format, Lang, tr, tr_args};
use nsh_core::prelude::*;
use nsh_core::storage::Storage;

use nb_core::Clock;
use nb_domain::features::{Feature, FeatureSet};
use nb_store::usecases::access::{SignedIn, session_of, sign_in_required};
use nb_store::usecases::org as org_usecases;

use crate::nav::Route;
use crate::ui::tokens::{Accent, Corners, Density, Look, TextSize, UiFont};

/// Wall-clock milliseconds (works in the browser too).
pub fn now_ms() -> i64 {
    nb_core::SystemClock.now_ms()
}

/// Something a screen should do when it opens (e.g. from the command palette).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    AddProduct,
    /// Invoice this approved quote or order at the till (its id).
    Invoice(String),
    /// Reopen this sale at the till to correct it (its number).
    Reopen(String),
    /// The products of this category (its id).
    ProductsIn(String),
}

/// Light, dark, or as the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System];
    pub fn code(self) -> &'static str {
        match self {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.code() == s)
    }
    pub fn appearance(self) -> Appearance {
        match self {
            ThemeMode::System => Appearance::System,
            ThemeMode::Light => Appearance::Light,
            ThemeMode::Dark => Appearance::Dark,
        }
    }
}

/// The user's display preferences, saved on the device.
#[derive(Clone, Copy)]
pub struct Prefs {
    pub accent: Signal<Accent>,
    pub text: Signal<TextSize>,
    pub density: Signal<Density>,
    pub corners: Signal<Corners>,
    pub font: Signal<UiFont>,
    pub theme: Signal<ThemeMode>,
    pub digits: Signal<Digits>,
}

const K_ACCENT: &str = "nb.look.accent";
const K_TEXT: &str = "nb.look.text";
const K_DENSITY: &str = "nb.look.density";
const K_CORNERS: &str = "nb.look.corners";
const K_FONT: &str = "nb.look.font";
const K_THEME: &str = "nb.look.theme";
const K_DIGITS: &str = "nb.look.digits";
const K_LANG: &str = "nb.lang";

fn stored<T>(key: &str, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
    Storage::get_item(key).as_deref().and_then(parse)
}

fn save(key: &str, value: &str) {
    // A full storage only loses the preference, never the session.
    if Storage::set_item(key, value).is_err() {
        log::warn!("settings storage is full; {key} not saved");
    }
}

impl Prefs {
    fn load() -> Self {
        Self {
            accent: signal(stored(K_ACCENT, Accent::from_code).unwrap_or_default()),
            text: signal(stored(K_TEXT, TextSize::from_code).unwrap_or_default()),
            density: signal(stored(K_DENSITY, Density::from_code).unwrap_or_default()),
            corners: signal(stored(K_CORNERS, Corners::from_code).unwrap_or_default()),
            font: signal(stored(K_FONT, UiFont::from_code).unwrap_or_default()),
            theme: signal(stored(K_THEME, ThemeMode::from_code).unwrap_or_default()),
            digits: signal(
                stored(K_DIGITS, |s| match s {
                    "arab" => Some(Digits::ArabicIndic),
                    "latn" => Some(Digits::Latin),
                    _ => None,
                })
                .unwrap_or_default(),
            ),
        }
    }

    /// The look the tokens follow (reading it subscribes).
    pub fn look(&self) -> Look {
        Look {
            accent: self.accent.get(),
            text: self.text.get(),
            density: self.density.get(),
            corners: self.corners.get(),
            font: self.font.get(),
        }
    }

    /// Back to the defaults.
    pub fn reset(&self) {
        self.accent.set(Accent::default());
        self.text.set(TextSize::default());
        self.density.set(Density::default());
        self.corners.set(Corners::default());
        self.font.set(UiFont::default());
        self.theme.set(ThemeMode::default());
        self.digits.set(Digits::default());
    }
}

/// A choice (by index) saved on the device.
pub fn persisted_index(key: &'static str, default: usize) -> Signal<usize> {
    let s = signal(stored(key, |v| v.parse().ok()).unwrap_or(default));
    effect(move || save(key, &s.get().to_string()));
    s
}

const K_PRINTERS: &str = "nb.printers";

/// An on/off setting saved on the device.
pub fn persisted_flag(key: &'static str, default: bool) -> Signal<bool> {
    let s = signal(stored(key, |v| v.parse().ok()).unwrap_or(default));
    effect(move || save(key, if s.get() { "true" } else { "false" }));
    s
}

/// Device and display settings kept on this device. The store's identity
/// and legal data live in the organization's database (settings › store).
#[derive(Clone, Copy)]
pub struct BizPrefs {
    pub start_page: Signal<usize>,
    pub confirm_delete: Signal<bool>,
    pub show_cents: Signal<bool>,
    pub prices_incl_tax: Signal<bool>,
    pub stamp_duty: Signal<bool>,
    /// This device's printers (settings › printing).
    pub printers: Signal<Vec<crate::printers::Printer>>,
    /// Print the receipt after each sale.
    pub auto_print: Signal<bool>,
    /// Show what will print before printing it.
    pub preview_first: Signal<bool>,
    pub scale: Signal<usize>,
    pub scanner: Signal<bool>,
    /// How the till looks on this device ([`PosPrefs`]).
    pub pos: PosPrefs,
}

/// The till's screen, as this device's cashier arranged it.
#[derive(Clone, Copy)]
pub struct PosPrefs {
    /// 0 products as cards, 1 as a list, 2 the cart as a table
    /// (supermarkets: scanning first), 3 large tiles (restaurants, cafés).
    pub layout: Signal<usize>,
    /// The cart at the reading start instead of the end.
    pub cart_start: Signal<bool>,
    /// Cards: 0 small, 1 medium, 2 large.
    pub tile: Signal<usize>,
    pub stock: Signal<bool>,
    pub categories: Signal<bool>,
    /// The total in a large band.
    pub big_total: Signal<bool>,
    pub sku: Signal<bool>,
}

impl BizPrefs {
    fn load() -> Self {
        // The first version kept one network printer: it carries over.
        let mut printers =
            stored(K_PRINTERS, |v| Some(crate::printers::parse(v))).unwrap_or_default();
        if printers.is_empty()
            && let Some(old) = crate::printers::from_old(
                &stored("nb.print.addr", |v| Some(v.to_owned())).unwrap_or_default(),
                stored("nb.print.width", |v| v.parse().ok()).unwrap_or(1),
                stored("nb.print.copies", |v| v.parse().ok()).unwrap_or(0),
                stored("nb.device.drawer", |v| v.parse().ok()).unwrap_or(false),
            )
        {
            printers.push(old);
        }
        let printers = signal(printers);
        effect(move || save(K_PRINTERS, &crate::printers::to_json(&printers.get())));
        Self {
            start_page: persisted_index("nb.app.start", 0),
            confirm_delete: persisted_flag("nb.app.confirm_delete", true),
            show_cents: persisted_flag("nb.fmt.cents", true),
            prices_incl_tax: persisted_flag("nb.tax.included", true),
            stamp_duty: persisted_flag("nb.tax.stamp", true),
            printers,
            auto_print: persisted_flag("nb.print.auto", false),
            preview_first: persisted_flag("nb.print.preview", false),
            scale: persisted_index("nb.device.scale", 0),
            scanner: persisted_flag("nb.device.scanner", true),
            pos: PosPrefs {
                layout: persisted_index("nb.pos.layout", 0),
                cart_start: persisted_flag("nb.pos.cart_start", false),
                tile: persisted_index("nb.pos.tile", 1),
                stock: persisted_flag("nb.pos.stock", true),
                categories: persisted_flag("nb.pos.categories", true),
                big_total: persisted_flag("nb.pos.big_total", true),
                sku: persisted_flag("nb.pos.sku", false),
            },
        }
    }
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub lang: Signal<Lang>,
    pub prefs: Prefs,
    pub biz: BizPrefs,
    pub route: Signal<Route>,
    /// Sidebar sections that are expanded (bit per section).
    pub open_sections: Signal<u32>,
    /// The settings page's open tab.
    pub settings_tab: Signal<usize>,
    /// The confirmation shown at the bottom of the screen (`ui::overlay::show_toast`).
    pub toast: Signal<Option<String>>,
    /// The command palette is open.
    pub palette: Signal<bool>,
    /// A pending request for the screen being opened.
    pub intent: Signal<Option<Intent>>,
    /// The local database of this app instance (set by [`AppState::connect`]).
    pub db: Stored<Option<crate::db::Db>>,
    /// The organization the local database works for, once it is open.
    pub session: Signal<Option<crate::db::Session>>,
    /// Why the database could not open, if it could not.
    pub db_error: Signal<Option<String>>,
    /// No shop on this device yet: the first-run setup shows.
    pub needs_setup: Signal<bool>,
    /// The shop's name and its branch's (the sidebar's header).
    pub shop: Signal<Option<(String, String)>>,
    /// The database checked at start found something wrong (after a crash
    /// or a disk fault): shown until it is looked at.
    pub unsound: Signal<bool>,
    /// Bumped after every write, so lists load again.
    pub data_rev: Signal<u64>,
    /// The person using the app; `None` while the lock screen shows.
    pub user: Signal<Option<SignedIn>>,
    /// Whether this device asks who is there (someone has a PIN or password).
    pub needs_sign_in: Signal<bool>,
    /// The organization's features (activity profile and switches); `None`
    /// until loaded, when everything shows.
    pub features: Signal<Option<FeatureSet>>,
    /// Last time someone touched the app (ms), for the automatic lock.
    pub last_active: Stored<i64>,
    /// The app's root: work that must outlive the screen or window that
    /// started it (printing after a sale) runs under it.
    pub root: Owner,
}

impl AppState {
    pub fn new() -> Self {
        let lang = stored(K_LANG, Lang::from_tag).unwrap_or_else(|| {
            locale()
                .as_str()
                .split(['-', '_'])
                .next()
                .and_then(Lang::from_tag)
                .unwrap_or(Lang::Ar)
        });
        let st = Self {
            lang: signal(lang),
            prefs: Prefs::load(),
            biz: BizPrefs::load(),
            route: signal(Route::Home),
            open_sections: signal(0),
            settings_tab: signal(0),
            toast: signal(None),
            palette: signal(false),
            intent: signal(None),
            db: nsh_core::prelude::stored(None),
            session: signal(None),
            db_error: signal(None),
            needs_setup: signal(false),
            shop: signal(None),
            unsound: signal(false),
            data_rev: signal(0),
            user: signal(None),
            needs_sign_in: signal(false),
            features: signal(None),
            last_active: nsh_core::prelude::stored(now_ms()),
            root: Owner::current(),
        };
        if st.biz.start_page.get_untracked() == 1 {
            st.route.set(Route::Sell);
        }
        st
    }

    /// Saves every setting when it changes, and applies the appearance.
    pub fn persist(self) {
        let p = self.prefs;
        effect(move || save(K_LANG, self.lang.get().code()));
        effect(move || save(K_ACCENT, p.accent.get().code()));
        effect(move || save(K_TEXT, p.text.get().code()));
        effect(move || save(K_DENSITY, p.density.get().code()));
        effect(move || save(K_CORNERS, p.corners.get().code()));
        effect(move || save(K_FONT, p.font.get().code()));
        effect(move || {
            let mode = p.theme.get();
            save(K_THEME, mode.code());
            set_color_scheme(mode.appearance());
        });
        effect(move || {
            save(
                K_DIGITS,
                match p.digits.get() {
                    Digits::ArabicIndic => "arab",
                    Digits::Latin => "latn",
                },
            )
        });
    }

    /// The local database (started by [`AppState::connect`] if needed).
    /// The demo's language, as the interface's.
    pub fn demo_lang(&self) -> nb_store::demo::DemoLang {
        match self.lang.get_untracked() {
            Lang::Ar => nb_store::demo::DemoLang::Ar,
            _ => nb_store::demo::DemoLang::Fr,
        }
    }

    pub fn db(&self) -> crate::db::Db {
        if let Some(db) = self.db.get() {
            return db;
        }
        let db = crate::db::Db::start(match self.lang.get_untracked() {
            Lang::Ar => nb_store::demo::DemoLang::Ar,
            _ => nb_store::demo::DemoLang::Fr,
        });
        self.db.replace(Some(db.clone()));
        db
    }

    /// Starts the database and records its session when ready. Without
    /// any PIN or password on the device, the owner works without signing in.
    pub fn connect(self) {
        let db = self.db();
        let task = spawn(async move {
            let opened = async {
                let s = db.session().await?;
                let owner = s.owner();
                let s_org = s.setup.org;
                let (needs, features, me, shop) = db
                    .call(move |store| {
                        let name = org_usecases::settings(store)?.name;
                        let branch = store.read(|r| {
                            Ok(nb_store::repo::org::branches(r.conn(), s_org)?
                                .into_iter()
                                .next()
                                .map(|b| b.name)
                                .unwrap_or_default())
                        })?;
                        let needs = sign_in_required(store)?;
                        let me = if needs {
                            None
                        } else {
                            Some(session_of(store, owner)?)
                        };
                        Ok((needs, org_usecases::features(store)?, me, (name, branch)))
                    })
                    .await?;
                Ok::<_, crate::db::DbError>((s, needs, features, me, shop))
            };
            match opened.await {
                Ok((s, needs, features, me, shop)) => {
                    self.shop.set(Some(shop));
                    self.needs_sign_in.set(needs);
                    self.features.set(Some(features));
                    self.session.set(Some(s));
                    if let Some(me) = me {
                        self.signed_in(me);
                    }
                    // Then checked end to end (after a crash, a disk fault),
                    // once the shop can already work: it is told, nothing
                    // is changed.
                    match db.call(|store| store.health()).await {
                        Ok(h) if !h.is_sound() => {
                            log::error!("database check: {h:?}");
                            self.unsound.set(true);
                        }
                        Ok(_) => {}
                        Err(e) => log::error!("database check: {}", e.message),
                    }
                }
                Err(e) if e.codes.iter().any(|c| c.1 == crate::db::NEEDS_SETUP) => {
                    self.needs_setup.set(true);
                }
                Err(e) => {
                    log::error!("database: {}", e.message);
                    self.db_error.set(Some(e.message));
                }
            }
        });
        drop(task); // runs to completion: tasks end with their owner, not their handle
    }

    /// Someone signed in: operations now run in their name.
    pub fn signed_in(self, me: SignedIn) {
        if let Ok(id) = me.user_id.parse() {
            self.session.update(|s| {
                if let Some(s) = s {
                    s.actor = id;
                }
            });
        }
        self.touch();
        self.user.set(Some(me));
    }

    /// Shows the lock screen (by hand, or after inactivity).
    pub fn lock(self) {
        if self.needs_sign_in.get_untracked() {
            self.user.set(None);
        }
    }

    /// Records activity (postpones the automatic lock).
    pub fn touch(self) {
        self.last_active.replace(now_ms());
    }

    /// Whether the person signed in holds `permission` somewhere.
    pub fn can(&self, permission: &str) -> bool {
        self.user
            .get()
            .is_some_and(|u| u.permissions.iter().any(|p| p == permission))
    }

    /// Whether a feature is on (everything is, until the profile is loaded).
    pub fn has(&self, f: Feature) -> bool {
        self.features.get().is_none_or(|set| set.has(f))
    }

    /// Whether a page shows: its feature is on and the person may see it.
    pub fn shows(&self, r: Route) -> bool {
        r.feature().is_none_or(|f| self.has(f))
            && r.permission()
                .is_none_or(|p| self.user.get().is_none() || self.can(p))
    }

    /// Lists depending on data load again.
    pub fn changed(self) {
        self.data_rev.update(|n| *n += 1);
    }

    pub fn lang(&self) -> Lang {
        self.lang.get()
    }

    pub fn fmt(&self) -> Format {
        Format {
            lang: self.lang.get(),
            digits: self.prefs.digits.get(),
        }
    }

    pub fn rtl(&self) -> bool {
        self.lang.get().direction() == Direction::Rtl
    }

    pub fn direction(&self) -> LayoutDirection {
        if self.rtl() {
            LayoutDirection::Rtl
        } else {
            LayoutDirection::Ltr
        }
    }

    /// A reactive translated string.
    pub fn t(self, key: &'static str) -> impl Fn() -> String + 'static {
        move || tr(self.lang.get(), key).into_owned()
    }

    /// A translated string now (for one-off labels built inside a reactive scope).
    pub fn tn(self, key: &str) -> String {
        tr(self.lang.get(), key).into_owned()
    }

    /// A reactive translated string with arguments computed on each update.
    pub fn t_with(
        self,
        key: &'static str,
        args: impl Fn(&AppState) -> Vec<(&'static str, String)> + 'static,
    ) -> impl Fn() -> String + 'static {
        move || {
            let a = args(&self);
            let pairs: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
            tr_args(self.lang.get(), key, &pairs)
        }
    }

    /// Light ↔ dark, starting from what is shown.
    pub fn toggle_appearance(self) {
        self.prefs.theme.set(if dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        });
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

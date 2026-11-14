//! The bridge to the local database (ADR-0003): the store lives on its own
//! thread (desktop and mobile), and screens await results, so the interface
//! never waits on SQLite. In the browser, which has no threads here, calls
//! run in place until the Web Worker adapter arrives (M2.10).
//!
//! The database file is chosen by the desktop entry point (`configure`);
//! without it (UI tests, browser) the store is in memory, seeded with the
//! demo catalog — so tests always start from the same data.

use std::path::PathBuf;
use std::sync::OnceLock;

use nb_core::Currency;
use nb_domain::{BranchId, UserId, WarehouseId};
use nb_store::demo::{self, DemoLang};
use nb_store::usecases::setup::{SetupInput, SetupResult, setup_organization};
use nb_store::{Error, OpMeta, Store, StoreOptions};

/// Where the database lives (set once, before the first call).
static PATH: OnceLock<PathBuf> = OnceLock::new();

/// The database of an app packaged by NSH for Windows, macOS or Android, whose
/// launcher is NSH's and not `main.rs`: in the user's data folder (or
/// `NB_DATA_DIR`). Linux runs `main.rs`, which configures it, and the
/// headless tests keep their database in memory.
fn packaged_path() -> Option<PathBuf> {
    if !cfg!(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "android"
    )) {
        return None;
    }
    let base = std::env::var_os("NB_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            if cfg!(target_os = "android") {
                // The app's private files (NSH gives Rust no data-dir call).
                let dir = PathBuf::from("/data/data/dz.nshbusiness.app/files");
                std::fs::create_dir_all(&dir).ok()?;
                Some(dir)
            } else if cfg!(target_os = "windows") {
                std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("dz.nshbusiness.app"))
            } else {
                std::env::var_os("HOME").map(|h| {
                    PathBuf::from(h).join("Library/Application Support/dz.nshbusiness.app")
                })
            }
        })?;
    Some(base.join("store.db"))
}

/// Uses the database file at `path` (desktop). Call before the app starts.
pub fn configure(path: PathBuf) {
    let _ = PATH.set(path);
}

/// An error a screen can show: rule codes (translated as `error.<code>`)
/// with the field they concern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbError {
    pub codes: Vec<(Option<&'static str>, &'static str)>,
    pub message: String,
}

impl From<Error> for DbError {
    fn from(e: Error) -> Self {
        Self {
            codes: e.rule_codes(),
            message: e.to_string(),
        }
    }
}

impl DbError {
    /// The first code concerning `field`.
    pub fn on(&self, field: &str) -> Option<&'static str> {
        self.codes
            .iter()
            .find(|(f, _)| *f == Some(field))
            .map(|(_, c)| *c)
    }
}

/// What every operation needs to know about the organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    pub setup: SetupResult,
    pub currency: Currency,
    /// Who works on this device now (the owner until someone signs in).
    pub actor: UserId,
}

impl Session {
    /// A new operation run by the person signed in.
    pub fn meta(&self, kind: &'static str) -> OpMeta {
        OpMeta::new(kind, Some(self.setup.org), Some(self.actor))
    }
    pub fn warehouse(&self) -> WarehouseId {
        self.setup.warehouse
    }
    pub fn branch(&self) -> BranchId {
        self.setup.branch
    }
    pub fn owner(&self) -> UserId {
        self.setup.owner
    }
}

/// The organization this database works for, if it has one.
fn read_session(store: &Store) -> Result<Option<Session>, Error> {
    store.read(|r| {
        let Some(o) = nb_store::repo::org::the_org(r.conn())? else {
            return Ok(None);
        };
        let org = o.org_id()?;
        let branch = nb_store::repo::org::branches(r.conn(), org)?
            .into_iter()
            .next()
            .ok_or(Error::NotFound("branch"))?;
        let warehouse = nb_store::repo::org::warehouses(r.conn(), org)?
            .into_iter()
            .next()
            .ok_or(Error::NotFound("warehouse"))?;
        let owner: String = r.conn().query_row(
            "SELECT id FROM users WHERE org_id = ?1 AND role = 'owner' ORDER BY created_at LIMIT 1",
            [org.to_string()],
            |row| row.get(0),
        )?;
        let owner = nb_store::repo::parse_id(&owner)?;
        Ok(Some(Session {
            setup: SetupResult {
                org,
                branch: nb_store::repo::parse_id(&branch.id)?,
                warehouse: nb_store::repo::parse_id(&warehouse.id)?,
                owner,
            },
            currency: o.currency()?,
            actor: owner,
        }))
    })
}

/// The owner of the store's organization (its user id).
pub fn owner_of(store: &Store) -> Result<String, Error> {
    read_session(store)?
        .map(|s| s.owner().to_string())
        .ok_or(Error::NotFound("organization"))
}

/// The store's organization.
pub fn org_of(store: &Store) -> Result<nb_core::OrganizationId, Error> {
    read_session(store)?
        .map(|s| s.setup.org)
        .ok_or(Error::NotFound("organization"))
}

/// The answer of a store with no organization yet: the first-run setup
/// shows (a store on disk); in memory (tests, the browser) a demo shop is
/// made instead.
pub const NEEDS_SETUP: &str = "setup.needed";

fn needs_setup() -> DbError {
    DbError {
        codes: vec![(None, NEEDS_SETUP)],
        message: "no organization yet".into(),
    }
}

/// The demo shop, in the interface's language.
fn demo_shop(store: &mut Store, lang: DemoLang) -> Result<Session, Error> {
    let (name, branch, owner) = match lang {
        DemoLang::Ar => ("متجر النور", "الوادي", "أحمد بن علي"),
        DemoLang::Fr => ("Magasin En-Nour", "El Oued", "Ahmed Benali"),
    };
    let setup = setup_organization(
        store,
        SetupInput {
            org_name: name.into(),
            // NB_ACTIVITY (development): another activity's modules, e.g.
            // `wholesale` for distribution.
            activity: std::env::var("NB_ACTIVITY").unwrap_or_else(|_| "grocery".into()),
            country: "DZ".into(),
            currency: "DZD".into(),
            currency_exp: 2,
            branch_name: branch.into(),
            owner_name: owner.into(),
            language: match lang {
                DemoLang::Ar => "ar",
                DemoLang::Fr => "fr",
            }
            .into(),
            fiscal_year_start: 1,
        },
    )?;
    demo::seed(store, &setup, lang, 2_000)?;
    demo::seed_distribution(store, &setup, lang)?;
    Ok(Session {
        setup,
        currency: Currency::DZD,
        actor: setup.owner,
    })
}

/// Opens the store and reads its organization. A store on disk with none
/// yet waits for the first-run setup (`None`), unless `NB_DEMO=1`; in
/// memory a demo shop is made at once.
fn open(lang: DemoLang) -> Result<(Store, Option<Session>), Error> {
    let opts = StoreOptions {
        device_name: "client".into(),
        ..StoreOptions::default()
    };
    let on_disk = PATH.get().cloned().or_else(packaged_path);
    let demo_on_disk = std::env::var("NB_DEMO").is_ok_and(|v| v == "1");
    // The browser starts on the first screen (create a shop, join one, or
    // the demo) unless it kept a copy (`web_store`, loaded next).
    let wizard = (on_disk.is_some() && !demo_on_disk) || cfg!(target_arch = "wasm32");
    let mut store = match on_disk {
        Some(p) => {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).map_err(|e| Error::Backup(e.to_string()))?;
            }
            Store::open(&p, opts)?
        }
        None => Store::open_in_memory(opts)?,
    };
    let session = match read_session(&store)? {
        Some(s) => Some(s),
        None if wizard => None,
        None => Some(demo_shop(&mut store, lang)?),
    };
    Ok((store, session))
}

/// What the database thread holds: the store once open, or why it is not.
struct State {
    store: Option<Store>,
    session: Result<Session, DbError>,
}

impl State {
    fn opening(lang: DemoLang) -> Self {
        match open(lang) {
            Ok((store, session)) => Self {
                store: Some(store),
                session: session.ok_or_else(needs_setup),
            },
            Err(e) => Self {
                store: None,
                session: Err(DbError::from(e)),
            },
        }
    }
}

type Job = Box<dyn FnOnce(&mut State) + Send>;

fn stopped() -> DbError {
    DbError {
        codes: vec![(None, "internal")],
        message: "database thread stopped".into(),
    }
}

/// A handle to the database thread.
#[derive(Clone)]
pub struct Db {
    #[cfg(not(target_arch = "wasm32"))]
    jobs: std::sync::mpsc::Sender<Job>,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static STATE: std::cell::RefCell<Option<State>> = const { std::cell::RefCell::new(None) };
}

impl Db {
    /// Starts a database thread, which opens the store (and seeds a demo
    /// organization on a first start) without holding the interface:
    /// screens await [`Db::session`]. One per app instance.
    pub fn start(lang: DemoLang) -> Db {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel::<Job>();
            let spawned = std::thread::Builder::new()
                .name("nb-db".into())
                .spawn(move || {
                    let mut state = State::opening(lang);
                    while let Ok(job) = rx.recv() {
                        job(&mut state);
                    }
                });
            if let Err(e) = spawned {
                log::error!("database thread: {e}");
            }
            Db { jobs: tx }
        }
        #[cfg(target_arch = "wasm32")]
        {
            STATE.with(|s| *s.borrow_mut() = Some(State::opening(lang)));
            Db {}
        }
    }
}

impl Db {
    async fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut State) -> Result<R, DbError> + Send + 'static,
    ) -> Result<R, DbError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let job: Job = Box::new(move |state| {
                let _ = tx.send(f(state));
            });
            if self.jobs.send(job).is_err() {
                return Err(stopped());
            }
            rx.await.unwrap_or_else(|_| Err(stopped()))
        }
        #[cfg(target_arch = "wasm32")]
        {
            STATE.with(|s| match s.borrow_mut().as_mut() {
                Some(state) => f(state),
                None => Err(stopped()),
            })
        }
    }

    /// Replaces the data with a backup's, then reads the organization
    /// again (a backup from another device brings its own).
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn restore(
        &self,
        meta: OpMeta,
        path: PathBuf,
        passphrase: String,
    ) -> Result<nb_store::store::Restored, DbError> {
        self.run(move |state| {
            let store = state.store.as_mut().ok_or_else(stopped)?;
            let r = nb_store::usecases::backups::restore_backup(store, meta, &path, &passphrase)
                .map_err(DbError::from)?;
            if let Ok(Some(s)) = read_session(store) {
                state.session = Ok(s);
            }
            Ok(r)
        })
        .await
    }

    /// The first-run setup: the shop as entered (its legal identity
    /// too), or the demo shop.
    pub async fn setup(
        &self,
        input: Option<(SetupInput, nb_store::usecases::org::BusinessProfile)>,
        lang: DemoLang,
    ) -> Result<Session, DbError> {
        self.run(move |state| {
            let store = state.store.as_mut().ok_or_else(stopped)?;
            let session = match input {
                Some((input, profile)) => {
                    let setup = setup_organization(store, input).map_err(DbError::from)?;
                    let s = read_session(store)
                        .map_err(DbError::from)?
                        .ok_or_else(stopped)?;
                    if profile != nb_store::usecases::org::BusinessProfile::default() {
                        let meta = OpMeta::new("setup.profile", Some(setup.org), Some(setup.owner));
                        nb_store::usecases::org::update_business_profile(store, meta, profile)
                            .map_err(DbError::from)?;
                    }
                    s
                }
                None => demo_shop(store, lang).map_err(DbError::from)?,
            };
            state.session = Ok(session);
            Ok(session)
        })
        .await
    }

    /// The browser's saved copy of the shop, opened in place of the empty
    /// start.
    #[cfg(target_arch = "wasm32")]
    pub async fn adopt_saved(&self, bytes: Vec<u8>) -> Result<(), DbError> {
        self.run(move |state| {
            let store = state.store.as_mut().ok_or_else(stopped)?;
            store.restore_bytes(&bytes).map_err(DbError::from)?;
            state.session = read_session(store)
                .map_err(DbError::from)?
                .ok_or_else(needs_setup);
            Ok(())
        })
        .await
    }

    /// The database as bytes when it changed since `since` (its change
    /// count), for the browser to keep.
    #[cfg(target_arch = "wasm32")]
    pub async fn copy_if_changed(&self, since: u64) -> Result<Option<(u64, Vec<u8>)>, DbError> {
        self.run(move |state| {
            let Some(store) = state.store.as_ref() else {
                return Ok(None);
            };
            // Nothing to keep before a shop exists.
            if state.session.is_err() {
                return Ok(None);
            }
            let now = store.changes();
            if now == since {
                return Ok(None);
            }
            Ok(Some((
                now,
                store.snapshot_in_memory().map_err(DbError::from)?,
            )))
        })
        .await
    }

    /// This device joins an organization that already exists (M11/M2):
    /// its database becomes the server's copy, with its own identity.
    pub async fn join(
        &self,
        snapshot: Vec<u8>,
        at: nb_store::sync::Position,
        joined: nb_store::sync::Joined,
        server_url: String,
        register_name: String,
    ) -> Result<Session, DbError> {
        self.run(move |state| {
            let store = state.store.as_mut().ok_or_else(stopped)?;
            store
                .join_with(&snapshot, at, &joined, &server_url)
                .map_err(DbError::from)?;
            let session = read_session(store)
                .map_err(DbError::from)?
                .ok_or_else(stopped)?;
            // Its own till: a drawer of its own, counted on its own (the
            // register travels to the others like any operation). A device
            // that takes back its place (a reinstall) finds the one it had.
            let mine = store.register_made_here().map_err(DbError::from)?;
            let till = match mine {
                Some(id) => id,
                None => {
                    nb_store::usecases::org::create_register(
                        store,
                        session.meta("register.create"),
                        session.branch(),
                        &register_name,
                    )
                    .map_err(DbError::from)?
                    .id
                }
            };
            store.set_own_register(Some(&till)).map_err(DbError::from)?;
            state.session = Ok(session);
            Ok(session)
        })
        .await
    }

    /// The organization this device works for, once the store is open.
    pub async fn session(&self) -> Result<Session, DbError> {
        self.run(|state| state.session.clone()).await
    }

    /// Runs `f` with the store and awaits its result.
    pub async fn call<R, F>(&self, f: F) -> Result<R, DbError>
    where
        R: Send + 'static,
        F: FnOnce(&mut Store) -> Result<R, Error> + Send + 'static,
    {
        self.run(move |state| match state.store.as_mut() {
            Some(store) => f(store).map_err(DbError::from),
            None => Err(state.session.clone().err().unwrap_or_else(stopped)),
        })
        .await
    }
}

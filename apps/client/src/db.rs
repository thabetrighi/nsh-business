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
}

impl Session {
    pub fn meta(&self, kind: &'static str) -> OpMeta {
        OpMeta::new(kind, Some(self.setup.org), Some(self.setup.owner))
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

/// Opens the store and makes sure an organization exists (a demo one on a
/// first start, in the interface's language).
fn open(lang: DemoLang) -> Result<(Store, Session), Error> {
    let opts = StoreOptions {
        device_name: "client".into(),
        ..StoreOptions::default()
    };
    let mut store = match PATH.get() {
        Some(p) => {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).map_err(|e| Error::Backup(e.to_string()))?;
            }
            Store::open(p, opts)?
        }
        None => Store::open_in_memory(opts)?,
    };
    let existing = store.read(|r| {
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
        Ok(Some(Session {
            setup: SetupResult {
                org,
                branch: nb_store::repo::parse_id(&branch.id)?,
                warehouse: nb_store::repo::parse_id(&warehouse.id)?,
                owner: nb_store::repo::parse_id(&owner)?,
            },
            currency: o.currency()?,
        }))
    })?;
    let session = match existing {
        Some(s) => s,
        None => {
            let (name, branch, owner) = match lang {
                DemoLang::Ar => ("متجر النور", "الوادي", "أحمد بن علي"),
                DemoLang::Fr => ("Magasin En-Nour", "El Oued", "Ahmed Benali"),
            };
            let setup = setup_organization(
                &mut store,
                SetupInput {
                    org_name: name.into(),
                    activity: "grocery".into(),
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
            demo::seed(&mut store, &setup, lang, 2_000)?;
            Session {
                setup,
                currency: Currency::DZD,
            }
        }
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
                session: Ok(session),
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

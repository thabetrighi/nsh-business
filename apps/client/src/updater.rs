//! The app updates itself on the desktop (ADR-0011), with NSH's updater:
//! the packaged app holds `nsh-updater.json` (where its updates come from
//! and its version, written by `nsh package` from `nsh.json`'s `"updater"`),
//! the release holds `latest.json` (the newest version and an archive per
//! platform with its SHA-256). The archive is checked as it arrives, put in
//! place of the installed app, and the app restarts when the cashier says
//! so: never in the middle of a sale.
//!
//! A development run, or an app installed where the user cannot write, has
//! nothing to update: the page says so.

use nsh_core::platform::HttpRequest;
use nsh_core::prelude::*;

use crate::state::AppState;

/// The updater's configuration, as the package holds it.
const CONFIG: &str = "asset:/nsh-updater.json";

/// Where the updater is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateState {
    /// Nothing asked yet.
    #[default]
    Idle,
    Checking,
    /// The newest version runs.
    UpToDate,
    /// A newer version is out.
    Available {
        version: String,
        notes: Option<String>,
        size: Option<u64>,
    },
    Downloading,
    /// Installed: it runs from the next start.
    Ready {
        version: String,
    },
    /// This copy does not update itself (a development run, or installed
    /// where it cannot be replaced): the reason's key.
    Unsupported(&'static str),
    /// The key of what went wrong, and its detail.
    Failed(&'static str, String),
}

impl UpdateState {
    pub fn busy(&self) -> bool {
        matches!(self, UpdateState::Checking | UpdateState::Downloading)
    }
}

thread_local! {
    /// What the last check found, for the download.
    static FOUND: std::cell::RefCell<Option<Found>> = const { std::cell::RefCell::new(None) };
}

#[derive(Clone)]
struct Found {
    config: Vec<u8>,
    id: String,
    available: nsh_updater::Available,
}

async fn get(url: &str) -> Result<Vec<u8>, String> {
    let res = fetch(HttpRequest::new("GET", url.to_owned()))
        .await
        .map_err(|e| e.to_string())?;
    match res.status() {
        200..=299 => Ok(res.body),
        s => Err(format!("HTTP {s}")),
    }
}

/// The package's configuration (its version shown from then on): `None`
/// in a development run.
async fn config(st: AppState) -> Option<(Vec<u8>, nsh_updater::Config)> {
    let bytes = get(CONFIG).await.ok()?;
    let config = nsh_updater::Config::parse(&bytes).ok()?;
    st.app_version.set(config.version.clone());
    Some((bytes, config))
}

/// Reads the package's version for the page (no network).
fn read_version(st: AppState) {
    st.root.run(|| {
        let task = spawn(async move {
            let _ = config(st).await;
        });
        drop(task); // runs to completion under the root
    });
}

/// Asks the release whether a newer version is out. `quiet`: a check the
/// app makes by itself, which says nothing unless there is one.
pub fn check(st: AppState, quiet: bool) {
    if st.update.get_untracked().busy() {
        return;
    }
    st.update.set(UpdateState::Checking);
    st.root.run(|| {
        let task = spawn(async move {
            let found = find(st).await;
            let state = match found {
                Ok(Some(f)) => {
                    let a = f.available.clone();
                    FOUND.with(|x| *x.borrow_mut() = Some(f));
                    if quiet {
                        crate::ui::overlay::show_toast(
                            st,
                            st.tn("update.toast").replace("{version}", &a.version),
                        );
                    }
                    UpdateState::Available {
                        version: a.version,
                        notes: a.notes,
                        size: a.size,
                    }
                }
                Ok(None) => UpdateState::UpToDate,
                Err(state) => state,
            };
            // A quiet check that failed leaves the page as it was.
            if quiet && matches!(state, UpdateState::Failed(..) | UpdateState::Unsupported(_)) {
                st.update.set(UpdateState::Idle);
            } else {
                st.update.set(state);
            }
        });
        drop(task); // runs to completion under the root
    });
}

/// The installed app this one can replace.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn installed(id: &str, config: &[u8]) -> Result<nsh_updater::install::Installation, UpdateState> {
    let Some(installed) = nsh_updater::install::Installation::current(id, config) else {
        return Err(UpdateState::Unsupported("update.dev"));
    };
    if !installed.can_install() {
        return Err(UpdateState::Unsupported("update.read_only"));
    }
    Ok(installed)
}

/// Phones and the browser update through their store or the page.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn installed(_: &str, _: &[u8]) -> Result<Installed, UpdateState> {
    Err(UpdateState::Unsupported("update.store"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
struct Installed;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl Installed {
    fn remove_leftovers(&self) {}
    fn install(&self, _: &std::path::Path) -> Result<(), String> {
        Err(String::new())
    }
}

async fn find(st: AppState) -> Result<Option<Found>, UpdateState> {
    let Some((bytes, config)) = config(st).await else {
        return Err(UpdateState::Unsupported("update.dev"));
    };
    // What an earlier update could not remove (Windows: the old program).
    installed(&config.id, &bytes)?.remove_leftovers();
    let manifest = get(&config.manifest_url())
        .await
        .map_err(|e| UpdateState::Failed("update.err.network", e))?;
    let signature = match config.signature_url() {
        Some(url) => Some(
            get(&url)
                .await
                .map_err(|e| UpdateState::Failed("update.err.network", e))?,
        ),
        None => None,
    };
    let available = config
        .check(&manifest, signature.as_deref(), &nsh_updater::platform())
        .map_err(|e| UpdateState::Failed("update.err.manifest", e.to_string()))?;
    Ok(available.map(|available| Found {
        config: bytes,
        id: config.id.clone(),
        available,
    }))
}

/// Downloads the version found, checks it and puts it in place of this
/// one. The app keeps running; [`restart`] starts the new one.
pub fn install(st: AppState) {
    let Some(found) = FOUND.with(|x| x.borrow().clone()) else {
        return;
    };
    if st.update.get_untracked().busy() {
        return;
    }
    st.update.set(UpdateState::Downloading);
    st.root.run(|| {
        let task = spawn(async move {
            let version = found.available.version.clone();
            let state = match download_and_install(found).await {
                Ok(()) => UpdateState::Ready { version },
                Err(state) => state,
            };
            st.update.set(state);
        });
        drop(task); // runs to completion under the root
    });
}

async fn download_and_install(found: Found) -> Result<(), UpdateState> {
    let a = &found.available;
    let Some(sha256) = a.sha256.as_deref() else {
        // A page, not an archive: nothing to install here.
        return Err(UpdateState::Unsupported("update.page"));
    };
    let bytes = get(&a.url)
        .await
        .map_err(|e| UpdateState::Failed("update.err.network", e))?;
    let mut check = nsh_updater::Download::new(sha256, a.size);
    check
        .update(&bytes)
        .and_then(|_| check.finish())
        .map_err(|e| UpdateState::Failed("update.err.download", e.to_string()))?;
    // The archive's kind is the URL's (`.zip` on Windows, `.tar.gz`
    // elsewhere): the installer reads it by its name.
    let name = a.url.rsplit('/').next().unwrap_or("update.zip");
    let ext = if name.ends_with(".zip") {
        "zip"
    } else {
        "tar.gz"
    };
    let file = std::env::temp_dir().join(format!("nsh-business-update.{ext}"));
    std::fs::write(&file, &bytes)
        .map_err(|e| UpdateState::Failed("update.err.install", e.to_string()))?;
    let installed = installed(&found.id, &found.config)?;
    // The program's path, taken before the app on disk changes: after the
    // swap the running executable is the one moved aside, and the restart
    // would start what is gone.
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    nsh_core::restart::remember_program();
    let result = installed
        .install(&file)
        .map_err(|e| UpdateState::Failed("update.err.install", e.to_string()));
    let _ = std::fs::remove_file(&file);
    result
}

/// Starts the new version (the app quits, then starts again).
pub fn restart() {
    #[cfg(not(target_arch = "wasm32"))]
    nsh_core::element::desktop::restart();
}

/// The app looks by itself: a minute after it starts, then twice a day.
pub fn watch(st: AppState) {
    read_version(st);
    let look = move || {
        if st.biz.auto_update.get_untracked()
            && matches!(
                st.update.get_untracked(),
                UpdateState::Idle | UpdateState::UpToDate
            )
        {
            check(st, true);
        }
    };
    st.root.run(|| {
        let first = set_timeout(std::time::Duration::from_secs(60), look);
        drop(first); // fires once, under the root
        let every = set_interval(std::time::Duration::from_secs(12 * 3600), look);
        drop(every); // runs while the root lives
    });
}

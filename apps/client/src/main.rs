//! Runs NSH Business on the desktop backends.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl std::process::Termination {
    env_logger::init();
    let size = std::env::var("NB_WINDOW")
        .ok()
        .and_then(|s| {
            s.split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        })
        .unwrap_or((1360, 860));
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<nb_client::App>()),
        platform::AppOptions {
            title: "NSH Business".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: "dz.nshbusiness.app".to_owned(),
            size,
            ..Default::default()
        },
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn main() {}

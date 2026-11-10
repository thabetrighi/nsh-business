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
    // The bundled fonts (OFL), registered before the first frame: a Rust root
    // run directly carries its own fonts (packaged apps get them from assets).
    let root = platform::Root::of::<nb_client::App>()
        .font(&include_bytes!("../assets/fonts/Almarai-Bold.ttf")[..])
        .font(&include_bytes!("../assets/fonts/Almarai-ExtraBold.ttf")[..])
        .font(&include_bytes!("../assets/fonts/Almarai-Regular.ttf")[..])
        .font(&include_bytes!("../assets/fonts/Cairo-Bold.ttf")[..])
        .font(&include_bytes!("../assets/fonts/Cairo-Medium.ttf")[..])
        .font(&include_bytes!("../assets/fonts/Cairo-Regular.ttf")[..])
        .font(&include_bytes!("../assets/fonts/Cairo-SemiBold.ttf")[..])
        .font(&include_bytes!("../assets/fonts/IBMPlexSansArabic-Bold.ttf")[..])
        .font(&include_bytes!("../assets/fonts/IBMPlexSansArabic-Medium.ttf")[..])
        .font(&include_bytes!("../assets/fonts/IBMPlexSansArabic-Regular.ttf")[..])
        .font(&include_bytes!("../assets/fonts/IBMPlexSansArabic-SemiBold.ttf")[..])
        .font(&include_bytes!("../assets/fonts/ReadexPro-Bold.ttf")[..])
        .font(&include_bytes!("../assets/fonts/ReadexPro-Medium.ttf")[..])
        .font(&include_bytes!("../assets/fonts/ReadexPro-Regular.ttf")[..])
        .font(&include_bytes!("../assets/fonts/ReadexPro-SemiBold.ttf")[..])
        .font(&include_bytes!("../assets/fonts/Tajawal-Bold.ttf")[..])
        .font(&include_bytes!("../assets/fonts/Tajawal-Medium.ttf")[..])
        .font(&include_bytes!("../assets/fonts/Tajawal-Regular.ttf")[..]);
    platform::run_app(
        platform::AppSource::Rust(root),
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

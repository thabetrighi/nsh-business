//! What the cashier hears and feels at the till: a beep for a scan, a low
//! tone for a code that is not known, a chime for a sale paid; on phones a
//! light buzz, and a stronger one on an error. Each is a setting of this
//! device (settings › devices); a device with no sound, or a phone with
//! haptics turned off, simply stays quiet.
//!
//! The sounds are made for the app (`sounds/`, short WAV files) and embedded:
//! they are decoded once, at the start, and play at once.

use std::cell::RefCell;

use nsh_core::prelude::*;

use crate::state::AppState;

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// A product went into the cart.
    Scanned,
    /// The code is not known.
    Unknown,
    /// The sale is paid.
    Paid,
    /// Something was refused.
    Error,
}

static SCAN: &[u8] = include_bytes!("../../sounds/scan.wav");
static UNKNOWN: &[u8] = include_bytes!("../../sounds/unknown.wav");
static PAID: &[u8] = include_bytes!("../../sounds/paid.wav");

thread_local! {
    /// The sounds, once decoded: scan, unknown, paid.
    static SOUNDS: RefCell<Option<[Sound; 3]>> = const { RefCell::new(None) };
}

/// Decodes the sounds, from the root (a screen's task would end with it).
pub fn preload(st: AppState) {
    st.root.run(|| {
        let task = spawn(async move {
            if !audio_available().await {
                return;
            }
            let loaded = async {
                Ok::<_, AudioFailure>([
                    load_sound(SCAN).await?,
                    load_sound(UNKNOWN).await?,
                    load_sound(PAID).await?,
                ])
            };
            match loaded.await {
                Ok(sounds) => SOUNDS.with(|s| *s.borrow_mut() = Some(sounds)),
                Err(e) => log::warn!("sounds: {}", e.message),
            }
        });
        drop(task); // runs to completion under the root
    });
}

/// Plays `cue` as this device is set to.
pub fn cue(st: AppState, cue: Cue) {
    if st.biz.scan_sound.get_untracked() {
        let which = match cue {
            Cue::Scanned => Some(0),
            Cue::Unknown | Cue::Error => Some(1),
            Cue::Paid => Some(2),
        };
        if let Some(i) = which {
            SOUNDS.with(|s| {
                if let Some(sounds) = s.borrow().as_ref() {
                    sounds[i].play(0.8, 1.0);
                }
            });
        }
    }
    if st.biz.haptics.get_untracked() {
        haptic(match cue {
            Cue::Scanned => Haptic::Impact(HapticImpact::Light),
            Cue::Paid => Haptic::Notification(HapticNotification::Success),
            Cue::Unknown => Haptic::Notification(HapticNotification::Warning),
            Cue::Error => Haptic::Notification(HapticNotification::Error),
        });
    }
}

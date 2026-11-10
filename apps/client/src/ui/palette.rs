//! The palette of `docs/design/DESIGN.md` §3, as tones with a light and a
//! dark value. `theme.json` holds the same values for nsh-base surfaces;
//! these are for app-drawn elements (icons, money states).

use nsh_core::prelude::*;

/// A semantic color. Money states carry meaning only (paid, owed, warn).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ink,
    Muted,
    Primary,
    OnPrimary,
    Paid,
    Owed,
    Warn,
    Paper,
    Surface,
    Line,
    Accent,
}

impl Tone {
    /// (light, dark) as 0xRRGGBB.
    const fn rgb(self) -> (u32, u32) {
        match self {
            Tone::Ink => (0x18212B, 0xE7EBEE),
            Tone::Muted => (0x5B6670, 0x93A0AA),
            Tone::Primary => (0x1E4DB7, 0x7AA2FF),
            Tone::OnPrimary => (0xFFFFFF, 0x0F1418),
            Tone::Paid => (0x127A4F, 0x4CC48C),
            Tone::Owed => (0xB3261E, 0xFF7A70),
            Tone::Warn => (0xA15C07, 0xF0B35A),
            Tone::Paper => (0xF5F6F4, 0x0F1418),
            Tone::Surface => (0xFFFFFF, 0x161D23),
            Tone::Line => (0xE2E5E2, 0x25303A),
            Tone::Accent => (0xE7EDFA, 0x1C2A44),
        }
    }

    /// A color that follows the appearance natively.
    pub fn color(self) -> AdaptiveColor {
        let (l, d) = self.rgb();
        AdaptiveColor::new(rgb(l), rgb(d))
    }

    /// `#RRGGBB` for the current appearance (reading it subscribes to `dark()`).
    pub fn hex(self) -> String {
        let (l, d) = self.rgb();
        format!("#{:06X}", if dark() { d } else { l })
    }
}

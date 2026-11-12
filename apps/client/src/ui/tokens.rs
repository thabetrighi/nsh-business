//! Design tokens (DESIGN.md v2): colors, type scale, spacing, radii.
//!
//! The user's appearance settings (accent, text size, density, corner
//! radius) are read here from the current [`Look`]. The root installs a new
//! look and rebuilds the tree when a setting changes, so components read
//! tokens as plain values.

use std::cell::Cell;

use nsh_core::prelude::*;

// ------------------------------------------------------------------ look

/// The brand color the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accent {
    #[default]
    Blurple,
    Ocean,
    Emerald,
    Teal,
    Amber,
    Rose,
    Slate,
}

impl Accent {
    pub const ALL: [Accent; 7] = [
        Accent::Blurple,
        Accent::Ocean,
        Accent::Emerald,
        Accent::Teal,
        Accent::Amber,
        Accent::Rose,
        Accent::Slate,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Accent::Blurple => "blurple",
            Accent::Ocean => "ocean",
            Accent::Emerald => "emerald",
            Accent::Teal => "teal",
            Accent::Amber => "amber",
            Accent::Rose => "rose",
            Accent::Slate => "slate",
        }
    }

    pub fn from_code(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.code() == s)
    }

    /// (solid light, solid dark, soft light, soft dark, on-solid light, on-solid dark).
    const fn rgb(self) -> [u32; 6] {
        match self {
            Accent::Blurple => [0x635BFF, 0x8B85FF, 0xF0EFFF, 0x24224A, 0xFFFFFF, 0x0E1117],
            Accent::Ocean => [0x0570DE, 0x4C9BFF, 0xE6F0FC, 0x10294A, 0xFFFFFF, 0x0E1117],
            Accent::Emerald => [0x047857, 0x34D399, 0xE5F5EE, 0x0F2E24, 0xFFFFFF, 0x0E1117],
            Accent::Teal => [0x0F766E, 0x2DD4BF, 0xE3F4F2, 0x0D2E2C, 0xFFFFFF, 0x0E1117],
            Accent::Amber => [0xB45309, 0xFBBF24, 0xFDF1E2, 0x3A2A0C, 0xFFFFFF, 0x0E1117],
            Accent::Rose => [0xE11D48, 0xFB7185, 0xFDE8ED, 0x3A1320, 0xFFFFFF, 0x0E1117],
            Accent::Slate => [0x334155, 0xCBD5E1, 0xEEF1F5, 0x252C37, 0xFFFFFF, 0x0E1117],
        }
    }

    /// The swatch shown in settings (light value).
    pub fn swatch(self) -> AdaptiveColor {
        let c = self.rgb();
        AdaptiveColor::new(rgb(c[0]), rgb(c[1]))
    }
}

/// Text size, as a factor on the type scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextSize {
    Small,
    #[default]
    Default,
    Large,
    Larger,
}

impl TextSize {
    pub const ALL: [TextSize; 4] = [
        TextSize::Small,
        TextSize::Default,
        TextSize::Large,
        TextSize::Larger,
    ];
    pub fn code(self) -> &'static str {
        match self {
            TextSize::Small => "s",
            TextSize::Default => "m",
            TextSize::Large => "l",
            TextSize::Larger => "xl",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.code() == s)
    }
    fn factor(self) -> f32 {
        match self {
            TextSize::Small => 0.93,
            TextSize::Default => 1.0,
            TextSize::Large => 1.08,
            TextSize::Larger => 1.16,
        }
    }
}

/// How much room controls and rows take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
    Spacious,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Compact, Density::Comfortable, Density::Spacious];
    pub fn code(self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Comfortable => "comfortable",
            Density::Spacious => "spacious",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.code() == s)
    }
    fn factor(self) -> f32 {
        match self {
            Density::Compact => 0.86,
            Density::Comfortable => 1.0,
            Density::Spacious => 1.14,
        }
    }
}

/// Corner rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Corners {
    Sharp,
    #[default]
    Soft,
    Round,
}

impl Corners {
    pub const ALL: [Corners; 3] = [Corners::Sharp, Corners::Soft, Corners::Round];
    pub fn code(self) -> &'static str {
        match self {
            Corners::Sharp => "sharp",
            Corners::Soft => "soft",
            Corners::Round => "round",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.code() == s)
    }
    fn base(self) -> f32 {
        match self {
            Corners::Sharp => 4.0,
            Corners::Soft => 8.0,
            Corners::Round => 12.0,
        }
    }
}

/// The interface font (bundled in `assets/fonts`, all OFL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiFont {
    Cairo,
    #[default]
    Tajawal,
    Almarai,
    ReadexPro,
    IbmPlex,
}

impl UiFont {
    pub const ALL: [UiFont; 5] = [
        UiFont::Tajawal,
        UiFont::Cairo,
        UiFont::Almarai,
        UiFont::ReadexPro,
        UiFont::IbmPlex,
    ];
    pub fn code(self) -> &'static str {
        match self {
            UiFont::Cairo => "cairo",
            UiFont::Tajawal => "tajawal",
            UiFont::Almarai => "almarai",
            UiFont::ReadexPro => "readex",
            UiFont::IbmPlex => "plex",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.code() == s)
    }
    /// The family name the font files register.
    pub fn family(self) -> &'static str {
        match self {
            UiFont::Cairo => "Cairo",
            UiFont::Tajawal => "Tajawal",
            UiFont::Almarai => "Almarai",
            UiFont::ReadexPro => "Readex Pro",
            UiFont::IbmPlex => "IBM Plex Sans Arabic",
        }
    }
}

/// Figures (money, quantities, table numbers) always use a font with
/// equal-width digits, whatever the interface font: columns line up.
pub const FIGURE_FONT: &str = "Cairo";

/// The appearance settings the tokens are computed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Look {
    pub accent: Accent,
    pub text: TextSize,
    pub density: Density,
    pub corners: Corners,
    pub font: UiFont,
}

thread_local! {
    static LOOK: Cell<Look> = const { Cell::new(Look {
        accent: Accent::Blurple,
        text: TextSize::Default,
        density: Density::Comfortable,
        corners: Corners::Soft,
        font: UiFont::Tajawal,
    }) };
}

/// Makes `look` the one tokens are computed from (the root calls it before
/// building the tree).
pub fn install(look: Look) {
    LOOK.with(|l| l.set(look));
}

pub fn look() -> Look {
    LOOK.with(Cell::get)
}

thread_local! {
    static RTL: Cell<bool> = const { Cell::new(false) };
}

/// Whether the interface reads right to left (set with the look).
pub fn set_rtl(on: bool) {
    RTL.with(|r| r.set(on));
}

pub fn rtl() -> bool {
    RTL.with(Cell::get)
}

// ------------------------------------------------------------------ colors

/// A semantic color with a light and a dark value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Page background.
    Canvas,
    /// Sidebar and quiet panels.
    Sidebar,
    /// Cards, dialogs, inputs.
    Surface,
    /// Table headers, input wells, segmented tracks.
    Subtle,
    /// Hovered rows and items.
    Hover,
    Border,
    BorderStrong,
    /// Titles and key figures.
    Ink,
    /// Body text.
    Body,
    /// Secondary text, labels.
    Muted,
    /// Placeholders, disabled text.
    Faint,
    /// The user's accent.
    Primary,
    PrimarySoft,
    OnPrimary,
    Success,
    SuccessSoft,
    Warning,
    WarningSoft,
    Danger,
    DangerSoft,
    Info,
    InfoSoft,
    Neutral,
    NeutralSoft,
    /// Toasts and tooltips (inverted surface).
    Inverse,
    OnInverse,
    /// A success mark on the inverse surface.
    InverseSuccess,
}

impl Tone {
    /// (light, dark) as 0xRRGGBB.
    fn rgb(self) -> (u32, u32) {
        let a = look().accent.rgb();
        match self {
            Tone::Canvas => (0xFFFFFF, 0x0D1117),
            Tone::Sidebar => (0xF7F8FA, 0x11161D),
            Tone::Surface => (0xFFFFFF, 0x161B22),
            Tone::Subtle => (0xF6F8FA, 0x1B2129),
            Tone::Hover => (0xF1F3F6, 0x212833),
            Tone::Border => (0xEBEEF1, 0x262D38),
            Tone::BorderStrong => (0xD5DBE1, 0x353E4B),
            Tone::Ink => (0x1A1B25, 0xEDF0F4),
            Tone::Body => (0x414552, 0xC4CBD5),
            Tone::Muted => (0x687385, 0x8D97A7),
            Tone::Faint => (0xA3ACBA, 0x5D6777),
            Tone::Primary => (a[0], a[1]),
            Tone::PrimarySoft => (a[2], a[3]),
            Tone::OnPrimary => (a[4], a[5]),
            Tone::Success => (0x0E6B2E, 0x6BD48F),
            Tone::SuccessSoft => (0xD7F7C2, 0x12301D),
            Tone::Warning => (0xA03A00, 0xF5B65A),
            Tone::WarningSoft => (0xFCEDB9, 0x3A2A0C),
            Tone::Danger => (0xB3093C, 0xFF7EA6),
            Tone::DangerSoft => (0xFFE7F0, 0x3D1020),
            Tone::Info => (0x0055BC, 0x6CB4FF),
            Tone::InfoSoft => (0xD8ECFF, 0x0E2A3D),
            Tone::Neutral => (0x545969, 0xA9B2C0),
            Tone::NeutralSoft => (0xEBEEF1, 0x262D38),
            Tone::Inverse => (0x1A1B25, 0xEDF0F4),
            Tone::OnInverse => (0xFFFFFF, 0x0D1117),
            Tone::InverseSuccess => (0x6BD48F, 0x0E6B2E),
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

/// The colors of a status: text/icon and soft background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Success,
    Warning,
    Danger,
    Info,
    Neutral,
    Accent,
}

impl Status {
    pub fn fg(self) -> Tone {
        match self {
            Status::Success => Tone::Success,
            Status::Warning => Tone::Warning,
            Status::Danger => Tone::Danger,
            Status::Info => Tone::Info,
            Status::Neutral => Tone::Neutral,
            Status::Accent => Tone::Primary,
        }
    }
    pub fn bg(self) -> Tone {
        match self {
            Status::Success => Tone::SuccessSoft,
            Status::Warning => Tone::WarningSoft,
            Status::Danger => Tone::DangerSoft,
            Status::Info => Tone::InfoSoft,
            Status::Neutral => Tone::NeutralSoft,
            Status::Accent => Tone::PrimarySoft,
        }
    }
}

// ------------------------------------------------------------------ scales

/// Type scale (points at the default text size).
pub mod ty {
    /// Badges, table meta, axis labels.
    pub const XS: f32 = 12.0;
    /// Table cells, secondary text, buttons.
    pub const SM: f32 = 13.0;
    /// Body.
    pub const MD: f32 = 14.0;
    /// Lead, card titles.
    pub const LG: f32 = 16.0;
    /// Section titles.
    pub const XL: f32 = 18.0;
    /// Page titles.
    pub const H2: f32 = 24.0;
    /// KPI figures.
    pub const H1: f32 = 28.0;
    /// Hero figure.
    pub const DISPLAY: f32 = 34.0;
}

/// A part of the screen drawn larger or smaller than the rest (a till's
/// panel set to large type): provided above it, its text, icons and
/// controls follow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale(pub f32);

/// The scale provided above what is being made (1 where none is).
pub fn scale() -> f32 {
    nsh_core::prelude::use_context::<Scale>().map_or(1.0, |s| s.0)
}

/// A type size scaled by the user's text size (and a part's scale).
pub fn fs(base: f32) -> f32 {
    (base * look().text.factor() * scale()).round()
}

/// A length scaled by density (control heights, paddings, row heights).
pub fn sp(base: f32) -> f32 {
    (base * look().density.factor()).round()
}

/// Corner radii, from the user's setting.
pub mod radius {
    use super::look;
    /// Badges, checkboxes, kbd.
    pub fn sm() -> f32 {
        (look().corners.base() * 0.5).max(3.0)
    }
    /// Buttons, inputs, menu items.
    pub fn md() -> f32 {
        look().corners.base() * 0.75
    }
    /// Cards, tables.
    pub fn lg() -> f32 {
        look().corners.base()
    }
    /// Dialogs and sheets.
    pub fn xl() -> f32 {
        look().corners.base() * 1.5
    }
}

/// Control heights (points at comfortable density).
pub mod control {
    use super::{scale, sp};
    pub fn sm() -> f32 {
        (sp(28.0) * scale()).round()
    }
    pub fn md() -> f32 {
        (sp(34.0) * scale()).round()
    }
    pub fn lg() -> f32 {
        (sp(42.0) * scale()).round()
    }
}

/// Chart series colors: the accent first, then quiet companions.
pub fn series_hex(i: usize) -> String {
    match i {
        0 => Tone::Primary.hex(),
        1 => {
            if dark() {
                "#4C5566".into()
            } else {
                "#C9CFD8".into()
            }
        }
        2 => Tone::Info.hex(),
        3 => Tone::Success.hex(),
        _ => Tone::Warning.hex(),
    }
}

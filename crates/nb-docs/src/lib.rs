//! Documents (M12): receipts, invoices, vouchers.
//!
//! One model ([`Doc`]) is laid out once with cosmic-text — Arabic shaping,
//! bidi, line breaking, with the fonts bundled here so every device prints
//! the same — and rendered three ways:
//!
//! - [`escpos`]: a 1-bit raster for thermal printers (`GS v 0`), which
//!   prints Arabic on printers that have no Arabic code page;
//! - [`pdf`]: the same glyphs placed in a PDF (A4, A5), text selectable;
//! - [`preview`]: a PNG of the page for the screen.
//!
//! Money arrives already formatted by the templates ([`templates`]): this
//! crate draws text, it never computes amounts. Geometry is in floating
//! points (dots and points), never money.
#![allow(clippy::float_arithmetic)] // page geometry only; money is text here

pub mod barcode;
mod escpos;
pub mod label;
mod layout;
pub mod logo;
mod pdf;
mod preview;
pub mod render;
pub mod templates;

pub use escpos::{EscPos, OPEN_DRAWER, decode as decode_escpos, escpos, escpos_with};
pub use pdf::{page_count, pdf};
pub use preview::preview;

/// The paper a document is laid out for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Paper {
    /// A thermal roll, by its printable width in dots (203 dpi: 8 dots a
    /// millimetre); its length follows the content.
    Roll {
        dots: u32,
    },
    /// A label of a fixed size in millimetres (thermal, 203 dpi).
    Label {
        w_mm: u16,
        h_mm: u16,
    },
    A4,
    A5,
}

impl Paper {
    /// 58 mm paper, 48 mm printed (384 dots).
    pub const ROLL58: Paper = Paper::Roll { dots: 384 };
    /// 80 mm paper, 72 mm printed (576 dots).
    pub const ROLL80: Paper = Paper::Roll { dots: 576 };

    /// A roll printing `mm` millimetres across (whole bytes of dots).
    pub fn roll_mm(mm: u16) -> Self {
        let dots = (u32::from(mm.clamp(20, 120)) * 8) / 8 * 8;
        Paper::Roll { dots }
    }

    /// What a roll prints on paper `paper_mm` wide: the usual printable
    /// width of thermal heads (58 → 48, 80 → 72), else 5 mm off each side.
    pub fn printable_mm(paper_mm: u16) -> u16 {
        match paper_mm {
            58 => 48,
            80 => 72,
            w => w.saturating_sub(10).max(20),
        }
    }

    pub fn is_roll(self) -> bool {
        matches!(self, Paper::Roll { .. })
    }

    /// Drawn in thermal dots (a roll or a label), not on a sheet.
    pub fn is_thermal(self) -> bool {
        matches!(self, Paper::Roll { .. } | Paper::Label { .. })
    }

    /// Narrow enough to need the small type (58 mm and below).
    pub fn is_narrow(self) -> bool {
        matches!(self, Paper::Roll { dots } if dots <= 400)
    }

    /// Printable dots across a roll or a label.
    pub fn dots(self) -> u32 {
        match self {
            Paper::Roll { dots } => dots,
            Paper::Label { w_mm, .. } => u32::from(w_mm) * 8,
            Paper::A4 | Paper::A5 => 576,
        }
    }

    /// The sheet in points (1/72 inch), portrait; a roll's height is 0
    /// (it follows the content).
    pub fn sheet(self) -> (f32, f32) {
        match self {
            Paper::A4 => (595.28, 841.89),
            Paper::A5 => (419.53, 595.28),
            Paper::Label { w_mm, h_mm } => {
                (f32::from(w_mm) * 72.0 / 25.4, f32::from(h_mm) * 72.0 / 25.4)
            }
            r => (r.dots() as f32 * 72.0 / 203.0, 0.0),
        }
    }
}

/// Where a text sits on its line, in reading order: in Arabic the start is
/// on the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Start,
    Center,
    End,
}

/// A cell of a [`Block::Row`]: its text, its share of the width (the
/// shares of a row add up to 1), where it sits.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub text: String,
    pub share: f32,
    pub place: Place,
}

impl Cell {
    pub fn new(text: impl Into<String>, share: f32, place: Place) -> Self {
        Self {
            text: text.into(),
            share,
            place,
        }
    }
}

/// A piece of a document, top to bottom. Sizes are in points.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A paragraph across the width; it wraps.
    Text {
        text: String,
        size: f32,
        bold: bool,
        place: Place,
    },
    /// A label at the start and a value at the end of one line (the value
    /// goes under the label when both do not fit).
    Pair {
        start: String,
        end: String,
        size: f32,
        bold: bool,
    },
    /// Columns side by side (a table line); each cell wraps in its column.
    Row {
        cells: Vec<Cell>,
        size: f32,
        bold: bool,
    },
    /// A dashed line across.
    Rule,
    /// A solid line across (table borders).
    Line,
    /// A barcode, centred: its modules (`true` = bar) drawn `width` points
    /// wide and `height` high, with its text under it.
    Bars {
        modules: Vec<bool>,
        width: f32,
        height: f32,
        caption: Option<String>,
    },
    /// A picture (a logo), centred, `width` points wide: grey levels row
    /// by row (0 black, 255 white).
    Image {
        gray: Vec<u8>,
        w: u32,
        h: u32,
        width: f32,
    },
    /// Empty space, in points.
    Gap(f32),
    /// Blocks side by side, each column a share of the width (in reading
    /// order: the first is on the right in Arabic).
    Columns {
        cols: Vec<(f32, Vec<Block>)>,
        gap: f32,
    },
    /// Blocks on a filled and/or bordered box, `pad` points inside.
    Panel {
        blocks: Vec<Block>,
        fill: Option<Rgb>,
        border: Option<Rgb>,
        pad: f32,
    },
    /// Blocks whose text is drawn in `color`.
    Paint { color: Rgb, blocks: Vec<Block> },
}

/// A colour: red, green, blue.
pub type Rgb = [u8; 3];

/// Black, the ink of thermal printers.
pub const INK: Rgb = [0, 0, 0];

/// `#RRGGBB` as a colour (black when it is not one).
pub fn rgb(hex: &str) -> Rgb {
    let h = hex.trim_start_matches('#');
    let part = |i: usize| {
        h.get(i..i + 2)
            .and_then(|p| u8::from_str_radix(p, 16).ok())
            .unwrap_or(0)
    };
    if h.len() == 6 {
        [part(0), part(2), part(4)]
    } else {
        INK
    }
}

/// `c` mixed with white: `keep` of 255 of the colour (a tint).
pub fn tint(c: Rgb, keep: u8) -> Rgb {
    let mix =
        |v: u8| ((u16::from(v) * u16::from(keep) + 255 * (255 - u16::from(keep))) / 255) as u8;
    [mix(c[0]), mix(c[1]), mix(c[2])]
}

/// Perceived lightness, 0–255.
pub fn luma(c: Rgb) -> u8 {
    ((u32::from(c[0]) * 299 + u32::from(c[1]) * 587 + u32::from(c[2]) * 114) / 1000) as u8
}

impl Block {
    pub fn text(text: impl Into<String>, size: f32, bold: bool, place: Place) -> Self {
        Block::Text {
            text: text.into(),
            size,
            bold,
            place,
        }
    }

    pub fn pair(start: impl Into<String>, end: impl Into<String>, size: f32, bold: bool) -> Self {
        Block::Pair {
            start: start.into(),
            end: end.into(),
            size,
            bold,
        }
    }
}

/// A document: blocks in a reading direction, for a paper.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub paper: Paper,
    /// Right to left (Arabic).
    pub rtl: bool,
    pub blocks: Vec<Block>,
}

impl Doc {
    /// The document's text, line by line in logical order (tests, search).
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        plain(&self.blocks, &mut out);
        nb_i18n::bidi::strip_controls(&out)
    }
}

fn plain(blocks: &[Block], out: &mut String) {
    {
        for b in blocks {
            match b {
                Block::Text { text, .. } => out.push_str(text),
                Block::Pair { start, end, .. } => {
                    out.push_str(start);
                    out.push_str(" | ");
                    out.push_str(end);
                }
                Block::Row { cells, .. } => {
                    let v: Vec<&str> = cells.iter().map(|c| c.text.as_str()).collect();
                    out.push_str(&v.join(" | "));
                }
                Block::Rule | Block::Line => out.push_str("---"),
                Block::Bars { caption, .. } => match caption {
                    Some(c) => out.push_str(&format!("[{c}]")),
                    None => out.push_str("[bars]"),
                },
                Block::Image { .. } => out.push_str("[image]"),
                Block::Gap(_) => continue,
                Block::Columns { cols, .. } => {
                    for (_, c) in cols {
                        plain(c, out);
                    }
                    continue;
                }
                Block::Panel { blocks, .. } | Block::Paint { blocks, .. } => {
                    plain(blocks, out);
                    continue;
                }
            }
            out.push('\n');
        }
    }
}

/// Why a document could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocError(pub String);

impl std::fmt::Display for DocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "document: {}", self.0)
    }
}

impl std::error::Error for DocError {}

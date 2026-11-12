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

mod escpos;
mod layout;
mod pdf;
mod preview;
pub mod templates;

pub use escpos::{EscPos, OPEN_DRAWER, decode as decode_escpos, escpos};
pub use pdf::pdf;
pub use preview::preview;

/// The paper a document is laid out for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Paper {
    /// Thermal roll, 58 mm (384 dots at 203 dpi).
    Roll58,
    /// Thermal roll, 80 mm (576 dots at 203 dpi).
    Roll80,
    A4,
    A5,
}

impl Paper {
    pub fn code(self) -> &'static str {
        match self {
            Paper::Roll58 => "58mm",
            Paper::Roll80 => "80mm",
            Paper::A4 => "a4",
            Paper::A5 => "a5",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        [Paper::Roll58, Paper::Roll80, Paper::A4, Paper::A5]
            .into_iter()
            .find(|p| p.code() == code)
    }

    pub fn is_roll(self) -> bool {
        matches!(self, Paper::Roll58 | Paper::Roll80)
    }

    /// Printable dots across a roll.
    pub fn dots(self) -> u32 {
        match self {
            Paper::Roll58 => 384,
            Paper::A4 | Paper::A5 | Paper::Roll80 => 576,
        }
    }

    /// The sheet in points (1/72 inch), portrait.
    pub fn sheet(self) -> (f32, f32) {
        match self {
            Paper::A4 => (595.28, 841.89),
            Paper::A5 => (419.53, 595.28),
            // A roll's width in points at 203 dpi; its height follows the content.
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
    /// Empty space, in points.
    Gap(f32),
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
        for b in &self.blocks {
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
                Block::Rule => out.push_str("---"),
                Block::Gap(_) => continue,
            }
            out.push('\n');
        }
        nb_i18n::bidi::strip_controls(&out)
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

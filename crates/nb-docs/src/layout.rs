//! Laying out a [`Doc`]: every text shaped once (cosmic-text) with the
//! bundled fonts, placed on a page `width` wide, top to bottom.

use std::cell::RefCell;

use cosmic_text::{Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};

use crate::{Block, Doc, Place};

const ARABIC_REGULAR: &[u8] = include_bytes!("../fonts/NotoSansArabic-Regular.ttf");
const ARABIC_BOLD: &[u8] = include_bytes!("../fonts/NotoSansArabic-Bold.ttf");
const LATIN_REGULAR: &[u8] = include_bytes!("../fonts/NotoSans-Regular.ttf");
const LATIN_BOLD: &[u8] = include_bytes!("../fonts/NotoSans-Bold.ttf");
/// Arabic first; Latin letters and figures fall back to Noto Sans.
const FAMILY: &str = "Noto Sans Arabic";

thread_local! {
    /// Loading fonts takes a while: once per thread.
    static FONTS: RefCell<Option<FontSystem>> = const { RefCell::new(None) };
}

/// Runs `f` with this thread's font system (only the bundled fonts, so the
/// output is the same on every device).
pub(crate) fn with_fonts<R>(f: impl FnOnce(&mut FontSystem) -> R) -> R {
    FONTS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let fs = slot.get_or_insert_with(|| {
            let mut db = cosmic_text::fontdb::Database::new();
            for data in [ARABIC_REGULAR, ARABIC_BOLD, LATIN_REGULAR, LATIN_BOLD] {
                db.load_font_data(data.to_vec());
            }
            FontSystem::new_with_locale_and_db("ar-DZ".into(), db)
        });
        f(fs)
    })
}

/// One shaped glyph at its place (baseline), in layout units.
pub(crate) struct PlacedGlyph {
    pub font_id: cosmic_text::fontdb::ID,
    pub glyph_id: u16,
    pub x: f32,
    pub baseline: f32,
    pub size: f32,
    pub advance: f32,
    pub cluster: String,
}

/// A laid-out page: the shaped buffers (for drawing pixels), the glyphs
/// (for PDF), the rules, and the height used.
pub(crate) struct Layout {
    pub height: f32,
    pub buffers: Vec<(Buffer, f32, f32)>,
    pub glyphs: Vec<PlacedGlyph>,
    /// (y, x from, x to).
    pub rules: Vec<(f32, f32, f32)>,
}

fn align_for(place: Place, rtl: bool) -> Align {
    match (place, rtl) {
        (Place::Center, _) => Align::Center,
        (Place::Start, true) | (Place::End, false) => Align::Right,
        (Place::Start, false) | (Place::End, true) => Align::Left,
    }
}

fn shape(
    fs: &mut FontSystem,
    text: &str,
    size: f32,
    bold: bool,
    width: f32,
    align: Align,
) -> Buffer {
    let mut buf = Buffer::new(fs, Metrics::new(size, size * 1.45));
    buf.set_size(Some(width.max(1.0)), None);
    let weight = if bold { Weight::BOLD } else { Weight::NORMAL };
    let attrs = Attrs::new().family(Family::Name(FAMILY)).weight(weight);
    buf.set_text(text, &attrs, Shaping::Advanced, Some(align));
    buf.shape_until_scroll(fs, false);
    buf
}

fn widest(b: &Buffer) -> f32 {
    b.layout_runs().map(|r| r.line_w).fold(0.0_f32, f32::max)
}

impl Layout {
    /// Places a shaped buffer at (x, y); returns its height.
    fn place(&mut self, buf: Buffer, x: f32, y: f32, text: &str) -> f32 {
        let mut h: f32 = 0.0;
        for run in buf.layout_runs() {
            h = h.max(run.line_top + run.line_height);
            for g in run.glyphs {
                self.glyphs.push(PlacedGlyph {
                    font_id: g.font_id,
                    glyph_id: g.glyph_id,
                    x: x + g.x + g.font_size * g.x_offset,
                    baseline: y + run.line_y - g.font_size * g.y_offset,
                    size: g.font_size,
                    advance: g.w,
                    cluster: text.get(g.start..g.end).unwrap_or("").to_owned(),
                });
            }
        }
        self.buffers.push((buf, x, y));
        h
    }
}

/// Lays `doc` out on a page `width` wide with `margin`, `scale` units per
/// point.
pub(crate) fn layout(
    fs: &mut FontSystem,
    doc: &Doc,
    width: f32,
    margin: f32,
    scale: f32,
) -> Layout {
    let inner = width - 2.0 * margin;
    let rtl = doc.rtl;
    let mut y = margin;
    let mut out = Layout {
        height: 0.0,
        buffers: Vec::new(),
        glyphs: Vec::new(),
        rules: Vec::new(),
    };
    for b in &doc.blocks {
        match b {
            Block::Text {
                text,
                size,
                bold,
                place,
            } => {
                let buf = shape(fs, text, size * scale, *bold, inner, align_for(*place, rtl));
                y += out.place(buf, margin, y, text);
            }
            Block::Pair {
                start,
                end,
                size,
                bold,
            } => {
                // Two cells at opposite edges; when they do not fit side by
                // side (narrow paper), the end goes on its own line.
                let a = shape(
                    fs,
                    start,
                    size * scale,
                    *bold,
                    inner,
                    align_for(Place::Start, rtl),
                );
                let b2 = shape(
                    fs,
                    end,
                    size * scale,
                    *bold,
                    inner,
                    align_for(Place::End, rtl),
                );
                let fits = a.layout_runs().count() == 1
                    && b2.layout_runs().count() == 1
                    && widest(&a) + widest(&b2) + 6.0 * scale <= inner;
                let ha = out.place(a, margin, y, start);
                if fits {
                    let hb = out.place(b2, margin, y, end);
                    y += ha.max(hb);
                } else {
                    y += ha;
                    y += out.place(b2, margin, y, end);
                }
            }
            Block::Row { cells, size, bold } => {
                // Columns in reading order: from the right in Arabic.
                let gap = 4.0 * scale;
                let mut at = 0.0_f32;
                let mut h: f32 = 0.0;
                for c in cells {
                    let w = (inner * c.share).max(1.0);
                    let x = if rtl {
                        margin + inner - at - w
                    } else {
                        margin + at
                    };
                    let buf = shape(
                        fs,
                        &c.text,
                        size * scale,
                        *bold,
                        (w - gap).max(1.0),
                        align_for(c.place, rtl),
                    );
                    let cx = if rtl { x + gap } else { x };
                    h = h.max(out.place(buf, cx, y, &c.text));
                    at += w;
                }
                y += h;
            }
            Block::Rule => {
                y += 3.0 * scale;
                out.rules.push((y, margin, width - margin));
                y += 4.0 * scale;
            }
            Block::Gap(g) => y += g * scale,
        }
    }
    out.height = y + margin;
    out
}

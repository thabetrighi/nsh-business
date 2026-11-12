//! Laying out a [`Doc`]: every text shaped once (cosmic-text) with the
//! bundled fonts, placed on a page `width` wide, top to bottom.

use std::cell::RefCell;

use cosmic_text::{Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};

use crate::{Block, Doc, INK, Place, Rgb};

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
    pub color: Rgb,
}

/// A picture placed: (x, y, w, h) and its grey pixels (w × h source).
pub(crate) struct PlacedImage {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub gray: Vec<u8>,
    pub src_w: u32,
    pub src_h: u32,
}

/// A laid-out page: the shaped buffers (for drawing pixels), the glyphs
/// (for PDF), the rules, boxes, pictures, and the height used.
pub(crate) struct Layout {
    pub height: f32,
    /// Shaped text at (x, y) in a colour.
    pub buffers: Vec<(Buffer, f32, f32, Rgb)>,
    pub glyphs: Vec<PlacedGlyph>,
    /// Dashed: (y, x from, x to).
    pub rules: Vec<(f32, f32, f32)>,
    /// Filled rectangles (bars, lines, panels): (x, y, w, h, colour),
    /// drawn in order (backgrounds first).
    pub rects: Vec<(f32, f32, f32, f32, Rgb)>,
    pub images: Vec<PlacedImage>,
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
    /// Places a shaped buffer at (x, y) in `color`; returns its height.
    fn place(&mut self, buf: Buffer, x: f32, y: f32, text: &str, color: Rgb) -> f32 {
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
                    color,
                });
            }
        }
        self.buffers.push((buf, x, y, color));
        h
    }
}

/// Where blocks are laid: from `x`, `w` wide, text in `color`.
#[derive(Clone, Copy)]
struct Frame {
    x: f32,
    w: f32,
    color: Rgb,
}

struct Ctx<'a> {
    fs: &'a mut FontSystem,
    rtl: bool,
    scale: f32,
    out: Layout,
}

impl Ctx<'_> {
    /// Lays `blocks` in `f` from `y`; returns the y after them.
    fn blocks(&mut self, blocks: &[Block], f: Frame, mut y: f32) -> f32 {
        for b in blocks {
            y = self.block(b, f, y);
        }
        y
    }

    fn block(&mut self, b: &Block, f: Frame, mut y: f32) -> f32 {
        let (rtl, scale) = (self.rtl, self.scale);
        let inner = f.w;
        match b {
            Block::Text {
                text,
                size,
                bold,
                place,
            } => {
                let buf = shape(
                    self.fs,
                    text,
                    size * scale,
                    *bold,
                    inner,
                    align_for(*place, rtl),
                );
                y += self.out.place(buf, f.x, y, text, f.color);
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
                    self.fs,
                    start,
                    size * scale,
                    *bold,
                    inner,
                    align_for(Place::Start, rtl),
                );
                let b2 = shape(
                    self.fs,
                    end,
                    size * scale,
                    *bold,
                    inner,
                    align_for(Place::End, rtl),
                );
                let fits = a.layout_runs().count() <= 1
                    && b2.layout_runs().count() <= 1
                    && widest(&a) + widest(&b2) + 6.0 * scale <= inner;
                let ha = self.out.place(a, f.x, y, start, f.color);
                if fits {
                    let hb = self.out.place(b2, f.x, y, end, f.color);
                    y += ha.max(hb);
                } else {
                    y += ha;
                    y += self.out.place(b2, f.x, y, end, f.color);
                }
            }
            Block::Row { cells, size, bold } => {
                // Columns in reading order: from the right in Arabic.
                let gap = 4.0 * scale;
                let mut at = 0.0_f32;
                let mut h: f32 = 0.0;
                for c in cells {
                    let w = (inner * c.share).max(1.0);
                    let x = if rtl { f.x + inner - at - w } else { f.x + at };
                    let buf = shape(
                        self.fs,
                        &c.text,
                        size * scale,
                        *bold,
                        (w - gap).max(1.0),
                        align_for(c.place, rtl),
                    );
                    let cx = if rtl { x + gap } else { x };
                    h = h.max(self.out.place(buf, cx, y, &c.text, f.color));
                    at += w;
                }
                y += h;
            }
            Block::Rule => {
                y += 3.0 * scale;
                self.out.rules.push((y, f.x, f.x + inner));
                y += 4.0 * scale;
            }
            Block::Line => {
                y += 2.0 * scale;
                self.out
                    .rects
                    .push((f.x, y, inner, (0.8 * scale).max(1.0), f.color));
                y += 3.0 * scale;
            }
            Block::Bars {
                modules,
                width: w,
                height,
                caption,
            } => {
                // Whole dots per module, so every bar prints the same.
                let n = modules.len().max(1) as f32;
                let module = ((w * scale / n).floor()).max(1.0).min(inner / n);
                let total = module * n;
                let x0 = f.x + ((inner - total) / 2.0).max(0.0);
                y += 2.0 * scale;
                let mut i = 0;
                while i < modules.len() {
                    if modules[i] {
                        let start = i;
                        while i < modules.len() && modules[i] {
                            i += 1;
                        }
                        self.out.rects.push((
                            x0 + start as f32 * module,
                            y,
                            (i - start) as f32 * module,
                            height * scale,
                            INK,
                        ));
                    } else {
                        i += 1;
                    }
                }
                y += height * scale;
                if let Some(c) = caption {
                    let buf = shape(self.fs, c, 7.0 * scale, false, inner, Align::Center);
                    y += self.out.place(buf, f.x, y, c, INK);
                }
            }
            Block::Image {
                gray,
                w: iw,
                h: ih,
                width: target,
            } => {
                let w = (target * scale).min(inner);
                let h = w * (*ih).max(1) as f32 / (*iw).max(1) as f32;
                self.out.images.push(PlacedImage {
                    x: f.x + (inner - w) / 2.0,
                    y,
                    w,
                    h,
                    gray: gray.clone(),
                    src_w: *iw,
                    src_h: *ih,
                });
                y += h + 2.0 * scale;
            }
            Block::Gap(g) => y += g * scale,
            Block::Columns { cols, gap } => {
                let gap = gap * scale;
                let n = cols.len().max(1) as f32;
                let total: f32 = cols.iter().map(|c| c.0).sum::<f32>().max(0.0001);
                let usable = (inner - gap * (n - 1.0)).max(1.0);
                let mut at = 0.0_f32;
                let mut bottom = y;
                for (share, blocks) in cols {
                    let w = usable * share / total;
                    let x = if rtl { f.x + inner - at - w } else { f.x + at };
                    let end = self.blocks(blocks, Frame { x, w, ..f }, y);
                    bottom = bottom.max(end);
                    at += w + gap;
                }
                y = bottom;
            }
            Block::Panel {
                blocks,
                fill,
                border,
                pad,
            } => {
                let pad = pad * scale;
                // The box goes under its content: its rectangle first, its
                // size once the content is laid.
                let at = self.out.rects.len();
                let end = self.blocks(
                    blocks,
                    Frame {
                        x: f.x + pad,
                        w: (inner - 2.0 * pad).max(1.0),
                        ..f
                    },
                    y + pad,
                );
                let h = end + pad - y;
                let mut boxes = Vec::new();
                if let Some(b) = border {
                    let t = (0.7 * scale).max(1.0);
                    boxes.push((f.x, y, inner, h, *b));
                    if let Some(c) = fill {
                        boxes.push((f.x + t, y + t, inner - 2.0 * t, h - 2.0 * t, *c));
                    } else {
                        boxes.push((
                            f.x + t,
                            y + t,
                            inner - 2.0 * t,
                            h - 2.0 * t,
                            [255, 255, 255],
                        ));
                    }
                } else if let Some(c) = fill {
                    boxes.push((f.x, y, inner, h, *c));
                }
                for (i, r) in boxes.into_iter().enumerate() {
                    self.out.rects.insert(at + i, r);
                }
                y = end + pad;
            }
            Block::Paint { color, blocks } => {
                y = self.blocks(blocks, Frame { color: *color, ..f }, y);
            }
        }
        y
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
    let mut c = Ctx {
        fs,
        rtl: doc.rtl,
        scale,
        out: Layout {
            height: 0.0,
            buffers: Vec::new(),
            glyphs: Vec::new(),
            rules: Vec::new(),
            rects: Vec::new(),
            images: Vec::new(),
        },
    };
    let y = c.blocks(
        &doc.blocks,
        Frame {
            x: margin,
            w: width - 2.0 * margin,
            color: INK,
        },
        margin,
    );
    c.out.height = y + margin;
    c.out
}

//! M0.8 spike: Arabic receipts and invoices from one layout engine.
//!
//! One document model is laid out with cosmic-text (shaping, bidi, line
//! breaking) and rendered by two backends:
//! - thermal: 1-bit raster sent as ESC/POS `GS v 0` (works on printers with
//!   no Arabic code page), for 58 mm (384 dots) and 80 mm (576 dots);
//! - PDF: the same glyphs placed with krilla (A5), text stays selectable.
//!
//! Money is integer centimes throughout. Run: `cargo run --release`, outputs in `out/`.

use std::fs;
use std::path::Path;

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
};
use krilla::geom::{PathBuilder, Point};
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::{Data, Document};

const FONT_REGULAR: &[u8] = include_bytes!("../fonts/NotoSansArabic-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../fonts/NotoSansArabic-Bold.ttf");
const LATIN_REGULAR: &[u8] = include_bytes!("../fonts/NotoSans-Regular.ttf");
const LATIN_BOLD: &[u8] = include_bytes!("../fonts/NotoSans-Bold.ttf");

/// A font system with only the bundled fonts, so output is identical on every
/// device (no dependence on installed system fonts).
fn font_system() -> FontSystem {
    let mut db = cosmic_text::fontdb::Database::new();
    for f in [FONT_REGULAR, FONT_BOLD, LATIN_REGULAR, LATIN_BOLD] {
        db.load_font_data(f.to_vec());
    }
    FontSystem::new_with_locale_and_db("ar-DZ".into(), db)
}
const FAMILY: &str = "Noto Sans Arabic";

// ---------------------------------------------------------------- model

/// Horizontal placement of a text cell, in reading order (start = right in RTL).
#[derive(Clone, Copy)]
enum Place {
    Start,
    Center,
    End,
}

enum Block {
    /// A paragraph that wraps across the full width.
    Text { text: String, size: f32, bold: bool, place: Place },
    /// Two cells on one line: `start` at the reading start, `end` at the reading end.
    Pair { start: String, end: String, size: f32, bold: bool },
    Rule,
    Gap(f32),
}

struct Line {
    name: &'static str,
    qty_milli: i64,
    unit_minor: i64,
}

/// Amount for `qty_milli / 1000` units, rounded half away from zero to the centime.
fn line_amount(l: &Line) -> i64 {
    let n = l.qty_milli * l.unit_minor;
    (n + n.signum() * 500) / 1000
}

/// `1 234.50 دج`; the number is isolated (FSI/PDI) so bidi keeps it whole.
fn dzd(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    let units = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in units.chars().enumerate() {
        if i > 0 && (units.len() - i).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(c);
    }
    format!("\u{2068}{sign}{grouped}.{:02}\u{2069} دج", abs % 100)
}

fn qty(milli: i64) -> String {
    if milli % 1000 == 0 {
        format!("{}", milli / 1000)
    } else {
        format!("{}.{:03}", milli / 1000, milli % 1000).trim_end_matches('0').to_owned()
    }
}

fn receipt() -> Vec<Block> {
    let lines = [
        Line { name: "حليب كامل الدسم Soummam 1L", qty_milli: 3000, unit_minor: 12_000 },
        Line { name: "خبز تقليدي", qty_milli: 5000, unit_minor: 1_500 },
        Line { name: "طماطم طازجة (موزونة)", qty_milli: 1_250, unit_minor: 18_000 },
        Line { name: "زيت مائدة Elio 5L", qty_milli: 1000, unit_minor: 95_000 },
        Line { name: "قهوة مطحونة 250g", qty_milli: 2000, unit_minor: 32_050 },
    ];
    let mut b = vec![
        Block::Text { text: "متجر النور للمواد الغذائية".into(), size: 15.0, bold: true, place: Place::Center },
        Block::Text { text: "الوادي — حي 400 مسكن · الهاتف \u{2068}0550 00 00 00\u{2069}".into(), size: 9.0, bold: false, place: Place::Center },
        Block::Text { text: "NIF: 000000000000000 · RC: 00/00-0000000".into(), size: 8.0, bold: false, place: Place::Center },
        Block::Rule,
        Block::Pair { start: "فاتورة رقم D1-2026-0001234".into(), end: "2026-11-10 18:45".into(), size: 9.0, bold: false },
        Block::Pair { start: "الكاشير: أحمد".into(), end: "الصندوق 1".into(), size: 9.0, bold: false },
        Block::Rule,
    ];
    let mut subtotal = 0;
    for l in &lines {
        let amount = line_amount(l);
        subtotal += amount;
        b.push(Block::Text { text: l.name.into(), size: 10.0, bold: false, place: Place::Start });
        b.push(Block::Pair {
            start: format!("\u{2068}{}\u{2069} × {}", qty(l.qty_milli), dzd(l.unit_minor)),
            end: dzd(amount),
            size: 10.0,
            bold: false,
        });
    }
    // Prices include 19% TVA: tax = total * 19 / 119, rounded to the centime.
    let tva = (subtotal * 19 + 59) / 119;
    let paid = 300_000;
    b.extend([
        Block::Rule,
        Block::Pair { start: "المبلغ دون رسم".into(), end: dzd(subtotal - tva), size: 10.0, bold: false },
        Block::Pair { start: "الرسم على القيمة المضافة 19%".into(), end: dzd(tva), size: 10.0, bold: false },
        Block::Pair { start: "الإجمالي".into(), end: dzd(subtotal), size: 14.0, bold: true },
        Block::Pair { start: "نقداً".into(), end: dzd(paid), size: 10.0, bold: false },
        Block::Pair { start: "الباقي".into(), end: dzd(paid - subtotal), size: 10.0, bold: false },
        Block::Rule,
        Block::Text { text: "شكراً لزيارتكم — Merci de votre visite".into(), size: 9.0, bold: false, place: Place::Center },
        Block::Gap(6.0),
    ]);
    b
}

// ---------------------------------------------------------------- layout

/// One shaped glyph at an absolute position (baseline), in layout units.
struct PlacedGlyph {
    font_id: cosmic_text::fontdb::ID,
    glyph_id: u16,
    x: f32,
    baseline: f32,
    size: f32,
    advance: f32,
    cluster: String,
}

/// Laid out page: text buffers for raster drawing plus glyph and rule lists.
struct Layout {
    height: f32,
    buffers: Vec<(Buffer, f32, f32)>,
    glyphs: Vec<PlacedGlyph>,
    rules: Vec<f32>,
}

fn align_for(place: Place) -> Align {
    // Content is right-to-left: reading start is the right edge.
    match place {
        Place::Start => Align::Right,
        Place::Center => Align::Center,
        Place::End => Align::Left,
    }
}

fn shape(fs: &mut FontSystem, text: &str, size: f32, bold: bool, width: f32, place: Place) -> Buffer {
    let mut buf = Buffer::new(fs, Metrics::new(size, size * 1.45));
    buf.set_size(Some(width), None);
    let weight = if bold { Weight::BOLD } else { Weight::NORMAL };
    let attrs = Attrs::new().family(Family::Name(FAMILY)).weight(weight);
    buf.set_text(text, &attrs, Shaping::Advanced, Some(align_for(place)));
    buf.shape_until_scroll(fs, false);
    buf
}

/// Lays out blocks for a page `width` wide with `margin`, in units where `scale` maps points.
fn layout(fs: &mut FontSystem, blocks: &[Block], width: f32, margin: f32, scale: f32) -> Layout {
    let inner = width - 2.0 * margin;
    let mut y = margin;
    let mut out = Layout { height: 0.0, buffers: Vec::new(), glyphs: Vec::new(), rules: Vec::new() };
    let place_buf = |out: &mut Layout, buf: Buffer, x: f32, y: f32, text: &str| -> f32 {
        let mut h: f32 = 0.0;
        for run in buf.layout_runs() {
            h = h.max(run.line_top + run.line_height);
            for g in run.glyphs {
                out.glyphs.push(PlacedGlyph {
                    font_id: g.font_id,
                    glyph_id: g.glyph_id,
                    x: x + g.x + g.font_size * g.x_offset,
                    baseline: y + run.line_y - g.font_size * g.y_offset,
                    size: g.font_size,
                    advance: g.w,
                    cluster: text[g.start..g.end].to_owned(),
                });
            }
        }
        out.buffers.push((buf, x, y));
        h
    };
    for b in blocks {
        match b {
            Block::Text { text, size, bold, place } => {
                let buf = shape(fs, text, size * scale, *bold, inner, *place);
                y += place_buf(&mut out, buf, margin, y, text);
            }
            Block::Pair { start, end, size, bold } => {
                // Two full-width cells aligned to opposite edges. When their
                // measured widths do not fit side by side (narrow paper), the
                // end cell moves to its own line instead of overlapping.
                let a = shape(fs, start, size * scale, *bold, inner, Place::Start);
                let b2 = shape(fs, end, size * scale, *bold, inner, Place::End);
                let width_of = |b: &Buffer| b.layout_runs().map(|r| r.line_w).fold(0.0_f32, f32::max);
                let lines_of = |b: &Buffer| b.layout_runs().count();
                let fits = lines_of(&a) == 1
                    && lines_of(&b2) == 1
                    && width_of(&a) + width_of(&b2) + 6.0 * scale <= inner;
                let ha = place_buf(&mut out, a, margin, y, start);
                if fits {
                    let hb = place_buf(&mut out, b2, margin, y, end);
                    y += ha.max(hb);
                } else {
                    y += ha;
                    y += place_buf(&mut out, b2, margin, y, end);
                }
            }
            Block::Rule => {
                y += 3.0 * scale;
                out.rules.push(y);
                y += 4.0 * scale;
            }
            Block::Gap(g) => y += g * scale,
        }
    }
    out.height = y + margin;
    out
}

// ---------------------------------------------------------------- thermal backend

/// Renders to an 8-bit coverage bitmap, thresholds to 1-bit, encodes ESC/POS.
fn escpos(fs: &mut FontSystem, blocks: &[Block], dots: u32) -> (Vec<u8>, Vec<Vec<bool>>) {
    // 203 dpi printers: 8 dots per mm; 1 pt = 203/72 dots.
    let scale = 203.0 / 72.0;
    let margin = 8.0;
    let mut l = layout(fs, blocks, dots as f32, margin, scale);
    let h = l.height.ceil() as usize;
    let w = dots as usize;
    let mut gray = vec![0u8; w * h];
    let mut cache = SwashCache::new();
    for (buf, ox, oy) in &mut l.buffers {
        let (ox, oy) = (*ox as i32, *oy as i32);
        buf.draw(fs, &mut cache, Color::rgb(0, 0, 0), |x, y, cw, ch, color| {
            for dy in 0..ch as i32 {
                for dx in 0..cw as i32 {
                    let (px, py) = (ox + x + dx, oy + y + dy);
                    if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        let i = py as usize * w + px as usize;
                        gray[i] = gray[i].max(color.a());
                    }
                }
            }
        });
    }
    for &ry in &l.rules {
        let ry = ry as usize;
        for x in margin as usize..w - margin as usize {
            if (x / 4) % 2 == 0 && ry < h {
                gray[ry * w + x] = 255;
            }
        }
    }
    let bits: Vec<Vec<bool>> = (0..h).map(|y| (0..w).map(|x| gray[y * w + x] >= 110).collect()).collect();

    let mut out = vec![0x1B, 0x40]; // ESC @ : initialize
    let bytes_per_row = w.div_ceil(8);
    // Bands of at most 256 rows keep each raster command within common printer buffers.
    for band in bits.chunks(256) {
        let rows = band.len();
        out.extend([0x1D, 0x76, 0x30, 0x00]); // GS v 0, normal density
        out.extend([(bytes_per_row & 0xFF) as u8, (bytes_per_row >> 8) as u8]);
        out.extend([(rows & 0xFF) as u8, (rows >> 8) as u8]);
        for row in band {
            for byte in 0..bytes_per_row {
                let mut v = 0u8;
                for bit in 0..8 {
                    let x = byte * 8 + bit;
                    if x < w && row[x] {
                        v |= 0x80 >> bit;
                    }
                }
                out.push(v);
            }
        }
    }
    out.extend([0x1B, 0x64, 0x04]); // ESC d 4 : feed 4 lines
    out.extend([0x1D, 0x56, 0x42, 0x00]); // GS V 66 0 : partial cut
    out.extend([0x1B, 0x70, 0x00, 0x19, 0xFA]); // ESC p : open cash drawer (pin 2)
    (out, bits)
}

/// Decodes our own ESC/POS stream back to a bitmap, proving the bytes are well formed.
fn decode_escpos(data: &[u8]) -> Result<(usize, Vec<Vec<bool>>), String> {
    let mut i = 0;
    let mut rows = Vec::new();
    let mut width = 0;
    while i < data.len() {
        match data[i..] {
            [0x1B, 0x40, ..] => i += 2,
            [0x1D, 0x76, 0x30, _, xl, xh, yl, yh, ..] => {
                let bpr = xl as usize | (xh as usize) << 8;
                let n = yl as usize | (yh as usize) << 8;
                width = bpr * 8;
                let body = data.get(i + 8..i + 8 + bpr * n).ok_or("truncated raster")?;
                for r in body.chunks(bpr) {
                    rows.push((0..width).map(|x| r[x / 8] & (0x80 >> (x % 8)) != 0).collect());
                }
                i += 8 + bpr * n;
            }
            [0x1B, 0x64, _, ..] => i += 3,
            [0x1D, 0x56, 0x42, _, ..] => i += 4,
            [0x1B, 0x70, _, _, _, ..] => i += 5,
            _ => return Err(format!("unknown command at byte {i}: {:02X?}", &data[i..(i + 4).min(data.len())])),
        }
    }
    Ok((width, rows))
}

fn write_png(path: &Path, w: usize, rows: &[Vec<bool>]) -> Result<(), Box<dyn std::error::Error>> {
    let mut enc = png::Encoder::new(fs::File::create(path)?, w as u32, rows.len() as u32);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header()?;
    let px: Vec<u8> = rows.iter().flat_map(|r| r.iter().map(|&b| if b { 0 } else { 255 })).collect();
    wr.write_image_data(&px)?;
    Ok(())
}

// ---------------------------------------------------------------- PDF backend

fn pdf(fs: &mut FontSystem, blocks: &[Block]) -> Result<Vec<u8>, String> {
    // A5 portrait in points.
    let (w, h) = (419.53_f32, 595.28_f32);
    let l = layout(fs, blocks, w, 36.0, 1.0);
    // Each glyph is drawn with the exact face the shaper chose (Arabic or Latin).
    let mut fonts: std::collections::HashMap<cosmic_text::fontdb::ID, Font> = Default::default();
    for g in &l.glyphs {
        if !fonts.contains_key(&g.font_id) {
            let font = fs
                .db()
                .with_face_data(g.font_id, |data, index| Font::new(Data::from(data.to_vec()), index))
                .flatten()
                .ok_or("font face")?;
            fonts.insert(g.font_id, font);
        }
    }
    let mut doc = Document::new();
    {
        let size = krilla::geom::Size::from_wh(w, h.max(l.height)).ok_or("page size")?;
        let mut page = doc.start_page_with(PageSettings::new(size));
        let mut s = page.surface();
        s.set_fill(Some(Fill::default()));
        for g in &l.glyphs {
            let font = fonts[&g.font_id].clone();
            let glyph = KrillaGlyph::new(
                GlyphId::new(g.glyph_id as u32),
                g.advance / g.size,
                0.0,
                0.0,
                0.0,
                0..g.cluster.len(),
                None,
            );
            s.draw_glyphs(Point::from_xy(g.x, g.baseline), &[glyph], font, &g.cluster, g.size, false);
        }
        s.set_fill(None);
        s.set_stroke(Some(Stroke { width: 0.5, ..Default::default() }));
        for &ry in &l.rules {
            let mut pb = PathBuilder::new();
            pb.move_to(36.0, ry);
            pb.line_to(w - 36.0, ry);
            if let Some(p) = pb.finish() {
                s.draw_path(&p);
            }
        }
        s.finish();
        page.finish();
    }
    doc.finish().map_err(|e| format!("{e:?}"))
}

// ---------------------------------------------------------------- main

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut fs = font_system();
    fs::create_dir_all("out")?;
    let blocks = receipt();

    for (name, dots) in [("58mm", 384u32), ("80mm", 576)] {
        let t = std::time::Instant::now();
        let (bytes, _) = escpos(&mut fs, &blocks, dots);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        fs::write(format!("out/receipt-{name}.bin"), &bytes)?;
        let (w, rows) = decode_escpos(&bytes)?;
        write_png(Path::new(&format!("out/receipt-{name}.png")), w, &rows)?;
        println!("receipt {name}: {} dots × {} rows, {} bytes ESC/POS, {ms:.1} ms", w, rows.len(), bytes.len());
    }
    let t = std::time::Instant::now();
    let bytes = pdf(&mut fs, &blocks)?;
    println!("invoice A5 PDF: {} bytes, {:.1} ms", bytes.len(), t.elapsed().as_secs_f64() * 1000.0);
    fs::write("out/invoice-a5.pdf", bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_exact() {
        assert_eq!(line_amount(&Line { name: "", qty_milli: 1_250, unit_minor: 18_000 }), 22_500);
        assert_eq!(line_amount(&Line { name: "", qty_milli: 333, unit_minor: 1_000 }), 333);
        assert_eq!(dzd(123_450), "\u{2068}1 234.50\u{2069} دج");
        assert_eq!(qty(1_250), "1.25");
        assert_eq!(qty(3_000), "3");
    }

    #[test]
    fn escpos_round_trips() {
        let mut fs = font_system();
        let (bytes, bits) = escpos(&mut fs, &receipt(), 384);
        let (w, rows) = decode_escpos(&bytes).expect("decode");
        assert_eq!(w, 384);
        assert_eq!(rows, bits);
        assert!(rows.iter().flatten().filter(|&&b| b).count() > 1000, "receipt has ink");
    }
}

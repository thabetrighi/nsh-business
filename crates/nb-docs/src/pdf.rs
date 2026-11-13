//! PDF: the laid-out glyphs placed with krilla, each with the face the
//! shaper chose (Arabic or Latin); fonts embedded (subsets), text
//! selectable. Long documents flow onto more pages.

use std::collections::HashMap;

use krilla::geom::{PathBuilder, Point, Rect, Transform};
use krilla::image::Image;
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::{Data, Document};

use crate::layout::{Layout, layout, with_fonts};
use crate::{Doc, DocError};

const MARGIN: f32 = 36.0;

/// How many pages the PDF of `doc` has.
pub fn page_count(doc: &Doc) -> usize {
    if doc.paper.is_thermal() {
        return 1;
    }
    with_fonts(|fs| {
        let (w, sheet_h) = doc.paper.sheet();
        let l = layout(fs, doc, w, MARGIN, 1.0);
        let usable = sheet_h - 2.0 * MARGIN;
        (((l.height - 2.0 * MARGIN) / usable).ceil() as usize).max(1)
    })
}

type Fonts = HashMap<cosmic_text::fontdb::ID, Font>;

/// The faces `l` uses, added to `fonts`.
fn faces(fs: &mut cosmic_text::FontSystem, l: &Layout, fonts: &mut Fonts) -> Result<(), DocError> {
    for g in &l.glyphs {
        if let std::collections::hash_map::Entry::Vacant(slot) = fonts.entry(g.font_id) {
            let font = fs
                .db()
                .with_face_data(g.font_id, |data, index| {
                    Font::new(Data::from(data.to_vec()), index)
                })
                .flatten()
                .ok_or_else(|| DocError("font face".into()))?;
            slot.insert(font);
        }
    }
    Ok(())
}

/// Draws what `keep(y)` lets through of `l`, moved by (`dx`, `dy`).
fn paint(
    s: &mut krilla::surface::Surface,
    l: &Layout,
    fonts: &Fonts,
    dx: f32,
    dy: f32,
    keep: impl Fn(f32) -> bool,
) {
    let fill_of = |c: crate::Rgb| Fill {
        paint: krilla::color::rgb::Color::new(c[0], c[1], c[2]).into(),
        ..Default::default()
    };
    // Boxes and bars first: text goes on top of them.
    for &(rx, ry, rw, rh, col) in &l.rects {
        if !keep(ry) {
            continue;
        }
        s.set_fill(Some(fill_of(col)));
        let mut pb = PathBuilder::new();
        if let Some(r) = Rect::from_xywh(rx + dx, ry + dy, rw, rh) {
            pb.push_rect(r);
        }
        if let Some(path) = pb.finish() {
            s.draw_path(&path);
        }
    }
    for im in &l.images {
        if !keep(im.y) {
            continue;
        }
        let rgba: Vec<u8> = im.gray.iter().flat_map(|&g| [g, g, g, 255]).collect();
        let image = Image::from_rgba8(rgba, im.src_w, im.src_h);
        if let Some(size) = krilla::geom::Size::from_wh(im.w, im.h) {
            s.push_transform(&Transform::from_translate(im.x + dx, im.y + dy));
            s.draw_image(image, size);
            s.pop();
        }
    }
    s.set_fill(None);
    s.set_stroke(Some(Stroke {
        width: 0.5,
        paint: krilla::color::rgb::Color::new(110, 110, 110).into(),
        ..Default::default()
    }));
    for &(ry, from, to) in &l.rules {
        if !keep(ry) {
            continue;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(from + dx, ry + dy);
        pb.line_to(to + dx, ry + dy);
        if let Some(path) = pb.finish() {
            s.draw_path(&path);
        }
    }
    s.set_stroke(None);
    let mut current: Option<crate::Rgb> = None;
    for g in &l.glyphs {
        if !keep(g.baseline) {
            continue;
        }
        let Some(font) = fonts.get(&g.font_id).cloned() else {
            continue;
        };
        if current != Some(g.color) {
            s.set_fill(Some(fill_of(g.color)));
            current = Some(g.color);
        }
        let glyph = KrillaGlyph::new(
            GlyphId::new(u32::from(g.glyph_id)),
            g.advance / g.size,
            0.0,
            0.0,
            0.0,
            0..g.cluster.len(),
            None,
        );
        s.draw_glyphs(
            Point::from_xy(g.x + dx, g.baseline + dy),
            &[glyph],
            font,
            &g.cluster,
            g.size,
            false,
        );
    }
}

/// `doc` as a PDF. Roll documents become one page as long as the receipt.
pub fn pdf(doc: &Doc) -> Result<Vec<u8>, DocError> {
    with_fonts(|fs| {
        let (w, sheet_h) = doc.paper.sheet();
        let margin = if doc.paper.is_thermal() { 6.0 } else { MARGIN };
        // A roll or a label is one page: its own length, or the label's.
        let single = doc.paper.is_thermal();
        let l = layout(fs, doc, w, margin, 1.0);
        let mut fonts = Fonts::new();
        faces(fs, &l, &mut fonts)?;
        // Pages: a roll is one page; a sheet cuts the layout every
        // (height − margins), moving content up on each next page.
        let (page_h, usable) = if doc.paper.is_roll() {
            (l.height.max(1.0), l.height.max(1.0))
        } else if single {
            (sheet_h, sheet_h)
        } else {
            (sheet_h, sheet_h - 2.0 * margin)
        };
        let pages = if single {
            1
        } else {
            (((l.height - 2.0 * margin) / usable).ceil() as usize).max(1)
        };
        let mut pdf = Document::new();
        for p in 0..pages {
            let top = p as f32 * usable;
            let bottom = top + usable;
            let shift = if single { 0.0 } else { top };
            let size = krilla::geom::Size::from_wh(w, page_h)
                .ok_or_else(|| DocError("page size".into()))?;
            let mut page = pdf.start_page_with(PageSettings::new(size));
            let mut s = page.surface();
            let in_page = |y: f32| single || (y - margin >= top && y - margin < bottom);
            paint(&mut s, &l, &fonts, 0.0, -shift, in_page);
            s.finish();
            page.finish();
        }
        pdf.finish().map_err(|e| DocError(format!("{e:?}")))
    })
}

/// A sheet of labels (A4, printed on an office printer): how many across
/// and down, each label's size, and where the first one starts (mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelSheet {
    pub cols: u16,
    pub rows: u16,
    pub w_mm: f32,
    pub h_mm: f32,
    pub top_mm: f32,
    pub left_mm: f32,
    /// The space between two labels across, and down.
    pub gap_x_mm: f32,
    pub gap_y_mm: f32,
}

impl LabelSheet {
    pub fn per_page(&self) -> usize {
        usize::from(self.cols) * usize::from(self.rows)
    }
}

const PT_PER_MM: f32 = 72.0 / 25.4;

/// `count` copies of `label` on A4 sheets of `sheet`, the first `skip`
/// places left empty (a sheet already partly used).
pub fn label_sheet_pdf(
    label: &Doc,
    sheet: LabelSheet,
    count: usize,
    skip: usize,
) -> Result<Vec<u8>, DocError> {
    let per = sheet.per_page();
    if per == 0 {
        return Err(DocError("empty sheet".into()));
    }
    with_fonts(|fs| {
        let (w, h) = label.paper.sheet();
        let l = layout(fs, label, w, 6.0, 1.0);
        let mut fonts = Fonts::new();
        faces(fs, &l, &mut fonts)?;
        let (page_w, page_h) = crate::Paper::A4.sheet();
        let skip = skip.min(per - 1);
        let total = skip + count.max(1);
        let pages = total.div_ceil(per);
        let mut pdf = Document::new();
        for p in 0..pages {
            let size = krilla::geom::Size::from_wh(page_w, page_h)
                .ok_or_else(|| DocError("page size".into()))?;
            let mut page = pdf.start_page_with(PageSettings::new(size));
            let mut s = page.surface();
            for place in 0..per {
                let n = p * per + place;
                if n < skip || n >= total {
                    continue;
                }
                let (c, r) = (
                    place % usize::from(sheet.cols),
                    place / usize::from(sheet.cols),
                );
                let x = (sheet.left_mm + c as f32 * (sheet.w_mm + sheet.gap_x_mm)) * PT_PER_MM;
                let y = (sheet.top_mm + r as f32 * (sheet.h_mm + sheet.gap_y_mm)) * PT_PER_MM;
                paint(&mut s, &l, &fonts, x, y, |yy| yy < h);
            }
            s.finish();
            page.finish();
        }
        pdf.finish().map_err(|e| DocError(format!("{e:?}")))
    })
}

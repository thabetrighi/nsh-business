//! PDF: the laid-out glyphs placed with krilla, each with the face the
//! shaper chose (Arabic or Latin); fonts embedded (subsets), text
//! selectable. Long documents flow onto more pages.

use std::collections::HashMap;

use krilla::geom::{PathBuilder, Point};
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::{Data, Document};

use crate::layout::{layout, with_fonts};
use crate::{Doc, DocError};

const MARGIN: f32 = 36.0;

/// `doc` as a PDF. Roll documents become one page as long as the receipt.
pub fn pdf(doc: &Doc) -> Result<Vec<u8>, DocError> {
    with_fonts(|fs| {
        let (w, sheet_h) = doc.paper.sheet();
        let margin = if doc.paper.is_roll() { 8.0 } else { MARGIN };
        let l = layout(fs, doc, w, margin, 1.0);
        let mut fonts: HashMap<cosmic_text::fontdb::ID, Font> = HashMap::new();
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
        // Pages: a roll is one page; a sheet cuts the layout every
        // (height − margins), moving content up on each next page.
        let (page_h, usable) = if doc.paper.is_roll() {
            (l.height.max(1.0), l.height.max(1.0))
        } else {
            (sheet_h, sheet_h - 2.0 * margin)
        };
        let pages = if doc.paper.is_roll() {
            1
        } else {
            (((l.height - 2.0 * margin) / usable).ceil() as usize).max(1)
        };
        let mut pdf = Document::new();
        for p in 0..pages {
            let top = p as f32 * usable;
            let bottom = top + usable;
            let shift = if doc.paper.is_roll() { 0.0 } else { top };
            let size = krilla::geom::Size::from_wh(w, page_h)
                .ok_or_else(|| DocError("page size".into()))?;
            let mut page = pdf.start_page_with(PageSettings::new(size));
            let mut s = page.surface();
            s.set_fill(Some(Fill::default()));
            for g in &l.glyphs {
                let in_page = doc.paper.is_roll()
                    || (g.baseline - margin >= top && g.baseline - margin < bottom);
                if !in_page {
                    continue;
                }
                let Some(font) = fonts.get(&g.font_id).cloned() else {
                    continue;
                };
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
                    Point::from_xy(g.x, g.baseline - shift),
                    &[glyph],
                    font,
                    &g.cluster,
                    g.size,
                    false,
                );
            }
            s.set_fill(None);
            s.set_stroke(Some(Stroke {
                width: 0.5,
                ..Default::default()
            }));
            for &(ry, from, to) in &l.rules {
                if !doc.paper.is_roll() && !(ry - margin >= top && ry - margin < bottom) {
                    continue;
                }
                let mut pb = PathBuilder::new();
                pb.move_to(from, ry - shift);
                pb.line_to(to, ry - shift);
                if let Some(path) = pb.finish() {
                    s.draw_path(&path);
                }
            }
            s.finish();
            page.finish();
        }
        pdf.finish().map_err(|e| DocError(format!("{e:?}")))
    })
}

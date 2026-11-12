//! A PNG of a document for the screen: a roll as the printer will print
//! it (the very raster), a sheet drawn at 96 dpi in grey levels.

use crate::escpos::{DOTS_PER_PT, paint, raster};
use crate::{Doc, DocError};

pub(crate) fn png_of(width: usize, height: usize, gray: &[u8]) -> Result<Vec<u8>, DocError> {
    let mut out = Vec::new();
    {
        let w = u32::try_from(width).map_err(|_| DocError("width".into()))?;
        let h = u32::try_from(height).map_err(|_| DocError("height".into()))?;
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| DocError(e.to_string()))?;
        wr.write_image_data(gray)
            .map_err(|e| DocError(e.to_string()))?;
    }
    Ok(out)
}

fn png_rgb(width: usize, height: usize, rgb: &[u8]) -> Result<Vec<u8>, DocError> {
    let mut out = Vec::new();
    {
        let w = u32::try_from(width).map_err(|_| DocError("width".into()))?;
        let h = u32::try_from(height).map_err(|_| DocError("height".into()))?;
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| DocError(e.to_string()))?;
        wr.write_image_data(rgb)
            .map_err(|e| DocError(e.to_string()))?;
    }
    Ok(out)
}

/// Pixels per point of a sheet's preview (144 dpi: sharp when shown
/// smaller).
const SHEET_SCALE: f32 = 2.0;

/// The document as a PNG and its size in pixels: a roll or a label as the
/// printer prints it (its very dots); a sheet in colour, as on paper.
pub fn preview(doc: &Doc) -> Result<(Vec<u8>, u32, u32), DocError> {
    let size = |w: usize, h: usize| (u32::try_from(w).unwrap_or(1), u32::try_from(h).unwrap_or(1));
    if doc.paper.is_thermal() {
        let rows = if matches!(doc.paper, crate::Paper::Label { .. }) {
            crate::label::label_rows(doc)?
        } else {
            raster(doc, doc.paper.dots(), 8.0, DOTS_PER_PT)
        };
        let width = rows.first().map_or(1, Vec::len);
        let height = rows.len().max(1);
        let gray: Vec<u8> = rows
            .iter()
            .flat_map(|r| r.iter().map(|&b| if b { 0 } else { 255 }))
            .collect();
        let (w, h) = size(width, height);
        return Ok((png_of(width, height, &gray)?, w, h));
    }
    let (sw, sh) = doc.paper.sheet();
    let (width, h, c) = paint(
        doc,
        (sw * SHEET_SCALE) as u32,
        36.0 * SHEET_SCALE,
        SHEET_SCALE,
        false,
    );
    // At least one page tall, like the paper.
    let height = h.max((sh * SHEET_SCALE) as usize);
    let mut rgb = Vec::with_capacity(width * height * 3);
    for p in &c {
        rgb.extend_from_slice(p);
    }
    rgb.resize(width * height * 3, 255);
    let (w, hh) = size(width, height);
    Ok((png_rgb(width, height, &rgb)?, w, hh))
}

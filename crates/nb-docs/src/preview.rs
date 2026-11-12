//! A PNG of a document for the screen: a roll as the printer will print
//! it (the very raster), a sheet drawn at 96 dpi in grey levels.

use crate::escpos::{DOTS_PER_PT, raster};
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

/// The document as a PNG (white page, black ink), and its size in pixels.
pub fn preview(doc: &Doc) -> Result<(Vec<u8>, u32, u32), DocError> {
    let rows = if matches!(doc.paper, crate::Paper::Label { .. }) {
        crate::label::label_rows(doc)?
    } else if doc.paper.is_roll() {
        raster(doc, doc.paper.dots(), 8.0, DOTS_PER_PT)
    } else {
        // A sheet at 96 dpi, with the PDF's margins.
        let (w, _) = doc.paper.sheet();
        let scale = 96.0 / 72.0;
        raster(doc, (w * scale) as u32, 36.0 * scale, scale)
    };
    let width = rows.first().map_or(1, Vec::len);
    let height = rows.len().max(1);
    let gray: Vec<u8> = rows
        .iter()
        .flat_map(|r| r.iter().map(|&b| if b { 0 } else { 255 }))
        .collect();
    let bytes = png_of(width, height, &gray)?;
    Ok((
        bytes,
        u32::try_from(width).unwrap_or(1),
        u32::try_from(height).unwrap_or(1),
    ))
}

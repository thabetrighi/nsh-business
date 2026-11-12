//! The shop's logo for its documents: a PNG or JPEG brought to grey
//! levels, no wider than a receipt prints (576 dots), stored as a PNG.

use std::io::Cursor;

use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::DocError;

/// A picture in grey levels (0 black, 255 white), row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gray {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

/// The widest a logo is kept: an 80 mm receipt's dots.
pub const MAX_W: u32 = 576;

fn err(e: impl std::fmt::Display) -> DocError {
    DocError(e.to_string())
}

fn from_png(bytes: &[u8]) -> Result<Gray, DocError> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(err)?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| DocError("image too large".into()))?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(err)?;
    let (w, h) = (info.width, info.height);
    let n = (w as usize) * (h as usize);
    let px: Vec<u8> = match info.color_type {
        png::ColorType::Grayscale => buf[..n].to_vec(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .take(n)
            .map(|p| over_white(p[0], p[1]))
            .collect(),
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .take(n)
            .map(|p| luma(p[0], p[1], p[2]))
            .collect(),
        png::ColorType::Rgba => buf
            .chunks_exact(4)
            .take(n)
            .map(|p| over_white(luma(p[0], p[1], p[2]), p[3]))
            .collect(),
        png::ColorType::Indexed => return Err(DocError("indexed png".into())),
    };
    Ok(Gray { w, h, px })
}

fn from_jpeg(bytes: &[u8]) -> Result<Gray, DocError> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::Luma);
    let mut dec = JpegDecoder::new_with_options(Cursor::new(bytes), options);
    let px = dec.decode().map_err(|e| DocError(format!("{e:?}")))?;
    let info = dec.info().ok_or_else(|| DocError("jpeg".into()))?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    Ok(Gray { w, h, px })
}

/// Rec. 601 luma, in integers.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000) as u8
}

/// A pixel over a white page.
fn over_white(v: u8, a: u8) -> u8 {
    let (v, a) = (u32::from(v), u32::from(a));
    ((v * a + 255 * (255 - a)) / 255) as u8
}

/// Reads a PNG or JPEG.
pub fn decode(bytes: &[u8]) -> Result<Gray, DocError> {
    let g = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        from_png(bytes)?
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        from_jpeg(bytes)?
    } else {
        return Err(DocError("PNG or JPEG only".into()));
    };
    if g.w == 0 || g.h == 0 || g.px.len() < (g.w as usize) * (g.h as usize) {
        return Err(DocError("empty image".into()));
    }
    Ok(g)
}

/// No wider than `max_w`: averaged down (a box filter).
pub fn shrink(g: &Gray, max_w: u32) -> Gray {
    if g.w <= max_w {
        return g.clone();
    }
    let w = max_w.max(1);
    let h = ((u64::from(g.h) * u64::from(w)) / u64::from(g.w)).max(1) as u32;
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let (y0, y1) = (y * g.h / h, ((y + 1) * g.h / h).max(y * g.h / h + 1));
        for x in 0..w {
            let (x0, x1) = (x * g.w / w, ((x + 1) * g.w / w).max(x * g.w / w + 1));
            let mut sum = 0u64;
            let mut n = 0u64;
            for sy in y0..y1.min(g.h) {
                for sx in x0..x1.min(g.w) {
                    sum += u64::from(g.px[(sy * g.w + sx) as usize]);
                    n += 1;
                }
            }
            px.push((sum / n.max(1)) as u8);
        }
    }
    Gray { w, h, px }
}

/// A logo as kept: decoded, at most [`MAX_W`] wide, a grey PNG.
pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>, DocError> {
    let g = shrink(&decode(bytes)?, MAX_W);
    crate::preview::png_of(g.w as usize, g.h as usize, &g.px)
}

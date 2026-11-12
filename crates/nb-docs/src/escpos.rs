//! Thermal printers: the page drawn to a 1-bit raster at 203 dpi and sent
//! as ESC/POS `GS v 0` bands, then fed, cut, and the drawer opened if
//! asked. Works on printers with no Arabic code page.

use cosmic_text::{Color, SwashCache};

use crate::layout::{layout, with_fonts};
use crate::{Doc, DocError};

/// What a thermal printout is: the bytes to send, and the picture they
/// print (for a preview, or a test).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscPos {
    pub bytes: Vec<u8>,
    pub width: usize,
    /// Rows of dots, `true` = black.
    pub rows: Vec<Vec<bool>>,
}

/// ESC p: a pulse on the drawer kick-out connector (pin 2).
pub const OPEN_DRAWER: [u8; 5] = [0x1B, 0x70, 0x00, 0x19, 0xFA];

/// 203 dpi: dots per point.
pub(crate) const DOTS_PER_PT: f32 = 203.0 / 72.0;

/// Draws `doc` on `dots` across at `scale` dots per point; 8-bit
/// coverage then a threshold.
pub(crate) fn raster(doc: &Doc, dots: u32, margin_dots: f32, scale: f32) -> Vec<Vec<bool>> {
    with_fonts(|fs| {
        let mut l = layout(fs, doc, dots as f32, margin_dots, scale);
        let h = l.height.ceil().max(1.0) as usize;
        let w = dots as usize;
        let mut gray = vec![0u8; w * h];
        let mut cache = SwashCache::new();
        for (buf, ox, oy) in &mut l.buffers {
            let (ox, oy) = (*ox as i32, *oy as i32);
            buf.draw(
                fs,
                &mut cache,
                Color::rgb(0, 0, 0),
                |x, y, cw, ch, color| {
                    for dy in 0..ch as i32 {
                        for dx in 0..cw as i32 {
                            let (px, py) = (ox + x + dx, oy + y + dy);
                            if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                                let i = py as usize * w + px as usize;
                                gray[i] = gray[i].max(color.a());
                            }
                        }
                    }
                },
            );
        }
        for &(ry, from, to) in &l.rules {
            let ry = ry as usize;
            if ry >= h {
                continue;
            }
            for x in (from as usize)..(to as usize).min(w) {
                if (x / 4) % 2 == 0 {
                    gray[ry * w + x] = 255;
                }
            }
        }
        for &(rx, ry, rw, rh) in &l.rects {
            let (x0, y0) = (rx.round().max(0.0) as usize, ry.round().max(0.0) as usize);
            let (x1, y1) = (
                ((rx + rw).round().max(0.0) as usize).min(w),
                ((ry + rh).round().max(0.0) as usize).min(h),
            );
            for yy in y0..y1 {
                for xx in x0..x1 {
                    gray[yy * w + xx] = 255;
                }
            }
        }
        // Pictures: nearest pixel, then an ordered dither so grey shows
        // as dots on a printer that only prints black.
        const BAYER: [[u16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        for &(ix, iy, iw, ih, index) in &l.images {
            let Some(crate::Block::Image {
                gray: px,
                w: sw,
                h: sh,
                ..
            }) = doc.blocks.get(index)
            else {
                continue;
            };
            let (sw, sh) = (*sw as usize, *sh as usize);
            if sw == 0 || sh == 0 || px.len() < sw * sh {
                continue;
            }
            let (x0, y0) = (ix.round().max(0.0) as usize, iy.round().max(0.0) as usize);
            let (dw, dh) = (iw.round().max(1.0) as usize, ih.round().max(1.0) as usize);
            for dy in 0..dh {
                let yy = y0 + dy;
                if yy >= h {
                    break;
                }
                let sy = (dy * sh / dh).min(sh - 1);
                for dx in 0..dw {
                    let xx = x0 + dx;
                    if xx >= w {
                        break;
                    }
                    let sx = (dx * sw / dw).min(sw - 1);
                    let ink = 255 - u16::from(px[sy * sw + sx]);
                    let level = (BAYER[yy % 4][xx % 4] * 16) + 8;
                    if ink > level {
                        gray[yy * w + xx] = 255;
                    }
                }
            }
        }
        (0..h)
            .map(|y| (0..w).map(|x| gray[y * w + x] >= 110).collect())
            .collect()
    })
}

/// A roll printout of `doc`: raster bands, feed, cut; `drawer` opens the
/// cash drawer (pin 2) after.
pub fn escpos(doc: &Doc, drawer: bool) -> Result<EscPos, DocError> {
    escpos_with(doc, drawer, true)
}

/// [`escpos`], cutting the paper or not (printers without a cutter, or
/// receipts torn by hand).
pub fn escpos_with(doc: &Doc, drawer: bool, cut: bool) -> Result<EscPos, DocError> {
    if !doc.paper.is_roll() {
        return Err(DocError("ESC/POS prints on rolls".into()));
    }
    let w = doc.paper.dots() as usize;
    let rows = raster(doc, doc.paper.dots(), 8.0, DOTS_PER_PT);
    let mut out = vec![0x1B, 0x40]; // ESC @: initialize
    let bytes_per_row = w.div_ceil(8);
    // Bands of 256 rows at most fit common printer buffers.
    for band in rows.chunks(256) {
        let n = band.len();
        out.extend([0x1D, 0x76, 0x30, 0x00]); // GS v 0, normal density
        out.extend([(bytes_per_row & 0xFF) as u8, (bytes_per_row >> 8) as u8]);
        out.extend([(n & 0xFF) as u8, (n >> 8) as u8]);
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
    out.extend([0x1B, 0x64, 0x04]); // ESC d 4: feed 4 lines
    if cut {
        out.extend([0x1D, 0x56, 0x42, 0x00]); // GS V 66 0: partial cut
    }
    if drawer {
        out.extend(OPEN_DRAWER);
    }
    Ok(EscPos {
        bytes: out,
        width: w,
        rows,
    })
}

/// Reads our own ESC/POS back to the picture it prints (checks the bytes
/// are well formed).
pub fn decode(data: &[u8]) -> Result<(usize, Vec<Vec<bool>>), DocError> {
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
                let body = data
                    .get(i + 8..i + 8 + bpr * n)
                    .ok_or_else(|| DocError("truncated raster".into()))?;
                for r in body.chunks(bpr) {
                    rows.push(
                        (0..width)
                            .map(|x| r[x / 8] & (0x80 >> (x % 8)) != 0)
                            .collect(),
                    );
                }
                i += 8 + bpr * n;
            }
            [0x1B, 0x64, _, ..] => i += 3,
            [0x1D, 0x56, 0x42, _, ..] => i += 4,
            [0x1B, 0x70, _, _, _, ..] => i += 5,
            _ => return Err(DocError(format!("unknown command at byte {i}"))),
        }
    }
    Ok((width, rows))
}

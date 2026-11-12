//! Label printers (M12.7): the label drawn in dots at 203 dpi, its height
//! fixed by the paper, sent as a TSPL bitmap (Xprinter, TSC, Gprinter and
//! most label printers sold here). Arabic prints as on a receipt.

use crate::escpos::{DOTS_PER_PT, raster};
use crate::{Doc, DocError, Paper};

/// The label's dots, exactly the label's size (cut or padded).
pub fn label_rows(doc: &Doc) -> Result<Vec<Vec<bool>>, DocError> {
    let Paper::Label { h_mm, .. } = doc.paper else {
        return Err(DocError("not a label".into()));
    };
    let w = doc.paper.dots() as usize;
    let h = usize::from(h_mm) * 8;
    let mut rows = raster(doc, doc.paper.dots(), 6.0, DOTS_PER_PT);
    rows.truncate(h);
    rows.resize(h, vec![false; w]);
    Ok(rows)
}

/// TSPL for `copies` of the label, with a `gap_mm` between labels.
pub fn tspl(doc: &Doc, copies: u32, gap_mm: u16) -> Result<Vec<u8>, DocError> {
    let Paper::Label { w_mm, h_mm } = doc.paper else {
        return Err(DocError("not a label".into()));
    };
    let rows = label_rows(doc)?;
    let w = doc.paper.dots() as usize;
    let bytes_per_row = w.div_ceil(8);
    let mut out = format!(
        "SIZE {w_mm} mm,{h_mm} mm\r\nGAP {gap_mm} mm,0 mm\r\nDIRECTION 1\r\nCLS\r\nBITMAP 0,0,{bytes_per_row},{},0,",
        rows.len()
    )
    .into_bytes();
    for row in &rows {
        for byte in 0..bytes_per_row {
            // TSPL: a set bit leaves the dot white.
            let mut v = 0xFFu8;
            for bit in 0..8 {
                let x = byte * 8 + bit;
                if x < w && row[x] {
                    v &= !(0x80 >> bit);
                }
            }
            out.push(v);
        }
    }
    out.extend(format!("\r\nPRINT 1,{}\r\n", copies.max(1)).into_bytes());
    Ok(out)
}

/// Reads our TSPL back: (copies, label rows) — tests.
pub fn decode_tspl(data: &[u8]) -> Result<(u32, Vec<Vec<bool>>), DocError> {
    let bad = || DocError("malformed TSPL".into());
    let at = data
        .windows(7)
        .position(|w| w == b"BITMAP ")
        .ok_or_else(bad)?;
    let head_end = at
        + data[at..]
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b',')
            .nth(4)
            .map(|(i, _)| i)
            .ok_or_else(bad)?;
    let head = std::str::from_utf8(&data[at + 7..head_end]).map_err(|_| bad())?;
    let nums: Vec<usize> = head
        .split(',')
        .map(|n| n.trim().parse().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    let [_, _, bpr, h, _] = nums[..] else {
        return Err(bad());
    };
    let body = data
        .get(head_end + 1..head_end + 1 + bpr * h)
        .ok_or_else(bad)?;
    let rows = body
        .chunks(bpr)
        .map(|r| {
            (0..bpr * 8)
                .map(|x| r[x / 8] & (0x80 >> (x % 8)) == 0)
                .collect()
        })
        .collect();
    let tail = std::str::from_utf8(&data[head_end + 1 + bpr * h..]).map_err(|_| bad())?;
    let copies = tail
        .trim()
        .strip_prefix("PRINT 1,")
        .and_then(|c| c.trim().parse().ok())
        .ok_or_else(bad)?;
    Ok((copies, rows))
}

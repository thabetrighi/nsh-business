//! Pictures made by the app: a barcode drawn from its bars, and stored
//! pictures shown from their bytes.

use nsh_core::element::{ImageSrc, View};
use nsh_core::prelude::*;

/// An EAN-13 drawn as an SVG (`None` for codes that are not EAN-13):
/// black bars on white, with quiet zones, `width` × `height` points.
pub fn barcode(code: &str, width: f32, height: f32) -> Option<View> {
    let modules = nb_domain::barcode::ean13_modules(code)?;
    let quiet = 9;
    let total = modules.len() + 2 * quiet;
    let mut rects = String::new();
    for (i, on) in modules.iter().enumerate() {
        if *on {
            // Guards run longer, as on a printed EAN-13.
            let guard = i < 3 || (45..50).contains(&i) || i >= 92;
            let h = if guard { 100 } else { 88 };
            rects.push_str(&format!(
                r#"<rect x="{}" y="0" width="1" height="{h}"/>"#,
                i + quiet
            ));
        }
    }
    let markup = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total} 100" preserveAspectRatio="none"><rect width="{total}" height="100" fill="#ffffff"/><g fill="#0a0d12">{rects}</g></svg>"##
    );
    Some(
        view().child(
            svg(ImageSrc::svg_markup(&markup))
                .w(px(width))
                .h(px(height)),
        ),
    )
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (for `data:` URLs).
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A stored picture as an image source.
pub fn data_url(mime: &str, bytes: &[u8]) -> ImageSrc {
    format!("data:{mime};base64,{}", base64(bytes)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_like_everyone() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}

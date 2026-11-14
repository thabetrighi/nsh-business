//! One-time codes from an authenticator app (RFC 6238: TOTP, SHA-1, six
//! digits, 30-second steps), the second factor of the platform's
//! administrators. A code is taken once: the step it belongs to is kept,
//! and a code of that step or an earlier one is refused (replay).

use hmac::{Hmac, Mac};
use sha1::Sha1;

/// Seconds per step.
pub const STEP_S: i64 = 30;
/// Digits per code.
const DIGITS: u32 = 6;

/// A new secret: 20 random bytes, in base32 as the apps take it.
pub fn new_secret() -> Result<String, String> {
    let mut b = [0u8; 20];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(data_encoding::BASE32_NOPAD.encode(&b))
}

/// The code of `step` for `secret` (base32).
pub fn code_at(secret: &str, step: i64) -> Option<String> {
    let key = data_encoding::BASE32_NOPAD
        .decode(secret.trim().to_ascii_uppercase().as_bytes())
        .ok()?;
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).ok()?;
    mac.update(&u64::try_from(step).ok()?.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest.get(19)? & 0x0f);
    let bytes: [u8; 4] = digest.get(offset..offset + 4)?.try_into().ok()?;
    let value = u32::from_be_bytes(bytes) & 0x7fff_ffff;
    Some(format!(
        "{:0width$}",
        value % 10u32.pow(DIGITS),
        width = DIGITS as usize
    ))
}

/// The step `now_ms` falls in.
pub fn step_of(now_ms: i64) -> i64 {
    now_ms.div_euclid(1000).div_euclid(STEP_S)
}

/// The step `code` belongs to, if it is the current one or a neighbour
/// (a phone's clock a little off), and newer than `last_step`.
pub fn check(secret: &str, code: &str, now_ms: i64, last_step: i64) -> Option<i64> {
    let code: String = code.chars().filter(char::is_ascii_digit).collect();
    if code.len() != DIGITS as usize {
        return None;
    }
    let now = step_of(now_ms);
    let mut found = None;
    // Every candidate is computed (no early return on a match).
    for step in [now - 1, now, now + 1] {
        if let Some(c) = code_at(secret, step)
            && same(&c, &code)
            && step > last_step
        {
            found = Some(step);
        }
    }
    found
}

/// Equal, in a time that does not depend on where they differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// What an authenticator app scans: `otpauth://totp/…`.
pub fn uri(issuer: &str, login: &str, secret: &str) -> String {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    };
    format!(
        "otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period=30",
        enc(issuer),
        enc(login),
        enc(issuer)
    )
}

/// The same, as a QR code drawn with characters (for a terminal).
pub fn qr_text(text: &str) -> String {
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .build(),
        Err(_) => String::new(),
    }
}

/// The same, as an SVG picture (for the platform's page).
pub fn qr_svg(text: &str) -> String {
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color<'_>>()
            .min_dimensions(200, 200)
            .build(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238, appendix B (SHA-1, key "12345678901234567890").
    #[test]
    fn the_rfc_vectors() {
        let secret = data_encoding::BASE32_NOPAD.encode(b"12345678901234567890");
        for (t, want) in [
            (59_i64, "94287082"),
            (1_111_111_109, "07081804"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
        ] {
            // The RFC shows 8 digits; ours are the last 6.
            assert_eq!(code_at(&secret, t / STEP_S).as_deref(), Some(&want[2..]));
        }
    }

    #[test]
    fn a_code_works_once_and_only_near_its_time() {
        let secret = new_secret().unwrap_or_default();
        let now = 1_794_304_800_000;
        let step = step_of(now);
        let code = code_at(&secret, step).unwrap_or_default();
        assert_eq!(check(&secret, &code, now, 0), Some(step));
        assert_eq!(check(&secret, &code, now, step), None, "replayed");
        assert_eq!(
            check(&secret, &code, now + 30_000, 0),
            Some(step),
            "a step late"
        );
        assert_eq!(check(&secret, &code, now + 120_000, 0), None, "too late");
        assert_eq!(check(&secret, "12345", now, 0), None);
        assert!(
            uri("NSH Business", "admin", &secret)
                .starts_with("otpauth://totp/NSH%20Business:admin?")
        );
        assert!(qr_svg("x").contains("<svg"));
    }
}

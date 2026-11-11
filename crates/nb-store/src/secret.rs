//! Password and PIN hashing (M4.7): argon2id with a random salt, stored as
//! a PHC string. Verification takes the same time whatever the input.

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

use crate::{Error, Result};

/// Hashes `secret` for storage.
pub fn hash(secret: &str) -> Result<String> {
    let salt = SaltString::encode_b64(&nb_core::random_bytes())
        .map_err(|e| Error::Corrupt(format!("salt: {e}")))?;
    Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| Error::Corrupt(format!("hash: {e}")))
}

/// Whether `secret` matches a stored hash. A malformed hash matches nothing.
pub fn verify(secret: &str, stored: &str) -> bool {
    PasswordHash::new(stored).is_ok_and(|h| {
        Argon2::default()
            .verify_password(secret.as_bytes(), &h)
            .is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_verify_and_are_salted() -> Result<()> {
        let a = hash("2580")?;
        let b = hash("2580")?;
        assert_ne!(a, b, "salted");
        assert!(a.starts_with("$argon2id$"));
        assert!(verify("2580", &a) && verify("2580", &b));
        assert!(!verify("2581", &a));
        assert!(!verify("2580", "not a hash"));
        Ok(())
    }
}

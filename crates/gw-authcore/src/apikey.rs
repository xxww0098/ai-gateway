//! Panel-issued API keys (`agw-…`).
//!
//! The hash is the at-rest lookup key in `api_keys.key_hash`, so it must stay
//! byte-identical to what existing rows hold.

use crate::error::AuthError;
use rand::RngCore;
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests;

/// Prefix length: `"agw-"` (4) + 8 hex chars.
pub const KEY_PREFIX_LEN: usize = 12;

/// The literal `"agw-"` prefix every minted key starts with.
pub const API_KEY_PREFIX: &str = "agw-";

/// Number of random bytes behind the prefix (64 hex chars).
const API_KEY_ENTROPY_BYTES: usize = 32;

/// The displayable prefix stored in `api_keys.key_prefix`.
///
/// Shorter inputs are returned whole. We stop at the nearest character boundary
/// instead of slicing raw bytes, which is identical for the ASCII keys we mint
/// and merely avoids a panic on hand-typed UTF-8 input.
#[must_use]
pub fn api_key_prefix(plaintext: &str) -> &str {
    let mut end = KEY_PREFIX_LEN.min(plaintext.len());
    while !plaintext.is_char_boundary(end) {
        end -= 1;
    }
    &plaintext[..end]
}

/// The canonical at-rest representation of an API key: lowercase hex SHA-256.
///
/// Every `api_keys.key_hash` row in the existing database was produced by this
/// function — it can never change.
#[must_use]
pub fn hash_api_key(plaintext: &str) -> String {
    let digest = Sha256::digest(plaintext.as_bytes());
    hex::encode(digest)
}

/// Mints a new plaintext API key: `"agw-"` + 64 hex chars (32 OS-random bytes).
///
/// The caller shows the plaintext once and persists only [`hash_api_key`] and
/// [`api_key_prefix`] of it.
///
/// # Errors
/// [`AuthError::Random`] when the OS entropy source fails.
pub fn new_api_key() -> Result<String, AuthError> {
    let mut bytes = [0u8; API_KEY_ENTROPY_BYTES];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|err| AuthError::Random(err.to_string()))?;
    Ok(format!("{API_KEY_PREFIX}{}", hex::encode(bytes)))
}

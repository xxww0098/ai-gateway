//! Panel password hashing.
//!
//! Every `users.password_hash` in the existing database is a `$2a$10$…` string
//! written by bcrypt at default cost; [`verify_password`] must keep accepting
//! them.

use crate::error::AuthError;

#[cfg(test)]
mod tests;

/// Cost 10, matching the hashes already in `users.password_hash`.
///
/// Not the `bcrypt` crate's `DEFAULT_COST` (12) — a mismatch would not break
/// verification (the cost is encoded in every hash) but it would silently change
/// login latency by ~4x for every account created after the rewrite.
pub const BCRYPT_COST: u32 = 10;

/// bcrypt refuses to hash more than this many bytes.
const MAX_PASSWORD_BYTES: usize = 72;

/// Hashes a plaintext password for storage in `users.password_hash`.
///
/// # Errors
/// [`AuthError::PasswordTooLong`] for inputs over 72 bytes — mirroring
/// `bcrypt.ErrPasswordTooLong`, so an over-long password is rejected at
/// registration instead of being silently truncated (which would let a shorter
/// prefix log the account in later).
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(AuthError::PasswordTooLong);
    }
    bcrypt::hash(password, BCRYPT_COST).map_err(AuthError::from)
}

/// Checks a plaintext password against a stored hash.
///
/// Accepts every bcrypt variant (`$2a$`, `$2b$`, `$2y$`) and compares only the
/// first 72 bytes of the candidate.
///
/// # Errors
/// [`AuthError::Bcrypt`] when `hash` is not a well-formed bcrypt string. A
/// well-formed hash with the wrong password is `Ok(false)`, not an error.
pub fn verify_password(password: &str, hash: &str) -> Result<bool, AuthError> {
    bcrypt::verify(password, hash).map_err(AuthError::from)
}

//! PKCE (RFC 7636) and random tokens. 32-byte verifier matches official CLIs.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore as _;
use sha2::{Digest, Sha256};

use crate::Error;

#[cfg(test)]
mod tests;

/// Fresh PKCE pair: 32 random bytes, S256 challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// Mint a PKCE pair.
///
/// # Errors
/// When the OS entropy source fails.
pub fn create_pkce() -> Result<Pkce, Error> {
    let mut raw = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut raw)
        .map_err(|_| Error::Entropy)?;
    let verifier = URL_SAFE_NO_PAD.encode(raw);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok(Pkce {
        verifier,
        challenge,
    })
}

/// URL-safe random token (default 32 bytes) for OAuth `state`.
///
/// # Errors
/// When the OS entropy source fails.
pub fn random_token(bytes: usize) -> Result<String, Error> {
    let n = bytes.max(1);
    let mut raw = vec![0u8; n];
    rand::rngs::OsRng
        .try_fill_bytes(&mut raw)
        .map_err(|_| Error::Entropy)?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

/// Lowercase hex random bytes (Grok `nonce`).
///
/// # Errors
/// When the OS entropy source fails.
pub fn random_hex(bytes: usize) -> Result<String, Error> {
    let n = bytes.max(1);
    let mut raw = vec![0u8; n];
    rand::rngs::OsRng
        .try_fill_bytes(&mut raw)
        .map_err(|_| Error::Entropy)?;
    Ok(hex::encode(raw))
}

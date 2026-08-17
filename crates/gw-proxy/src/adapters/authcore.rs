//! [`AuthCrypto`] over `gw_authcore`.
//!
//! `hash_api_key` for the `agw-` credential path and `validate_jwt` for the
//! panel token path. The JWT secret is held here rather than passed per call,
//! which is what lets [`crate::access::AccessProvider`] stay free of
//! configuration.

use gw_authcore::{Claims, hash_api_key, validate_jwt};

use crate::ports::AuthCrypto;

/// Signing/verification material for the `/v1/*` credential paths.
#[derive(Debug, Clone)]
pub struct AuthcoreCrypto {
    jwt_secret: String,
}

impl AuthcoreCrypto {
    /// `secret` is `auth.jwt.secret`. An empty secret makes every JWT fail to
    /// verify — `gw_authcore::validate_jwt` rejects it outright — so the JWT
    /// path fails closed rather than accepting unsigned tokens. The composition
    /// root already refuses to boot on a weak-but-present secret.
    pub fn new(jwt_secret: impl Into<String>) -> Self {
        Self {
            jwt_secret: jwt_secret.into(),
        }
    }
}

impl AuthCrypto for AuthcoreCrypto {
    fn hash_api_key(&self, plaintext: &str) -> String {
        hash_api_key(plaintext)
    }

    fn sha256_hex(&self, input: &str) -> String {
        // Same primitive, different purpose: `hash_api_key` IS "lowercase hex
        // SHA-256 of this string", which is also what idempotency key scoping
        // needs. Routing both through one function keeps this crate free of a
        // hashing dependency of its own.
        hash_api_key(input)
    }

    fn verify_jwt(&self, token: &str) -> Option<Claims> {
        validate_jwt(token, &self.jwt_secret).ok()
    }
}

#[cfg(test)]
mod tests;

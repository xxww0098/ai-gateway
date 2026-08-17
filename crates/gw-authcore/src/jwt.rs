//! HS256 panel session tokens.
//!
//! The wire format is frozen: existing tokens must validate here and vice versa,
//! so the claim names (`user_id`, `email`, `tv`, `exp`, `iat`, `nbf`, `iss`, `sub`)
//! and the `tv`-is-omitted-when-zero rule are load-bearing, not cosmetic.

use crate::{UserId, error::AuthError};
use chrono::{DateTime, TimeDelta, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// Fallback expiry window when `expiry_hours <= 0`.
pub const DEFAULT_EXPIRY_HOURS: i64 = 24;

/// Issuer stamped on every token we mint.
pub const ISSUER: &str = "ai-gateway";

/// Claims carried by a panel session token.
///
/// `token_version` is the user's session epoch at issuance. The panel rejects a
/// token whose version is below the user's persisted version, which is how "log
/// out everywhere" works — see [`crate::token_version`]. It serialises as `tv`
/// and is *omitted when zero*, so a token minted before the feature existed still
/// decodes (to 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// `users.id` of the authenticated user.
    pub user_id: UserId,
    /// `users.email` at issuance (informational; authorization reads the DB).
    pub email: String,
    /// Session epoch at issuance. 64-bit to match the `bigint`
    /// `user_token_versions.version` column it is compared against.
    #[serde(rename = "tv", default, skip_serializing_if = "is_zero")]
    pub token_version: i64,
    /// Expiry, seconds since the Unix epoch.
    pub exp: i64,
    /// Issued-at, seconds since the Unix epoch.
    pub iat: i64,
    /// Not-before, seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nbf: Option<i64>,
    /// Always [`ISSUER`] on tokens we mint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    /// The user id, stringified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
}

fn is_zero(version: &i64) -> bool {
    *version == 0
}

impl Claims {
    /// Whether this token predates a session revocation and must be rejected.
    #[must_use]
    pub fn is_revoked(&self, current_version: i64) -> bool {
        crate::token_version::token_revoked(self.token_version, current_version)
    }
}

/// Issues an HS256 token with token version 0.
///
/// Retained for callers that do not participate in session revocation;
/// production login uses [`generate_jwt_with_version`].
///
/// # Errors
/// [`AuthError::MissingJwtSecret`] when `secret` is empty.
pub fn generate_jwt(
    user_id: UserId,
    email: &str,
    secret: &str,
    expiry_hours: i64,
) -> Result<String, AuthError> {
    generate_jwt_with_version(user_id, email, secret, expiry_hours, 0)
}

/// Issues an HS256 token embedding `token_version`.
///
/// `expiry_hours <= 0` falls back to [`DEFAULT_EXPIRY_HOURS`].
///
/// # Errors
/// [`AuthError::MissingJwtSecret`] when `secret` is empty,
/// [`AuthError::InvalidExpiry`] when `expiry_hours` overflows a timestamp.
pub fn generate_jwt_with_version(
    user_id: UserId,
    email: &str,
    secret: &str,
    expiry_hours: i64,
    token_version: i64,
) -> Result<String, AuthError> {
    sign_at(
        Utc::now(),
        user_id,
        email,
        secret,
        expiry_hours,
        token_version,
    )
}

/// The clock-injected core of [`generate_jwt_with_version`], so expiry
/// behaviour is testable without sleeping.
fn sign_at(
    now: DateTime<Utc>,
    user_id: UserId,
    email: &str,
    secret: &str,
    expiry_hours: i64,
    token_version: i64,
) -> Result<String, AuthError> {
    if secret.is_empty() {
        return Err(AuthError::MissingJwtSecret);
    }

    let hours = if expiry_hours <= 0 {
        DEFAULT_EXPIRY_HOURS
    } else {
        expiry_hours
    };
    let expires_at = TimeDelta::try_hours(hours)
        .and_then(|d| now.checked_add_signed(d))
        .ok_or(AuthError::InvalidExpiry(hours))?;

    let claims = Claims {
        user_id,
        email: email.to_owned(),
        token_version,
        exp: expires_at.timestamp(),
        iat: now.timestamp(),
        nbf: Some(now.timestamp()),
        iss: Some(ISSUER.to_owned()),
        sub: Some(user_id.to_string()),
    };

    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(AuthError::from)
}

/// Parses and validates an HS256 token, returning its claims.
///
/// Algorithm confusion (`alg: none`, RS256 with the HMAC secret as a public key)
/// is rejected because the accepted-algorithm list is pinned to HS256.
///
/// `exp` and `nbf` are checked with **zero leeway** (the `jsonwebtoken` default
/// of 60s would accept expired tokens).
///
/// # Errors
/// [`AuthError::MissingJwtSecret`] when `secret` is empty; [`AuthError::InvalidJwt`]
/// for a bad signature, an expired/not-yet-valid token, or malformed claims.
pub fn validate_jwt(token: &str, secret: &str) -> Result<Claims, AuthError> {
    if secret.is_empty() {
        return Err(AuthError::MissingJwtSecret);
    }

    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.validate_nbf = true;
    validation.validate_aud = false;

    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    Ok(data.claims)
}

//! Session revocation.
//!
//! A JWT carries the epoch in force when it was minted ([`crate::jwt::Claims`]'s
//! `tv`). Bumping the user's stored epoch invalidates every token issued before
//! the bump — that is what "log out everywhere" and forced revocation do —
//! while tokens minted afterwards stay valid. **A missing row means version 0**,
//! the epoch every fresh token carries, so users who never revoked are
//! unaffected.

use crate::{UserId, error::AuthError};
use chrono::Utc;
use sqlx::PgPool;

#[cfg(test)]
mod tests;

/// A `Scan`, not a `First`, so a missing row reads as 0 rather than an error.
const SELECT_VERSION: &str =
    "SELECT version::bigint FROM user_token_versions WHERE user_id = $1 LIMIT 1";

/// Collapses first-or-create + increment into one atomic upsert. A new row lands
/// at 1.
const BUMP_VERSION: &str = "INSERT INTO user_token_versions (user_id, version, updated_at) \
     VALUES ($1, 1, $2) \
     ON CONFLICT (user_id) DO UPDATE SET version = user_token_versions.version + 1, \
     updated_at = $2 \
     RETURNING version::bigint";

/// Whether a token must be rejected because its epoch predates the user's.
#[must_use]
pub fn token_revoked(token_version: i64, current_version: i64) -> bool {
    token_version < current_version
}

/// Reads and bumps the per-user session epoch in `user_token_versions`.
#[derive(Debug, Clone)]
pub struct TokenVersionStore {
    pool: PgPool,
}

impl TokenVersionStore {
    /// Binds the store to a connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The user's current session epoch, `0` when they have never revoked.
    ///
    /// A database error is propagated so the caller can fail closed — treating a
    /// failed lookup as "version 0" would silently un-revoke every session.
    ///
    /// # Errors
    /// [`AuthError::Database`] when the query fails.
    pub async fn current(&self, user_id: UserId) -> Result<i64, AuthError> {
        if user_id == 0 {
            return Ok(0);
        }
        let version: Option<i64> = sqlx::query_scalar(SELECT_VERSION)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| AuthError::db("reading token version", err))?;
        Ok(version.unwrap_or(0))
    }

    /// Increments the user's session epoch, invalidating every outstanding
    /// token, and returns the new value.
    ///
    /// A single `ON CONFLICT DO UPDATE` is atomic on its own, so concurrent
    /// logouts cannot lose a bump.
    ///
    /// # Errors
    /// [`AuthError::Database`] when the upsert fails.
    pub async fn bump(&self, user_id: UserId) -> Result<i64, AuthError> {
        if user_id == 0 {
            return Ok(0); // a zero id is a no-op
        }
        let version: i64 = sqlx::query_scalar(BUMP_VERSION)
            .bind(user_id)
            .bind(Utc::now())
            .fetch_one(&self.pool)
            .await
            .map_err(|err| AuthError::db("bumping token version", err))?;
        Ok(version)
    }
}

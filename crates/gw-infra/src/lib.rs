//! Infrastructure: Postgres pool, Redis, in-process caches, rate limiter,
//! circuit breaker.
//!
//! OWNER: worker `infra`.
//!
//! | module | responsibility |
//! | --- | --- |
//! | [`db`] | `init_db` + pool tuning + SQL log level |
//! | [`redis`] | `init_redis` (fail-soft ping) |
//! | [`cache`] | `ApiKeyCache` / `UserStatusCache` + sweepers |
//! | [`rate_limiter`] | Redis sliding-window limiter |
//! | [`circuit_breaker`] | per-provider breaker |
//!
//! Migrations and seed data are **not** here — they belong to `gw-model` /
//! the SQL migrations (see `CONTRACT.md` §3.5).
//!
//! Settings structs ([`db::DbSettings`], [`rate_limiter::RateLimitSettings`],
//! [`circuit_breaker::CircuitBreakerSettings`]) hold the matching
//! `gw_config` shape field-for-field but are owned here, because this
//! is where their "0 means use the gateway default" convention is resolved —
//! `gw_config` deliberately keeps the raw YAML values. Each has a
//! `From<&gw_config::…>`, so the composition root reads:
//!
//! ```ignore
//! let db = gw_infra::init_db(
//!     &DbSettings::from(&cfg.database),
//!     SqlLogLevel::parse(&cfg.server.log_level),
//! ).await?;
//! let redis = gw_infra::init_redis(&cfg.redis.addr, &cfg.redis.password, cfg.redis.db).await;
//! let limiter = RateLimiter::new(redis.clone(), (&cfg.rate_limit).into());
//! let breaker = CircuitBreaker::new(redis.clone(), (&cfg.circuit_breaker).into());
//! ```

// Rule 5.3 ratchet: this crate carries no `todo!()`/`unimplemented!()`, so it
// denies them locally instead of waiting for the last crate in the workspace to
// catch up. Scoped to two named lints — never `#![deny(warnings)]`.
#![deny(clippy::todo, clippy::unimplemented)]

pub mod cache;
pub mod circuit_breaker;
pub mod db;
pub mod rate_limiter;
pub mod redis;

#[cfg(test)]
mod testsupport;

pub use cache::{ApiKeyCache, CachedKey, SweepHandle, UserStatus, UserStatusCache};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerSettings, CircuitState};
pub use db::{DbSettings, SqlLogLevel, init_db};
pub use rate_limiter::{
    DeniedDimension, RateLimitDecision, RateLimitOverride, RateLimitSettings, RateLimiter,
};
pub use redis::init_redis;

/// The Postgres handle passed around the gateway.
pub type Db = sqlx::PgPool;

/// The Redis handle passed around the gateway. A `None` stands in for an
/// absent client (every Redis-backed component here degrades gracefully
/// without it).
pub type Redis = ::redis::aio::ConnectionManager;

/// Everything that can go wrong while talking to the gateway's infrastructure.
///
/// Note what is *absent*: the rate limiter and the circuit breaker's `allow`
/// deliberately never surface a Redis error — they fail **open**, because a
/// broken limiter must not take the proxy down.
#[derive(Debug, thiserror::Error)]
pub enum InfraError {
    /// Postgres connect/query failure.
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    /// Redis command failure on a path that reports errors (as opposed to the
    /// fail-open paths, which swallow and log).
    #[error("redis: {0}")]
    Redis(#[from] ::redis::RedisError),
    /// A setting could not be turned into a usable connection parameter.
    #[error("invalid {field}: {reason}")]
    Invalid {
        /// The offending settings field.
        field: &'static str,
        /// Why it could not be used.
        reason: String,
    },
}

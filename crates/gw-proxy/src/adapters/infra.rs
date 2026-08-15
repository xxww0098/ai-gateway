//! [`RateLimiter`], [`CircuitBreaker`] and [`IdempotencyStore`] over `gw-infra`.
//!
//! The three collaborators injected into `HoldMiddleware` (`with_rate_limiter`
//! / `with_circuit_breaker` / `with_idempotency`). The serialization and replay
//! policy for idempotency stays in [`crate::idempotency`]; this file only moves
//! bytes.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use gw_infra::{CircuitBreaker as InfraBreaker, RateLimiter as InfraLimiter, Redis};
use redis::AsyncCommands as _;

use crate::ports::{CircuitBreaker, Id, IdempotencyStore, RateLimiter};

/// The distributed sliding-window limiter.
#[derive(Debug, Clone)]
pub struct SharedRateLimiter(Arc<InfraLimiter>);

impl SharedRateLimiter {
    pub fn new(limiter: Arc<InfraLimiter>) -> Self {
        Self(limiter)
    }
}

#[async_trait]
impl RateLimiter for SharedRateLimiter {
    async fn allow(
        &self,
        identity: &str,
        tokens: i64,
        model: &str,
        group_id: Option<Id>,
    ) -> anyhow::Result<(bool, Option<String>)> {
        // `infra::RateLimiter::allow` cannot fail: it fails open internally and
        // says so in the decision, which is the posture the middleware wants —
        // a Redis outage must not stop traffic. The port's `Result` exists for
        // implementations that cannot make that promise.
        let decision = self.0.allow(identity, tokens, model, group_id).await;
        Ok(match decision {
            gw_infra::RateLimitDecision::Allowed { release_id } => (true, release_id),
            gw_infra::RateLimitDecision::Denied { .. } => (false, None),
        })
    }

    async fn release_concurrency(&self, identity: &str, release_id: &str) -> anyhow::Result<()> {
        Ok(self.0.release_conc(identity, release_id).await?)
    }
}

/// The per-upstream failure-rate breaker.
#[derive(Debug, Clone)]
pub struct SharedCircuitBreaker(Arc<InfraBreaker>);

impl SharedCircuitBreaker {
    pub fn new(breaker: Arc<InfraBreaker>) -> Self {
        Self(breaker)
    }
}

#[async_trait]
impl CircuitBreaker for SharedCircuitBreaker {
    async fn allow(&self, provider: &str) -> anyhow::Result<bool> {
        Ok(self.0.allow(provider).await)
    }

    async fn record(&self, provider: &str, success: bool) {
        // Best-effort by design: a breaker that cannot record is a degraded
        // signal, not a reason to fail a request that already succeeded.
        let result = if success {
            self.0.record_success(provider).await
        } else {
            self.0.record_failure(provider).await
        };
        if let Err(err) = result {
            tracing::debug!(%err, provider, success, "circuit breaker record failed");
        }
    }
}

/// Redis-backed storage for idempotency entries.
#[derive(Clone)]
pub struct RedisIdempotencyStore {
    redis: Redis,
}

impl std::fmt::Debug for RedisIdempotencyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisIdempotencyStore")
            .finish_non_exhaustive()
    }
}

impl RedisIdempotencyStore {
    pub fn new(redis: Redis) -> Self {
        Self { redis }
    }
}

#[async_trait]
impl IdempotencyStore for RedisIdempotencyStore {
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let mut conn = self.redis.clone();
        Ok(conn.get(key).await?)
    }

    async fn set(&self, key: &str, value: Vec<u8>, ttl: Duration) -> anyhow::Result<()> {
        let mut conn = self.redis.clone();
        // A zero TTL would mean "never expire" to Redis, which turns a cache
        // into a leak; the manager already defaults it, so treat it as a bug.
        let seconds = ttl.as_secs().max(1);
        conn.set_ex::<_, _, ()>(key, value, seconds).await?;
        Ok(())
    }

    async fn set_nx(&self, key: &str, value: Vec<u8>, ttl: Duration) -> anyhow::Result<bool> {
        let mut conn = self.redis.clone();
        let options = redis::SetOptions::default()
            .conditional_set(redis::ExistenceCheck::NX)
            .with_expiration(redis::SetExpiry::EX(ttl.as_secs().max(1)));
        // A lost race replies with a nil bulk string, which decodes as `None`.
        let claimed: Option<String> = conn.set_options(key, value, options).await?;
        Ok(claimed.is_some())
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        let mut conn = self.redis.clone();
        conn.del::<_, ()>(key).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

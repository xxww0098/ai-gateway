//! Per-upstream circuit breaker.
//!
//! State lives in one Redis hash per provider
//! (`cpa-gateway:circuit:{provider}`, fields `state` / `failures` /
//! `successes` / `total` / `opened_at`), so every gateway instance trips and
//! recovers together.
//!
//! The "window" is a key TTL rather than a sliding window: counters live for
//! `cooldown + window` and then vanish wholesale, which is why a long-quiet
//! provider starts from a clean slate.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use ::redis::AsyncCommands;
use chrono::Utc;

use crate::{InfraError, Redis};

#[cfg(test)]
mod tests;

/// Minimum number of observations before the breaker may trip: the
/// `if total < 5 { return nil }` guard in `RecordFailure`.
pub const MIN_SAMPLES: i64 = 5;
/// Default trip threshold, as a failure *rate*.
pub const DEFAULT_FAILURE_THRESHOLD: f64 = 0.5;
/// Default observation window, in seconds.
pub const DEFAULT_WINDOW_SECONDS: i64 = 30;
/// Default time spent open before a probe is allowed, in seconds.
pub const DEFAULT_COOLDOWN_SECONDS: i64 = 30;

/// Redis hash key holding a provider's breaker state.
pub fn circuit_key(provider: &str) -> String {
    format!("cpa-gateway:circuit:{provider}")
}

/// Breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CircuitState {
    /// Healthy: requests pass through.
    #[default]
    Closed,
    /// Tripped: requests are rejected without touching the upstream.
    Open,
    /// Cooldown elapsed: one probe request is in flight.
    HalfOpen,
}

impl CircuitState {
    /// The value stored in the hash's `state` field.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }

    /// Reads a stored `state` field. Anything unrecognised — including a
    /// missing field — is treated as closed, matching the `default:` arm of
    /// the `Allow` switch: an unreadable breaker must not block traffic.
    pub fn from_wire(raw: &str) -> Self {
        match raw {
            "open" => Self::Open,
            "half_open" => Self::HalfOpen,
            _ => Self::Closed,
        }
    }
}

impl fmt::Display for CircuitState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Breaker configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CircuitBreakerSettings {
    /// Failure rate (0.0–1.0) at which the breaker trips.
    pub failure_threshold: f64,
    /// Observation window, in seconds; also half of the key TTL.
    pub window_seconds: i64,
    /// How long the breaker stays open before allowing a probe, in seconds.
    pub cooldown_seconds: i64,
}

impl Default for CircuitBreakerSettings {
    /// The form with every non-positive field defaulted.
    fn default() -> Self {
        Self {
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            window_seconds: DEFAULT_WINDOW_SECONDS,
            cooldown_seconds: DEFAULT_COOLDOWN_SECONDS,
        }
    }
}

impl CircuitBreakerSettings {
    /// Replaces every non-positive field with its default.
    #[must_use]
    pub fn with_defaults(mut self) -> Self {
        let defaults = Self::default();
        if self.failure_threshold <= 0.0 {
            self.failure_threshold = defaults.failure_threshold;
        }
        if self.window_seconds <= 0 {
            self.window_seconds = defaults.window_seconds;
        }
        if self.cooldown_seconds <= 0 {
            self.cooldown_seconds = defaults.cooldown_seconds;
        }
        self
    }
}

impl From<&gw_config::CircuitBreakerConfig> for CircuitBreakerSettings {
    /// Lifts the `circuit_breaker:` block of `config.yaml` into this crate's
    /// settings.
    fn from(cfg: &gw_config::CircuitBreakerConfig) -> Self {
        Self {
            failure_threshold: cfg.failure_threshold,
            window_seconds: i64::from(cfg.window_seconds),
            cooldown_seconds: i64::from(cfg.cooldown_seconds),
        }
    }
}

/// Redis-backed per-provider breaker.
#[derive(Clone)]
pub struct CircuitBreaker {
    redis: Option<Redis>,
    failure_threshold: f64,
    window: Duration,
    cooldown: Duration,
}

impl fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("redis_enabled", &self.redis.is_some())
            .field("failure_threshold", &self.failure_threshold)
            .field("window", &self.window)
            .field("cooldown", &self.cooldown)
            .finish()
    }
}

impl CircuitBreaker {
    /// Builds a breaker, applying the zero-value defaulting.
    ///
    /// `redis` is `None` for an absent client: the breaker then never trips
    /// and never rejects.
    pub fn new(redis: Option<Redis>, settings: CircuitBreakerSettings) -> Self {
        let settings = settings.with_defaults();
        Self {
            redis,
            failure_threshold: settings.failure_threshold,
            window: Duration::from_secs(settings.window_seconds.unsigned_abs()),
            cooldown: Duration::from_secs(settings.cooldown_seconds.unsigned_abs()),
        }
    }

    /// Whether a request to `provider` may go out.
    ///
    /// * closed — allow;
    /// * open — allow only once the cooldown has elapsed, transitioning to
    ///   half-open on the way through;
    /// * half-open — allow (the probe is already in flight).
    ///
    /// **Fails open**: any Redis trouble yields `true`, because a breaker that
    /// cannot read its own state must not become the outage.
    pub async fn allow(&self, provider: &str) -> bool {
        let Some(conn) = self.redis.as_ref() else {
            return true;
        };
        let key = circuit_key(provider);
        let mut conn = conn.clone();

        let fields: HashMap<String, String> = match conn.hgetall(&key).await {
            Ok(fields) => fields,
            Err(error) => {
                tracing::warn!(
                    provider,
                    %error,
                    "CircuitBreaker.Allow: Redis error, allowing request (fail-open)"
                );
                return true;
            }
        };

        match allow_outcome(&fields, Utc::now().timestamp(), self.cooldown) {
            AllowOutcome::Allow => true,
            AllowOutcome::Reject => false,
            AllowOutcome::ResetThenAllow => {
                self.reset_state(&key).await;
                true
            }
            AllowOutcome::ProbeAfterCooldown => {
                let ttl = self.ttl_seconds();
                let transition: Result<(), _> = ::redis::pipe()
                    .atomic()
                    .hset(&key, "state", CircuitState::HalfOpen.as_str())
                    .ignore()
                    .expire(&key, ttl)
                    .ignore()
                    .query_async(&mut conn)
                    .await;
                if let Err(error) = transition {
                    tracing::warn!(
                        provider,
                        %error,
                        "CircuitBreaker.Allow: failed to transition to half_open"
                    );
                }
                true
            }
        }
    }

    /// Records a successful call: a success in half-open closes the breaker
    /// and clears the counters, otherwise it just counts.
    pub async fn record_success(&self, provider: &str) -> Result<(), InfraError> {
        let Some(conn) = self.redis.as_ref() else {
            return Ok(());
        };
        let key = circuit_key(provider);
        let mut conn = conn.clone();

        let fields: HashMap<String, String> = conn.hgetall(&key).await.map_err(|error| {
            tracing::warn!(provider, %error, "CircuitBreaker.RecordSuccess: Redis error");
            InfraError::Redis(error)
        })?;

        if stored_state(&fields) == CircuitState::HalfOpen {
            self.reset_state(&key).await;
            return Ok(());
        }

        let ttl = self.ttl_seconds();
        ::redis::pipe()
            .atomic()
            .hincr(&key, "successes", 1)
            .ignore()
            .hincr(&key, "total", 1)
            .ignore()
            .expire(&key, ttl)
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .map_err(|error| {
                tracing::warn!(provider, %error, "CircuitBreaker.RecordSuccess: pipeline error");
                InfraError::Redis(error)
            })
    }

    /// Records a failed call and trips the breaker when the failure rate has
    /// crossed the threshold.
    pub async fn record_failure(&self, provider: &str) -> Result<(), InfraError> {
        let Some(conn) = self.redis.as_ref() else {
            return Ok(());
        };
        let key = circuit_key(provider);
        let mut conn = conn.clone();
        let ttl = self.ttl_seconds();

        let (failures, total): (i64, i64) = ::redis::pipe()
            .atomic()
            .hincr(&key, "failures", 1)
            .hincr(&key, "total", 1)
            .expire(&key, ttl)
            .ignore()
            .query_async(&mut conn)
            .await
            .map_err(|error| {
                tracing::warn!(provider, %error, "CircuitBreaker.RecordFailure: pipeline error");
                InfraError::Redis(error)
            })?;

        if !should_trip(failures, total, self.failure_threshold) {
            return Ok(());
        }

        ::redis::pipe()
            .atomic()
            .hset(&key, "state", CircuitState::Open.as_str())
            .ignore()
            .hset(&key, "opened_at", Utc::now().timestamp())
            .ignore()
            .query_async::<()>(&mut conn)
            .await
            .map_err(|error| {
                tracing::warn!(provider, %error, "CircuitBreaker.RecordFailure: failed to open circuit");
                InfraError::Redis(error)
            })?;

        tracing::warn!(
            provider,
            failure_rate = failures as f64 / total as f64,
            total,
            "CircuitBreaker: circuit opened"
        );
        Ok(())
    }

    /// The stored state for `provider`, or [`CircuitState::Closed`] when none
    /// is stored.
    pub async fn state(&self, provider: &str) -> Result<CircuitState, InfraError> {
        let Some(conn) = self.redis.as_ref() else {
            return Ok(CircuitState::Closed);
        };
        let mut conn = conn.clone();

        let stored: Option<String> =
            conn.hget(circuit_key(provider), "state")
                .await
                .map_err(|error| {
                    tracing::warn!(provider, %error, "CircuitBreaker.State: Redis error");
                    InfraError::Redis(error)
                })?;

        Ok(stored.map_or(CircuitState::Closed, |raw| CircuitState::from_wire(&raw)))
    }

    /// Rewrites the hash as a clean closed breaker, errors swallowed and
    /// logged included.
    async fn reset_state(&self, key: &str) {
        let Some(conn) = self.redis.as_ref() else {
            return;
        };
        let mut conn = conn.clone();
        let ttl = self.ttl_seconds();

        let reset: Result<(), _> = ::redis::pipe()
            .atomic()
            .del(key)
            .ignore()
            .hset(key, "state", CircuitState::Closed.as_str())
            .ignore()
            .hset(key, "failures", 0)
            .ignore()
            .hset(key, "successes", 0)
            .ignore()
            .hset(key, "total", 0)
            .ignore()
            .hset(key, "opened_at", "")
            .ignore()
            .expire(key, ttl)
            .ignore()
            .query_async(&mut conn)
            .await;

        if let Err(error) = reset {
            tracing::warn!(key, %error, "CircuitBreaker.resetState: pipeline error");
        }
    }

    /// Key lifetime: `cooldown + window`, so a breaker that nobody exercises
    /// forgets its counters instead of tripping on ancient failures.
    fn ttl_seconds(&self) -> i64 {
        i64::try_from((self.cooldown + self.window).as_secs()).unwrap_or(i64::MAX)
    }
}

/// What [`CircuitBreaker::allow`] should do with a stored hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllowOutcome {
    /// Closed, half-open, or no state at all: let the request through.
    Allow,
    /// Open and still inside the cooldown: reject without touching upstream.
    Reject,
    /// Open and past the cooldown: flip to half-open and let one probe through.
    ProbeAfterCooldown,
    /// Open but with an unusable `opened_at`: the cooldown can never be
    /// evaluated, so clear the state rather than reject forever.
    ResetThenAllow,
}

/// The `state` field of a stored hash, defaulting to closed.
fn stored_state(fields: &HashMap<String, String>) -> CircuitState {
    fields
        .get("state")
        .map_or(CircuitState::Closed, |raw| CircuitState::from_wire(raw))
}

/// The state machine of `CircuitBreaker.Allow`, separated from Redis so the
/// cooldown boundary can be tested without one.
fn allow_outcome(
    fields: &HashMap<String, String>,
    now_unix: i64,
    cooldown: Duration,
) -> AllowOutcome {
    if fields.is_empty() || stored_state(fields) != CircuitState::Open {
        return AllowOutcome::Allow;
    }

    let Some(opened_at) = fields
        .get("opened_at")
        .filter(|raw| !raw.is_empty())
        .and_then(|raw| raw.parse::<i64>().ok())
    else {
        return AllowOutcome::ResetThenAllow;
    };

    let elapsed = now_unix.saturating_sub(opened_at);
    let cooldown_secs = i64::try_from(cooldown.as_secs()).unwrap_or(i64::MAX);
    if elapsed >= cooldown_secs {
        AllowOutcome::ProbeAfterCooldown
    } else {
        AllowOutcome::Reject
    }
}

/// The sample-size guard plus the `failureRate >= threshold` test.
/// Returns whether the observed failure rate should open the breaker.
fn should_trip(failures: i64, total: i64, threshold: f64) -> bool {
    if total < MIN_SAMPLES {
        return false;
    }
    failures as f64 / total as f64 >= threshold
}

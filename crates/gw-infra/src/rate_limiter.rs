//! Distributed multi-dimensional rate limiter.
//!
//! One Lua script checks four dimensions — per-identity request count,
//! per-identity token consumption, per-identity concurrency, and the two global
//! caps — and records the request, all in a single atomic round-trip. That
//! atomicity is what makes the limit hold across gateway instances.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ::redis::{AsyncCommands, Script};
use chrono::Utc;
use gw_model::Id;

use crate::{InfraError, Redis};

#[cfg(test)]
mod tests;

/// Shared prefix of every key this limiter touches.
pub const KEY_PREFIX: &str = "cpa-gateway:ratelimit";
/// Sliding-window width, in milliseconds.
pub const WINDOW_MS: i64 = 60_000;

/// Default per-identity request budget (`RequestsPerMin`).
pub const DEFAULT_REQUESTS_PER_MIN: i64 = 60;
/// Default per-identity token budget (`TokensPerMin`).
pub const DEFAULT_TOKENS_PER_MIN: i64 = 100_000;
/// Default per-identity concurrency budget (`MaxConcurrent`).
pub const DEFAULT_MAX_CONCURRENT: i64 = 10;
/// Default burst allowance (`BurstSize`). See [`RateLimitSettings::burst_size`].
pub const DEFAULT_BURST_SIZE: i64 = 2;
/// Default gateway-wide request cap (`GlobalRequestCap`).
pub const DEFAULT_GLOBAL_REQUEST_CAP: i64 = 10_000;
/// Default gateway-wide token cap (`GlobalTokenCap`).
pub const DEFAULT_GLOBAL_TOKEN_CAP: i64 = 10_000_000;

/// Atomic sliding-window + concurrency check, run as one Lua script.
///
/// Token members are split at their first colon instead of at a fixed byte
/// offset, because request ids here are not 36-byte UUIDs: [`new_request_id`]
/// guarantees the id half is colon-free, so the split point is still
/// unambiguous.
const RATE_LIMIT_LUA: &str = r#"
local req_key   = KEYS[1]
local tok_key   = KEYS[2]
local conc_key  = KEYS[3]
local greq_key  = KEYS[4]
local gtok_key  = KEYS[5]

local now_ms       = tonumber(ARGV[1])
local window_ms    = tonumber(ARGV[2])
local max_req      = tonumber(ARGV[3])
local token_count  = tonumber(ARGV[4])
local max_tok      = tonumber(ARGV[5])
local max_conc     = tonumber(ARGV[6])
local request_id   = ARGV[7]
local global_max_req = tonumber(ARGV[8])
local global_max_tok = tonumber(ARGV[9])

local window_start = now_ms - window_ms
local expire_sec   = math.ceil(window_ms / 1000) + 60

-- 1. Clean expired entries from all sliding windows via ZREMRANGEBYSCORE.
--    All sorted sets use timestamp (ms) as score for uniform expiration.
redis.call('ZREMRANGEBYSCORE', req_key, '-inf', window_start)
redis.call('ZREMRANGEBYSCORE', tok_key, '-inf', window_start)
redis.call('ZREMRANGEBYSCORE', greq_key, '-inf', window_start)
redis.call('ZREMRANGEBYSCORE', gtok_key, '-inf', window_start)

-- 2. Check per-identity request count (sliding window).
local current_req = redis.call('ZCARD', req_key)
if current_req >= max_req then
    return "DENIED:request_count"
end

-- 3. Check per-identity token consumption (sliding window).
--    Token sorted set members are formatted as "requestID:tokenCount".
local tok_members = redis.call('ZRANGEBYSCORE', tok_key, window_start, '+inf')
local current_tok = 0
for i = 1, #tok_members do
    local sep = string.find(tok_members[i], ":", 1, true)
    if sep then
        current_tok = current_tok + tonumber(string.sub(tok_members[i], sep + 1))
    end
end
if (current_tok + token_count) > max_tok then
    return "DENIED:token_limit"
end

-- 4. Check concurrent requests.
local current_conc = redis.call('SCARD', conc_key)
if current_conc >= max_conc then
    return "DENIED:concurrent"
end

-- 5. Check global request count (if enabled).
if global_max_req > 0 then
    local global_req = redis.call('ZCARD', greq_key)
    if global_req >= global_max_req then
        return "DENIED:global_request_count"
    end
end

-- 6. Check global token consumption (if enabled).
if global_max_tok > 0 then
    local gtok_members = redis.call('ZRANGEBYSCORE', gtok_key, window_start, '+inf')
    local global_tok = 0
    for i = 1, #gtok_members do
        local sep = string.find(gtok_members[i], ":", 1, true)
        if sep then
            global_tok = global_tok + tonumber(string.sub(gtok_members[i], sep + 1))
        end
    end
    if (global_tok + token_count) > global_max_tok then
        return "DENIED:global_token_limit"
    end
end

-- 7. All checks passed — record the request atomically.

-- Per-identity request window: member=requestID, score=timestamp_ms
redis.call('ZADD', req_key, now_ms, request_id)
redis.call('EXPIRE', req_key, expire_sec)

-- Per-identity token window: member="requestID:tokenCount", score=timestamp_ms
local tok_member = request_id .. ":" .. tostring(token_count)
redis.call('ZADD', tok_key, now_ms, tok_member)
redis.call('EXPIRE', tok_key, expire_sec)

-- Concurrent set: add request ID with 10-minute TTL for stale cleanup.
redis.call('SADD', conc_key, request_id)
redis.call('EXPIRE', conc_key, 600)

-- Global request window: member=requestID, score=timestamp_ms
redis.call('ZADD', greq_key, now_ms, request_id)
redis.call('EXPIRE', greq_key, expire_sec)

-- Global token window: member="requestID:tokenCount", score=timestamp_ms
redis.call('ZADD', gtok_key, now_ms, tok_member)
redis.call('EXPIRE', gtok_key, expire_sec)

return "ALLOWED"
"#;

/// Per-group limit overrides. A field of zero (or less) means "inherit the
/// default", it does not mean "forbid".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimitOverride {
    /// Overrides [`RateLimitSettings::requests_per_min`] when positive.
    pub requests_per_min: i64,
    /// Overrides [`RateLimitSettings::tokens_per_min`] when positive.
    pub tokens_per_min: i64,
    /// Overrides [`RateLimitSettings::max_concurrent`] when positive.
    pub max_concurrent: i64,
    /// Overrides [`RateLimitSettings::burst_size`] when positive.
    pub burst_size: i64,
}

/// Limiter configuration. Mirrors `gw_config::RateLimitConfig` field-for-field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitSettings {
    /// Per-identity requests allowed in one window.
    pub requests_per_min: i64,
    /// Per-identity tokens allowed in one window.
    pub tokens_per_min: i64,
    /// Per-identity in-flight requests.
    pub max_concurrent: i64,
    /// Burst allowance. Carried for config parity: the script does not consume
    /// it — the sliding window *is* the burst budget.
    pub burst_size: i64,
    /// Gateway-wide request cap; `<= 0` disables the global check.
    pub global_request_cap: i64,
    /// Gateway-wide token cap; `<= 0` disables the global check.
    pub global_token_cap: i64,
    /// Per-group overrides, keyed by the group id rendered in decimal (the YAML
    /// key form, e.g. `"3"`).
    pub group_overrides: HashMap<String, RateLimitOverride>,
    /// Per-model token ceilings, keyed by model name.
    pub model_token_limits: HashMap<String, i64>,
}

impl Default for RateLimitSettings {
    /// The form with every non-positive field defaulted.
    fn default() -> Self {
        Self {
            requests_per_min: DEFAULT_REQUESTS_PER_MIN,
            tokens_per_min: DEFAULT_TOKENS_PER_MIN,
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            burst_size: DEFAULT_BURST_SIZE,
            global_request_cap: DEFAULT_GLOBAL_REQUEST_CAP,
            global_token_cap: DEFAULT_GLOBAL_TOKEN_CAP,
            group_overrides: HashMap::new(),
            model_token_limits: HashMap::new(),
        }
    }
}

impl RateLimitSettings {
    /// Replaces every non-positive scalar with its default.
    #[must_use]
    pub fn with_defaults(mut self) -> Self {
        let defaults = Self::default();
        for (value, default) in [
            (&mut self.requests_per_min, defaults.requests_per_min),
            (&mut self.tokens_per_min, defaults.tokens_per_min),
            (&mut self.max_concurrent, defaults.max_concurrent),
            (&mut self.burst_size, defaults.burst_size),
            (&mut self.global_request_cap, defaults.global_request_cap),
            (&mut self.global_token_cap, defaults.global_token_cap),
        ] {
            if *value <= 0 {
                *value = default;
            }
        }
        self
    }

    /// Resolves the limits that apply to one request: a group override wins
    /// over the default for each field it sets positively, and a per-model
    /// token limit then wins over both.
    pub fn effective_limits(&self, group_id: Option<Id>, model: &str) -> EffectiveLimits {
        let mut limits = EffectiveLimits {
            max_requests: self.requests_per_min,
            max_tokens: self.tokens_per_min,
            max_concurrent: self.max_concurrent,
        };

        if let Some(group_id) = group_id
            && let Some(over) = self.group_overrides.get(&group_id.to_string())
        {
            if over.requests_per_min > 0 {
                limits.max_requests = over.requests_per_min;
            }
            if over.tokens_per_min > 0 {
                limits.max_tokens = over.tokens_per_min;
            }
            if over.max_concurrent > 0 {
                limits.max_concurrent = over.max_concurrent;
            }
        }

        if !model.is_empty()
            && let Some(&limit) = self.model_token_limits.get(model)
            && limit > 0
        {
            limits.max_tokens = limit;
        }

        limits
    }
}

impl From<&gw_config::RateLimitOverride> for RateLimitOverride {
    fn from(cfg: &gw_config::RateLimitOverride) -> Self {
        Self {
            requests_per_min: i64::from(cfg.requests_per_min),
            tokens_per_min: cfg.tokens_per_min,
            max_concurrent: i64::from(cfg.max_concurrent),
            burst_size: i64::from(cfg.burst_size),
        }
    }
}

impl From<&gw_config::RateLimitConfig> for RateLimitSettings {
    /// Lifts the `rate_limit:` block of `config.yaml` into this crate's
    /// settings. Widening to `i64` is not cosmetic: every budget ends up as a
    /// Lua number compared against Redis counters.
    fn from(cfg: &gw_config::RateLimitConfig) -> Self {
        Self {
            requests_per_min: i64::from(cfg.requests_per_min),
            tokens_per_min: cfg.tokens_per_min,
            max_concurrent: i64::from(cfg.max_concurrent),
            burst_size: i64::from(cfg.burst_size),
            global_request_cap: i64::from(cfg.global_request_cap),
            global_token_cap: cfg.global_token_cap,
            group_overrides: cfg
                .group_overrides
                .iter()
                .map(|(group, over)| (group.clone(), over.into()))
                .collect(),
            model_token_limits: cfg
                .model_token_limits
                .iter()
                .map(|(model, limit)| (model.clone(), *limit))
                .collect(),
        }
    }
}

/// The three per-identity budgets after override resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveLimits {
    /// Requests allowed in one window.
    pub max_requests: i64,
    /// Tokens allowed in one window.
    pub max_tokens: i64,
    /// In-flight requests allowed.
    pub max_concurrent: i64,
}

/// Which budget rejected a request. Carries the `DENIED:{dimension}` payloads
/// of the Lua script so callers can map each one onto its own error body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeniedDimension {
    /// Per-identity request count for the window.
    RequestCount,
    /// Per-identity token consumption for the window.
    TokenLimit,
    /// Per-identity in-flight requests.
    Concurrent,
    /// Gateway-wide request cap.
    GlobalRequestCount,
    /// Gateway-wide token cap.
    GlobalTokenLimit,
    /// The script denied with a payload this build does not know. Treated as a
    /// denial rather than silently letting the request past.
    Unspecified,
}

impl DeniedDimension {
    /// The wire spelling used inside the script's `DENIED:` payload.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequestCount => "request_count",
            Self::TokenLimit => "token_limit",
            Self::Concurrent => "concurrent",
            Self::GlobalRequestCount => "global_request_count",
            Self::GlobalTokenLimit => "global_token_limit",
            Self::Unspecified => "unspecified",
        }
    }

    /// Whether the rejection came from a gateway-wide cap rather than from the
    /// caller's own budget — the two deserve different messages upstream.
    pub fn is_global(self) -> bool {
        matches!(self, Self::GlobalRequestCount | Self::GlobalTokenLimit)
    }

    fn from_wire(raw: &str) -> Self {
        match raw {
            "request_count" => Self::RequestCount,
            "token_limit" => Self::TokenLimit,
            "concurrent" => Self::Concurrent,
            "global_request_count" => Self::GlobalRequestCount,
            "global_token_limit" => Self::GlobalTokenLimit,
            _ => Self::Unspecified,
        }
    }
}

impl fmt::Display for DeniedDimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Outcome of [`RateLimiter::allow`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitDecision {
    /// The request may proceed.
    Allowed {
        /// The concurrency slot that was reserved. `Some` means the caller
        /// **must** hand it to [`RateLimiter::release_conc`] when the request
        /// finishes, or the slot leaks until its 10-minute TTL and
        /// `max_concurrent` degrades into a second per-window request cap.
        ///
        /// `None` means no slot was taken because the limiter failed open
        /// (no Redis, or Redis errored) — nothing to release.
        release_id: Option<String>,
    },
    /// The request must be rejected, with the budget that rejected it.
    Denied {
        /// Which budget was exhausted.
        dimension: DeniedDimension,
    },
}

impl RateLimitDecision {
    /// Whether the request may proceed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    /// The reserved concurrency slot, if one was taken.
    pub fn release_id(&self) -> Option<&str> {
        match self {
            Self::Allowed { release_id } => release_id.as_deref(),
            Self::Denied { .. } => None,
        }
    }

    /// The exhausted budget, if this is a denial.
    pub fn denied_dimension(&self) -> Option<DeniedDimension> {
        match self {
            Self::Denied { dimension } => Some(*dimension),
            Self::Allowed { .. } => None,
        }
    }
}

/// Redis-backed sliding-window limiter.
pub struct RateLimiter {
    redis: Option<Redis>,
    settings: RateLimitSettings,
    script: Script,
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimiter")
            .field("redis_enabled", &self.redis.is_some())
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl RateLimiter {
    /// Builds a limiter, applying the zero-value defaulting.
    ///
    /// `redis` is `None` for an absent client: every call then fails open.
    pub fn new(redis: Option<Redis>, settings: RateLimitSettings) -> Self {
        Self {
            redis,
            settings: settings.with_defaults(),
            script: Script::new(RATE_LIMIT_LUA),
        }
    }

    /// The settings actually in force, after defaulting.
    pub fn settings(&self) -> &RateLimitSettings {
        &self.settings
    }

    /// Checks every budget for one request and, when they all pass, records it.
    ///
    /// * `identity` — the rate-limit subject (user id, API key id, …).
    /// * `token_count` — estimated token spend of this request.
    /// * `model` — requested model, for the per-model token ceiling; `""` skips it.
    /// * `group_id` — the key's group, for per-group overrides.
    ///
    /// **Fails open.** A missing or broken Redis yields
    /// `Allowed { release_id: None }` and a warning, never an error: a limiter
    /// outage must not become a gateway outage.
    pub async fn allow(
        &self,
        identity: &str,
        token_count: i64,
        model: &str,
        group_id: Option<Id>,
    ) -> RateLimitDecision {
        let Some(conn) = self.redis.as_ref() else {
            tracing::warn!("RateLimiter: Redis client is nil, allowing request (fail-open)");
            return RateLimitDecision::Allowed { release_id: None };
        };

        let limits = self.settings.effective_limits(group_id, model);
        let request_id = new_request_id();
        let mut conn = conn.clone();

        let mut invocation = self.script.prepare_invoke();
        invocation
            .key(request_key(identity))
            .key(token_key(identity))
            .key(concurrency_key(identity))
            .key(global_request_key())
            .key(global_token_key())
            .arg(Utc::now().timestamp_millis())
            .arg(WINDOW_MS)
            .arg(limits.max_requests)
            .arg(token_count)
            .arg(limits.max_tokens)
            .arg(limits.max_concurrent)
            .arg(&request_id)
            .arg(self.settings.global_request_cap)
            .arg(self.settings.global_token_cap);
        let outcome: Result<String, _> = invocation.invoke_async(&mut conn).await;

        let raw = match outcome {
            Ok(raw) => raw,
            Err(error) => {
                tracing::warn!(
                    identity,
                    %error,
                    "RateLimiter: Redis Lua script error, allowing request (fail-open)"
                );
                return RateLimitDecision::Allowed { release_id: None };
            }
        };

        let decision = parse_script_result(&raw, &request_id);
        if let Some(dimension) = decision.denied_dimension() {
            tracing::info!(
                identity,
                dimension = dimension.as_str(),
                model,
                token_count,
                "RateLimiter: request denied"
            );
        }
        decision
    }

    /// Frees the concurrency slot reserved by [`Self::allow`]. Must be called
    /// on both the success and the failure path of a request.
    pub async fn release_conc(&self, identity: &str, release_id: &str) -> Result<(), InfraError> {
        let Some(conn) = self.redis.as_ref() else {
            return Ok(());
        };
        if release_id.is_empty() {
            // Nothing was reserved (fail-open path).
            return Ok(());
        }

        let mut conn = conn.clone();
        let removed: i64 = conn
            .srem(concurrency_key(identity), release_id)
            .await
            .map_err(|error| {
                tracing::warn!(
                    identity,
                    release_id,
                    %error,
                    "RateLimiter: failed to release concurrent slot"
                );
                InfraError::Redis(error)
            })?;

        if removed == 0 {
            tracing::debug!(
                identity,
                release_id,
                "RateLimiter: concurrency slot was already gone (expired or double-released)"
            );
        }
        Ok(())
    }
}

/// Turns the script's reply into a decision, treating an unrecognised payload
/// as a denial.
fn parse_script_result(raw: &str, request_id: &str) -> RateLimitDecision {
    if raw == "ALLOWED" {
        return RateLimitDecision::Allowed {
            release_id: Some(request_id.to_owned()),
        };
    }
    RateLimitDecision::Denied {
        dimension: raw
            .strip_prefix("DENIED:")
            .map_or(DeniedDimension::Unspecified, DeniedDimension::from_wire),
    }
}

/// Per-identity request window key.
fn request_key(identity: &str) -> String {
    format!("{KEY_PREFIX}:req:{identity}")
}

/// Per-identity token window key.
fn token_key(identity: &str) -> String {
    format!("{KEY_PREFIX}:tok:{identity}")
}

/// Per-identity in-flight request set key.
fn concurrency_key(identity: &str) -> String {
    format!("{KEY_PREFIX}:conc:{identity}")
}

/// Gateway-wide request window key.
fn global_request_key() -> String {
    format!("{KEY_PREFIX}:global:req")
}

/// Gateway-wide token window key.
fn global_token_key() -> String {
    format!("{KEY_PREFIX}:global:tok")
}

/// A process-unique, colon-free request id.
///
/// Colon-free is a correctness requirement, not a style choice: the Lua script
/// splits token members at their first colon to recover the token count.
/// Uniqueness comes from a per-process random seed (so two gateway instances
/// cannot collide in shared Redis) plus a monotonic counter (so one process
/// cannot collide with itself, even within the same nanosecond).
pub(crate) fn new_request_id() -> String {
    static SEED: OnceLock<u64> = OnceLock::new();
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let seed = *SEED.get_or_init(|| {
        // RandomState is seeded by the OS once per process.
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u32(std::process::id());
        hasher.finish()
    });
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as u64);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

    format!("{seed:016x}{nanos:016x}{counter:016x}")
}

//! Typed configuration: YAML file + environment-variable overrides.
//!
//! Field names match `config.yaml`, so every `*_*` env override keeps working
//! unchanged.
//!
//! Three things happen in [`Config::load`], in this order:
//!
//! 1. the YAML file is parsed (unknown keys — e.g. the `frontend:` block in
//!    `config.example.yaml` — are ignored);
//! 2. environment overrides are applied ([`Config::apply_env_overrides`]);
//! 3. [`Config::normalize`] materializes the defaults that consumers would
//!    otherwise apply themselves (rate limiting, circuit breaking, ledger,
//!    hold middleware, ...). Consumers may still re-apply their own
//!    `<= 0 → default` clamp: it is idempotent.
//!
//! OWNER: worker `config-server`.

// Rule 5.3 ratchet: this crate has zero stubs, so it denies what the workspace
// can still only warn about (other crates carry `todo!()`s of their own).
#![deny(clippy::todo, clippy::unimplemented)]

mod env;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use env::parse_loose_bool;

/// Failure modes of [`Config::load`].
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("parse yaml: {0}")]
    Parse(String),
    #[error("invalid config: {0}")]
    Invalid(String),
}

// ---------------------------------------------------------------------------
// Defaults
//
// Every constant below is the authoritative default for one config field.
// Where a field's consumer owns its own fallback, that is noted on the constant
// so drift is greppable.
// ---------------------------------------------------------------------------

/// Empty host falls back to loopback.
pub const DEFAULT_SERVER_HOST: &str = "127.0.0.1";
/// Port 0 → 8888.
pub const DEFAULT_SERVER_PORT: u16 = 8888;
/// `ServerConfig.LogLevel` doc comment: empty falls back to `warn`.
pub const DEFAULT_LOG_LEVEL: &str = "warn";

/// Hold TTL default (5 minutes), shared by the ledger and the hold middleware.
pub const DEFAULT_HOLD_TTL_SECONDS: i32 = 300;
/// Balance cache TTL default.
pub const DEFAULT_BALANCE_CACHE_TTL_SECONDS: i32 = 30;
/// `BillingConfig.BudgetTokenMultiplier` doc comment.
pub const DEFAULT_BUDGET_TOKEN_MULTIPLIER: i32 = 10;
/// `BillingConfig.BudgetTokenTTLSeconds` doc comment.
pub const DEFAULT_BUDGET_TOKEN_TTL_SECONDS: i32 = 60;
/// `BillingConfig.LowBalanceThresholdUSD` doc comment.
pub const DEFAULT_LOW_BALANCE_THRESHOLD_USD: f64 = 1.0;
/// Price-cache refresh falls back to 60 seconds when unset (<= 0).
pub const DEFAULT_PRICE_CACHE_REFRESH_SECONDS: i32 = 60;

/// Default requests-per-minute cap.
pub const DEFAULT_REQUESTS_PER_MIN: i32 = 60;
/// Default tokens-per-minute cap.
pub const DEFAULT_TOKENS_PER_MIN: i64 = 100_000;
/// Default concurrent-request cap.
pub const DEFAULT_MAX_CONCURRENT: i32 = 10;
/// Default burst size.
pub const DEFAULT_BURST_SIZE: i32 = 2;
/// Default global request cap.
pub const DEFAULT_GLOBAL_REQUEST_CAP: i32 = 10_000;
/// Default global token cap.
pub const DEFAULT_GLOBAL_TOKEN_CAP: i64 = 10_000_000;

/// Default failure threshold.
pub const DEFAULT_FAILURE_THRESHOLD: f64 = 0.5;
/// Default window length, in seconds.
pub const DEFAULT_WINDOW_SECONDS: i32 = 30;
/// Default cooldown length, in seconds.
pub const DEFAULT_COOLDOWN_SECONDS: i32 = 30;

fn default_server_host() -> String {
    DEFAULT_SERVER_HOST.to_owned()
}
fn default_server_port() -> u16 {
    DEFAULT_SERVER_PORT
}
fn default_log_level() -> String {
    DEFAULT_LOG_LEVEL.to_owned()
}
fn default_hold_ttl_seconds() -> i32 {
    DEFAULT_HOLD_TTL_SECONDS
}
fn default_balance_cache_ttl_seconds() -> i32 {
    DEFAULT_BALANCE_CACHE_TTL_SECONDS
}
fn default_budget_token_multiplier() -> i32 {
    DEFAULT_BUDGET_TOKEN_MULTIPLIER
}
fn default_budget_token_ttl_seconds() -> i32 {
    DEFAULT_BUDGET_TOKEN_TTL_SECONDS
}
fn default_low_balance_threshold_usd() -> f64 {
    DEFAULT_LOW_BALANCE_THRESHOLD_USD
}
fn default_price_cache_refresh_seconds() -> i32 {
    DEFAULT_PRICE_CACHE_REFRESH_SECONDS
}
fn default_requests_per_min() -> i32 {
    DEFAULT_REQUESTS_PER_MIN
}
fn default_tokens_per_min() -> i64 {
    DEFAULT_TOKENS_PER_MIN
}
fn default_max_concurrent() -> i32 {
    DEFAULT_MAX_CONCURRENT
}
fn default_burst_size() -> i32 {
    DEFAULT_BURST_SIZE
}
fn default_global_request_cap() -> i32 {
    DEFAULT_GLOBAL_REQUEST_CAP
}
fn default_global_token_cap() -> i64 {
    DEFAULT_GLOBAL_TOKEN_CAP
}
fn default_failure_threshold() -> f64 {
    DEFAULT_FAILURE_THRESHOLD
}
fn default_window_seconds() -> i32 {
    DEFAULT_WINDOW_SECONDS
}
fn default_cooldown_seconds() -> i32 {
    DEFAULT_COOLDOWN_SECONDS
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

/// Root configuration.
///
/// Field names / `serde(rename)` match the YAML keys in
/// `config.example.yaml` exactly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub redis: RedisConfig,
    #[serde(default)]
    pub sdk: SdkConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub billing: BillingConfig,
    #[serde(default, rename = "rate_limit")]
    pub rate_limit: RateLimitConfig,
    #[serde(default, rename = "circuit_breaker")]
    pub circuit_breaker: CircuitBreakerConfig,
}

/// HTTP server settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_server_host")]
    pub host: String,
    /// A gateway port is a `u16` everywhere it is actually used (bind address,
    /// health probe), so an out-of-range `SERVER_PORT` is ignored rather than
    /// stored as an unusable value.
    #[serde(default = "default_server_port")]
    pub port: u16,
    /// SQL-logger verbosity: `silent` / `error` / `warn` / `info`.
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_server_host(),
            port: default_server_port(),
            log_level: default_log_level(),
        }
    }
}

/// PostgreSQL connection settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub dbname: String,
    #[serde(default)]
    pub sslmode: String,
    /// Pool tuning. `0` keeps the consumer-side defaults (25 idle / 200 open /
    /// 30m lifetime) — deliberately NOT normalized here.
    #[serde(default)]
    pub max_idle_conns: i32,
    #[serde(default)]
    pub max_open_conns: i32,
    #[serde(default)]
    pub conn_max_lifetime_minutes: i32,
}

impl DatabaseConfig {
    /// The libpq key/value connection string.
    pub fn dsn(&self) -> String {
        format!(
            "host={} port={} user={} password={} dbname={} sslmode={}",
            self.host, self.port, self.user, self.password, self.dbname, self.sslmode
        )
    }
}

/// Redis connection settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RedisConfig {
    #[serde(default)]
    pub addr: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub db: i64,
}

/// Upstream-provider settings.
///
/// The `sdk` naming is historical: these fields feed `gw-provider` /
/// `gw-proxy` directly, but the YAML keys must not move.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SdkConfig {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub timeout_seconds: i32,
    #[serde(default)]
    pub openai: SdkProviderConfig,
    #[serde(default)]
    pub openai_compatible: SdkProviderConfig,
    #[serde(default)]
    pub claude: SdkProviderConfig,
    #[serde(default)]
    pub gemini: SdkProviderConfig,
    #[serde(default)]
    pub codex: SdkProviderConfig,
    #[serde(default)]
    pub vertex: SdkProviderConfig,
}

impl SdkConfig {
    /// Resolve the OpenAI-compatible provider, falling back to the legacy
    /// top-level `base_url` / `api_key`.
    pub fn openai_provider_config(&self) -> SdkProviderConfig {
        let mut provider = self.openai.clone();
        if !provider.configured() && self.openai_compatible.configured() {
            provider = self.openai_compatible.clone();
        }

        if provider.base_url.trim().is_empty() {
            provider.base_url = self.base_url.clone();
        }
        if provider.api_key.trim().is_empty() {
            provider.api_key = self.api_key.clone();
        }
        if !provider.enabled
            && !provider.base_url.trim().is_empty()
            && !provider.api_key.trim().is_empty()
        {
            provider.enabled = true;
        }

        provider
    }

    /// The upstream request timeout, `0` meaning "no configured timeout".
    pub fn timeout(&self) -> Option<Duration> {
        (self.timeout_seconds > 0).then(|| Duration::from_secs(self.timeout_seconds as u64))
    }
}

/// Per-provider upstream settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkProviderConfig {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub enabled: bool,
}

impl SdkProviderConfig {
    /// Whether any meaningful value is set.
    pub fn configured(&self) -> bool {
        self.enabled || !self.base_url.trim().is_empty() || !self.api_key.trim().is_empty()
    }

    /// Whether enough configuration is present to be used.
    pub fn complete(&self) -> bool {
        self.enabled && !self.base_url.trim().is_empty() && !self.api_key.trim().is_empty()
    }
}

/// Authentication settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub jwt: JwtConfig,
    /// 32-byte key (hex or base64) enabling AES-GCM encryption-at-rest of
    /// upstream credentials. Empty = cleartext (backward compatible).
    #[serde(default)]
    pub credential_encryption_key: String,
    /// Retained for backward-compatible parsing only: runtime admin
    /// authorization lives in `users.role`, never in an email claim.
    #[serde(default)]
    pub admin_emails: Vec<String>,
    /// One-time, server-side admin bootstrap. Inert once any admin exists.
    #[serde(default)]
    pub bootstrap_admin_email: String,
}

/// JWT settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JwtConfig {
    #[serde(default)]
    pub secret: String,
    #[serde(default)]
    pub expiry_hours: i32,
}

/// Billing / pre-charge settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingConfig {
    /// No fallback default: `0` stays `0`.
    #[serde(default)]
    pub hold_amount: i32,
    /// No fallback default: `0` stays `0`. Note the unit — per **1K** tokens,
    /// while `pricing::Calculator` works per 1M (see
    /// [`BillingConfig::default_price_per_1m_tokens`]).
    #[serde(default)]
    pub default_price_per_1k_tokens: f64,
    #[serde(default = "default_hold_ttl_seconds")]
    pub hold_ttl_seconds: i32,
    #[serde(default = "default_balance_cache_ttl_seconds")]
    pub balance_cache_ttl_seconds: i32,
    #[serde(default = "default_budget_token_multiplier")]
    pub budget_token_multiplier: i32,
    #[serde(default = "default_budget_token_ttl_seconds")]
    pub budget_token_ttl_seconds: i32,
    #[serde(default = "default_low_balance_threshold_usd")]
    pub low_balance_threshold_usd: f64,
    /// `false` = conservative fallback settlement; `true` = suspend billing on
    /// upstream responses that strip the usage envelope.
    #[serde(default)]
    pub strict_usage_metadata_mode: bool,
    #[serde(default = "default_price_cache_refresh_seconds")]
    pub price_cache_refresh_seconds: i32,
}

impl Default for BillingConfig {
    fn default() -> Self {
        Self {
            hold_amount: 0,
            default_price_per_1k_tokens: 0.0,
            hold_ttl_seconds: default_hold_ttl_seconds(),
            balance_cache_ttl_seconds: default_balance_cache_ttl_seconds(),
            budget_token_multiplier: default_budget_token_multiplier(),
            budget_token_ttl_seconds: default_budget_token_ttl_seconds(),
            low_balance_threshold_usd: default_low_balance_threshold_usd(),
            strict_usage_metadata_mode: false,
            price_cache_refresh_seconds: default_price_cache_refresh_seconds(),
        }
    }
}

impl BillingConfig {
    /// The ledger and the hold middleware MUST share one uniform TTL — a
    /// per-request value desyncs the hold/balance cleanup cutoffs.
    pub fn hold_ttl(&self) -> Duration {
        seconds_or(self.hold_ttl_seconds, DEFAULT_HOLD_TTL_SECONDS)
    }

    /// Balance-cache TTL, defaulted when unset (<= 0).
    pub fn balance_cache_ttl(&self) -> Duration {
        seconds_or(
            self.balance_cache_ttl_seconds,
            DEFAULT_BALANCE_CACHE_TTL_SECONDS,
        )
    }

    /// Price-cache refresh interval, defaulted when unset (<= 0).
    pub fn price_cache_refresh(&self) -> Duration {
        seconds_or(
            self.price_cache_refresh_seconds,
            DEFAULT_PRICE_CACHE_REFRESH_SECONDS,
        )
    }

    /// Config stores "per 1K tokens", the calculator operates in "per 1M
    /// tokens".
    pub fn default_price_per_1m_tokens(&self) -> f64 {
        self.default_price_per_1k_tokens * 1000.0
    }
}

fn seconds_or(value: i32, fallback: i32) -> Duration {
    let secs = if value > 0 { value } else { fallback };
    Duration::from_secs(secs as u64)
}

/// Per-group rate-limit override.
///
/// Unlike [`RateLimitConfig`], `0` here means "no override" (the consumer
/// treats a non-positive value as absent), so these fields are never defaulted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitOverride {
    #[serde(default)]
    pub requests_per_min: i32,
    #[serde(default)]
    pub tokens_per_min: i64,
    #[serde(default)]
    pub max_concurrent: i32,
    #[serde(default)]
    pub burst_size: i32,
}

/// Rate-limiting settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default = "default_requests_per_min")]
    pub requests_per_min: i32,
    #[serde(default = "default_tokens_per_min")]
    pub tokens_per_min: i64,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: i32,
    #[serde(default = "default_burst_size")]
    pub burst_size: i32,
    #[serde(default = "default_global_request_cap")]
    pub global_request_cap: i32,
    #[serde(default = "default_global_token_cap")]
    pub global_token_cap: i64,
    #[serde(default)]
    pub group_overrides: BTreeMap<String, RateLimitOverride>,
    #[serde(default)]
    pub model_token_limits: BTreeMap<String, i64>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_min: default_requests_per_min(),
            tokens_per_min: default_tokens_per_min(),
            max_concurrent: default_max_concurrent(),
            burst_size: default_burst_size(),
            global_request_cap: default_global_request_cap(),
            global_token_cap: default_global_token_cap(),
            group_overrides: BTreeMap::new(),
            model_token_limits: BTreeMap::new(),
        }
    }
}

/// Circuit-breaker settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: f64,
    #[serde(default = "default_window_seconds")]
    pub window_seconds: i32,
    #[serde(default = "default_cooldown_seconds")]
    pub cooldown_seconds: i32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: default_failure_threshold(),
            window_seconds: default_window_seconds(),
            cooldown_seconds: default_cooldown_seconds(),
        }
    }
}

impl CircuitBreakerConfig {
    /// Rolling failure-rate window, defaulted when unset.
    pub fn window(&self) -> Duration {
        seconds_or(self.window_seconds, DEFAULT_WINDOW_SECONDS)
    }

    /// Open-state cooldown, defaulted when unset.
    pub fn cooldown(&self) -> Duration {
        seconds_or(self.cooldown_seconds, DEFAULT_COOLDOWN_SECONDS)
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

impl Config {
    /// Read the YAML file, apply env overrides, then materialize consumer-side
    /// defaults.
    pub fn load(path: &str) -> Result<Config, ConfigError> {
        let data = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;

        let mut cfg = Config::parse_yaml(&data)?;
        cfg.apply_env_overrides();
        cfg.normalize();
        Ok(cfg)
    }

    /// Parse YAML without touching the environment. Unknown keys are ignored,
    /// and an empty document yields defaults.
    pub fn parse_yaml(data: &str) -> Result<Config, ConfigError> {
        if data.trim().is_empty() {
            return Ok(Config::default());
        }
        serde_yaml_ng::from_str(data).map_err(|err| ConfigError::Parse(err.to_string()))
    }

    /// Apply overrides from the process environment.
    ///
    /// Only non-empty values override, and an unparseable numeric/bool value is
    /// ignored rather than treated as fatal.
    pub fn apply_env_overrides(&mut self) {
        self.apply_env_overrides_from(|key| std::env::var(key).ok());
    }

    /// [`Config::apply_env_overrides`] against an arbitrary lookup.
    ///
    /// Tests use this instead of `std::env::set_var`, which is `unsafe` in
    /// edition 2024 and forbidden workspace-wide (and racy under the parallel
    /// test harness anyway).
    pub fn apply_env_overrides_from<F>(&mut self, get: F)
    where
        F: Fn(&str) -> Option<String>,
    {
        env::apply(self, get);
    }

    /// Materialize consumer-side defaults, so every reader of a loaded `Config`
    /// sees the same effective values.
    ///
    /// Only fields that have a `<= 0 → default` fallback are clamped;
    /// `billing.hold_amount`, `billing.default_price_per_1k_tokens`,
    /// `billing.low_balance_threshold_usd`, the `budget_token_*` pair and the
    /// database pool knobs have no such fallback, so an explicit `0` survives.
    pub fn normalize(&mut self) {
        if self.server.host.trim().is_empty() {
            self.server.host = DEFAULT_SERVER_HOST.to_owned();
        }
        if self.server.port == 0 {
            self.server.port = DEFAULT_SERVER_PORT;
        }
        if self.server.log_level.trim().is_empty() {
            self.server.log_level = DEFAULT_LOG_LEVEL.to_owned();
        }

        clamp_i32(&mut self.billing.hold_ttl_seconds, DEFAULT_HOLD_TTL_SECONDS);
        clamp_i32(
            &mut self.billing.balance_cache_ttl_seconds,
            DEFAULT_BALANCE_CACHE_TTL_SECONDS,
        );
        clamp_i32(
            &mut self.billing.price_cache_refresh_seconds,
            DEFAULT_PRICE_CACHE_REFRESH_SECONDS,
        );

        clamp_i32(
            &mut self.rate_limit.requests_per_min,
            DEFAULT_REQUESTS_PER_MIN,
        );
        clamp_i64(&mut self.rate_limit.tokens_per_min, DEFAULT_TOKENS_PER_MIN);
        clamp_i32(&mut self.rate_limit.max_concurrent, DEFAULT_MAX_CONCURRENT);
        clamp_i32(&mut self.rate_limit.burst_size, DEFAULT_BURST_SIZE);
        clamp_i32(
            &mut self.rate_limit.global_request_cap,
            DEFAULT_GLOBAL_REQUEST_CAP,
        );
        clamp_i64(
            &mut self.rate_limit.global_token_cap,
            DEFAULT_GLOBAL_TOKEN_CAP,
        );

        if self.circuit_breaker.failure_threshold <= 0.0 {
            self.circuit_breaker.failure_threshold = DEFAULT_FAILURE_THRESHOLD;
        }
        clamp_i32(
            &mut self.circuit_breaker.window_seconds,
            DEFAULT_WINDOW_SECONDS,
        );
        clamp_i32(
            &mut self.circuit_breaker.cooldown_seconds,
            DEFAULT_COOLDOWN_SECONDS,
        );
    }
}

fn clamp_i32(value: &mut i32, fallback: i32) {
    if *value <= 0 {
        *value = fallback;
    }
}

fn clamp_i64(value: &mut i64, fallback: i64) {
    if *value <= 0 {
        *value = fallback;
    }
}

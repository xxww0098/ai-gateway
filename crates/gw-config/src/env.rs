//! Environment-variable overrides, applied field-by-field by
//! `Config::apply_env_overrides_from`.
//!
//! Two behaviours are load-bearing and reproduced exactly:
//!
//! * an override applies only when the variable is a **non-empty** string
//!   (an empty var is indistinguishable from an unset one);
//! * a value that fails integer / float / bool parsing is **silently
//!   discarded**, leaving the YAML value in place.

use crate::Config;

/// Apply every documented override to `cfg`, resolving variables through `get`.
pub(crate) fn apply<F>(cfg: &mut Config, get: F)
where
    F: Fn(&str) -> Option<String>,
{
    let raw = |key: &str| get(key).filter(|value| !value.is_empty());
    let int = |key: &str| raw(key).and_then(|value| value.parse::<i32>().ok());
    let long = |key: &str| raw(key).and_then(|value| value.parse::<i64>().ok());
    let port = |key: &str| raw(key).and_then(|value| value.parse::<u16>().ok());
    let float = |key: &str| raw(key).and_then(|value| value.parse::<f64>().ok());

    // -- Server. SERVER_HOST is required for containerized deploys: the process
    //    must bind 0.0.0.0 inside the container while the shipped config
    //    defaults to loopback.
    if let Some(v) = raw("SERVER_HOST") {
        cfg.server.host = v;
    }
    if let Some(v) = port("SERVER_PORT") {
        cfg.server.port = v;
    }
    if let Some(v) = raw("LOG_LEVEL") {
        cfg.server.log_level = v;
    }

    // -- Database
    if let Some(v) = raw("DB_HOST") {
        cfg.database.host = v;
    }
    if let Some(v) = port("DB_PORT") {
        cfg.database.port = v;
    }
    if let Some(v) = raw("DB_USER") {
        cfg.database.user = v;
    }
    if let Some(v) = raw("DB_PASSWORD") {
        cfg.database.password = v;
    }
    if let Some(v) = raw("DB_NAME") {
        cfg.database.dbname = v;
    }
    if let Some(v) = raw("DB_SSLMODE") {
        cfg.database.sslmode = v;
    }
    if let Some(v) = int("DB_MAX_IDLE_CONNS") {
        cfg.database.max_idle_conns = v;
    }
    if let Some(v) = int("DB_MAX_OPEN_CONNS") {
        cfg.database.max_open_conns = v;
    }
    if let Some(v) = int("DB_CONN_MAX_LIFETIME_MINUTES") {
        cfg.database.conn_max_lifetime_minutes = v;
    }

    // -- Redis
    if let Some(v) = raw("REDIS_ADDR") {
        cfg.redis.addr = v;
    }
    if let Some(v) = raw("REDIS_PASSWORD") {
        cfg.redis.password = v;
    }
    if let Some(v) = long("REDIS_DB") {
        cfg.redis.db = v;
    }

    // -- Auth / JWT
    if let Some(v) = raw("JWT_SECRET") {
        cfg.auth.jwt.secret = v;
    }
    if let Some(v) = int("JWT_EXPIRY_HOURS") {
        cfg.auth.jwt.expiry_hours = v;
    }
    if let Some(v) = raw("CREDENTIAL_ENCRYPTION_KEY") {
        cfg.auth.credential_encryption_key = v;
    }
    if let Some(v) = raw("ADMIN_EMAILS") {
        cfg.auth.admin_emails = v
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect();
    }
    if let Some(v) = raw("BOOTSTRAP_ADMIN_EMAIL") {
        cfg.auth.bootstrap_admin_email = v.trim().to_lowercase();
    }

    // -- SDK (upstream provider defaults)
    if let Some(v) = raw("SDK_BASE_URL") {
        cfg.sdk.base_url = v;
    }
    if let Some(v) = raw("SDK_API_KEY") {
        cfg.sdk.api_key = v;
    }
    if let Some(v) = int("SDK_TIMEOUT_SECONDS") {
        cfg.sdk.timeout_seconds = v;
    }

    // -- Billing
    if let Some(v) = int("BILLING_HOLD_AMOUNT") {
        cfg.billing.hold_amount = v;
    }
    if let Some(v) = float("BILLING_DEFAULT_PRICE_PER_1K_TOKENS") {
        cfg.billing.default_price_per_1k_tokens = v;
    }
    if let Some(v) = int("BILLING_HOLD_TTL_SECONDS") {
        cfg.billing.hold_ttl_seconds = v;
    }
    if let Some(v) = int("BILLING_BALANCE_CACHE_TTL_SECONDS") {
        cfg.billing.balance_cache_ttl_seconds = v;
    }
    if let Some(v) = int("BILLING_BUDGET_TOKEN_MULTIPLIER") {
        cfg.billing.budget_token_multiplier = v;
    }
    if let Some(v) = int("BILLING_BUDGET_TOKEN_TTL_SECONDS") {
        cfg.billing.budget_token_ttl_seconds = v;
    }
    if let Some(v) = int("BILLING_PRICE_CACHE_REFRESH_SECONDS") {
        cfg.billing.price_cache_refresh_seconds = v;
    }
    if let Some(v) = float("BILLING_LOW_BALANCE_THRESHOLD_USD") {
        cfg.billing.low_balance_threshold_usd = v;
    }
    if let Some(v) = raw("BILLING_STRICT_USAGE_METADATA_MODE").and_then(|v| parse_loose_bool(&v)) {
        cfg.billing.strict_usage_metadata_mode = v;
    }

    // -- Rate limit
    if let Some(v) = int("RATE_LIMIT_REQUESTS_PER_MIN") {
        cfg.rate_limit.requests_per_min = v;
    }
    if let Some(v) = long("RATE_LIMIT_TOKENS_PER_MIN") {
        cfg.rate_limit.tokens_per_min = v;
    }
    if let Some(v) = int("RATE_LIMIT_MAX_CONCURRENT") {
        cfg.rate_limit.max_concurrent = v;
    }
    if let Some(v) = int("RATE_LIMIT_BURST_SIZE") {
        cfg.rate_limit.burst_size = v;
    }
    if let Some(v) = int("RATE_LIMIT_GLOBAL_REQUEST_CAP") {
        cfg.rate_limit.global_request_cap = v;
    }
    if let Some(v) = long("RATE_LIMIT_GLOBAL_TOKEN_CAP") {
        cfg.rate_limit.global_token_cap = v;
    }

    // -- Circuit breaker
    if let Some(v) = float("CIRCUIT_BREAKER_FAILURE_THRESHOLD") {
        cfg.circuit_breaker.failure_threshold = v;
    }
    if let Some(v) = int("CIRCUIT_BREAKER_WINDOW_SECONDS") {
        cfg.circuit_breaker.window_seconds = v;
    }
    if let Some(v) = int("CIRCUIT_BREAKER_COOLDOWN_SECONDS") {
        cfg.circuit_breaker.cooldown_seconds = v;
    }
}

/// The accepted bool spellings — `"1"` and `"0"` included, which Rust's own
/// `str::parse::<bool>()` rejects. Anything else is `None`, i.e. the override
/// is skipped.
pub fn parse_loose_bool(value: &str) -> Option<bool> {
    match value {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Some(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Some(false),
        _ => None,
    }
}

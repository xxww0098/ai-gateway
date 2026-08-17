//! Unit tests for the config crate.
//!
//! Two rules shape what is asserted here:
//!
//! * **Parity, not restatement** (rule 2.11). The expected values are either
//!   read off the checked-in YAML files or the documented defaults. A test
//!   that merely echoes a constant from `lib.rs` would pass by construction.
//! * **No `set_var`**. `std::env::set_var` is `unsafe` in edition 2024 (and the
//!   workspace forbids `unsafe`), plus it races the parallel test harness — so
//!   overrides go through [`Config::apply_env_overrides_from`] with an explicit
//!   map, never the process environment.

use std::collections::BTreeMap;

use crate::*;

/// The config template the gateway ships with, resolved at compile time.
const EXAMPLE_YAML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../config.example.yaml"
));

/// A deployment-shaped config whose `rate_limit` / `circuit_breaker` sections
/// are absent entirely.
///
/// This used to `include_str!` the repo's real `config.yaml`. That file is
/// gitignored because it holds secrets, so the test could only compile on a
/// machine that happened to have one — a fresh clone or CI failed at build
/// time, and a colleague with a different `config.yaml` failed at assert time.
/// The invariant under test is about *absent sections*, not about anyone's
/// local values, so the fixture belongs here.
const SPARSE_YAML: &str = r#"
server:
  host: 127.0.0.1
  port: 8888
database:
  host: 127.0.0.1
  port: 5432
  user: ai-gateway
  dbname: ai_gateway
  sslmode: disable
redis:
  addr: 127.0.0.1:6379
auth:
  jwt:
    secret: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    expiry_hours: 24
  admin_emails:
    - admin@example.com
"#;

fn env_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// Parse + normalize, then apply exactly the given overrides — i.e. everything
/// `Config::load` does, minus the file read and the process environment.
fn load_str(yaml: &str, env: &[(&str, &str)]) -> Config {
    let map = env_of(env);
    let mut cfg = Config::parse_yaml(yaml).expect("yaml parses");
    cfg.apply_env_overrides_from(|key| map.get(key).cloned());
    cfg.normalize();
    cfg
}

// ---------------------------------------------------------------------------
// The shipped YAML files
// ---------------------------------------------------------------------------

#[test]
fn example_config_parses_every_shipped_key() {
    let cfg = load_str(EXAMPLE_YAML, &[]);

    assert_eq!(cfg.server.host, "127.0.0.1");
    assert_eq!(cfg.server.port, 8888);

    assert_eq!(cfg.database.host, "127.0.0.1");
    assert_eq!(cfg.database.port, 5432);
    assert_eq!(cfg.database.user, "ai_gateway");
    assert_eq!(cfg.database.dbname, "ai_gateway");
    assert_eq!(cfg.database.sslmode, "disable");
    // Explicit 0s in the file: the pool knobs have no consumer-side clamp, so
    // they must survive normalize() untouched (the consumer owns those defaults).
    assert_eq!(cfg.database.max_idle_conns, 0);
    assert_eq!(cfg.database.max_open_conns, 0);
    assert_eq!(cfg.database.conn_max_lifetime_minutes, 0);

    assert_eq!(cfg.redis.addr, "127.0.0.1:6379");
    assert_eq!(cfg.redis.db, 0);

    assert_eq!(cfg.auth.jwt.secret, "");
    assert_eq!(cfg.auth.jwt.expiry_hours, 24);
    assert_eq!(cfg.auth.credential_encryption_key, "");
    assert_eq!(cfg.auth.bootstrap_admin_email, "");

    assert_eq!(cfg.sdk.base_url, "https://sdk.example.com");
    assert_eq!(cfg.sdk.timeout_seconds, 30);
    assert!(cfg.sdk.openai.enabled);
    assert!(!cfg.sdk.openai_compatible.enabled);
    assert!(!cfg.sdk.claude.enabled);
    assert!(!cfg.sdk.gemini.enabled);
    assert!(!cfg.sdk.codex.enabled);
    assert!(!cfg.sdk.vertex.enabled);

    assert_eq!(cfg.billing.hold_amount, 100);
    assert_eq!(cfg.billing.default_price_per_1k_tokens, 0.001);
    assert_eq!(cfg.billing.hold_ttl_seconds, 3600);
    assert!(!cfg.billing.strict_usage_metadata_mode);
}

#[test]
fn unknown_yaml_keys_are_ignored() {
    // config.example.yaml carries a `frontend:` block with no matching field;
    // the parser drops it silently and so must we. If this ever starts
    // failing, serde has been switched to deny_unknown_fields and every deploy
    // with an extra key would refuse to boot.
    assert!(EXAMPLE_YAML.contains("frontend:"));
    assert!(Config::parse_yaml(EXAMPLE_YAML).is_ok());
}

#[test]
fn runtime_config_parses_and_fills_absent_sections() {
    let cfg = load_str(SPARSE_YAML, &[]);

    assert_eq!(cfg.auth.admin_emails.len(), 1);
    assert_eq!(cfg.auth.jwt.expiry_hours, 24);
    assert_eq!(cfg.auth.jwt.secret.len(), 64);
    assert_eq!(cfg.database.user, "ai-gateway");

    // The fixture has no rate_limit / circuit_breaker section at all: the
    // effective values must equal the rate-limiter / circuit-breaker defaults.
    assert_eq!(cfg.rate_limit.requests_per_min, 60);
    assert_eq!(cfg.rate_limit.tokens_per_min, 100_000);
    assert_eq!(cfg.rate_limit.max_concurrent, 10);
    assert_eq!(cfg.rate_limit.burst_size, 2);
    assert_eq!(cfg.rate_limit.global_request_cap, 10_000);
    assert_eq!(cfg.rate_limit.global_token_cap, 10_000_000);
    assert!(cfg.rate_limit.group_overrides.is_empty());
    assert!(cfg.rate_limit.model_token_limits.is_empty());

    assert_eq!(cfg.circuit_breaker.failure_threshold, 0.5);
    assert_eq!(cfg.circuit_breaker.window_seconds, 30);
    assert_eq!(cfg.circuit_breaker.cooldown_seconds, 30);

    // Billing keys the file omits, filled with their documented defaults.
    assert_eq!(cfg.billing.balance_cache_ttl_seconds, 30);
    assert_eq!(cfg.billing.budget_token_multiplier, 10);
    assert_eq!(cfg.billing.budget_token_ttl_seconds, 60);
    assert_eq!(cfg.billing.low_balance_threshold_usd, 1.0);
    assert_eq!(cfg.billing.price_cache_refresh_seconds, 60);
    // ...and log_level, which defaults to "warn".
    assert_eq!(cfg.server.log_level, "warn");
}

#[test]
fn load_reports_the_path_it_could_not_read() {
    let err = Config::load("/nonexistent/ai-gateway/config.yaml").unwrap_err();
    match err {
        ConfigError::Read { path, .. } => {
            assert_eq!(path, "/nonexistent/ai-gateway/config.yaml");
        }
        other => panic!("expected a Read error, got {other:?}"),
    }
}

#[test]
fn load_accepts_the_shipped_template() {
    // No field assertions: `load` consults the real process environment, so any
    // value could legitimately be overridden by the shell running the tests.
    // What must hold unconditionally is that the checked-in template still
    // parses through the full `load` path, file read included.
    //
    // Only `config.example.yaml` is asserted on. `config.yaml` is gitignored —
    // testing against it would pass here and fail on every fresh clone.
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    Config::load(&format!("{root}/config.example.yaml")).expect("example config loads");
}

#[test]
fn malformed_yaml_is_a_parse_error() {
    let err = Config::parse_yaml("server:\n  port: not-a-number\n").unwrap_err();
    assert!(matches!(err, ConfigError::Parse(_)), "got {err:?}");
}

#[test]
fn empty_document_yields_defaults() {
    // An empty document yields defaults rather than erroring; the Rust side
    // must not turn an empty file into a failure.
    let cfg = Config::parse_yaml("   \n").expect("empty document parses");
    assert_eq!(cfg.server.port, DEFAULT_SERVER_PORT);
}

// ---------------------------------------------------------------------------
// YAML parsing cases
// ---------------------------------------------------------------------------

#[test]
fn yaml_billing_config_new_fields() {
    let cfg = load_str(
        "billing:
  hold_amount: 5
  default_price_per_1k_tokens: 0.03
  hold_ttl_seconds: 300
  balance_cache_ttl_seconds: 45
  budget_token_multiplier: 15
  budget_token_ttl_seconds: 90
  low_balance_threshold_usd: 2.5
",
        &[],
    );

    assert_eq!(cfg.billing.hold_amount, 5);
    assert_eq!(cfg.billing.default_price_per_1k_tokens, 0.03);
    assert_eq!(cfg.billing.hold_ttl_seconds, 300);
    assert_eq!(cfg.billing.balance_cache_ttl_seconds, 45);
    assert_eq!(cfg.billing.budget_token_multiplier, 15);
    assert_eq!(cfg.billing.budget_token_ttl_seconds, 90);
    assert_eq!(cfg.billing.low_balance_threshold_usd, 2.5);
}

#[test]
fn yaml_rate_limit_config() {
    let cfg = load_str(
        r#"rate_limit:
  requests_per_min: 120
  tokens_per_min: 200000
  max_concurrent: 20
  burst_size: 4
  global_request_cap: 50000
  global_token_cap: 5000000
  group_overrides:
    "premium":
      requests_per_min: 300
      tokens_per_min: 500000
      max_concurrent: 50
      burst_size: 10
    "basic":
      requests_per_min: 30
      tokens_per_min: 50000
      max_concurrent: 5
      burst_size: 1
  model_token_limits:
    "claude-opus": 50000
    "gpt-4o": 80000
"#,
        &[],
    );

    let rl = &cfg.rate_limit;
    assert_eq!(rl.requests_per_min, 120);
    assert_eq!(rl.tokens_per_min, 200_000);
    assert_eq!(rl.max_concurrent, 20);
    assert_eq!(rl.burst_size, 4);
    assert_eq!(rl.global_request_cap, 50_000);
    assert_eq!(rl.global_token_cap, 5_000_000);

    assert_eq!(rl.group_overrides.len(), 2);
    let premium = rl.group_overrides.get("premium").expect("premium override");
    assert_eq!(
        premium,
        &RateLimitOverride {
            requests_per_min: 300,
            tokens_per_min: 500_000,
            max_concurrent: 50,
            burst_size: 10,
        }
    );
    assert_eq!(
        rl.group_overrides
            .get("basic")
            .expect("basic override")
            .requests_per_min,
        30
    );

    assert_eq!(rl.model_token_limits.len(), 2);
    assert_eq!(rl.model_token_limits["claude-opus"], 50_000);
    assert_eq!(rl.model_token_limits["gpt-4o"], 80_000);
}

#[test]
fn yaml_circuit_breaker_config() {
    let cfg = load_str(
        "circuit_breaker:
  failure_threshold: 0.7
  window_seconds: 60
  cooldown_seconds: 45
",
        &[],
    );
    assert_eq!(cfg.circuit_breaker.failure_threshold, 0.7);
    assert_eq!(cfg.circuit_breaker.window_seconds, 60);
    assert_eq!(cfg.circuit_breaker.cooldown_seconds, 45);
}

#[test]
fn env_override_server_host_and_port() {
    let cfg = load_str(
        "server:\n  host: 127.0.0.1\n  port: 8888\n",
        &[("SERVER_HOST", "0.0.0.0"), ("SERVER_PORT", "9999")],
    );
    assert_eq!(cfg.server.host, "0.0.0.0");
    assert_eq!(cfg.server.port, 9999);
}

#[test]
fn env_override_billing_new_fields() {
    let cfg = load_str(
        "billing:
  hold_amount: 1
  balance_cache_ttl_seconds: 10
  budget_token_multiplier: 5
  budget_token_ttl_seconds: 30
  low_balance_threshold_usd: 0.5
",
        &[
            ("BILLING_BALANCE_CACHE_TTL_SECONDS", "60"),
            ("BILLING_BUDGET_TOKEN_MULTIPLIER", "20"),
            ("BILLING_BUDGET_TOKEN_TTL_SECONDS", "120"),
            ("BILLING_LOW_BALANCE_THRESHOLD_USD", "5.0"),
        ],
    );
    assert_eq!(cfg.billing.balance_cache_ttl_seconds, 60);
    assert_eq!(cfg.billing.budget_token_multiplier, 20);
    assert_eq!(cfg.billing.budget_token_ttl_seconds, 120);
    assert_eq!(cfg.billing.low_balance_threshold_usd, 5.0);
    assert_eq!(cfg.billing.hold_amount, 1);
}

#[test]
fn env_override_rate_limit_fields() {
    let cfg = load_str(
        "rate_limit:
  requests_per_min: 60
  tokens_per_min: 100000
",
        &[
            ("RATE_LIMIT_REQUESTS_PER_MIN", "200"),
            ("RATE_LIMIT_TOKENS_PER_MIN", "500000"),
            ("RATE_LIMIT_MAX_CONCURRENT", "50"),
            ("RATE_LIMIT_BURST_SIZE", "8"),
            ("RATE_LIMIT_GLOBAL_REQUEST_CAP", "99999"),
            ("RATE_LIMIT_GLOBAL_TOKEN_CAP", "8888888"),
        ],
    );
    let rl = &cfg.rate_limit;
    assert_eq!(rl.requests_per_min, 200);
    assert_eq!(rl.tokens_per_min, 500_000);
    assert_eq!(rl.max_concurrent, 50);
    assert_eq!(rl.burst_size, 8);
    assert_eq!(rl.global_request_cap, 99_999);
    assert_eq!(rl.global_token_cap, 8_888_888);
}

#[test]
fn env_override_circuit_breaker_fields() {
    let cfg = load_str(
        "circuit_breaker:\n  failure_threshold: 0.5\n",
        &[
            ("CIRCUIT_BREAKER_FAILURE_THRESHOLD", "0.8"),
            ("CIRCUIT_BREAKER_WINDOW_SECONDS", "120"),
            ("CIRCUIT_BREAKER_COOLDOWN_SECONDS", "90"),
        ],
    );
    assert_eq!(cfg.circuit_breaker.failure_threshold, 0.8);
    assert_eq!(cfg.circuit_breaker.window_seconds, 120);
    assert_eq!(cfg.circuit_breaker.cooldown_seconds, 90);
}

#[test]
fn strict_usage_metadata_mode_env_table() {
    // Both directions, including the values Rust's own bool parser rejects.
    let absent = load_str("billing:\n  hold_amount: 1\n", &[]);
    assert!(!absent.billing.strict_usage_metadata_mode);

    for (value, want) in [
        ("true", true),
        ("1", true),
        ("false", false),
        ("0", false),
        ("invalid", false), // ParseBool fails -> YAML value preserved
        ("", false),        // empty env -> override branch skipped
    ] {
        let cfg = load_str(
            "billing:\n  hold_amount: 1\n",
            &[("BILLING_STRICT_USAGE_METADATA_MODE", value)],
        );
        assert_eq!(
            cfg.billing.strict_usage_metadata_mode, want,
            "env={value:?} against a false baseline"
        );
    }

    for (value, want) in [
        ("false", false),
        ("0", false),
        ("invalid", true), // YAML value preserved
        ("", true),        // YAML value preserved
    ] {
        let cfg = load_str(
            "billing:\n  strict_usage_metadata_mode: true\n",
            &[("BILLING_STRICT_USAGE_METADATA_MODE", value)],
        );
        assert_eq!(
            cfg.billing.strict_usage_metadata_mode, want,
            "env={value:?} against a true baseline"
        );
    }
}

// ---------------------------------------------------------------------------
// Env-override semantics
// ---------------------------------------------------------------------------

#[test]
fn every_documented_env_var_reaches_its_field() {
    // One pass over the whole table: a typo'd key or a copy-pasted assignment
    // in env.rs shows up as exactly one failed field here.
    let cfg = load_str(
        "",
        &[
            ("SERVER_HOST", "0.0.0.0"),
            ("SERVER_PORT", "9001"),
            ("LOG_LEVEL", "info"),
            ("DB_HOST", "db.internal"),
            ("DB_PORT", "6543"),
            ("DB_USER", "gw"),
            ("DB_PASSWORD", "s3cret"),
            ("DB_NAME", "gwdb"),
            ("DB_SSLMODE", "require"),
            ("DB_MAX_IDLE_CONNS", "7"),
            ("DB_MAX_OPEN_CONNS", "77"),
            ("DB_CONN_MAX_LIFETIME_MINUTES", "17"),
            ("REDIS_ADDR", "redis.internal:6379"),
            ("REDIS_PASSWORD", "rpw"),
            ("REDIS_DB", "3"),
            ("JWT_SECRET", "jwt-secret"),
            ("JWT_EXPIRY_HOURS", "48"),
            ("CREDENTIAL_ENCRYPTION_KEY", "cek"),
            ("ADMIN_EMAILS", "a@example.com, b@example.com"),
            ("BOOTSTRAP_ADMIN_EMAIL", "  Root@Example.COM "),
            ("SDK_BASE_URL", "https://upstream.example"),
            ("SDK_API_KEY", "sk-test"),
            ("SDK_TIMEOUT_SECONDS", "45"),
            ("BILLING_HOLD_AMOUNT", "9"),
            ("BILLING_DEFAULT_PRICE_PER_1K_TOKENS", "0.02"),
            ("BILLING_HOLD_TTL_SECONDS", "600"),
            ("BILLING_PRICE_CACHE_REFRESH_SECONDS", "15"),
        ],
    );

    assert_eq!(cfg.server.host, "0.0.0.0");
    assert_eq!(cfg.server.port, 9001);
    assert_eq!(cfg.server.log_level, "info");
    assert_eq!(cfg.database.host, "db.internal");
    assert_eq!(cfg.database.port, 6543);
    assert_eq!(cfg.database.user, "gw");
    assert_eq!(cfg.database.password, "s3cret");
    assert_eq!(cfg.database.dbname, "gwdb");
    assert_eq!(cfg.database.sslmode, "require");
    assert_eq!(cfg.database.max_idle_conns, 7);
    assert_eq!(cfg.database.max_open_conns, 77);
    assert_eq!(cfg.database.conn_max_lifetime_minutes, 17);
    assert_eq!(cfg.redis.addr, "redis.internal:6379");
    assert_eq!(cfg.redis.password, "rpw");
    assert_eq!(cfg.redis.db, 3);
    assert_eq!(cfg.auth.jwt.secret, "jwt-secret");
    assert_eq!(cfg.auth.jwt.expiry_hours, 48);
    assert_eq!(cfg.auth.credential_encryption_key, "cek");
    assert_eq!(cfg.auth.admin_emails, ["a@example.com", "b@example.com"]);
    // BOOTSTRAP_ADMIN_EMAIL is trimmed + lowercased so it can be compared
    // against a stored email verbatim.
    assert_eq!(cfg.auth.bootstrap_admin_email, "root@example.com");
    assert_eq!(cfg.sdk.base_url, "https://upstream.example");
    assert_eq!(cfg.sdk.api_key, "sk-test");
    assert_eq!(cfg.sdk.timeout_seconds, 45);
    assert_eq!(cfg.billing.hold_amount, 9);
    assert_eq!(cfg.billing.default_price_per_1k_tokens, 0.02);
    assert_eq!(cfg.billing.hold_ttl_seconds, 600);
    assert_eq!(cfg.billing.price_cache_refresh_seconds, 15);
}

#[test]
fn unparseable_numeric_env_leaves_the_yaml_value() {
    let cfg = load_str(
        "server:\n  port: 8888\nbilling:\n  hold_amount: 42\n",
        &[
            ("SERVER_PORT", "not-a-port"),
            ("BILLING_HOLD_AMOUNT", "12.5"),
            ("BILLING_DEFAULT_PRICE_PER_1K_TOKENS", "cheap"),
        ],
    );
    assert_eq!(cfg.server.port, 8888);
    assert_eq!(cfg.billing.hold_amount, 42);
    assert_eq!(cfg.billing.default_price_per_1k_tokens, 0.0);
}

#[test]
fn empty_env_value_is_treated_as_unset() {
    let cfg = load_str(
        "server:\n  host: 127.0.0.1\n",
        &[("SERVER_HOST", ""), ("DB_SSLMODE", "")],
    );
    assert_eq!(cfg.server.host, "127.0.0.1");
    assert_eq!(cfg.database.sslmode, "");
}

#[test]
fn admin_emails_drops_blank_entries() {
    let cfg = load_str("", &[("ADMIN_EMAILS", " a@x.com ,, ,b@x.com,")]);
    assert_eq!(cfg.auth.admin_emails, ["a@x.com", "b@x.com"]);
}

#[test]
fn parse_loose_bool_accepts_extended_spellings() {
    for value in ["1", "t", "T", "TRUE", "true", "True"] {
        assert_eq!(parse_loose_bool(value), Some(true), "{value:?}");
    }
    for value in ["0", "f", "F", "FALSE", "false", "False"] {
        assert_eq!(parse_loose_bool(value), Some(false), "{value:?}");
    }
    for value in ["", "yes", "no", "TrUe", "2", " true"] {
        assert_eq!(parse_loose_bool(value), None, "{value:?}");
    }
}

// ---------------------------------------------------------------------------
// normalize()
// ---------------------------------------------------------------------------

#[test]
fn normalize_replaces_non_positive_clamped_knobs() {
    let cfg = load_str(
        "server:
  host: ''
  port: 0
  log_level: ''
rate_limit:
  requests_per_min: 0
  tokens_per_min: -1
  max_concurrent: 0
  burst_size: 0
  global_request_cap: 0
  global_token_cap: 0
circuit_breaker:
  failure_threshold: 0
  window_seconds: 0
  cooldown_seconds: -5
billing:
  hold_ttl_seconds: 0
  balance_cache_ttl_seconds: 0
  price_cache_refresh_seconds: 0
",
        &[],
    );

    assert_eq!(cfg.server.host, "127.0.0.1");
    assert_eq!(cfg.server.port, 8888);
    assert_eq!(cfg.server.log_level, "warn");
    assert_eq!(cfg.rate_limit.requests_per_min, 60);
    assert_eq!(cfg.rate_limit.tokens_per_min, 100_000);
    assert_eq!(cfg.rate_limit.max_concurrent, 10);
    assert_eq!(cfg.rate_limit.burst_size, 2);
    assert_eq!(cfg.rate_limit.global_request_cap, 10_000);
    assert_eq!(cfg.rate_limit.global_token_cap, 10_000_000);
    assert_eq!(cfg.circuit_breaker.failure_threshold, 0.5);
    assert_eq!(cfg.circuit_breaker.window_seconds, 30);
    assert_eq!(cfg.circuit_breaker.cooldown_seconds, 30);
    assert_eq!(cfg.billing.hold_ttl_seconds, 300);
    assert_eq!(cfg.billing.balance_cache_ttl_seconds, 30);
    assert_eq!(cfg.billing.price_cache_refresh_seconds, 60);
}

#[test]
fn normalize_keeps_unclamped_values() {
    // These four have no `<= 0 -> default` fallback, so an operator writing
    // 0 means 0 (e.g. "never warn about a low balance").
    let cfg = load_str(
        "billing:
  hold_amount: 0
  default_price_per_1k_tokens: 0
  low_balance_threshold_usd: 0
  budget_token_multiplier: 0
  budget_token_ttl_seconds: 0
database:
  max_idle_conns: 0
",
        &[],
    );
    assert_eq!(cfg.billing.hold_amount, 0);
    assert_eq!(cfg.billing.default_price_per_1k_tokens, 0.0);
    assert_eq!(cfg.billing.low_balance_threshold_usd, 0.0);
    assert_eq!(cfg.billing.budget_token_multiplier, 0);
    assert_eq!(cfg.billing.budget_token_ttl_seconds, 0);
    assert_eq!(cfg.database.max_idle_conns, 0);
}

#[test]
fn normalize_is_idempotent() {
    let mut once = load_str(EXAMPLE_YAML, &[]);
    let snapshot = format!("{once:?}");
    once.normalize();
    assert_eq!(format!("{once:?}"), snapshot);
}

// ---------------------------------------------------------------------------
// Derived accessors
// ---------------------------------------------------------------------------

#[test]
fn dsn_matches_the_libpq_key_value_layout() {
    let cfg = load_str(EXAMPLE_YAML, &[]);
    assert_eq!(
        cfg.database.dsn(),
        "host=127.0.0.1 port=5432 user=ai_gateway password= dbname=ai_gateway sslmode=disable"
    );
}

#[test]
fn billing_durations_apply_consumer_defaults() {
    let cfg = load_str("billing:\n  hold_ttl_seconds: 3600\n", &[]);
    assert_eq!(cfg.billing.hold_ttl(), Duration::from_secs(3600));

    // A hand-built config never went through normalize(); the accessors must
    // still apply the same defaulting consumers rely on.
    let raw = BillingConfig {
        hold_ttl_seconds: 0,
        balance_cache_ttl_seconds: 0,
        price_cache_refresh_seconds: -1,
        ..BillingConfig::default()
    };
    assert_eq!(raw.hold_ttl(), Duration::from_secs(300));
    assert_eq!(raw.balance_cache_ttl(), Duration::from_secs(30));
    assert_eq!(raw.price_cache_refresh(), Duration::from_secs(60));
}

#[test]
fn default_price_converts_per_1k_to_per_1m() {
    let cfg = load_str("billing:\n  default_price_per_1k_tokens: 0.001\n", &[]);
    assert_eq!(cfg.billing.default_price_per_1m_tokens(), 1.0);
}

#[test]
fn openai_provider_falls_back_to_the_legacy_top_level_fields() {
    // Neither nested block configured -> inherit sdk.base_url / sdk.api_key and
    // auto-enable, which is what makes the pre-nesting config keep working.
    let cfg = load_str(
        "sdk:
  base_url: https://legacy.example
  api_key: sk-legacy
",
        &[],
    );
    let provider = cfg.sdk.openai_provider_config();
    assert_eq!(provider.base_url, "https://legacy.example");
    assert_eq!(provider.api_key, "sk-legacy");
    assert!(provider.enabled);
    assert!(provider.complete());
}

#[test]
fn openai_compatible_wins_when_openai_is_unconfigured() {
    let cfg = load_str(
        "sdk:
  base_url: https://legacy.example
  api_key: sk-legacy
  openai_compatible:
    enabled: true
    base_url: https://compat.example
    api_key: sk-compat
",
        &[],
    );
    let provider = cfg.sdk.openai_provider_config();
    assert_eq!(provider.base_url, "https://compat.example");
    assert_eq!(provider.api_key, "sk-compat");
    assert!(provider.enabled);
}

#[test]
fn explicit_openai_block_is_not_replaced_by_the_compatible_one() {
    let cfg = load_str(
        "sdk:
  openai:
    enabled: true
    base_url: https://primary.example
    api_key: sk-primary
  openai_compatible:
    enabled: true
    base_url: https://compat.example
    api_key: sk-compat
",
        &[],
    );
    assert_eq!(
        cfg.sdk.openai_provider_config().base_url,
        "https://primary.example"
    );
}

#[test]
fn provider_configured_and_complete_disagree_on_partial_setups() {
    let partial = SdkProviderConfig {
        base_url: "https://x.example".to_owned(),
        api_key: String::new(),
        enabled: true,
    };
    assert!(partial.configured());
    assert!(
        !partial.complete(),
        "an api-key-less provider is not usable"
    );

    let blank = SdkProviderConfig {
        base_url: "   ".to_owned(),
        ..SdkProviderConfig::default()
    };
    assert!(!blank.configured(), "whitespace is not configuration");
}

#[test]
fn sdk_timeout_is_absent_when_unset() {
    let cfg = load_str(EXAMPLE_YAML, &[]);
    assert_eq!(cfg.sdk.timeout(), Some(Duration::from_secs(30)));
    assert_eq!(SdkConfig::default().timeout(), None);
}

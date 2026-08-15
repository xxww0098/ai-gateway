use super::*;

/// A non-positive knob must fall back to the gateway default, a positive one
/// must be honoured verbatim.
#[test]
fn non_positive_pool_knobs_fall_back_to_defaults() {
    let zeroed = DbSettings {
        max_idle_conns: 0,
        max_open_conns: 0,
        conn_max_lifetime_minutes: 0,
        ..DbSettings::default()
    };
    let sizing = zeroed.pool_sizing();
    assert_eq!(sizing.idle, DEFAULT_MAX_IDLE_CONNS);
    assert_eq!(sizing.open, DEFAULT_MAX_OPEN_CONNS);
    assert_eq!(
        sizing.lifetime,
        Duration::from_secs(u64::from(DEFAULT_CONN_MAX_LIFETIME_MINUTES) * 60)
    );

    let negative = DbSettings {
        max_idle_conns: -1,
        max_open_conns: -7,
        conn_max_lifetime_minutes: -30,
        ..DbSettings::default()
    };
    assert_eq!(negative.pool_sizing(), sizing);

    let explicit = DbSettings {
        max_idle_conns: 3,
        max_open_conns: 9,
        conn_max_lifetime_minutes: 2,
        ..DbSettings::default()
    };
    let sizing = explicit.pool_sizing();
    assert_eq!(sizing.idle, 3);
    assert_eq!(sizing.open, 9);
    assert_eq!(sizing.lifetime, Duration::from_secs(120));
}

/// sqlx panics when `min_connections > max_connections`, so the idle floor is
/// clamped to the open ceiling rather than passed through.
#[test]
fn idle_floor_never_exceeds_open_ceiling() {
    let lopsided = DbSettings {
        max_idle_conns: 500,
        max_open_conns: 4,
        ..DbSettings::default()
    };
    let sizing = lopsided.pool_sizing();
    assert!(
        sizing.idle <= sizing.open,
        "idle={} must not exceed open={}",
        sizing.idle,
        sizing.open
    );
}

/// Unknown and empty log levels are not errors — they degrade to the quiet
/// default, which is what keeps a typo in `config.yaml` from flooding the hot
/// path.
#[test]
fn unknown_log_level_degrades_to_the_default() {
    assert_eq!(SqlLogLevel::parse(""), SqlLogLevel::default());
    assert_eq!(SqlLogLevel::parse("   "), SqlLogLevel::default());
    assert_eq!(SqlLogLevel::parse("verbose"), SqlLogLevel::default());
}

/// Parsing is whitespace- and case-insensitive, and every level round-trips
/// through its canonical spelling.
#[test]
fn log_levels_round_trip_case_and_whitespace_insensitively() {
    for level in [
        SqlLogLevel::Silent,
        SqlLogLevel::Error,
        SqlLogLevel::Warn,
        SqlLogLevel::Info,
    ] {
        let canonical = level.as_str();
        assert_eq!(SqlLogLevel::parse(canonical), level);
        assert_eq!(SqlLogLevel::parse(&canonical.to_uppercase()), level);
        assert_eq!(SqlLogLevel::parse(&format!("  {canonical}\t")), level);
    }
}

/// An empty `sslmode` is "unset", not a mistake: libpq resolves it to `prefer`.
#[test]
fn empty_ssl_mode_resolves_to_the_libpq_default() {
    // PgSslMode implements neither PartialEq nor Display, so match on it.
    assert!(matches!(parse_ssl_mode("").unwrap(), PgSslMode::Prefer));
    assert!(matches!(parse_ssl_mode("  ").unwrap(), PgSslMode::Prefer));
    assert!(matches!(
        parse_ssl_mode("PREFER").unwrap(),
        PgSslMode::Prefer
    ));
    assert!(matches!(
        parse_ssl_mode(" disable ").unwrap(),
        PgSslMode::Disable
    ));
}

/// A misspelled `sslmode` must be reported against its config field instead of
/// being silently downgraded to an insecure mode.
#[test]
fn unknown_ssl_mode_is_rejected_with_its_field_name() {
    let err = parse_ssl_mode("verify-none").unwrap_err();
    match err {
        InfraError::Invalid { field, .. } => assert_eq!(field, "database.sslmode"),
        other => panic!("expected an Invalid error, got {other:?}"),
    }
}

/// Every settings field must reach the DSN under its libpq keyword. Parsing the
/// rendered string back into pairs tests the mapping without restating the
/// format string.
#[test]
fn dsn_carries_every_field_under_its_libpq_keyword() {
    let settings = DbSettings {
        host: "db.internal".to_owned(),
        port: 6543,
        user: "gateway".to_owned(),
        password: "s3cret".to_owned(),
        dbname: "cpa".to_owned(),
        sslmode: "require".to_owned(),
        ..DbSettings::default()
    };

    let dsn = settings.dsn();
    let pairs: Vec<(&str, &str)> = dsn
        .split(' ')
        .map(|kv| kv.split_once('=').expect("every DSN token is key=value"))
        .collect();

    let get = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| *k == key)
            .unwrap_or_else(|| panic!("DSN is missing the {key} keyword"))
            .1
    };
    assert_eq!(get("host"), settings.host);
    assert_eq!(get("port"), settings.port.to_string());
    assert_eq!(get("user"), settings.user);
    assert_eq!(get("password"), settings.password);
    assert_eq!(get("dbname"), settings.dbname);
    assert_eq!(get("sslmode"), settings.sslmode);
}

/// End-to-end pool bootstrap. Ignored because it needs a live Postgres; run it
/// with `cargo test -p gw-infra -- --ignored` once one is reachable at the
/// `config.example.yaml` coordinates.
#[tokio::test]
#[ignore = "需要本地 PostgreSQL（config.example.yaml 的 127.0.0.1:5432/cpa_gateway）"]
async fn init_db_opens_a_usable_pool() {
    let settings = DbSettings::default();
    let pool = init_db(&settings, SqlLogLevel::Silent).await.expect(
        "PostgreSQL 未就绪：请启动本地 Postgres 并按 config.example.yaml 建好 cpa_gateway 库",
    );

    let one: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("SELECT 1 on a freshly opened pool");
    assert_eq!(one, 1);

    assert!(pool.size() <= settings.pool_sizing().open);
    pool.close().await;
}

/// `gw_config` leaves the pool knobs at 0 on purpose ("keep the consumer's own
/// default"), so lifting an untouched config must land on the gateway defaults
/// rather than on a zero-sized pool.
#[test]
fn an_untouched_config_lifts_to_the_gateway_pool_defaults() {
    let lifted = DbSettings::from(&gw_config::DatabaseConfig::default());
    let sizing = lifted.pool_sizing();
    assert_eq!(sizing.open, DEFAULT_MAX_OPEN_CONNS);
    assert_eq!(sizing.idle, DEFAULT_MAX_IDLE_CONNS);
    assert!(!sizing.lifetime.is_zero());
}

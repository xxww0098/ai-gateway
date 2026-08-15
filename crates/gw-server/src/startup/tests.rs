use super::*;

#[test]
fn an_empty_jwt_secret_boots_with_a_warning() {
    // The shipped config.example.yaml has `secret: ""`; refusing to boot on it
    // would break every dev/CI run.
    assert_eq!(validate_jwt_secret(""), Ok(()));
}

#[test]
fn a_short_jwt_secret_refuses_to_boot() {
    let err = validate_jwt_secret("short").expect_err("5 bytes must be rejected");
    assert_eq!(
        err.to_string(),
        "auth.jwt.secret is too weak: 5 bytes, need at least 32"
    );
}

#[test]
fn jwt_secret_acceptance_flips_exactly_at_the_hmac_key_size() {
    let boundary = "x".repeat(MIN_JWT_SECRET_BYTES);
    assert!(validate_jwt_secret(&boundary).is_ok());
    assert!(validate_jwt_secret(&boundary[..MIN_JWT_SECRET_BYTES - 1]).is_err());
}

#[test]
fn jwt_secret_length_is_measured_in_bytes_not_characters() {
    // The secret length is measured in bytes. A 16-char CJK passphrase is 48 bytes
    // and must pass; treating it as 16 "characters" would reject it.
    let cjk = "密钥".repeat(8);
    assert_eq!(cjk.chars().count(), 16);
    assert!(cjk.len() >= MIN_JWT_SECRET_BYTES);
    assert!(validate_jwt_secret(&cjk).is_ok());
}

#[test]
fn insecure_sslmodes_are_the_empty_and_disabled_ones() {
    for mode in ["", "  ", "disable", "DISABLE", " Disable "] {
        assert!(sslmode_is_insecure(mode), "{mode:?} must warn");
    }
    for mode in ["require", "verify-full", "verify-ca", "prefer"] {
        assert!(!sslmode_is_insecure(mode), "{mode:?} must not warn");
    }
}

#[test]
fn log_filter_maps_the_sql_levels_onto_the_sql_target() {
    // silent/error/info are honoured, and "warn", "" and anything unrecognized
    // all land on warn.
    assert_eq!(log_filter("silent"), "info,sqlx=off");
    assert_eq!(log_filter("error"), "info,sqlx=error");
    assert_eq!(log_filter("info"), "info,sqlx=info");
    for level in ["warn", "", "  ", "chatty", "WARN"] {
        assert_eq!(log_filter(level), "info,sqlx=warn", "{level:?}");
    }
}

#[test]
fn log_filter_never_silences_application_events() {
    // Application logging stays at Info regardless of log_level; losing the
    // startup/billing lines because the SQL logger was turned down would be a
    // silent observability regression.
    for level in ["silent", "error", "warn", "info"] {
        assert!(
            log_filter(level).starts_with("info"),
            "{level:?} must keep app events at info"
        );
    }
}

#[test]
fn log_filter_output_is_a_valid_directive_set() {
    for level in ["silent", "error", "warn", "info", "nonsense"] {
        let filter = log_filter(level);
        assert!(
            tracing_subscriber::EnvFilter::builder()
                .parse(&filter)
                .is_ok(),
            "{filter:?} must parse"
        );
    }
}

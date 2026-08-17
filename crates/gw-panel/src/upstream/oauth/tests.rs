//! Unit tests for the session record itself.
//!
//! The handlers need Postgres and live in the integration suite; [`flow`] and
//! [`exchange`] carry their own tests. What is left here is the stored session
//! shape — specifically, that the PKCE secret is not written into the row when
//! there is none, and that a session cannot be claimed after it expires.

use super::*;

fn session(status: &str, expires_at: DateTime<Utc>) -> SessionRow {
    SessionRow {
        id: 1,
        provider: Provider::Claude.as_str().to_owned(),
        status: Some(status.to_owned()),
        auth_id: None,
        config_data: None,
        created_at: DateTime::UNIX_EPOCH,
        expires_at,
    }
}

#[test]
fn an_expired_pending_session_is_displayed_as_failed() {
    // The sweeper is best-effort, so the list must not show a session the
    // callback would refuse.
    let row = session(STATUS_PENDING, Utc::now() - Duration::seconds(1));
    assert_eq!(row.to_json()["status"], json!(STATUS_FAILED));
}

#[test]
fn a_live_pending_session_is_displayed_as_pending() {
    let row = session(STATUS_PENDING, Utc::now() + Duration::minutes(5));
    assert_eq!(row.to_json()["status"], json!(STATUS_PENDING));
}

#[test]
fn a_terminal_session_keeps_its_status_past_the_expiry() {
    // A completed flow does not become "failed" just because time passed.
    for status in [STATUS_COMPLETED, STATUS_FAILED] {
        let row = session(status, Utc::now() - Duration::days(1));
        assert_eq!(row.to_json()["status"], json!(status));
    }
}

#[test]
fn a_session_with_no_credential_reports_a_null_auth_id() {
    // The console keys off this to decide whether to link to the credential.
    let row = session(STATUS_PENDING, Utc::now() + Duration::minutes(5));
    assert_eq!(row.to_json()["auth_id"], Value::Null);
}

#[test]
fn a_null_status_column_is_not_pending() {
    // A legacy row with no status must not be claimable.
    let mut row = session(STATUS_PENDING, Utc::now() + Duration::minutes(5));
    row.status = None;
    assert_ne!(row.status(), STATUS_PENDING);
}

#[test]
fn the_session_config_never_serialises_an_absent_verifier() {
    // Gemini's row has no PKCE; emitting `"code_verifier": ""` would suggest
    // there is one and that it is empty.
    let encoded = serde_json::to_value(SessionConfig::default()).expect("serializes");
    let object = encoded.as_object().expect("object");
    assert!(!object.contains_key("code_verifier"));
    assert!(!object.contains_key("code_challenge_method"));
    assert!(!object.contains_key("provider_alias"));
    assert!(!object.contains_key("device_code"));
    assert!(!object.contains_key("client_secret"));
    assert!(!object.contains_key("flow"));
}

#[test]
fn a_device_session_round_trips_through_the_config_column() {
    let config = SessionConfig {
        flow: "device".to_owned(),
        device_code: "dc-1".to_owned(),
        user_code: "WDJB-MJHT".to_owned(),
        verification_uri: "https://auth.x.ai/device".to_owned(),
        interval: 5,
        client_id: "public-client".to_owned(),
        client_secret: "s3cret".to_owned(),
        ..SessionConfig::default()
    };
    let encoded = serde_json::to_value(&config).expect("serializes");
    let decoded: SessionConfig = serde_json::from_value(encoded.clone()).expect("round-trips");
    assert_eq!(decoded.device_code, config.device_code);
    assert_eq!(decoded.user_code, config.user_code);
    assert_eq!(decoded.client_secret, config.client_secret);
    assert_eq!(decoded.interval, 5);
    let object = encoded.as_object().expect("object");
    assert!(object.contains_key("device_code"));
    assert!(object.contains_key("client_secret"));
}

#[test]
fn a_stored_verifier_round_trips_through_the_config_column() {
    // If it did not, every Claude and Codex callback would fail with "missing
    // PKCE verifier" after the row was written.
    let mut config = SessionConfig {
        redirect_uri: "https://gw.example.test/cb".to_owned(),
        ..SessionConfig::default()
    };
    build_authorize_url(Provider::Codex, "state-1", &mut config).expect("entropy");

    let encoded = serde_json::to_value(&config).expect("serializes");
    let decoded: SessionConfig = serde_json::from_value(encoded).expect("round-trips");
    assert_eq!(decoded.code_verifier, config.code_verifier);
    assert_eq!(decoded.redirect_uri, config.redirect_uri);
}

#[test]
fn a_missing_config_column_decodes_to_an_empty_config() {
    // A row written by an older binary must not panic the callback; it fails
    // the "incomplete" check instead, which is a 400 the operator can act on.
    let config: SessionConfig = serde_json::from_value(json!({})).expect("tolerates an empty blob");
    assert!(config.redirect_uri.is_empty());
    assert!(config.code_verifier.is_empty());
}

#[test]
fn the_session_ttl_is_long_enough_for_a_browser_round_trip() {
    // Short enough that a leaked state is not useful for long, long enough that
    // an operator can actually log in to the provider.
    assert!(SESSION_TTL >= Duration::minutes(5));
    assert!(SESSION_TTL <= Duration::hours(1));
}

#[test]
fn a_query_parameter_outranks_the_body() {
    // The provider redirects with query parameters; the console posts a body.
    // When both are present the redirect is the authoritative one.
    assert_eq!(
        first_non_empty(Some("from-query"), "from-body"),
        "from-query"
    );
    assert_eq!(first_non_empty(None, "from-body"), "from-body");
    assert_eq!(first_non_empty(Some("  "), "from-body"), "from-body");
    assert!(first_non_empty(None, "  ").is_empty());
}

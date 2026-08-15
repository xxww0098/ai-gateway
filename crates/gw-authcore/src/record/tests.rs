use super::{AuthRecord, AuthStatus, RUNTIME_ONLY_ATTRIBUTE};
use chrono::Utc;

#[test]
fn runtime_only_is_matched_the_way_go_matched_it() {
    let mut record = AuthRecord::new("cpa-gateway-claude", "claude", Utc::now());
    assert!(!record.is_runtime_only(), "no attribute means persistable");

    for spelling in ["true", "TRUE", "True", "  true  "] {
        record.set_attribute(RUNTIME_ONLY_ATTRIBUTE, spelling);
        assert!(
            record.is_runtime_only(),
            "{spelling:?} must keep a config credential out of the database"
        );
    }

    for spelling in ["false", "", "1", "yes"] {
        record.set_attribute(RUNTIME_ONLY_ATTRIBUTE, spelling);
        assert!(!record.is_runtime_only(), "{spelling:?} is not true");
    }
}

#[test]
fn an_empty_status_column_reads_back_as_active() {
    assert_eq!(AuthStatus::from(""), AuthStatus::Active);
    assert_eq!(AuthStatus::from("active"), AuthStatus::Active);
    assert_eq!(AuthStatus::default(), AuthStatus::Active);
}

#[test]
fn an_unknown_status_survives_the_round_trip_verbatim() {
    let status = AuthStatus::from("pending_oauth");

    assert_eq!(status.as_str(), "pending_oauth");
    assert_eq!(
        AuthStatus::from(status.as_str()),
        status,
        "rewriting the row must not silently rename an operator-visible state"
    );
}

#[test]
fn status_serialises_as_a_bare_string() {
    for raw in ["active", "disabled", "error", "quota_exhausted"] {
        let status = AuthStatus::from(raw);
        let encoded = serde_json::to_value(&status).expect("status serialises");

        assert_eq!(encoded, serde_json::Value::String(raw.to_owned()));
        assert_eq!(
            serde_json::from_value::<AuthStatus>(encoded).expect("status deserialises"),
            status
        );
    }
}

#[test]
fn a_credential_is_unusable_when_any_kill_switch_is_set() {
    let now = Utc::now();
    let healthy = AuthRecord::new("id", "claude", now);
    assert!(healthy.is_usable());

    let mut operator_disabled = healthy.clone();
    operator_disabled.disabled = true;
    assert!(!operator_disabled.is_usable());

    let mut health_disabled = healthy.clone();
    health_disabled.unavailable = true;
    assert!(!health_disabled.is_usable());

    let mut status_disabled = healthy.clone();
    status_disabled.status = AuthStatus::Disabled;
    assert!(!status_disabled.is_usable());

    let mut errored = healthy;
    errored.status = AuthStatus::Error;
    assert!(
        errored.is_usable(),
        "an errored credential is still routable — the proxy retries it"
    );
}

#[test]
fn a_fresh_record_is_active_and_stamped() {
    let now = Utc::now();
    let record = AuthRecord::new("cpa-gateway-codex", "codex", now);

    assert_eq!(record.status, AuthStatus::Active);
    assert_eq!(record.created_at, now);
    assert_eq!(record.updated_at, now);
    assert!(record.last_error.is_none());
    assert!(record.metadata.is_object());
}

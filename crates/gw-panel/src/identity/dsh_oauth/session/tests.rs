use chrono::{Duration, TimeZone, Utc};

use super::{
    DeviceStatus, PollOutcome, TransitionError, approve, deny, normalize_user_code, poll, start,
};

fn t0() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 17, 7, 0, 0).unwrap()
}

#[test]
fn start_is_pending_and_poll_stays_pending_before_expiry() {
    let ttl = Duration::seconds(30);
    let session = start(t0(), ttl, "https://gw.example".to_owned());
    assert_eq!(session.status, DeviceStatus::Pending);
    assert_eq!(session.expires_at, t0() + ttl);
    assert_eq!(poll(&session, t0()), PollOutcome::Pending);
    assert_eq!(
        poll(&session, t0() + ttl - Duration::seconds(1)),
        PollOutcome::Pending
    );
}

#[test]
fn pending_becomes_expired_at_the_deadline() {
    let ttl = Duration::seconds(10);
    let session = start(t0(), ttl, "https://gw.example".to_owned());
    assert_eq!(poll(&session, t0() + ttl), PollOutcome::Expired);
    assert_eq!(
        approve(&session, t0() + ttl, 7, "agw-key".to_owned()),
        Err(TransitionError::Expired)
    );
}

#[test]
fn approve_then_poll_returns_the_same_key_and_origin() {
    let session = start(t0(), Duration::seconds(60), "https://gw.example".to_owned());
    let key = "agw-issued-for-this-test";
    let approved = approve(&session, t0() + Duration::seconds(1), 42, key.to_owned()).unwrap();
    assert_eq!(approved.status, DeviceStatus::Approved);
    assert_eq!(approved.user_id, Some(42));
    assert_eq!(
        poll(&approved, t0() + Duration::seconds(2)),
        PollOutcome::Approved {
            api_key: key.to_owned(),
            origin: "https://gw.example".to_owned(),
        }
    );
}

#[test]
fn deny_then_poll_is_denied_and_cannot_be_approved() {
    let session = start(t0(), Duration::seconds(60), "https://gw.example".to_owned());
    let denied = deny(&session, t0() + Duration::seconds(1)).unwrap();
    assert_eq!(poll(&denied, t0() + Duration::seconds(2)), PollOutcome::Denied);
    assert_eq!(
        approve(&denied, t0() + Duration::seconds(3), 1, "agw-x".to_owned()),
        Err(TransitionError::AlreadyResolved)
    );
}

#[test]
fn approve_is_rejected_once_the_session_is_already_approved() {
    let session = start(t0(), Duration::seconds(60), "https://gw.example".to_owned());
    let approved = approve(&session, t0(), 1, "agw-one".to_owned()).unwrap();
    assert_eq!(
        approve(&approved, t0() + Duration::seconds(1), 2, "agw-two".to_owned()),
        Err(TransitionError::AlreadyResolved)
    );
}

#[test]
fn approved_grant_survives_expiry_so_a_late_poll_still_collects_the_key() {
    let ttl = Duration::seconds(5);
    let session = start(t0(), ttl, "https://gw.example".to_owned());
    let approved = approve(&session, t0() + Duration::seconds(1), 9, "agw-late".to_owned()).unwrap();
    assert_eq!(
        poll(&approved, t0() + ttl + Duration::seconds(30)),
        PollOutcome::Approved {
            api_key: "agw-late".to_owned(),
            origin: "https://gw.example".to_owned(),
        }
    );
}

#[test]
fn user_codes_compare_without_dashes_or_case() {
    let session = start(t0(), Duration::seconds(60), "https://gw.example".to_owned());
    assert_eq!(
        normalize_user_code(&session.user_code),
        normalize_user_code(&session.user_code.to_lowercase().replace('-', ""))
    );
    assert_ne!(normalize_user_code(&session.user_code).len(), 0);
    assert!(session.user_code.contains('-'));
}

#[test]
fn each_start_mints_distinct_device_and_user_codes() {
    let a = start(t0(), Duration::seconds(60), "https://a".to_owned());
    let b = start(t0(), Duration::seconds(60), "https://b".to_owned());
    assert_ne!(a.device_code, b.device_code);
    assert_ne!(a.user_code, b.user_code);
}

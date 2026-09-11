use chrono::DateTime;
use serde_json::json;

use super::{consume_reset_body, is_available_reset_credit, parse_reset_credits, parse_usage};

#[test]
fn usage_maps_five_hour_and_weekly_remaining() {
    let used_primary = 28;
    let used_weekly = 46;
    let parsed = parse_usage(&json!({
        "plan_type": "plus",
        "rate_limit": {
            "primary_window": {
                "used_percent": used_primary,
                "limit_window_seconds": 18_000,
                "reset_after_seconds": 3_600,
            },
            "secondary_window": {
                "used_percent": used_weekly,
                "limit_window_seconds": 604_800,
                "reset_at": 1_770_000_000,
            },
        },
    }));
    assert_eq!(parsed.plan_type.as_deref(), Some("plus"));
    assert_eq!(parsed.rows[0].kind, "primary");
    assert_eq!(
        parsed.rows[0].used_percent + parsed.rows[0].remaining_percent,
        100
    );
    assert_eq!(parsed.rows[0].used_percent, used_primary);
    assert_eq!(parsed.rows[0].window_minutes, Some(5 * 60));
    assert!(
        parsed.rows[0]
            .reset_at_ms
            .is_some_and(|stamp| stamp > chrono::Utc::now().timestamp_millis())
    );
    assert_eq!(parsed.rows[1].kind, "weekly");
    assert_eq!(parsed.rows[1].used_percent, used_weekly);
    assert_eq!(parsed.rows[1].remaining_percent, 100 - used_weekly);
    assert_eq!(parsed.rows[1].reset_at_ms, Some(1_770_000_000_000));
}

#[test]
fn usage_reads_resets_at_and_seconds_until_reset_aliases() {
    let iso = "2099-01-02T00:00:00Z";
    let parsed = parse_usage(&json!({
        "rate_limit": {
            "primary_window": { "used_percent": 10, "resets_at": iso },
            "secondary_window": { "used_percent": 20, "seconds_until_reset": 86_400 },
        },
    }));
    let expected = DateTime::parse_from_rfc3339(iso)
        .expect("rfc3339")
        .timestamp_millis();
    assert_eq!(parsed.rows[0].reset_at_ms, Some(expected));
    let now = chrono::Utc::now().timestamp_millis();
    assert!(
        parsed.rows[1]
            .reset_at_ms
            .is_some_and(|stamp| stamp > now + 80_000_000)
    );
}

#[test]
fn reset_credits_skip_redeemed_and_read_available_count() {
    let parsed = parse_reset_credits(&json!({
        "available_count": 1,
        "credits": [
            {
                "id": "credit-1",
                "status": "available",
                "expires_at": "2099-06-25T08:30:00Z",
            },
            {
                "id": "credit-2",
                "status": "redeemed",
                "expires_at": 1_782_451_200,
            },
        ],
    }));
    assert_eq!(parsed.available_count, 1);
    assert_eq!(parsed.credits.len(), 2);
    assert!(is_available_reset_credit(&parsed.credits[0]));
    assert!(!is_available_reset_credit(&parsed.credits[1]));
    let expected = DateTime::parse_from_rfc3339("2099-06-25T08:30:00Z")
        .expect("rfc3339")
        .timestamp_millis();
    assert_eq!(parsed.next_expires_at_ms, Some(expected));
}

#[test]
fn reset_credits_use_payload_expiry_when_rows_omit_it() {
    let iso = "2099-08-01T12:00:00Z";
    let parsed = parse_reset_credits(&json!({
        "available_count": 1,
        "expires_at": iso,
    }));
    assert_eq!(parsed.available_count, 1);
    let expected = DateTime::parse_from_rfc3339(iso)
        .expect("rfc3339")
        .timestamp_millis();
    assert_eq!(parsed.next_expires_at_ms, Some(expected));
}

#[test]
fn reset_credits_derive_count_and_mark_past_rows_expired() {
    let future = chrono::Utc::now().timestamp() + 3600;
    let past = chrono::Utc::now().timestamp() - 3600;
    let parsed = parse_reset_credits(&json!({
        "credits": [
            { "id": "available", "expires_at": future },
            { "id": "expired", "expires_at": past },
            { "id": "used", "status": "used", "expires_at": future },
        ],
    }));
    assert_eq!(parsed.available_count, 1);
    assert_eq!(parsed.credits[1].status.as_deref(), Some("expired"));
    assert_eq!(parsed.next_expires_at_ms, Some(future * 1000));
}

#[test]
fn consume_body_pairs_redeem_id_with_idempotency_key() {
    let id = "req-1";
    let body = consume_reset_body(id);
    assert_eq!(body["redeem_request_id"], id);
    assert_eq!(body["idempotencyKey"], id);
}

#[test]
fn non_object_usage_is_empty() {
    let parsed = parse_usage(&json!([1, 2, 3]));
    assert!(parsed.rows.is_empty());
    assert!(parsed.plan_type.is_none());
}

//! Properties of Grok billing JSON + credits-frame merge.
//!
//! Prepaid 0 and product shells with no numbers must not become card rows.
//! A JSON percent wins over the gRPC snapshot.

use serde_json::json;

use super::{QuotaUrls, apply_credits_snapshot, fetch_quota_at, parse_billing};
use crate::Session;
use crate::grok::credits::CreditsSnapshot;
use crate::grok::test_support::{self, Reply};

#[test]
fn billing_reads_weekly_credits_products_prepaid_and_plan() {
    let parsed = parse_billing(
        &json!({
            "config": {
                "subscription_tier": "SuperGrok",
                "creditUsagePercent": 32,
                "currentPeriod": {
                    "type": "weekly",
                    "start": "2026-08-18T00:00:00Z",
                    "end": "2026-08-25T00:00:00Z"
                },
                "weeklyCredits": {"used": 160, "total": 500},
                "prepaidBalance": 12.5,
                "productUsage": [
                    {"product": "API", "usagePercent": 40, "used": 80, "total": 200, "remaining": 120}
                ]
            }
        }),
        Some(&json!({
            "hasGrokCodeAccess": true,
            "subscription": {"tier": "SuperGrok", "status": "active"}
        })),
    );
    assert_eq!(parsed.plan_type, "SuperGrok");
    assert_eq!(parsed.has_grok_code_access, Some(true));
    assert_eq!(parsed.rows[0].kind, "weekly");
    assert_eq!(parsed.rows[0].remaining_percent, Some(68));
    assert_eq!(parsed.rows[0].used, Some(160.0));
    assert_eq!(parsed.rows[0].total, Some(500.0));
    assert_eq!(parsed.rows[1].kind, "prepaid");
    assert_eq!(parsed.rows[1].remaining, Some(12.5));
    assert_eq!(parsed.rows[2].product, "API");
    assert_eq!(parsed.rows[2].remaining_percent, Some(60));
}

#[test]
fn supergrok_pro_user_enum_is_heavy() {
    let parsed = parse_billing(
        &json!({"config": {"creditUsagePercent": 45}}),
        Some(&json!({"subscriptionTier": "SuperGrokPro", "hasGrokCodeAccess": true})),
    );
    assert_eq!(parsed.plan_type, "SuperGrok Heavy");
}

#[test]
fn numeric_subscription_tier_matches_jwt_names() {
    let plus = parse_billing(
        &json!({"config": {"subscription_tier": 4, "creditUsagePercent": 10}}),
        None,
    );
    assert_eq!(plus.plan_type, "X Premium+");
    let free = parse_billing(
        &json!({"config": {"subscription_tier": 0, "creditUsagePercent": 0}}),
        None,
    );
    assert_eq!(free.plan_type, "Free");
}

#[test]
fn unified_billing_hides_empty_prepaid_and_product_shells() {
    let parsed = parse_billing(
        &json!({
            "config": {
                "isUnifiedBillingUser": true,
                "prepaidBalance": 0,
                "productUsage": [{"product": "Grok Code"}],
                "currentPeriod": {"type": "USAGE_PERIOD_TYPE_WEEKLY", "end": "2026-09-05T00:00:00Z"}
            }
        }),
        None,
    );
    assert!(parsed.rows.is_empty(), "{:?}", parsed.rows);
}

#[test]
fn on_demand_used_cap_fills_a_weekly_bar() {
    let parsed = parse_billing(
        &json!({
            "config": {
                "subscription_tier": "SuperGrok Heavy",
                "onDemandUsed": {"val": 25},
                "onDemandCap": {"val": 100}
            }
        }),
        None,
    );
    assert_eq!(parsed.rows[0].kind, "weekly");
    assert_eq!(parsed.rows[0].used_percent, Some(25));
    assert_eq!(parsed.rows[0].remaining_percent, Some(75));
}

#[test]
fn snapshot_fills_weekly_usage_when_json_omitted_the_percent() {
    let parsed = parse_billing(
        &json!({
            "config": {
                "isUnifiedBillingUser": true,
                "prepaidBalance": 0,
                "currentPeriod": {"type": "USAGE_PERIOD_TYPE_WEEKLY", "end": "2026-09-05T00:00:00Z"}
            }
        }),
        None,
    );
    let merged = apply_credits_snapshot(
        parsed,
        Some(&CreditsSnapshot {
            used_percent: Some(37),
            reset_at_ms: Some(1_788_307_200_000),
        }),
    );
    assert_eq!(merged.rows[0].kind, "weekly");
    assert_eq!(merged.rows[0].used_percent, Some(37));
    assert_eq!(merged.rows[0].remaining_percent, Some(63));
}

#[test]
fn snapshot_does_not_override_a_json_percent() {
    let parsed = parse_billing(&json!({"config": {"creditUsagePercent": 10}}), None);
    let merged = apply_credits_snapshot(
        parsed,
        Some(&CreditsSnapshot {
            used_percent: Some(90),
            reset_at_ms: None,
        }),
    );
    assert_eq!(merged.rows[0].used_percent, Some(10));
}

#[tokio::test]
async fn user_probe_404_does_not_fail_the_card() {
    let server = test_support::spawn(3, |req| {
        if req.path.contains("billing") {
            return Reply::json(
                200,
                r#"{"config":{"subscription_tier":"X Premium+","creditUsagePercent":81}}"#,
            );
        }
        Reply::json(404, "nope")
    });
    let client = test_support::client();
    let access = {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let body = URL_SAFE_NO_PAD.encode(br#"{"sub":"user-1"}"#);
        format!("{header}.{body}.x")
    };
    let session = Session::new(crate::Family::Grok, access, "rt");
    let quota = fetch_quota_at(
        &session,
        &client,
        QuotaUrls {
            billing: &format!("{}/billing", server.base),
            user: &format!("{}/user", server.base),
            credits: &format!("{}/credits", server.base),
        },
    )
    .await
    .expect("quota");
    assert_eq!(quota.plan_type, "X Premium+");
    assert_eq!(quota.rows[0].remaining_percent, Some(19));
    let seen = server.seen.lock().unwrap_or_else(|p| p.into_inner());
    assert!(
        seen.iter()
            .any(|r| r.path.contains("billing") && r.method == "GET")
    );
    assert!(
        seen.iter()
            .any(|r| r.path.contains("credits") && r.method == "POST")
    );
}

#[tokio::test]
async fn credits_post_uses_the_empty_grpc_frame_when_json_omits_the_percent() {
    fn encode_varint(mut value: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        while value > 0x7f {
            bytes.push(u8::try_from((value & 0x7f) | 0x80).expect("byte"));
            value >>= 7;
        }
        bytes.push(u8::try_from(value).expect("byte"));
        bytes
    }
    fn proto_tag(field: u32, wire: u32) -> Vec<u8> {
        encode_varint((field << 3) | wire)
    }
    fn proto_len(field: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = proto_tag(field, 2);
        out.extend(encode_varint(u32::try_from(payload.len()).expect("len")));
        out.extend_from_slice(payload);
        out
    }
    fn proto_fixed32(field: u32, value: f32) -> Vec<u8> {
        let mut out = proto_tag(field, 5);
        out.extend_from_slice(&value.to_le_bytes());
        out
    }
    fn grpc_frame(payload: &[u8]) -> Vec<u8> {
        let mut header = vec![0, 0, 0, 0, 0];
        let len = u32::try_from(payload.len()).expect("len").to_be_bytes();
        header[1..5].copy_from_slice(&len);
        header.extend_from_slice(payload);
        header
    }
    let seconds = 1_788_307_200_u32;
    let mut inner = proto_fixed32(1, 0.19);
    let mut ts = proto_tag(1, 0);
    ts.extend(encode_varint(seconds));
    inner.extend(proto_len(5, &ts));
    let frame = grpc_frame(&proto_len(1, &inner));

    let server = test_support::spawn(3, move |req| {
        if req.path.contains("billing") {
            return Reply::json(
                200,
                r#"{"config":{"isUnifiedBillingUser":true,"prepaidBalance":0,"productUsage":[{"product":"Grok Code"}]}}"#,
            );
        }
        if req.path.contains("credits") {
            assert_eq!(req.body.as_slice(), crate::grok::EMPTY_FRAME.as_slice());
            return Reply::bytes(200, frame.clone(), "application/grpc-web+proto");
        }
        Reply::json(404, "nope")
    });
    let client = test_support::client();
    let session = Session::new(crate::Family::Grok, "tok", "rt");
    let quota = fetch_quota_at(
        &session,
        &client,
        QuotaUrls {
            billing: &format!("{}/billing", server.base),
            user: &format!("{}/user", server.base),
            credits: &format!("{}/credits", server.base),
        },
    )
    .await
    .expect("quota");
    assert_eq!(quota.rows[0].kind, "weekly");
    assert_eq!(quota.rows[0].used_percent, Some(19));
    assert_eq!(quota.rows[0].remaining_percent, Some(81));
    assert_eq!(quota.rows[0].reset_at_ms, Some(i64::from(seconds) * 1000));
    assert!(!quota.rows.iter().any(|row| row.kind == "prepaid"));
}

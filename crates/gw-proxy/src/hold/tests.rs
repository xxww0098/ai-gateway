//! Pre-flight ordering and the pure decisions it is built from.
//!
//! The invariant almost every test here defends: **a rejected request must not
//! leave a reservation behind.** That is enforced by ordering alone, so these
//! assertions pair "the client saw a 402/429/503" with "the ledger was never
//! touched".

use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use chrono::{Datelike, TimeZone, Timelike, Utc};

use super::*;
use crate::ports::SubscriptionQuota;
use crate::testsupport::{
    Harness, LedgerCall, TEST_USER_ID, anonymous_request, chat_body, send, signed_request,
};

// ---------------------------------------------------------------- billable paths

#[test]
fn the_v1_prefix_alone_decides_what_is_billable() {
    assert!(is_billable(&Method::POST, "/v1/chat/completions"));
    assert!(is_billable(&Method::POST, "/v1/messages"));
    assert!(is_billable(
        &Method::POST,
        "/v1/models/gemini-2.5-pro:generateContent"
    ));

    // Everything outside /v1 keeps its own auth and its own (non-)billing.
    assert!(!is_billable(&Method::POST, "/api/panel/billing/topup"));
    assert!(!is_billable(&Method::GET, "/metrics/prometheus"));
}

#[test]
fn the_two_zero_cost_endpoints_are_billed_regardless() {
    // Deliberate parity with a historical behaviour that looks like a product
    // defect — see `is_billable`'s docs for the mechanism and the traced
    // amount. The assertion is inverted from what the endpoints deserve on
    // purpose: if someone "fixes" the predicate without the product decision
    // behind it, this is what flags the drift.
    assert!(is_billable(&Method::GET, "/v1/models"));
    assert!(is_billable(&Method::POST, "/v1/messages/count_tokens"));
}

// ---------------------------------------------------------------- body peek

#[test]
fn the_peek_reads_model_and_stream_from_a_json_body() {
    let peek = parse_body_peek(
        Some("application/json"),
        br#"{"model":" gpt-4o ","stream":true}"#,
    );
    assert_eq!(peek.model, "gpt-4o", "the model name must be trimmed");
    assert!(peek.stream);
    assert!(peek.parsed);
}

#[test]
fn the_output_cap_resolves_max_tokens_before_max_completion_tokens() {
    let both = parse_body_peek(
        Some("application/json"),
        br#"{"max_tokens":100,"max_completion_tokens":200}"#,
    );
    assert_eq!(both.max_tokens, 100);

    let fallback = parse_body_peek(
        Some("application/json"),
        br#"{"max_completion_tokens":200}"#,
    );
    assert_eq!(fallback.max_tokens, 200);

    let neither = parse_body_peek(Some("application/json"), br#"{}"#);
    assert_eq!(
        neither.max_tokens, 0,
        "an absent cap means 'unbounded', which the estimator reads as 0",
    );

    let negative = parse_body_peek(Some("application/json"), br#"{"max_tokens":-5}"#);
    assert_eq!(negative.max_tokens, 0);
}

#[test]
fn an_unparsable_body_never_rejects_the_request() {
    // Billing must not be the layer that decides a payload is malformed.
    let peek = parse_body_peek(Some("application/json"), b"{not json");
    assert!(!peek.parsed);
    assert_eq!(peek.model, "");
    assert!(
        peek.input_tokens > 0,
        "the reservation still scales with the bytes we were asked to forward",
    );
}

#[test]
fn a_non_json_body_is_not_peeked_at_all() {
    let peek = parse_body_peek(Some("multipart/form-data"), b"whatever");
    assert_eq!(peek, BodyPeek::default());
}

#[test]
fn the_token_approximation_is_monotone_and_never_undercounts_by_more_than_a_token() {
    assert_eq!(approximate_tokens_from_bytes(0), 0);
    let mut previous = 0;
    for size in 1..200usize {
        let tokens = approximate_tokens_from_bytes(size);
        assert!(tokens >= previous, "approximation dipped at {size}");
        assert!(
            tokens * 4 >= size as i64,
            "approximation undercounts at {size}",
        );
        previous = tokens;
    }
}

// ---------------------------------------------------------------- upper bound

#[test]
fn the_upper_bound_dominates_every_estimate_it_is_built_from() {
    let calc = crate::testsupport::FakeCalculator::default();
    for max_tokens in [0, 1, 512, 100_000] {
        for stream in [false, true] {
            for hold in [0.0, 0.01, 5.0] {
                let bound = preflight_upper_bound(&calc, "gpt-4o", max_tokens, stream, 1.0, hold);
                assert!(bound >= hold);
                assert!(bound >= calc.estimate_with_max_tokens("gpt-4o", max_tokens, stream, 1.0));
                assert!(
                    bound >= calc.estimate("gpt-4o", true, 1.0),
                    "the streaming estimate is the guard against an absent or absurd cap",
                );
            }
        }
    }
}

// ---------------------------------------------------------------- quota

fn quota_with(daily: Option<f64>, used: f64) -> SubscriptionQuota {
    SubscriptionQuota {
        id: 1,
        daily_limit_usd: daily,
        daily_usage_usd: used,
        ..SubscriptionQuota::default()
    }
}

#[test]
fn a_quota_only_rejects_once_the_estimate_would_cross_it() {
    let quota = quota_with(Some(10.0), 9.0);
    assert_eq!(evaluate_quota(&quota, 0.5), None);
    assert_eq!(
        evaluate_quota(&quota, 1.0),
        None,
        "landing exactly on the limit is allowed"
    );
    assert!(evaluate_quota(&quota, 1.5).is_some());
}

#[test]
fn an_unset_limit_never_rejects() {
    assert_eq!(evaluate_quota(&quota_with(None, 1_000.0), 500.0), None);
}

#[test]
fn each_period_reports_its_own_reason() {
    let daily = evaluate_quota(&quota_with(Some(1.0), 1.0), 1.0).expect("daily rejects");
    assert!(daily.contains("daily"));

    let weekly = evaluate_quota(
        &SubscriptionQuota {
            weekly_limit_usd: Some(1.0),
            weekly_usage_usd: 1.0,
            ..SubscriptionQuota::default()
        },
        1.0,
    )
    .expect("weekly rejects");
    assert!(weekly.contains("weekly"));

    let monthly = evaluate_quota(
        &SubscriptionQuota {
            monthly_limit_usd: Some(1.0),
            monthly_usage_usd: 1.0,
            ..SubscriptionQuota::default()
        },
        1.0,
    )
    .expect("monthly rejects");
    assert!(monthly.contains("monthly"));
}

// ---------------------------------------------------------------- rotation

#[test]
fn only_periods_whose_boundary_has_passed_are_rotated() {
    let now = Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap();
    let mut quota = SubscriptionQuota {
        daily_usage_usd: 5.0,
        weekly_usage_usd: 5.0,
        monthly_usage_usd: 5.0,
        daily_reset_at: Some(now - chrono::Duration::hours(1)), // elapsed
        weekly_reset_at: Some(now + chrono::Duration::days(2)), // still running
        monthly_reset_at: None,                                 // rotation disabled
        ..SubscriptionQuota::default()
    };

    assert!(rotate_counters(&mut quota, now));
    assert_eq!(quota.daily_usage_usd, 0.0);
    assert_eq!(quota.weekly_usage_usd, 5.0);
    assert_eq!(quota.monthly_usage_usd, 5.0);
    assert!(quota.monthly_reset_at.is_none());
    assert!(
        quota.daily_reset_at.expect("advanced") > now,
        "a rotated boundary must land in the future or it rotates forever",
    );
}

#[test]
fn rotation_is_idempotent_within_one_period() {
    let now = Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap();
    let mut quota = SubscriptionQuota {
        daily_usage_usd: 5.0,
        daily_reset_at: Some(now - chrono::Duration::hours(1)),
        ..SubscriptionQuota::default()
    };
    assert!(rotate_counters(&mut quota, now));
    let after_first = quota.clone();
    assert!(
        !rotate_counters(&mut quota, now),
        "a second pass must not report the row dirty again",
    );
    assert_eq!(quota, after_first);
}

#[test]
fn every_reset_boundary_is_strictly_in_the_future_and_on_its_period_start() {
    // Sampled across a leap year, a year boundary and every weekday.
    for (y, m, d, h) in [
        (2026, 1, 1, 0),
        (2026, 2, 28, 23),
        (2028, 2, 29, 12),
        (2026, 8, 10, 0), // a Monday
        (2026, 8, 16, 6), // a Sunday
        (2026, 12, 31, 23),
    ] {
        let now = Utc.with_ymd_and_hms(y, m, d, h, 30, 0).unwrap();

        let daily = next_daily_reset_after(now);
        assert!(daily > now);
        assert_eq!((daily.hour(), daily.minute(), daily.second()), (0, 0, 0));

        let weekly = next_weekly_reset_after(now);
        assert!(weekly > now);
        assert_eq!(weekly.weekday(), chrono::Weekday::Mon);
        assert_eq!(weekly.hour(), 0);

        let monthly = next_monthly_reset_after(now);
        assert!(monthly > now);
        assert_eq!(monthly.day(), 1);
        assert_eq!(monthly.hour(), 0);
    }
}

#[test]
fn a_monday_midnight_rolls_to_the_following_monday() {
    // "Today is Monday at exactly midnight still counts as past", so the
    // boundary must advance a full week.
    let monday = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
    assert_eq!(monday.weekday(), chrono::Weekday::Mon);
    let next = next_weekly_reset_after(monday);
    assert_eq!((next - monday).num_days(), 7);
}

// ---------------------------------------------------------------- headers

#[test]
fn the_client_ip_prefers_the_forwarded_chain_then_the_real_ip() {
    let mut headers = HeaderMap::new();
    headers.insert("x-real-ip", HeaderValue::from_static("10.0.0.9"));
    assert_eq!(extract_ip_address(&headers), "10.0.0.9");

    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("203.0.113.7, 10.0.0.1"),
    );
    assert_eq!(
        extract_ip_address(&headers),
        "203.0.113.7",
        "the originating client is the first entry of the chain",
    );

    assert_eq!(extract_ip_address(&HeaderMap::new()), "");
}

#[test]
fn both_idempotency_header_spellings_are_accepted() {
    let mut headers = HeaderMap::new();
    assert_eq!(extract_idempotency_key(&headers), "");

    headers.insert("x-idempotency-key", HeaderValue::from_static("b"));
    assert_eq!(extract_idempotency_key(&headers), "b");

    headers.insert("idempotency-key", HeaderValue::from_static("a"));
    assert_eq!(
        extract_idempotency_key(&headers),
        "a",
        "the canonical spelling wins when both are present",
    );
}

#[test]
fn an_inbound_trace_id_is_honoured_and_otherwise_generated() {
    let mut headers = HeaderMap::new();
    headers.insert(TRACE_HEADER, HeaderValue::from_static("trace-123"));
    assert_eq!(trace_id_from(&headers), "trace-123");

    let generated = trace_id_from(&HeaderMap::new());
    assert!(!generated.is_empty());
    assert_ne!(
        generated,
        trace_id_from(&HeaderMap::new()),
        "generated hold keys must be unique per request",
    );
}

#[test]
fn the_breaker_key_is_derived_from_the_model_family() {
    assert_eq!(infer_provider("gpt-4o"), Some("openai"));
    assert_eq!(infer_provider("o3-mini"), Some("openai"));
    assert_eq!(infer_provider("claude-sonnet-5"), Some("anthropic"));
    assert_eq!(infer_provider("gemini-2.5-pro"), Some("google"));
    assert_eq!(infer_provider("gpt-5-codex"), Some("openai"));
    assert_eq!(infer_provider("codex-mini"), Some("codex"));
    assert_eq!(infer_provider("mystery-model"), None);
}

// ---------------------------------------------------------------- middleware

#[tokio::test]
async fn an_unauthenticated_request_is_refused_before_any_reservation() {
    let harness = Harness::build();
    let (status, _) = send(
        harness.stub_router(StatusCode::OK),
        anonymous_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        harness.ledger.calls().is_empty(),
        "authentication must gate the ledger entirely",
    );
}

#[tokio::test]
async fn a_successful_request_reserves_and_then_settles_exactly_once() {
    let harness = Harness::build();
    let (status, _) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let calls = harness.ledger.calls();
    assert!(
        matches!(calls.first(), Some(LedgerCall::Hold { user_id, amount })
            if *user_id == TEST_USER_ID && *amount > 0.0),
        "the reservation must come first: {calls:?}",
    );
    assert_eq!(
        harness.usage_store.settled_costs().len(),
        1,
        "the stub handler published no usage, so the fallback settles once",
    );
}

#[tokio::test]
async fn a_failed_downstream_releases_the_reservation_instead_of_charging() {
    let harness = Harness::build();
    let (status, _) = send(
        harness.stub_router(StatusCode::BAD_GATEWAY),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);

    let calls = harness.ledger.calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "a non-2xx must give the reservation back: {calls:?}",
    );
    assert!(
        harness.usage_store.settled_costs().is_empty(),
        "a failed request must not be charged",
    );
}

#[tokio::test]
async fn an_outstanding_debt_blocks_further_work_without_reserving() {
    let harness = Harness::build();
    *harness.ledger.shortfall.lock() = true;

    let (status, body) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"].as_str(), Some("outstanding_debt"));
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn a_shortfall_lookup_failure_fails_closed() {
    // A transient DB hiccup must not become a way for a debtor to slip through.
    let harness = Harness::build();
    *harness.ledger.shortfall_errors.lock() = true;

    let (status, body) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"].as_str(), Some("outstanding_debt"));
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn an_underfunded_tenant_is_refused_before_a_hold_is_created() {
    let harness = Harness::build();
    *harness.ledger.balance.lock() = 0.0;

    let (status, body) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"].as_str(), Some("insufficient_balance"));
    assert!(
        harness.ledger.calls().is_empty(),
        "the upper-bound gate exists precisely so no Redis hold is created",
    );
    assert!(body["required_amount"].as_f64().unwrap_or(0.0) > 0.0);
}

#[tokio::test]
async fn a_rate_limited_tenant_never_reaches_the_ledger() {
    let harness = Harness::build();
    *harness.rate_limiter.allow.lock() = false;

    let (status, body) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"].as_str(), Some("Too Many Requests"));
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn a_limiter_outage_fails_open_so_traffic_keeps_flowing() {
    let harness = Harness::build();
    *harness.rate_limiter.errors.lock() = true;

    let (status, _) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_concurrency_slot_is_returned_on_success_and_on_rejection() {
    // Without this the MaxConcurrent limit degrades into a TTL-length cap.
    let harness = Harness::build();
    send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(harness.rate_limiter.released.lock().len(), 1);

    *harness.ledger.shortfall.lock() = true;
    send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(
        harness.rate_limiter.released.lock().len(),
        2,
        "an aborted request must free its slot too",
    );
}

#[tokio::test]
async fn an_open_circuit_refuses_before_reserving() {
    let harness = Harness::build();
    *harness.breaker.allow.lock() = false;

    let (status, body) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"].as_str(), Some("Service Unavailable"));
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn an_exhausted_quota_refuses_before_reserving() {
    let harness = Harness::build();
    let quota = SubscriptionQuota {
        id: 55,
        daily_limit_usd: Some(0.000_001),
        ..SubscriptionQuota::default()
    };
    harness.quota.quotas.lock().insert(55, quota.clone());
    harness
        .directory
        .subscriptions
        .lock()
        .insert(TEST_USER_ID, quota);

    let (status, body) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("daily"),
        "the client should learn which period ran out: {body}",
    );
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn a_quota_lookup_failure_fails_closed() {
    let harness = Harness::build();
    harness.directory.subscriptions.lock().insert(
        TEST_USER_ID,
        SubscriptionQuota {
            id: 55,
            ..SubscriptionQuota::default()
        },
    );
    *harness.quota.errors.lock() = true;

    let (status, _) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn a_larger_prompt_reserves_more_than_a_small_one() {
    // The reservation scales with the request so a big prompt cannot slip past
    // the balance gate on a flat nominal assumption.
    async fn hold_amount_for(prompt: &str) -> f64 {
        let harness = Harness::build();
        let mut body = chat_body("gpt-4o");
        body["messages"][0]["content"] = serde_json::json!(prompt);
        send(
            harness.stub_router(StatusCode::OK),
            signed_request("/v1/chat/completions", body),
        )
        .await;
        match harness.ledger.calls().first() {
            Some(LedgerCall::Hold { amount, .. }) => *amount,
            other => panic!("expected a hold, got {other:?}"),
        }
    }

    let small = hold_amount_for("hi").await;
    let large = hold_amount_for(&"x".repeat(20_000)).await;
    assert!(large > small, "{large} should exceed {small}");
}

#[tokio::test]
async fn a_body_over_the_preflight_limit_is_refused_rather_than_truncated() {
    // Truncating in place would forward a corrupted payload upstream.
    let harness = Harness::build();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!("Bearer {}", crate::testsupport::TEST_API_KEY),
        )
        .body(axum::body::Body::from(vec![
            b'x';
            HOLD_REQUEST_BODY_LIMIT + 1
        ]))
        .expect("request builds");

    let (status, _) = send(harness.stub_router(StatusCode::OK), request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn a_non_v1_path_skips_billing_entirely() {
    let harness = Harness::build();
    let router = axum::Router::new()
        .route(
            "/api/panel/ping",
            axum::routing::get(|| async { StatusCode::OK }),
        )
        .layer(axum::middleware::from_fn_with_state(
            harness.state.clone(),
            layer,
        ))
        .with_state(harness.state.clone());

    let request = axum::http::Request::builder()
        .uri("/api/panel/ping")
        .body(axum::body::Body::empty())
        .expect("request builds");
    let (status, _) = send(router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert!(harness.ledger.calls().is_empty());
}

#[test]
fn a_zero_ttl_falls_back_to_the_documented_default() {
    // A hold that never expires would starve a balance after one crash.
    let harness = Harness::build();
    let middleware = HoldMiddleware::new(
        harness.ledger.clone(),
        harness.calc.clone(),
        harness.settlement.clone(),
        Duration::ZERO,
    );
    assert_eq!(middleware.ttl(), DEFAULT_HOLD_TTL);
}

// ---------------------------------------------------------------- idempotency

/// An authenticated request carrying an `Idempotency-Key`.
fn keyed_request(key: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!("Bearer {}", crate::testsupport::TEST_API_KEY),
        )
        .header("idempotency-key", key)
        .body(axum::body::Body::from(chat_body("gpt-4o").to_string()))
        .expect("request builds")
}

#[tokio::test]
async fn a_retried_request_replays_the_first_response_without_billing_again() {
    let harness = Harness::build();

    let (first, _) = send(harness.stub_router(StatusCode::OK), keyed_request("k-1")).await;
    assert_eq!(first, StatusCode::OK);
    let billed_once = harness.usage_store.settled_costs().len();

    let (second, _) = send(harness.stub_router(StatusCode::OK), keyed_request("k-1")).await;
    assert_eq!(second, StatusCode::OK);
    assert_eq!(
        harness.usage_store.settled_costs().len(),
        billed_once,
        "a replay must not settle a second time",
    );
    assert_eq!(
        harness
            .ledger
            .calls()
            .iter()
            .filter(|c| matches!(c, LedgerCall::Hold { .. }))
            .count(),
        1,
        "a replay must not reserve a second time",
    );
}

#[tokio::test]
async fn a_duplicate_arriving_mid_flight_is_told_to_wait_rather_than_re_run() {
    let harness = Harness::build();
    // Simulate the in-flight claim the first request would have taken.
    let key = harness.state.hold.clone();
    drop(key);
    let manager = crate::idempotency::IdempotencyManager::new(
        harness.idempotency.clone(),
        std::sync::Arc::new(crate::testsupport::FakeCrypto::default()),
        Duration::ZERO,
    );
    let scoped = manager.scoped_key(TEST_USER_ID, "POST", "/v1/chat/completions", "k-1");
    manager.claim(&scoped).await.expect("claim");

    let (status, body) = send(harness.stub_router(StatusCode::OK), keyed_request("k-1")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"].as_str(), Some("idempotency_conflict"));
    assert!(
        harness.ledger.calls().is_empty(),
        "a duplicate must be turned away before it reserves anything",
    );
}

#[tokio::test]
async fn a_failed_request_frees_its_key_so_the_retry_can_proceed() {
    let harness = Harness::build();

    let (first, _) = send(
        harness.stub_router(StatusCode::BAD_GATEWAY),
        keyed_request("k-1"),
    )
    .await;
    assert_eq!(first, StatusCode::BAD_GATEWAY);

    let (second, _) = send(harness.stub_router(StatusCode::OK), keyed_request("k-1")).await;
    assert_eq!(
        second,
        StatusCode::OK,
        "a retry after a failure must not be blocked by the abandoned claim",
    );
}

#[tokio::test]
async fn requests_without_a_key_are_never_deduplicated() {
    let harness = Harness::build();
    for _ in 0..2 {
        send(
            harness.stub_router(StatusCode::OK),
            signed_request("/v1/chat/completions", chat_body("gpt-4o")),
        )
        .await;
    }
    assert_eq!(
        harness.usage_store.settled_costs().len(),
        2,
        "idempotency is opt-in; two unrelated requests both bill",
    );
}

#[tokio::test]
async fn a_balance_lookup_outage_refuses_the_request_rather_than_letting_it_spend() {
    // Fail closed: spend admitted during a balance-store outage is spend the
    // ledger may not be able to reclaim.
    let harness = Harness::build();
    struct BlindLedger(std::sync::Arc<crate::testsupport::FakeLedger>);
    #[async_trait::async_trait]
    impl crate::ports::BillingLedger for BlindLedger {
        async fn hold(
            &self,
            user_id: crate::ports::Id,
            amount: f64,
            request_id: &str,
            ttl: Duration,
        ) -> Result<(), crate::ports::BillingError> {
            self.0.hold(user_id, amount, request_id, ttl).await
        }
        async fn settle(
            &self,
            user_id: crate::ports::Id,
            request_id: &str,
            amount: f64,
        ) -> Result<f64, crate::ports::BillingError> {
            self.0.settle(user_id, request_id, amount).await
        }
        async fn release(
            &self,
            user_id: crate::ports::Id,
            request_id: &str,
        ) -> Result<(), crate::ports::BillingError> {
            self.0.release(user_id, request_id).await
        }
        async fn active_hold_amount(
            &self,
            user_id: crate::ports::Id,
            request_id: &str,
        ) -> Result<Option<f64>, crate::ports::BillingError> {
            self.0.active_hold_amount(user_id, request_id).await
        }
        async fn has_unresolved_shortfall(
            &self,
            user_id: crate::ports::Id,
        ) -> Result<bool, crate::ports::BillingError> {
            self.0.has_unresolved_shortfall(user_id).await
        }
        async fn available_balance(
            &self,
            _user_id: crate::ports::Id,
        ) -> Result<f64, crate::ports::BillingError> {
            Err(crate::ports::BillingError::Other(anyhow::anyhow!(
                "balance store unreachable"
            )))
        }
    }

    let blind = std::sync::Arc::new(BlindLedger(harness.ledger.clone()));
    let hold = std::sync::Arc::new(HoldMiddleware::new(
        blind,
        harness.calc.clone(),
        harness.settlement.clone(),
        Duration::from_secs(60),
    ));
    let mut state = harness.state.clone();
    state.hold = hold;

    let router = axum::Router::new()
        .route(
            "/v1/chat/completions",
            axum::routing::post(|| async { StatusCode::OK }),
        )
        .layer(axum::middleware::from_fn_with_state(state.clone(), layer))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::access::layer,
        ))
        .with_state(state);

    let (status, body) = send(
        router,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"].as_str(), Some("insufficient_balance"));
    assert!(harness.ledger.calls().is_empty());
}

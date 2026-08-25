//! Pre-flight ordering and the pure decisions it is built from.
//!
//! The invariant almost every test here defends: **a rejected request must not
//! leave a reservation behind.** That is enforced by ordering alone, so these
//! assertions pair "the client saw a 402/429/503" with "the ledger was never
//! touched".

use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use chrono::{Datelike, TimeZone, Timelike, Utc};
use parking_lot::Mutex;

use super::*;
use crate::body::InboundBody;
use crate::ports::{HoldAdmit, SubscriptionQuota};
use crate::testsupport::{
    FakeLedger, Harness, LedgerCall, TEST_USER_ID, anonymous_request, chat_body, send,
    signed_request,
};

// ---------------------------------------------------------------- billable paths

#[test]
fn only_the_three_inference_entries_are_billable() {
    assert!(is_billable(&Method::POST, "/v1/chat/completions"));
    assert!(is_billable(&Method::POST, "/v1/responses"));
    assert!(is_billable(&Method::POST, "/v1/messages"));

    // /v1 之外各管各的鉴权与计费。
    assert!(!is_billable(&Method::POST, "/api/panel/billing/topup"));
    assert!(!is_billable(&Method::GET, "/metrics/prometheus"));
    // Gemini 原生面已硬删：它连鉴权面都不在了，更谈不上计费。
    assert!(!is_billable(
        &Method::POST,
        "/v1beta/models/gemini-2.5-pro:generateContent"
    ));
}

#[test]
fn the_zero_cost_endpoints_are_out_of_billing_scope() {
    // 收敛前 catalogue / count_tokens 按 LLM 价格收钱；`GET /v1/usage` 是
    // 新加的只读查询，全部 GET 都不进预扣。
    assert!(!is_billable(&Method::GET, "/v1/models"));
    assert!(!is_billable(&Method::GET, "/v1/models/gpt-4o"));
    assert!(!is_billable(&Method::GET, "/v1/usage"));
    assert!(!is_billable(&Method::POST, "/v1/messages/count_tokens"));
}

#[test]
fn a_safe_method_is_never_billable_whatever_the_path() {
    // 排除的是「方法」这条轴，不是某几条写死的路径：任何只读方法都不该预扣。
    for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
        for path in ["/v1/chat/completions", "/v1/messages", "/v1/anything"] {
            assert!(
                !is_billable(&method, path),
                "{method} {path} 会为一个只读请求预扣余额",
            );
        }
    }
}

// ---------------------------------------------------------------- body peek

/// 计费侧看到的 peek，走的是与生产同一条路径：`gw-relay` 的唯一一次解析
/// → [`BillingPeek::from_spec`]。
fn billing_peek(body: &[u8]) -> BillingPeek {
    let spec = RequestSpec::parse(gw_relay::Surface::OpenAiCompletions, Some(body));
    BillingPeek::from_spec(&spec, body.len())
}

/// A real cache-backed calculator so trim/case variants can miss the table
/// when they fail to share a key. The default rate is far from the row so a
/// miss cannot accidentally equal a hit.
fn priced_calculator(model_id: &str, input: f64, output: f64) -> crate::adapters::SharedCalculator {
    let row = gw_model::ModelPrice {
        id: 1,
        model_id: model_id.to_owned(),
        input_price_per_1m: input,
        output_price_per_1m: output,
        cached_input_price_per_1m: 0.0,
        reasoning_price_per_1m: 0.0,
        created_at: chrono::DateTime::<Utc>::UNIX_EPOCH,
        updated_at: chrono::DateTime::<Utc>::UNIX_EPOCH,
    };
    let cache = gw_pricing::ModelPriceCache::from_rows([row]);
    crate::adapters::SharedCalculator::new(std::sync::Arc::new(gw_pricing::Calculator::new(
        Some(std::sync::Arc::new(cache)),
        99.0,
    )))
}

#[test]
fn the_peek_reads_model_and_stream_from_a_json_body() {
    let peek = billing_peek(br#"{"model":" gpt-4o ","stream":true}"#);
    assert_eq!(peek.model, "gpt-4o", "the model name must be trimmed");
    assert_eq!(
        peek.price_key,
        gw_pricing::normalize_model_key(" gpt-4o "),
        "price_key is the public normalize of the raw model",
    );
    assert!(peek.stream);
}

#[test]
fn the_price_key_is_the_public_normalize_of_the_peeked_model() {
    for raw in ["Claude-3", "  R1  ", "MiXeD"] {
        let body = format!(r#"{{"model":"{raw}"}}"#);
        let peek = billing_peek(body.as_bytes());
        assert_eq!(peek.price_key, gw_pricing::normalize_model_key(raw));
        assert_eq!(peek.price_key, gw_pricing::normalize_model_key(&peek.model),);
    }
}

#[test]
fn the_output_cap_falls_back_across_the_three_dialect_spellings() {
    // `max_tokens`（A、C）→ `max_completion_tokens`（A 的新拼法）→
    // `max_output_tokens`（B）。收敛前最后一级不存在，于是**每一个**
    // `/v1/responses` 请求的 max_tokens 都是 0，预扣退化成保守估算、过度冻结余额。
    assert_eq!(
        billing_peek(br#"{"max_tokens":100,"max_completion_tokens":200}"#).max_tokens,
        100
    );
    assert_eq!(
        billing_peek(br#"{"max_completion_tokens":200,"max_output_tokens":300}"#).max_tokens,
        200
    );
    assert_eq!(
        billing_peek(br#"{"max_output_tokens":300}"#).max_tokens,
        300,
        "入口 B 的输出上限必须被读到",
    );

    assert_eq!(
        billing_peek(br#"{}"#).max_tokens,
        0,
        "an absent cap means 'unbounded', which the estimator reads as 0",
    );
    assert_eq!(billing_peek(br#"{"max_tokens":-5}"#).max_tokens, 0);
}

#[test]
fn an_unparsable_body_never_rejects_the_request() {
    // Billing must not be the layer that decides a payload is malformed.
    let peek = billing_peek(b"{not json");
    assert_eq!(peek.model, "");
    assert!(
        peek.input_tokens > 0,
        "the reservation still scales with the bytes we were asked to forward",
    );
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
    assert_eq!(client_trace_from(&headers).as_str(), "trace-123");

    let generated = client_trace_from(&HeaderMap::new());
    assert!(!generated.is_empty());
    assert_ne!(
        generated,
        client_trace_from(&HeaderMap::new()),
        "a generated trace id must be unique per request",
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

#[test]
fn compute_reservation_pairs_hold_with_a_dominating_floor() {
    let calc = crate::testsupport::FakeCalculator::default();
    let peek = billing_peek(chat_body("gpt-4o").to_string().as_bytes());
    let (hold, floor) = compute_reservation(&peek, 1.0, &calc);
    assert!(hold > 0.0);
    assert!(
        floor >= hold,
        "the floor must gate at least as much as the reservation",
    );
    assert_eq!(
        floor,
        preflight_upper_bound(
            &calc,
            &peek.price_key,
            peek.max_tokens,
            peek.stream,
            1.0,
            hold,
        ),
    );
}

/// Trim and case must not change the reservation: the three estimators share
/// one already-normalized key. A miss (different id) must not collide, so
/// the fixture is not a vacuous constant.
#[test]
fn compute_reservation_is_invariant_to_model_trim_and_case() {
    let calc = priced_calculator("Mix-Id", 3.0, 7.0);
    let variants = ["Mix-Id", "mix-id", "MIX-ID", "  mix-id  "];
    let pairs: Vec<(f64, f64)> = variants
        .iter()
        .map(|model| {
            let body = format!(r#"{{"model":"{model}","max_tokens":128}}"#);
            compute_reservation(&billing_peek(body.as_bytes()), 1.5, &calc)
        })
        .collect();
    let first = pairs[0];
    for pair in &pairs[1..] {
        assert_eq!(
            *pair, first,
            "trim/case variants must share one (hold, floor)",
        );
    }
    let miss = compute_reservation(
        &billing_peek(br#"{"model":"other-id","max_tokens":128}"#),
        1.5,
        &calc,
    );
    assert_ne!(miss, first, "a different model must not collide");
}

// ------------------------------------------------------------ admit_operation

/// Builds an operation reserving `amount` against `liability`.
fn new_operation(
    user_id: crate::ports::Id,
    amount: f64,
    liability: f64,
) -> gw_ledger::NewOperation {
    gw_ledger::NewOperation {
        operation_id: gw_ledger::BillingOperationId::mint(),
        user_id,
        reserved_amount: amount,
        admitted_liability: liability,
        request_fingerprint: "fingerprint".to_owned(),
        client_trace_id: "trace-the-client-saw".to_owned(),
    }
}

#[tokio::test]
async fn a_floor_refusal_leaves_neither_a_reservation_nor_an_operation() {
    let ledger = FakeLedger::with_balance(1.0);
    let operation = new_operation(1, 0.5, 2.0);
    let admit = ledger
        .admit_operation(&operation, Some(Duration::from_secs(60)))
        .await
        .expect("lookup");
    assert!(matches!(admit, HoldAdmit::Insufficient { .. }));
    assert!(ledger.calls().is_empty());
    assert!(ledger.held_amount(&operation.operation_id).is_none());
    assert!(
        ledger.operation_state(&operation.operation_id).is_none(),
        "a refused hold must not leave a reconcilable row behind",
    );
}

#[tokio::test]
async fn a_floor_refusal_quotes_the_available_balance() {
    let ledger = FakeLedger::with_balance(3.75);
    let admit = ledger
        .admit_operation(&new_operation(1, 1.0, 5.0), Some(Duration::from_secs(60)))
        .await
        .expect("lookup");
    assert_eq!(admit, HoldAdmit::Insufficient { available: 3.75 },);
}

#[tokio::test]
async fn a_budget_token_reservation_still_writes_the_durable_operation() {
    // `None` TTL = the reservation came from the process-local budget token.
    // The row must exist anyway: it is the operation's identity, and without it
    // a crash would leave money unaccounted for with nothing to reconcile.
    let ledger = FakeLedger::with_balance(100.0);
    let operation = new_operation(1, 1.0, 1.0);
    ledger
        .admit_operation(&operation, None)
        .await
        .expect("admit");
    assert_eq!(
        ledger.operation_state(&operation.operation_id),
        Some(gw_ledger::operation::OperationState::Held),
    );
    assert!(
        ledger.held_amount(&operation.operation_id).is_none(),
        "no Redis reservation is taken when the budget token paid",
    );
}

#[tokio::test]
async fn an_insufficient_balance_402_quotes_the_peeked_available() {
    let harness = Harness::build();
    let peek = billing_peek(chat_body("gpt-4o").to_string().as_bytes());
    let (_hold, floor) = compute_reservation(&peek, 1.0, harness.calc.as_ref());
    let quoted_available = floor - 0.01;
    assert!(
        quoted_available > 0.0,
        "fixture needs a positive balance below the floor",
    );
    *harness.ledger.balance.lock() = quoted_available;

    let (status, body) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"].as_str(), Some("insufficient_balance"));
    assert!(
        harness.ledger.calls().is_empty(),
        "floor refusal must not create a hold",
    );
    assert_eq!(
        body["current_balance"].as_f64(),
        Some(quoted_available),
        "the 402 must quote the balance seen at gate time",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_requests_cannot_pass_the_floor_twice_on_one_balance() {
    let harness = Harness::build();
    let peek = billing_peek(chat_body("gpt-4o").to_string().as_bytes());
    let (_hold, floor) = compute_reservation(&peek, 1.0, harness.calc.as_ref());
    *harness.ledger.balance.lock() = floor;

    let router = harness.stub_router(StatusCode::OK);
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let router = router.clone();
        tasks.push(tokio::spawn(async move {
            send(
                router,
                signed_request("/v1/chat/completions", chat_body("gpt-4o")),
            )
            .await
        }));
    }

    let mut ok = 0;
    let mut denied = 0;
    for task in tasks {
        let (status, _) = task.await.expect("request finishes");
        match status {
            StatusCode::OK => ok += 1,
            StatusCode::PAYMENT_REQUIRED => denied += 1,
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(ok, 1, "exactly one request may pass the atomic floor");
    assert_eq!(denied, 1);
    assert_eq!(
        harness
            .ledger
            .calls()
            .iter()
            .filter(|c| matches!(c, LedgerCall::Hold { .. }))
            .count(),
        1,
        "only the winner may create a reservation",
    );
}

mod middleware;

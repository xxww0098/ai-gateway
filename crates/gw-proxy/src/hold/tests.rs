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
fn new_operation(user_id: crate::ports::Id, amount: f64, liability: f64) -> gw_ledger::NewOperation {
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

/// The upper-bound gate is stricter than the reserved amount. A tenant who
/// can cover the hold but not `max(hold, EstimateWithMaxTokens, Estimate
/// (stream))` must still be refused, and that refusal must not create a
/// reservation (otherwise the next request sees a phantom hold).
#[tokio::test]
async fn covering_the_hold_but_not_the_upper_bound_is_refused_without_reserving() {
    let harness = Harness::build();
    let body = chat_body("gpt-4o");
    let peek = billing_peek(body.to_string().as_bytes());
    let hold_amount = harness.calc.estimate_with_tokens(
        &peek.price_key,
        peek.input_tokens,
        peek.max_tokens,
        peek.stream,
        1.0,
    );
    let upper_bound = preflight_upper_bound(
        harness.calc.as_ref(),
        &peek.price_key,
        peek.max_tokens,
        peek.stream,
        1.0,
        hold_amount,
    );
    assert!(
        hold_amount < upper_bound,
        "this fixture needs a gap between the reservation and the gate",
    );
    let mid = (hold_amount + upper_bound) / 2.0;
    *harness.ledger.balance.lock() = mid;

    let (status, body) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"].as_str(), Some("insufficient_balance"));
    assert!(
        harness.ledger.calls().is_empty(),
        "a floor refusal must not leave a hold: {calls:?}",
        calls = harness.ledger.calls(),
    );
    let quoted = body["current_balance"].as_f64().expect("current_balance");
    let required = body["required_amount"].as_f64().expect("required_amount");
    assert!(
        quoted < required,
        "the 402 must quote a gap, got {quoted} vs {required}",
    );
}

/// A funded request still reserves exactly the hold (not the upper bound)
/// and a downstream failure still releases rather than settling.
#[tokio::test]
async fn settle_and_release_still_match_the_reservation() {
    let ok = Harness::build();
    send(
        ok.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    let reserved = match ok.ledger.calls().first() {
        Some(LedgerCall::Hold { amount, .. }) => *amount,
        other => panic!("expected a hold, got {other:?}"),
    };
    assert!(reserved > 0.0);
    assert_eq!(ok.usage_store.settled_costs().len(), 1);
    assert!(
        !ok.ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "a 2xx must settle, not release",
    );

    let fail = Harness::build();
    send(
        fail.stub_router(StatusCode::BAD_GATEWAY),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert!(
        fail.ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Hold { .. })),
    );
    assert!(
        fail.ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "a non-2xx must give the reservation back",
    );
    assert!(fail.usage_store.settled_costs().is_empty());
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
async fn a_balance_store_outage_refuses_the_request_rather_than_letting_it_spend() {
    // Fail closed: spend admitted during a balance-store outage is spend the
    // ledger may not be able to reclaim. The admission is now one call, so the
    // outage surfaces there — and the middleware must refuse rather than run
    // the request unreserved.
    let harness = Harness::build();
    struct BlindLedger(std::sync::Arc<crate::testsupport::FakeLedger>);
    #[async_trait::async_trait]
    impl crate::ports::BillingLedger for BlindLedger {
        /// The store is unreachable, so admission cannot say whether the
        /// balance covers anything.
        async fn admit_operation(
            &self,
            _operation: &gw_ledger::NewOperation,
            _redis_ttl: Option<Duration>,
        ) -> Result<crate::ports::HoldAdmit, crate::ports::BillingError> {
            Err(crate::ports::BillingError::Other(anyhow::anyhow!(
                "balance store unreachable"
            )))
        }
        async fn settle_once(
            &self,
            user_id: crate::ports::Id,
            operation: &gw_ledger::BillingOperationId,
            amount: f64,
        ) -> Result<crate::ports::SettleTerminal, crate::ports::BillingError> {
            self.0.settle_once(user_id, operation, amount).await
        }
        async fn release_once(
            &self,
            user_id: crate::ports::Id,
            operation: &gw_ledger::BillingOperationId,
        ) -> Result<(), crate::ports::BillingError> {
            self.0.release_once(user_id, operation).await
        }
        async fn active_hold_amount(
            &self,
            user_id: crate::ports::Id,
            operation: &gw_ledger::BillingOperationId,
        ) -> Result<Option<f64>, crate::ports::BillingError> {
            self.0.active_hold_amount(user_id, operation).await
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
    assert_eq!(body["error"].as_str(), Some("Payment Required"));
    assert!(
        harness.ledger.calls().is_empty(),
        "an unadmitted request must not have moved money",
    );
}

// ============================================ X-Trace-ID is not the money key

/// Adds a client-chosen `X-Trace-ID` to a signed request.
fn traced(mut request: axum::http::Request<axum::body::Body>, trace: &str) -> axum::http::Request<axum::body::Body> {
    request.headers_mut().insert(
        TRACE_HEADER,
        HeaderValue::from_str(trace).expect("a header value"),
    );
    request
}

/// Drives one billable request carrying `trace` and returns the operation id
/// the ledger was asked to admit.
async fn operation_admitted_for(harness: &Harness, trace: &str) -> String {
    let before: std::collections::HashSet<String> =
        harness.ledger.admitted_operations().into_iter().collect();
    let (status, _) = send(
        harness.stub_router(StatusCode::OK),
        traced(
            signed_request("/v1/chat/completions", chat_body("gpt-4o")),
            trace,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut fresh: Vec<String> = harness
        .ledger
        .admitted_operations()
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(
        fresh.len(),
        1,
        "one billable request must admit exactly one operation",
    );
    fresh.pop().expect("exactly one")
}

#[tokio::test]
async fn a_client_trace_id_never_becomes_the_operation_id() {
    // The bug this pins: the hold, the settle and the usage event key used to
    // be an inbound header the client picked.
    let harness = Harness::build();
    let trace = "a-trace-the-client-chose";
    let operation = operation_admitted_for(&harness, trace).await;

    assert_ne!(operation, trace);
    assert!(
        !operation.contains(trace),
        "the operation id must not be derived from the trace header",
    );
}

#[tokio::test]
async fn the_operation_key_is_independent_of_whatever_trace_arrives() {
    // Two requests differing *only* in the header the client controls get two
    // distinct money keys, and neither key is a function of its header.
    let harness = Harness::build();
    for trace in ["trace-alpha", "trace-beta"] {
        let operation = operation_admitted_for(&harness, trace).await;
        assert_ne!(operation, trace);
        assert!(!operation.contains(trace));
    }
    let admitted = harness.ledger.admitted_operations();
    assert_eq!(
        admitted.len(),
        2,
        "each request owns its own operation: {admitted:?}",
    );
}

#[tokio::test]
async fn a_colliding_trace_id_does_not_collide_the_money_key() {
    // Replay the *same* client trace id many times. Every request is its own
    // billing operation; if the trace keyed the ledger they would all land on
    // one row, and the reservations would overwrite each other.
    let harness = Harness::build();
    let replayed = "the-same-trace-every-time";
    let mut seen = std::collections::HashSet::new();
    for _ in 0..8 {
        assert!(
            seen.insert(operation_admitted_for(&harness, replayed).await),
            "a replayed trace id produced a repeated operation id",
        );
    }
}

#[tokio::test]
async fn the_settled_usage_row_is_keyed_by_the_operation_not_by_the_trace() {
    // Every settled row must carry the operation id in `event_key` — the
    // column that was hard-coded to the empty string — while the trace the
    // client sent stays in `request_id`, where support tickets can find it.
    let harness = Harness::build();
    let trace = "a-trace-the-client-chose";
    let operation = operation_admitted_for(&harness, trace).await;

    let commits = harness.usage_store.commits.lock();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].operation.as_str(), operation);
    assert_eq!(commits[0].entry.event_key, operation);
    assert!(!commits[0].entry.event_key.is_empty());
    assert_eq!(commits[0].entry.request_id, trace);
    assert_ne!(commits[0].entry.request_id, commits[0].entry.event_key);
}

#[tokio::test]
async fn two_tenants_sharing_a_trace_id_get_separate_operations() {
    // The cross-tenant version of the collision: nothing about the header may
    // decide which ledger row is touched.
    let shared_trace = "a-trace-two-tenants-both-picked";
    let alpha = Harness::build();
    let beta = Harness::build();

    let one = operation_admitted_for(&alpha, shared_trace).await;
    let two = operation_admitted_for(&beta, shared_trace).await;
    assert_ne!(one, two);
}

#[tokio::test]
async fn a_request_without_a_trace_header_still_gets_an_operation() {
    // The trace is optional; the money key is not.
    let harness = Harness::build();
    let (status, _) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.ledger.admitted_operations().len(), 1);

    let commits = harness.usage_store.commits.lock();
    assert!(!commits[0].entry.event_key.is_empty());
}

#[tokio::test]
async fn the_reservation_is_the_admitted_liability_not_a_smaller_floor() {
    // Prepaid: what was compared against the balance is what is reserved.
    // Reserving the smaller `hold_amount` is the under-hold that lets a large
    // request settle into debt.
    let harness = Harness::build();
    let peek = billing_peek(chat_body("gpt-4o").to_string().as_bytes());
    let (hold_amount, upper_bound) = compute_reservation(&peek, 1.0, harness.calc.as_ref());
    assert!(
        upper_bound >= hold_amount,
        "the upper bound is by construction at least the hold estimate",
    );

    let (status, _) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let reserved = harness
        .ledger
        .calls()
        .into_iter()
        .find_map(|call| match call {
            LedgerCall::Hold { amount, .. } => Some(amount),
            _ => None,
        })
        .expect("the request reserved");
    assert_eq!(reserved, upper_bound);
}

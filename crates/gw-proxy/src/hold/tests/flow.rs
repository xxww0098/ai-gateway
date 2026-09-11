//! hold 生命周期（harness 驱动）的集成测试：预检、结算/释放与并发去重的完整路径。
//!
//! 断言基座与纯决策工具都住在父模块（[super]）里；本文件只关心
//! 「真实请求流过网关时，账本被恰当地触碰」。

use super::*;

// ---------------------------------------------------------------- hold_gated

#[tokio::test]
async fn hold_gated_floor_refusal_leaves_no_reservation() {
    let ledger = FakeLedger::with_balance(1.0);
    let admit = ledger
        .hold_gated(1, 0.5, 2.0, "req-a", Duration::from_secs(60))
        .await
        .expect("lookup");
    assert!(matches!(admit, HoldAdmit::Insufficient { .. }));
    assert!(ledger.calls().is_empty());
    assert!(ledger.held_amount("req-a").is_none());
}

#[tokio::test]
async fn hold_gated_quotes_the_available_balance_on_floor_refusal() {
    let ledger = FakeLedger::with_balance(3.75);
    let admit = ledger
        .hold_gated(1, 1.0, 5.0, "req-b", Duration::from_secs(60))
        .await
        .expect("lookup");
    assert_eq!(admit, HoldAdmit::Insufficient { available: 3.75 },);
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
async fn hold_passes_the_users_concurrency_cap_to_the_limiter() {
    let harness = Harness::build();
    harness.directory.concurrency.lock().insert(TEST_USER_ID, 3);

    send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(
        harness.rate_limiter.seen_user_concurrency.lock().as_slice(),
        &[3],
        "the limiter must see users.concurrency, not only the YAML default",
    );
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
async fn a_client_trace_id_does_not_collapse_two_holds_into_one() {
    // Lua treats a repeated hold member as success without adding funds.
    // The two requests have to overlap: if the first settled before the
    // second reserved, the member would already be gone and a reused
    // client id would look like a fresh hold.
    use std::sync::Arc;

    use axum::Router;
    use axum::http::HeaderValue;
    use axum::routing::post;
    use tower::ServiceExt;

    let harness = Harness::build();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let state = harness.state.clone();
    let handler_barrier = Arc::clone(&barrier);
    let router = Router::new()
        .route(
            "/v1/chat/completions",
            post(move || {
                let barrier = Arc::clone(&handler_barrier);
                async move {
                    barrier.wait().await;
                    StatusCode::OK
                }
            }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::kernel::layer,
        ))
        .with_state(state);

    let mut tasks = Vec::new();
    for _ in 0..2 {
        let router = router.clone();
        tasks.push(tokio::spawn(async move {
            let mut request = signed_request("/v1/chat/completions", chat_body("gpt-4o"));
            request
                .headers_mut()
                .insert(
                    crate::hold::TRACE_HEADER,
                    HeaderValue::from_static("shared-trace"),
                );
            let response = router.oneshot(request).await.expect("router responds");
            let echoed = response
                .headers()
                .get(crate::hold::TRACE_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            (response.status(), echoed)
        }));
    }

    let mut ids = Vec::new();
    for task in tasks {
        let (status, echoed) = task.await.expect("request finishes");
        assert_eq!(status, StatusCode::OK);
        assert_ne!(
            echoed, "shared-trace",
            "the hold key must not be the client-supplied trace id",
        );
        ids.push(echoed);
    }
    assert_ne!(ids[0], ids[1], "each request must mint its own hold key");
    assert_eq!(
        harness
            .ledger
            .calls()
            .iter()
            .filter(|c| matches!(c, LedgerCall::Hold { .. }))
            .count(),
        2,
        "two overlapping bodies sharing a client trace must still reserve twice",
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
async fn an_idempotency_store_outage_on_check_refuses_before_reserving() {
    let harness = Harness::build();
    *harness.idempotency.fail_reads.lock() = true;

    let (status, body) = send(harness.stub_router(StatusCode::OK), keyed_request("k-1")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"].as_str(), Some("idempotency_unavailable"));
    assert!(
        harness.ledger.calls().is_empty(),
        "a store outage on check must not create a hold",
    );
}

#[tokio::test]
async fn an_idempotency_store_outage_on_claim_releases_the_reservation() {
    let harness = Harness::build();
    *harness.idempotency.fail_writes.lock() = true;

    let (status, body) = send(harness.stub_router(StatusCode::OK), keyed_request("k-1")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"].as_str(), Some("idempotency_unavailable"));
    assert!(
        harness.ledger.holds.lock().is_empty(),
        "a store outage on claim must not leave a live hold",
    );
    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Hold { .. })),
        "the reservation was created before the claim failed",
    );
    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "the reservation must be released when the claim cannot be taken",
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
        async fn hold_gated(
            &self,
            user_id: crate::ports::Id,
            amount: f64,
            min_available: f64,
            request_id: &str,
            ttl: Duration,
        ) -> Result<crate::ports::HoldAdmit, crate::ports::BillingError> {
            // 余额读失败按 0 处理（fail-closed）：与生产 `SharedLedger::hold_gated`
            // 对 Redis 抖动的姿态一致 —— 门先拒，绝不落 hold。
            let available = self.available_balance(user_id).await.unwrap_or(0.0);
            if available < min_available {
                return Ok(crate::ports::HoldAdmit::Insufficient { available });
            }
            self.0
                .hold_gated(user_id, amount, min_available, request_id, ttl)
                .await
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

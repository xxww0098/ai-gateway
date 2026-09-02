//! The middleware itself, end to end: ordering, admission, the money key,
//! and idempotent replay.

use super::*;

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
    let quote = harness.calc.quote(&peek.price_key, 1.0);
    let hold_amount = quote.estimate_with_tokens(peek.input_tokens, peek.max_tokens, peek.stream);
    let upper_bound = preflight_upper_bound(&quote, peek.max_tokens, peek.stream, hold_amount);
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
    harness.quota.seed(quota.clone()).await;
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

/// 一个大到计费看不见的 body 走完 hold 之后，handler 到底拿到了什么。
struct Handled {
    /// handler 从 [`InboundBody`] 里抽干的字节 —— 也就是会被转发出去的那份。
    forwarded: Bytes,
    /// 计费那一次解析看得见 body 吗。
    body_visible: bool,
}

/// 挂着真实中间件栈、但 handler 会把入站 body 抽干并记下来的路由。
///
/// [`Harness::stub_router`] 的 handler 不看请求，而这里要断言的正是
/// **转发出去的字节**，所以这一条自己造。
fn recording_router(harness: &Harness, seen: Arc<Mutex<Option<Handled>>>) -> axum::Router {
    use http_body_util::BodyExt as _;

    let state = harness.state.clone();
    axum::Router::new()
        .route(
            "/v1/chat/completions",
            axum::routing::post(move |req: axum::extract::Request| {
                let seen = Arc::clone(&seen);
                async move {
                    let body = req
                        .extensions()
                        .get::<InboundBody>()
                        .cloned()
                        .and_then(InboundBody::take)
                        .expect("hold 必须把入站 body 交出来");
                    let body_visible = req
                        .extensions()
                        .get::<RequestSpec>()
                        .is_some_and(|spec| spec.body_visible);
                    let forwarded = match body {
                        gw_relay::RelayBody::Buffered(bytes) => bytes,
                        gw_relay::RelayBody::Streaming(body) => {
                            body.collect().await.expect("流不该失败").to_bytes()
                        }
                    };
                    *seen.lock() = Some(Handled {
                        forwarded,
                        body_visible,
                    });
                    (
                        StatusCode::OK,
                        axum::Json(serde_json::json!({"stub": true})),
                    )
                        .into_response()
                }
            }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::kernel::layer,
        ))
        .with_state(state)
}

/// 缺陷 #2 的守护测试：**计费降级，转发不降级**。
///
/// 收敛前这里是 413 —— 一个计费实现细节（「我要把整份 JSON 收进内存才能报价」）
/// 泄漏成了转发能力限制，Claude Code 的长会话必然撞上它。
/// 现在超阈值只意味着 `body_visible == false`：请求照转、hold 照建、
/// 预扣退化成保守估算。截断依然是禁止的 —— 转发出去的必须是完整的原始字节。
#[tokio::test]
async fn a_direct_body_over_the_peek_limit_is_forwarded_and_still_held() {
    let harness = Harness::build();
    // 一份 4 MiB 的提示词按 fixture 的费率就是几百美元 —— 那是**预扣在正常工作**，
    // 不是本条要测的东西。把余额抬到不挡路，让断言只关于 peek 上限。
    *harness.ledger.balance.lock() = 1_000_000.0;
    let seen = Arc::new(Mutex::new(None));

    // 可辨认的载荷：全 `x` 会让「前缀拼错顺序」这类错误看不出来。
    let payload: Bytes = (0..crate::body::BILLING_PEEK_LIMIT + 1)
        .map(|i| (i % 251) as u8)
        .collect::<Vec<u8>>()
        .into();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!("Bearer {}", crate::testsupport::TEST_API_KEY),
        )
        .body(axum::body::Body::from(payload.clone()))
        .expect("request builds");

    let (status, _) = send(recording_router(&harness, Arc::clone(&seen)), request).await;
    assert_eq!(status, StatusCode::OK, "超 peek 上限不再是 413");

    let calls = harness.ledger.calls();
    assert!(
        calls.iter().any(|c| matches!(c, LedgerCall::Hold { .. })),
        "看不见 body 也必须 mint 操作 id 并准入：{calls:?}",
    );

    let handled = seen.lock().take().expect("handler 跑过了");
    assert!(
        !handled.body_visible,
        "计费必须显式面对『我看不见这份 JSON』，而不是把它当成空 body",
    );
    assert_eq!(
        handled.forwarded, payload,
        "转发出去的必须是完整的原始 body，一个字节都不许截",
    );
}

/// 降级的代价必须是**往大了估**。
///
/// 预留得比认可的责任少，正是大请求结算成欠款的来路（AGENTS.md「计费身份」）。
/// 所以：看不见 body 时预留的钱不能少于一个刚好卡在阈值内的请求；
/// 客户端自报了更大的 `Content-Length` 时，还要跟着更大。
#[tokio::test]
async fn an_invisible_body_reserves_at_least_as_much_as_a_visible_one() {
    async fn reserved_for(len: usize, declared: Option<usize>) -> f64 {
        let harness = Harness::build();
        *harness.ledger.balance.lock() = 1_000_000.0;
        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .header(
                "authorization",
                format!("Bearer {}", crate::testsupport::TEST_API_KEY),
            );
        if let Some(declared) = declared {
            builder = builder.header("content-length", declared.to_string());
        }
        let request = builder
            .body(axum::body::Body::from(vec![b'x'; len]))
            .expect("request builds");
        send(harness.stub_router(StatusCode::OK), request).await;
        harness
            .ledger
            .calls()
            .into_iter()
            .find_map(|call| match call {
                LedgerCall::Hold { amount, .. } => Some(amount),
                _ => None,
            })
            .expect("三条路都必须建 hold")
    }

    let limit = crate::body::BILLING_PEEK_LIMIT;
    let visible = reserved_for(limit, None).await;
    let invisible = reserved_for(limit + 1, None).await;
    assert!(
        invisible >= visible,
        "看不见 body 时预留了更少的钱：{invisible} < {visible}",
    );

    let declared = reserved_for(limit + 1, Some(limit * 4)).await;
    assert!(
        declared > invisible,
        "客户端自报的 Content-Length 更大时预扣必须跟着更大：{declared} 未超过 {invisible}",
    );
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
        async fn renew_lease(
            &self,
            user_id: crate::ports::Id,
            operation: &gw_ledger::BillingOperationId,
        ) -> Result<f64, crate::ports::BillingError> {
            self.0.renew_lease(user_id, operation).await
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
fn traced(
    mut request: axum::http::Request<axum::body::Body>,
    trace: &str,
) -> axum::http::Request<axum::body::Body> {
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
    let (hold_amount, upper_bound) =
        compute_reservation(&peek, &harness.calc.quote(&peek.price_key, 1.0));
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

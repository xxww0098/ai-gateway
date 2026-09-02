//! 不计费的端点：模型目录、token 计数、用量查询。
//!
//! 它们与派发链共用中间件栈，但**一分钱都不该收** —— 收敛前 `GET /v1/models`
//! 按 fallback estimate 向租户收约 $0.004。

use super::*;

// ---------------------------------------------------------------- 零成本端点

#[tokio::test]
async fn listing_models_costs_the_tenant_nothing() {
    let harness = Harness::build();
    let (status, body) = send_settled(&harness, signed_get("/v1/models")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["object"].as_str(), Some("list"));
    let listed: Vec<&str> = body["data"]
        .as_array()
        .expect("a data array")
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    let catalogued: Vec<String> = harness
        .catalog
        .models
        .lock()
        .iter()
        .map(|m| m.id.clone())
        .collect();
    assert_eq!(listed, catalogued, "the catalogue is served verbatim");
    // 收敛前这里会先预扣、再因为「响应没有 usage 信封」落 fallback 结算，
    // 于是一次**纯 DB 读**按 LLM 价格收钱。已移出计费范围。
    assert!(
        harness.ledger.calls().is_empty(),
        "一次目录读取不该碰账本：{:?}",
        harness.ledger.calls(),
    );
    assert!(
        harness.usage_store.settled_costs().is_empty(),
        "更不该结算出一个金额",
    );
}

#[tokio::test]
async fn counting_tokens_costs_the_tenant_nothing() {
    let harness = Harness::build_with(vec![auth_record("acct-1", "claude")]);
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/messages/count_tokens")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .body(axum::body::Body::from(
            chat_body("claude-sonnet-5").to_string(),
        ))
        .expect("request builds");

    let (status, _) = send_settled(&harness, request).await;
    assert_eq!(status, StatusCode::OK);

    // Anthropic 自己对 token 计数收 0；它的回复是裸 `{"input_tokens": N}`，
    // 没有 `usage` 包装，所以收敛前 usage 解析器报 absent、fallback 结算按
    // **那个模型的真实费率**收钱 —— 一次免费调用被按 LLM 价格计价。
    assert!(
        harness.ledger.calls().is_empty(),
        "count_tokens 不该碰账本：{:?}",
        harness.ledger.calls(),
    );
    assert!(harness.usage_store.settled_costs().is_empty());
}

#[tokio::test]
async fn the_endpoints_moved_out_of_billing_are_still_behind_authentication() {
    // 「不计费」不等于「不鉴权」。两道门共用 `is_proxy_path`，
    // 但计费那道额外排除了 GET 与 count_tokens —— 排除的是**收钱**，不是**认人**。
    let harness = Harness::build();
    for (method, path) in [
        ("GET", "/v1/models"),
        ("GET", "/v1/usage"),
        ("POST", "/v1/messages/count_tokens"),
    ] {
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                chat_body("claude-sonnet-5").to_string(),
            ))
            .expect("request builds");
        let (status, _) = send_settled(&harness, request).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} 放行了匿名请求"
        );
    }
}

#[tokio::test]
async fn this_router_can_be_merged_with_one_that_owns_the_metrics_endpoint() {
    // `/metrics/prometheus` belongs to the composition root. Registering it
    // here too makes `Router::merge` panic on the duplicate and
    // the process never finishes booting — so the guard is that the merge is
    // simply possible.
    let harness = Harness::build();
    let host: axum::Router = axum::Router::new().route(
        "/metrics/prometheus",
        axum::routing::get(|| async { "agw_v1_requests_total 0" }),
    );

    let merged = host.merge(harness.router());

    let request = axum::http::Request::builder()
        .uri("/metrics/prometheus")
        .body(axum::body::Body::empty())
        .expect("request builds");
    use tower::ServiceExt;
    let response = merged.oneshot(request).await.expect("responds");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the host's metrics route must survive the merge",
    );
}

#[tokio::test]
async fn the_gauges_this_crate_observes_are_pushed_to_the_host_not_exported_here() {
    // The benched count is read at scrape time; the scrape lives in another
    // crate now, so the value travels through the sink instead.
    let harness = Harness::build();
    harness.health.record_result("acct-1", false, None);
    harness.health.record_result("acct-1", false, None);
    harness.health.record_result("acct-1", false, None);

    harness.state.publish_gauges();

    assert_eq!(
        harness.metrics.benched(),
        harness.health.benched_count(),
        "the gauge must reflect what the pool actually benched",
    );
}

#[tokio::test]
async fn an_error_status_the_upstream_answered_with_fails_over_too() {
    // A relayed 503 is a *response*, not a raised error, and it must still
    // cost that account its turn rather than reaching the client as a 503 the
    // next credential would have served.
    let harness = Harness::build_with(vec![
        auth_record("acct-1", "openai"),
        auth_record("acct-2", "openai"),
    ]);
    harness.transport.queue(Ok(CannedResponse::status(503)));
    harness.transport.queue(Ok(CannedResponse::ok(1, 1)));

    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.transport.call_count(), 2);
    assert_eq!(harness.usage_store.settled_costs().len(), 1);
}

#[tokio::test]
async fn an_upstream_error_response_keeps_its_headers_on_the_way_back() {
    // The defect this pins: turning a non-2xx into an error threw the headers
    // away, so a 429's `retry-after` never reached the client and its SDK
    // backed off on its own guess instead of on what the upstream asked for.
    let harness = Harness::build();
    let mut canned = CannedResponse::status(429);
    canned
        .headers
        .insert("retry-after", "12".parse().expect("header"));
    canned
        .headers
        .insert("x-ratelimit-remaining", "0".parse().expect("header"));
    // Three, because 429 is retryable and the pool gets its attempts.
    harness.transport.queue(Ok(canned));

    let response = {
        use tower::ServiceExt;
        harness
            .router()
            .oneshot(signed_request("/v1/chat/completions", chat_body("gpt-4o")))
            .await
            .expect("router responds")
    };

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers().get("retry-after").map(|v| v.as_bytes()),
        Some(&b"12"[..]),
        "an upstream 429 must arrive with the backoff it asked for",
    );
    assert_eq!(
        response
            .headers()
            .get("x-ratelimit-remaining")
            .map(|v| v.as_bytes()),
        Some(&b"0"[..]),
    );
}

#[tokio::test]
async fn listing_models_exposes_catalog_capabilities_verbatim() {
    use crate::ports::{ModelEntry, ModelReasoning, ModelReasoningEffort};

    let harness = Harness::build();
    {
        let mut models = harness.catalog.models.lock();
        models.clear();
        models.push(ModelEntry {
            id: "vision-thinker".to_owned(),
            created: 1,
            owned_by: "openai".to_owned(),
            context_length: Some(128_000),
            max_output_tokens: Some(16_384),
            input_modalities: vec!["text".into(), "image".into()],
            reasoning: Some(ModelReasoning {
                efforts: vec![
                    ModelReasoningEffort {
                        id: "low".into(),
                        name: "Low".into(),
                    },
                    ModelReasoningEffort {
                        id: "high".into(),
                        name: "High".into(),
                    },
                ],
                default_effort: Some("high".into()),
            }),
        });
        models.push(ModelEntry {
            id: "text-only".to_owned(),
            created: 2,
            owned_by: "openai".to_owned(),
            context_length: Some(8_192),
            max_output_tokens: Some(2_048),
            input_modalities: vec!["text".into()],
            reasoning: None,
        });
    }

    let (status, body) = send(harness.router(), signed_get("/v1/models")).await;
    assert_eq!(status, StatusCode::OK);
    let data = body["data"].as_array().expect("data array");
    let vision = data.iter().find(|m| m["id"] == "vision-thinker").unwrap();
    let text = data.iter().find(|m| m["id"] == "text-only").unwrap();

    assert_eq!(vision["context_length"], 128_000);
    assert_eq!(vision["max_output_tokens"], 16_384);
    assert_eq!(
        vision["input_modalities"],
        serde_json::json!(["text", "image"])
    );
    assert_eq!(
        vision["reasoning"]["efforts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["low", "high"]
    );

    assert_eq!(text["context_length"], 8_192);
    assert_eq!(text["max_output_tokens"], 2_048);
    assert_eq!(text["input_modalities"], serde_json::json!(["text"]));
    assert!(text.get("reasoning").is_none());
    assert!(
        text["input_modalities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m != "image")
    );
}

#[tokio::test]
async fn fetching_one_catalogued_model_returns_that_entry() {
    let harness = Harness::build();
    let expected = harness.catalog.models.lock()[0].clone();
    let (status, body) = send(
        harness.router(),
        signed_get(&format!("/v1/models/{}", expected.id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"].as_str(), Some(expected.id.as_str()));
    assert_eq!(body["object"].as_str(), Some("model"));
    assert_eq!(body["owned_by"].as_str(), Some(expected.owned_by.as_str()));
    assert_eq!(body["created"].as_i64(), Some(expected.created));
}

#[tokio::test]
async fn fetching_a_missing_or_hidden_model_is_not_found() {
    let harness = Harness::build();
    let (status, _) = send(
        harness.router(),
        signed_get("/v1/models/not-in-the-catalogue"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn fetching_one_model_does_not_walk_the_whole_catalogue() {
    let harness = Harness::build();
    let id = harness.catalog.models.lock()[0].id.clone();
    let listed_before = harness.catalog.list_calls();
    let (status, _) = send(harness.router(), signed_get(&format!("/v1/models/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.catalog.list_calls(),
        listed_before,
        "detail must look up one id instead of listing every model",
    );
    assert!(
        harness.catalog.get_calls() > 0,
        "the point-lookup path must have been used",
    );
}

// ---------------------------------------------------------------- GET /v1/usage

#[tokio::test]
async fn reading_usage_without_a_bearer_is_unauthorized() {
    let harness = Harness::build();
    let request = axum::http::Request::builder()
        .uri("/v1/usage")
        .body(axum::body::Body::empty())
        .expect("request builds");
    let (status, _) = send_settled(&harness, request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        harness.ledger.calls().is_empty(),
        "匿名查询不该碰账本：{:?}",
        harness.ledger.calls(),
    );
}

#[tokio::test]
async fn an_api_key_sees_wallet_balance_and_today_model_tokens() {
    let harness = Harness::build();
    let balance = 23.75;
    *harness.ledger.balance.lock() = balance;

    let mine = |model: &str, input: i64, output: i64| UsageLogEntry {
        user_id: TEST_USER_ID,
        model: model.to_owned(),
        input_tokens: input,
        output_tokens: output,
        ..UsageLogEntry::default()
    };
    let seeded = [
        mine("alpha", 10, 4),
        mine("alpha", 3, 1),
        mine("beta", 8, 2),
        UsageLogEntry {
            user_id: TEST_USER_ID + 1,
            model: "other".to_owned(),
            input_tokens: 99,
            output_tokens: 99,
            ..UsageLogEntry::default()
        },
    ];
    for entry in &seeded {
        harness
            .usage_store
            .insert_usage_log(entry)
            .await
            .expect("seed usage log");
    }

    let (status, body) = send_settled(&harness, signed_get("/v1/usage")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["object"].as_str(), Some("usage"));

    let quoted = body["balance_usd"].as_f64().expect("balance_usd");
    let ledger_balance = harness
        .ledger
        .available_balance(TEST_USER_ID)
        .await
        .expect("ledger balance");
    assert_eq!(quoted, ledger_balance);
    assert_eq!(quoted, balance);

    let expected = fold_model_usage(seeded.iter().filter(|entry| entry.user_id == TEST_USER_ID));
    let got = body["models"].as_array().expect("models array");
    assert_eq!(got.len(), expected.len());
    for (row, want) in got.iter().zip(expected.iter()) {
        assert_eq!(row["model"].as_str(), Some(want.model.as_str()));
        assert_eq!(row["requests"].as_i64(), Some(want.requests));
        assert_eq!(row["tokens_in"].as_i64(), Some(want.tokens_in));
        assert_eq!(row["tokens_out"].as_i64(), Some(want.tokens_out));
        assert_eq!(row["tokens"].as_i64(), Some(want.tokens()));
    }
    assert!(
        got.iter().all(|row| row["model"].as_str() != Some("other")),
        "别人的用量不能漏进来",
    );

    assert!(
        harness.ledger.calls().is_empty(),
        "用量查询是只读的，不该 Hold/Settle/Release：{:?}",
        harness.ledger.calls(),
    );
    assert!(harness.usage_store.settled_costs().is_empty());
}

#[tokio::test]
async fn usage_with_no_logs_reports_an_empty_model_list() {
    let harness = Harness::build();
    let (status, body) = send_settled(&harness, signed_get("/v1/usage")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["models"].as_array().map(Vec::is_empty),
        Some(true),
        "没有今日用量必须是空数组，不能缺字段",
    );
    assert!(
        body.get("subscription").is_none() && body.get("has_outstanding_debt").is_none(),
        "这条接口只报余额和模型 token，不应再带订阅或欠款",
    );
}

#[test]
fn an_empty_model_name_folds_to_unknown() {
    let entry = UsageLogEntry {
        model: "  ".to_owned(),
        input_tokens: 1,
        output_tokens: 2,
        ..UsageLogEntry::default()
    };
    let folded = fold_model_usage(std::iter::once(&entry));
    assert_eq!(folded.len(), 1);
    assert_eq!(folded[0].model, "unknown");
    assert_eq!(folded[0].tokens(), entry.input_tokens + entry.output_tokens);
}

#[tokio::test]
async fn the_tenant_credential_never_reaches_the_upstream() {
    // The relay strips every carrier it might have read — `authorization`,
    // `x-api-key`, `x-goog-api-key` — and sets the *upstream* credential in
    // its place. The tenant's own `agw-` key is for the client leg only, and
    // forwarding it is a credential leak into someone else's logs.
    let harness = Harness::build();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .header("x-custom", "kept")
        .body(axum::body::Body::from(chat_body("gpt-4o").to_string()))
        .expect("request builds");

    let (status, _) = send_settled(&harness, request).await;
    assert_eq!(status, StatusCode::OK);

    let outbound = harness.transport.only_headers();
    let authorization = outbound
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .expect("the upstream credential must be set");
    assert!(
        !authorization.contains(TEST_API_KEY),
        "the tenant credential reached the upstream: {authorization}",
    );
    assert_eq!(
        outbound.get("x-custom").and_then(|v| v.to_str().ok()),
        Some("kept"),
        "an unrelated inbound header must still be forwarded",
    );
}

//! Upstream dispatch for the `/v1` dialects: model-to-provider resolution,
//! cross-account failover, the streaming relay, and the middleware order the
//! whole billing pipeline depends on.

use axum::body::Bytes;

use super::*;
use crate::ports::{BillingLedger, UsageLogEntry, UsageStore, fold_model_usage};
use crate::testsupport::TEST_USER_ID;

// ---------------------------------------------------------------- routing

use gw_relay::endpoint::matrix;
use gw_relay::endpoint::upstream::{InMemoryChannelResolver, SelectionLevel};

/// 一个只认得 `house` 这一个渠道、并且把 `house` 映到 claude 的目录。
/// 断言里不出现任何在被测源码中写死的字面量：渠道名与映射都是**测试自己造的**。
fn resolver() -> InMemoryChannelResolver {
    InMemoryChannelResolver::new()
        .with_model("house-model", ["house"])
        .with_channel("house", matrix::Provider::Claude)
}

#[test]
fn without_a_resolver_the_chain_is_byte_for_byte_the_old_prefix_guess() {
    // 灰度的承重点：不装 resolver 时，四级链必须与收敛前的
    // `provider_candidates()` 结果完全一致 —— 否则「出了问题一定不是路由改的」
    // 这条判断就不成立了。对拍的参照是 gw-relay 自己保留的那份前缀表。
    for (surface, model) in [
        (Surface::OpenAiCompletions, "claude-sonnet-5"),
        (Surface::OpenAiCompletions, "gpt-4o"),
        (Surface::OpenAiCompletions, ""),
        (Surface::AnthropicMessages, "gemini-2.5-pro"),
        (Surface::AnthropicMessages, "mystery"),
        (Surface::OpenAiResponses, "something-codex-ish"),
    ] {
        let selection = select_upstreams(surface, model, None);
        assert_eq!(
            selection.candidates,
            gw_relay::endpoint::upstream::prefix_guess(surface, model),
            "{surface:?} / {model:?} 的兜底结果与前缀表不一致",
        );
        assert_eq!(selection.level, SelectionLevel::PrefixGuess);
    }
}

#[test]
fn the_catalogue_outranks_the_prefix_table() {
    // 测的是**性质**，不是某个渠道名：目录里写了什么，路由就走什么。
    // `house-model` 命中不了任何前缀分支，所以前缀表只会给出入口默认值；
    // 目录把它指到另一个 provider，四级链必须听目录的。
    let resolver = resolver();
    let guessed =
        gw_relay::endpoint::upstream::prefix_guess(Surface::OpenAiCompletions, "house-model");
    let selection = select_upstreams(Surface::OpenAiCompletions, "house-model", Some(&resolver));

    assert_eq!(selection.level, SelectionLevel::Catalog);
    assert_ne!(selection.candidates, guessed, "目录命中时不该再看前缀表",);
    assert_eq!(selection.candidates, vec![matrix::Provider::Claude]);
}

#[test]
fn an_explicit_channel_prefix_is_stripped_off_the_model_the_upstream_sees() {
    // L1 的张力：剥前缀选路由，但上游只认识剥掉之后的名字。
    let resolver = resolver();
    let selection = select_upstreams(
        Surface::OpenAiCompletions,
        "house/some-model",
        Some(&resolver),
    );

    assert_eq!(selection.level, SelectionLevel::ExplicitPrefix);
    assert_eq!(
        selection.upstream_model.as_deref(),
        Some("some-model"),
        "渠道前缀是给网关看的，不该原样转给上游",
    );
}

#[test]
fn stripping_the_channel_prefix_rewrites_the_body_the_upstream_receives() {
    // `upstream_model` 只是算出了名字；真正把它落到 body 上是本 crate 的职责
    // （gw-relay 的合同不允许它改 body）。不改的话上游会收到一个带前缀的模型名。
    let original = serde_json::json!({
        "model": "house/some-model",
        "messages": [{"role": "user", "content": "hi"}],
        "temperature": 0.25,
    });
    let rewritten = rewrite_model(&Bytes::from(original.to_string()), "some-model");
    let parsed: serde_json::Value = serde_json::from_slice(&rewritten).expect("still json");

    assert_eq!(parsed["model"].as_str(), Some("some-model"));
    assert_eq!(
        parsed["messages"], original["messages"],
        "改写只碰 model 这一个键",
    );
    assert_eq!(parsed["temperature"], original["temperature"]);
}

#[test]
fn a_body_that_is_not_a_json_object_is_left_exactly_as_it_arrived() {
    // 改不了就不改：绝不把一个转发不出去的 body 变成另一个转发不出去的 body。
    let raw = Bytes::from_static(b"not json at all");
    assert_eq!(rewrite_model(&raw, "whatever"), raw);
}

// ------------------------------------------------------- 超 peek 上限的直通体

/// 缺陷 #2 的另一半：**上游收到的是完整的原始 body，而且是边收边转的**。
///
/// 计费看不见它（模型名为空、预扣走保守估算），转发却不降级 ——
/// 超阈值的请求在网关里**不会被再完整缓冲一遍**，字节一个不少地流到上游。
/// 收敛前这条请求在 hold 层就是 413，永远走不到这里。
#[tokio::test]
async fn an_oversized_direct_body_reaches_the_upstream_whole_and_streamed() {
    let harness = Harness::build();
    // 4 MiB 提示词按 fixture 费率是几百美元；余额抬高，让断言只关于转发。
    *harness.ledger.balance.lock() = 1_000_000.0;

    // 可辨认载荷：全 `x` 会让「前缀与剩余部分拼错顺序」看不出来。
    let payload = Bytes::from(
        (0..crate::body::BILLING_PEEK_LIMIT + 1)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<u8>>(),
    );
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .body(axum::body::Body::from(payload.clone()))
        .expect("request builds");

    let (status, _) = send_settled(&harness, request).await;
    assert_eq!(status, StatusCode::OK, "超 peek 上限不再是 413");

    let (body, streamed) = harness.transport.only_body();
    assert_eq!(body, payload, "上游必须收到完整的原始字节");
    assert!(streamed, "超阈值的 body 不许在网关里被重新缓冲一遍");
}

// ---------------------------------------------------------------- 15 格矩阵

/// 一个把测试自造的渠道钉死到 gemini 的目录。
///
/// 用它而不是靠模型名前缀，是因为**没有目录时矩阵的 400 分支根本到不了**：
/// L4 兜底总会把入口的默认 provider 追加到候选末尾，而入口默认永远是直通格，
/// 于是候选列表里必有一个能转发的。这条性质由
/// [`the_prefix_only_chain_always_keeps_a_passthrough_escape_hatch`] 钉住。
fn gemini_only_resolver() -> Arc<dyn gw_relay::endpoint::upstream::ChannelResolver> {
    Arc::new(
        InMemoryChannelResolver::new()
            .with_model("house-model", ["house"])
            .with_channel("house", matrix::Provider::Gemini),
    )
}

#[test]
fn the_prefix_only_chain_always_keeps_a_passthrough_escape_hatch() {
    // L4 追加的入口默认 provider 永远落在直通格上，所以「不装 resolver」时
    // 任何请求都至少有一个能转发的候选 —— 矩阵的 400 分支只会在**目录或显式
    // 前缀明确说了这个模型属于哪个渠道**时才触发。这不是巧合，是灰度的保证：
    // 不装 resolver 就不会有任何请求因为矩阵而多出一个 400。
    for surface in [
        Surface::OpenAiCompletions,
        Surface::OpenAiResponses,
        Surface::AnthropicMessages,
    ] {
        for model in ["gemini-2.5-pro", "claude-sonnet-5", "gpt-4o", "mystery", ""] {
            let selection = select_upstreams(surface, model, None);
            let (routable, _) = partition_routable(surface, &selection, model);
            assert!(
                !routable.is_empty(),
                "{surface:?} / {model:?} 在纯前缀猜测下失去了可转发候选",
            );
        }
    }
}

#[tokio::test]
async fn an_openai_request_is_translated_to_google_and_back() {
    let harness = Harness::build_routed(
        vec![auth_record("acct-1", "gemini")],
        Some(gemini_only_resolver()),
    );
    harness.transport.queue(Ok(CannedResponse {
        status: 200,
        headers: http::HeaderMap::new(),
        frames: vec![Bytes::from_static(
            br#"{"candidates":[{"content":{"parts":[{"text":"hello"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":5}}"#,
        )],
    }));

    let (status, body) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("house-model")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["choices"][0]["message"]["content"].as_str(),
        Some("hello")
    );
    let request = harness.gemini.only_request();
    let translated: serde_json::Value =
        serde_json::from_slice(&request.payload).expect("translated Google JSON");
    assert!(translated.get("contents").is_some());
    assert!(translated.get("messages").is_none());

    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].input_tokens, 12);
    assert_eq!(logs[0].output_tokens, 5);
}

#[tokio::test]
async fn a_translation_request_error_releases_the_reservation_before_outbound_io() {
    let harness = Harness::build_routed(
        vec![auth_record("acct-1", "gemini")],
        Some(gemini_only_resolver()),
    );
    let mut body = chat_body("house-model");
    body["n"] = serde_json::json!(2);

    let (status, _) = send_settled(&harness, signed_request("/v1/chat/completions", body)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(harness.transport.call_count(), 0);
    assert!(harness.usage_store.settled_costs().is_empty());
    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|call| matches!(call, LedgerCall::Release { .. }))
    );
}

#[tokio::test]
async fn a_passthrough_cell_still_reaches_its_upstream() {
    // 矩阵不是一道全拒的闸：P0 的 5 个直通格必须原样通过。
    let harness = Harness::build();
    harness.transport.queue(Ok(CannedResponse::ok(10, 20)));

    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.provider.call_count(), 1);
    assert_eq!(
        harness.provider.dispatched(),
        vec![("gpt-4o".to_owned(), false)],
        "直通格的模型名与流式标志原样交给上游",
    );
}

// ---------------------------------------------------------------- 凭证快照

#[tokio::test]
async fn the_credential_table_is_not_reloaded_once_per_request() {
    // 热点 #5：收敛前每个请求、每个候选 provider 都调一次 `auth_store.list()`
    // —— 对 Postgres 后端那是一次全表 SELECT + 每行一次 AES-GCM 解密 +
    // 克隆整份凭证表，然后当场丢掉除一个 provider 之外的全部结果。
    //
    // 这条测的是**性质**（加载次数不随请求数线性增长），不是某个具体的 TTL 值。
    let harness = Harness::build();
    const REQUESTS: usize = 12;
    for _ in 0..REQUESTS {
        harness.transport.queue(Ok(CannedResponse::ok(1, 1)));
        send_settled(
            &harness,
            signed_request("/v1/chat/completions", chat_body("gpt-4o")),
        )
        .await;
    }

    assert_eq!(harness.provider.call_count(), REQUESTS);
    let loads = harness.auth_store.list_calls();
    assert!(
        loads < REQUESTS,
        "凭证表被加载了 {loads} 次 / {REQUESTS} 个请求：快照没有生效",
    );
}

#[tokio::test]
async fn raw_query_bytes_reach_the_planner_without_round_tripping() {
    let harness = Harness::build();
    harness.transport.queue(Ok(CannedResponse::ok(1, 1)));
    let raw = "tag=a%20b&plus=a+b&pct=%25&empty=&flag&key=a&key=b";
    let path = format!("/v1/chat/completions?{raw}");
    let (status, _) = send_settled(&harness, signed_request(&path, chat_body("gpt-4o"))).await;

    assert_eq!(status, StatusCode::OK);
    let request = harness.provider.only_request();
    assert_eq!(request.raw_query.as_deref(), Some(raw));
    assert!(request.query.is_empty());
}

#[test]
fn retry_after_seconds_are_bounded_and_forwarded_to_health() {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::RETRY_AFTER,
        http::HeaderValue::from_static("17"),
    );
    assert_eq!(
        retry_after_hint(&headers),
        Some(std::time::Duration::from_secs(17))
    );

    headers.insert(
        http::header::RETRY_AFTER,
        http::HeaderValue::from_static("999999999"),
    );
    assert_eq!(
        retry_after_hint(&headers),
        Some(std::time::Duration::from_secs(24 * 60 * 60)),
    );
}

#[test]
fn only_account_level_statuses_are_worth_another_credential() {
    assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
    assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
    assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
    assert!(
        !is_retryable_status(StatusCode::BAD_REQUEST),
        "a malformed request fails identically on every account",
    );
    assert!(!is_retryable_status(StatusCode::OK));
}

// ---------------------------------------------------------------- ordering

#[tokio::test]
async fn authentication_runs_before_billing_so_an_anonymous_call_costs_nothing() {
    // Blocker B1: with the layers the other way round every /v1 request aborts
    // with a pre-auth 401 and the billing hot path never executes.
    let harness = Harness::build();
    let (status, _) = send_settled(
        &harness,
        anonymous_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        harness.ledger.calls().is_empty(),
        "the hold layer must never see an unauthenticated request",
    );
    assert_eq!(
        harness.provider.call_count(),
        0,
        "and no upstream call may be made either",
    );
}

#[tokio::test]
async fn an_authenticated_call_reserves_dispatches_and_settles_in_that_order() {
    let harness = Harness::build();
    harness.transport.queue(Ok(CannedResponse::ok(100, 250)));

    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.provider.call_count(), 1);
    assert!(matches!(
        harness.ledger.calls().first(),
        Some(LedgerCall::Hold { .. })
    ));
    assert_eq!(
        harness.usage_store.settled_costs().len(),
        1,
        "exactly one settlement per request",
    );
}

#[tokio::test]
async fn the_reported_usage_is_what_gets_billed() {
    let harness = Harness::build();
    harness.transport.queue(Ok(CannedResponse::ok(100, 250)));
    send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    let logs = harness.usage_store.logs.lock();
    assert_eq!(logs[0].input_tokens, 100);
    assert_eq!(logs[0].output_tokens, 250);
    assert!(!logs[0].failed);
}

#[tokio::test]
async fn an_upstream_without_a_usage_envelope_falls_back_instead_of_billing_zero() {
    let harness = Harness::build();
    harness
        .transport
        .queue(Ok(CannedResponse::ok_without_usage()));
    send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    let costs = harness.usage_store.settled_costs();
    assert_eq!(costs.len(), 1);
    assert!(costs[0] > 0.0, "upstream output must never be free");
}

// ---------------------------------------------------------------- failover

#[tokio::test]
async fn a_failing_account_is_retried_on_a_different_one_and_billed_once() {
    // This is the invariant the channel selector calls out: cross-account
    // retry settles once, on the final response.
    let harness = Harness::build_with(vec![
        auth_record("acct-1", "openai"),
        auth_record("acct-2", "openai"),
    ]);
    harness.transport.queue(Ok(CannedResponse::status(503)));
    harness.transport.queue(Ok(CannedResponse::ok(10, 20)));

    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.transport.call_count(),
        2,
        "the retry must have happened"
    );
    let tried = harness.provider.seen_auth_ids.lock().clone();
    assert_ne!(tried[0], tried[1], "the retry must use a different account");
    assert_eq!(
        harness.usage_store.settled_costs().len(),
        1,
        "failover must not double-bill",
    );
}

#[tokio::test]
async fn a_client_error_is_surfaced_immediately_instead_of_burning_the_pool() {
    let harness = Harness::build_with(vec![
        auth_record("acct-1", "openai"),
        auth_record("acct-2", "openai"),
    ]);
    harness.transport.queue(Ok(CannedResponse::status(400)));

    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(harness.transport.call_count(), 1);
}

#[tokio::test]
async fn a_failed_dispatch_releases_the_reservation() {
    let harness = Harness::build();
    harness.transport.queue(Ok(CannedResponse::status(400)));

    send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "a request that never reached an upstream must not be charged",
    );
    assert!(harness.usage_store.settled_costs().is_empty());
}

#[tokio::test]
async fn an_empty_credential_pool_reports_unavailability_and_charges_nothing() {
    let harness = Harness::build_with(vec![]);
    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(harness.usage_store.settled_costs().is_empty());
    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
    );
}

#[tokio::test]
async fn an_account_that_keeps_failing_across_requests_is_benched_from_the_pool() {
    // Health is tracked across requests, not within one: a single client call
    // only ever tries a given account once.
    let harness = Harness::build_with(vec![auth_record("acct-1", "openai")]);
    for _ in 0..crate::channel::DEFAULT_FAILURE_THRESHOLD {
        harness.transport.queue(Ok(CannedResponse::status(503)));
        let (status, _) = send_settled(
            &harness,
            signed_request("/v1/chat/completions", chat_body("gpt-4o")),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    assert!(
        harness.health.benched_count() > 0,
        "consecutive failures must take the account out of rotation",
    );
    assert!(
        harness
            .breaker
            .recorded
            .lock()
            .iter()
            .any(|(_, success)| !success),
        "and must be reported to the circuit breaker",
    );
}

// ---------------------------------------------------------------- 凭证载体

#[tokio::test]
async fn the_v1_surface_keeps_reading_authorization_and_nothing_else() {
    // 三面收敛把 `x-goog-api-key` / `?key=` 两种载体整个下线了。
    // 它们不是「在别处仍然有效」，而是**不再是凭据** —— 带着它来就是 401。
    let harness = Harness::build();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions?key=agw-testkey")
        .header("content-type", "application/json")
        .header("x-goog-api-key", TEST_API_KEY)
        .body(axum::body::Body::from(chat_body("gpt-4o").to_string()))
        .expect("request builds");

    let (status, _) = send_settled(&harness, request).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(harness.ledger.calls().is_empty());
}

#[tokio::test]
async fn an_anthropic_key_header_still_reaches_the_claude_upstream_on_v1() {
    // 这条是删 `/v1beta` 时最容易被顺手带走的一条不变量：`/v1` 上的
    // `x-api-key` 是 **Anthropic 自己的上游头**，不是租户凭据，必须原样透传。
    // 历史上它由 `access::strip_consumed_credentials` 的
    // `if !path.starts_with("/v1beta/") { return; }` 守着；那个函数已随
    // `/v1beta` 一起删掉，语义只剩这条用例在钉。
    let harness = Harness::build_with(vec![auth_record("acct-1", "claude")]);
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .header("x-api-key", "caller-supplied")
        .body(axum::body::Body::from(
            chat_body("claude-sonnet-5").to_string(),
        ))
        .expect("request builds");

    let (status, _) = send_settled(&harness, request).await;
    assert_eq!(status, StatusCode::OK);

    let forwarded = harness.claude.only_request();
    assert_eq!(
        forwarded
            .headers
            .get("x-api-key")
            .map(|v| v.to_str().expect("ascii")),
        Some("caller-supplied"),
    );
}

// ---------------------------------------------------------------- 已删除的入口

#[tokio::test]
async fn the_six_converged_routes_are_gone_not_merely_unbilled() {
    // 硬删，不是 410 过渡。判定表见 `docs/relay-surface-plan.md` §2。
    let harness = Harness::build();
    let gone = |method: &'static str, path: &'static str| {
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .body(axum::body::Body::from(chat_body("gpt-4o").to_string()))
            .expect("request builds");
        send_settled(&harness, request)
    };

    for (method, path) in [
        ("POST", "/v1/completions"),
        ("POST", "/v1/embeddings"),
        ("GET", "/v1beta/models"),
        ("GET", "/v1beta/models/gemini-2.5-pro"),
        ("POST", "/v1beta/models/gemini-2.5-pro:generateContent"),
    ] {
        let (status, _) = gone(method, path).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} 仍然被路由表接住了",
        );
    }

    // `POST /v1/models/{model}`（Gemini GA 别名）与保留下来的
    // `GET /v1/models/{model}` 共用同一个路径模式，只靠方法区分。
    // 去掉 `.post(...)` 之后 axum 回的是 405（路径在、方法不在），不是 404
    // —— 这是对的，而且比 404 更准确。要点是它**不再被派发**。
    let (status, _) = gone("POST", "/v1/models/gemini-2.5-pro:generateContent").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

    assert!(
        harness.ledger.calls().is_empty(),
        "一条不存在的路由不该先创建一个 hold 再被 404/405 掉",
    );
    assert_eq!(
        harness.gemini.call_count(),
        0,
        "更不该有任何一条打到 Gemini 上游",
    );
}

mod catalogue;

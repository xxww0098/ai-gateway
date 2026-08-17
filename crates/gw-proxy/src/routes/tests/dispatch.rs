//! Upstream dispatch for the `/v1` dialects: model-to-provider resolution,
//! cross-account failover, the streaming relay, and the middleware order the
//! whole billing pipeline depends on.

use super::*;

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
async fn a_cell_the_gateway_cannot_serve_is_a_gateway_400_not_an_upstream_one() {
    // 收敛前这一格「直通 → 上游必 400」：OpenAI 形状的 body 被原样打到
    // Google 的 generateContent 端点。转发过去只会拿一个上游错误，
    // 而客户端从上游的错误里**读不出**「这是网关的路由问题、该改用哪个入口」。
    let harness = Harness::build_routed(
        vec![auth_record("acct-1", "gemini")],
        Some(gemini_only_resolver()),
    );

    let (status, body) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("house-model")),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        harness.gemini.call_count(),
        0,
        "拒绝要发生在出网之前，不该烧一次上游调用",
    );
    // 入口方言的错误信封：OpenAI 入口回 `{"error":{"message":...}}`。
    // 客户端 SDK 只会解析它自己那套结构，回一个陌生结构会被渲染成无字的红叉。
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "错误信封不是 OpenAI 形状：{body}",
    );
}

#[tokio::test]
async fn a_rejected_cell_releases_the_reservation_instead_of_settling_it() {
    // 400 **不计费**：走释放路径而不是结算。
    let harness = Harness::build_routed(
        vec![auth_record("acct-1", "gemini")],
        Some(gemini_only_resolver()),
    );
    send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("house-model")),
    )
    .await;

    assert!(
        harness.usage_store.settled_costs().is_empty(),
        "被网关拒掉的请求不该结算出任何金额",
    );
    assert!(
        harness
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "预扣必须被释放：{:?}",
        harness.ledger.calls(),
    );
}

#[tokio::test]
async fn a_passthrough_cell_still_reaches_its_upstream() {
    // 矩阵不是一道全拒的闸：P0 的 5 个直通格必须原样通过。
    let harness = Harness::build();
    harness.provider.queue(Ok(ok_response(10, 20)));

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
        harness.provider.queue(Ok(ok_response(1, 1)));
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

#[test]
fn query_pairs_keep_their_order_and_duplicates() {
    // Both are significant upstream, which is why this is a Vec and not a map.
    assert_eq!(
        parse_query("alt=sse&key=a&key=b"),
        vec![
            ("alt".to_owned(), "sse".to_owned()),
            ("key".to_owned(), "a".to_owned()),
            ("key".to_owned(), "b".to_owned()),
        ],
    );
    assert_eq!(
        parse_query("flag"),
        vec![("flag".to_owned(), String::new())]
    );
    assert!(parse_query("").is_empty());
}

#[test]
fn only_failures_another_account_could_survive_are_retried() {
    assert!(is_retryable(&ProviderError::Upstream {
        status: 503,
        body: String::new()
    }));
    assert!(is_retryable(&ProviderError::Upstream {
        status: 429,
        body: String::new()
    }));
    assert!(is_retryable(&ProviderError::Credential("expired".into())));
    assert!(
        !is_retryable(&ProviderError::Upstream {
            status: 400,
            body: String::new()
        }),
        "a malformed request fails identically on every account",
    );
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
    harness.provider.queue(Ok(ok_response(100, 250)));

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
    harness.provider.queue(Ok(ok_response(100, 250)));
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
    harness.provider.queue(Ok(ok_response_without_usage()));
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
    harness.provider.queue(Err(ProviderError::Upstream {
        status: 503,
        body: "overloaded".to_owned(),
    }));
    harness.provider.queue(Ok(ok_response(10, 20)));

    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.provider.call_count(),
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
    harness.provider.queue(Err(ProviderError::Upstream {
        status: 400,
        body: r#"{"error":"bad request"}"#.to_owned(),
    }));

    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(harness.provider.call_count(), 1);
}

#[tokio::test]
async fn a_failed_dispatch_releases_the_reservation() {
    let harness = Harness::build();
    harness.provider.queue(Err(ProviderError::Upstream {
        status: 400,
        body: String::new(),
    }));

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
        harness.provider.queue(Err(ProviderError::Upstream {
            status: 503,
            body: String::new(),
        }));
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
        .uri("/v1/chat/completions?key=cpa-testkey")
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
    for (method, path) in [("GET", "/v1/models"), ("POST", "/v1/messages/count_tokens")] {
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
        axum::routing::get(|| async { "cpa_v1_requests_total 0" }),
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
async fn an_error_status_relayed_in_band_fails_over_like_a_raised_one() {
    // Some providers surface upstream 5xx as a normal response; the client
    // should not get a 503 that a different credential would have served.
    let harness = Harness::build_with(vec![
        auth_record("acct-1", "openai"),
        auth_record("acct-2", "openai"),
    ]);
    harness.provider.queue(Ok(ProviderResponse {
        status: 503,
        headers: http::HeaderMap::new(),
        body: bytes::Bytes::from_static(b"overloaded"),
        usage: None,
    }));
    harness.provider.queue(Ok(ok_response(10, 20)));

    let (status, _) = send_settled(
        &harness,
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.provider.call_count(), 2);
    assert_eq!(harness.usage_store.settled_costs().len(), 1);
}

#[test]
fn only_account_level_statuses_are_worth_another_credential() {
    assert!(is_retryable_status(500));
    assert!(is_retryable_status(429));
    assert!(!is_retryable_status(400));
    assert!(!is_retryable_status(200));
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
    assert_eq!(vision["input_modalities"], serde_json::json!(["text", "image"]));
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

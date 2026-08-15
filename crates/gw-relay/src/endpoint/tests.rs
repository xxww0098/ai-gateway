//! OWNER: worker `relay-endpoints`。跨四个子模块的端到端接线测试。
//!
//! 单个模块自己的性质在各自的 `tests.rs` 里测；这里只测**四个模块串起来**
//! 才成立的东西：一个真实请求从 `validate` 走到 `route`，P0 五格必须直通，
//! P3 三格必须拿到入口方言的 400。
//!
//! 规范 2.11：不抄任何一格的判定，也不抄前缀表。断言的是链路性质。

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, header};

use super::*;
use crate::contract::{Surface, UpstreamDialect};

fn json_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h
}

/// 走一遍完整链路：校验 → 解析（唯一一次）→ 选上游 → 查表派发。
fn dispatch(
    path: &str,
    body: &[u8],
    resolver: Option<&dyn ChannelResolver>,
) -> (RequestSpec, Route) {
    let surface = validate(&Method::POST, path, &json_headers()).expect("入口应当有效");
    let spec = RequestSpec::parse(surface, Some(body));
    let selection = select(surface, spec.model(), resolver);
    let provider = *selection
        .candidates
        .first()
        .expect("四级链必须给得出至少一个候选");
    let route = route(surface, provider, spec.model());
    (spec, route)
}

/// **P0 五格恒等转发**：直通格里，网关对请求体一个字节都不改。
///
/// 这五格的正确性是本项目的第一性原理。这条测试把它落成一句可执行的断言：
/// 走完整条链路之后，仍然没有任何 body 改写被安排 ——
/// 除了流式 OpenAI Chat 上那一次被明确授权的 `stream_options` 定点插入。
///
/// 守护的 bug：在链路里插入任何一次「顺手」的 body 规整（补默认值、
/// 重排字段、统一大小写）。透传的定义就是不做这些。
#[test]
fn the_passthrough_cells_leave_the_request_body_untouched() {
    let resolver = InMemoryChannelResolver::new()
        .with_model("m-openai", ["c-openai"])
        .with_channel("c-openai", Provider::OpenAi)
        .with_model("m-codex", ["c-codex"])
        .with_channel("c-codex", Provider::Codex)
        .with_model("m-claude", ["c-claude"])
        .with_channel("c-claude", Provider::Claude);

    // 五个 P0 格：A×openai、A×codex、B×openai、B×codex、C×claude
    let p0 = [
        ("/v1/chat/completions", "m-openai"),
        ("/v1/chat/completions", "m-codex"),
        ("/v1/responses", "m-openai"),
        ("/v1/responses", "m-codex"),
        ("/v1/messages", "m-claude"),
    ];
    for (path, model) in p0 {
        let raw =
            serde_json::to_vec(&serde_json::json!({ "model": model, "stream": false })).unwrap();
        let body = Bytes::from(raw);
        let (spec, route) = dispatch(path, &body, Some(&resolver));
        let Route::Passthrough { upstream } = route else {
            panic!("{path} × {model} 必须是直通格，实际是 {route:?}")
        };
        assert!(
            splice_include_usage(&body, &spec, upstream, IncludeUsagePolicy::Force).is_none(),
            "非流式的直通格不该安排任何 body 改写"
        );
    }
    assert_eq!(p0.len(), 5, "P0 直通格的数量变了");
}

/// **缺陷 #1（S1）的端到端守护**：`/v1/responses` 拼的是 Responses 端点。
///
/// 今天 `client.responses.create()` 100% 拿到 `400 Unrecognized request argument
/// supplied: input`，因为端点由 provider 猜。这条测试从入站路径一路走到上游方言，
/// 确认这个猜测没有在链路的任何一环复活。
///
/// 守护的 bug：让 provider 决定端点。那样入口 B 会静默塌回 chat/completions。
#[test]
fn the_responses_entry_reaches_the_responses_upstream_end_to_end() {
    let resolver = InMemoryChannelResolver::new()
        .with_model("m", ["c"])
        .with_channel("c", Provider::OpenAi);
    let body = br#"{"model":"m","input":[]}"#;

    let (_, responses) = dispatch("/v1/responses", body, Some(&resolver));
    let (_, chat) = dispatch("/v1/chat/completions", body, Some(&resolver));

    let Route::Passthrough { upstream: a } = responses else {
        panic!("入口 B × openai 必须直通")
    };
    let Route::Passthrough { upstream: b } = chat else {
        panic!("入口 A × openai 必须直通")
    };
    assert_ne!(a, b, "两个 OpenAI 入口被折叠到了同一个上游端点");
    assert_eq!(a, UpstreamDialect::OpenAiResponses);
}

/// **P3 三格**：`/v1/responses` 打到 claude / gemini / vertex 时拿到 400，
/// 且信封是 OpenAI 方言（因为入口 B 是 OpenAI 方言）。
///
/// 守护的 bug：把这三格「勉强翻译」过去。Responses API 的有状态 item 模型在对端
/// 没有对应概念，翻过去只能翻文本部分，而客户端会以为 `previous_response_id`
/// 生效了 —— 一个静默的跨轮次正确性错误，比一个 400 坏得多。
#[test]
fn the_responses_entry_refuses_the_upstreams_that_would_lose_semantics() {
    let mut resolver = InMemoryChannelResolver::new();
    for (model, channel, provider) in [
        ("m-claude", "c-claude", Provider::Claude),
        ("m-gemini", "c-gemini", Provider::Gemini),
        ("m-vertex", "c-vertex", Provider::Vertex),
    ] {
        resolver = resolver
            .with_model(model, [channel])
            .with_channel(channel, provider);
    }

    for model in ["m-claude", "m-gemini", "m-vertex"] {
        let body = serde_json::to_vec(&serde_json::json!({ "model": model })).unwrap();
        let (_, route) = dispatch("/v1/responses", &body, Some(&resolver));
        let Route::Reject(bytes) = route else {
            panic!("{model} 必须被拒，实际是 {route:?}")
        };
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("必须是合法 JSON");
        assert!(
            v["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains(model)),
            "OpenAI 方言的错误体里读不到可执行指引"
        );
        // 同一个 model 走 A 或 C 入口时不该被拒 —— 否则那句指引就是空头支票。
        let alt = if v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("/v1/messages")
        {
            "/v1/messages"
        } else {
            "/v1/chat/completions"
        };
        let (_, alt_route) = dispatch(alt, &body, Some(&resolver));
        assert!(
            !matches!(alt_route, Route::Reject(_)),
            "指引指向的 {alt} 同样会拒绝"
        );
    }
}

/// P1/P2 七格走到转义分支，且转义的目标方言与入口方言确实不同。
///
/// 本 worker **不实现**转义器本身（那是 relay-google / relay-anthropic 的活），
/// 只负责把请求送到正确的转义器门口。这条测试守护的就是「送对门」。
///
/// 守护的 bug：把某个跨方言格误判成直通，于是 body 原样打给一个说另一种话的上游。
#[test]
fn the_translating_cells_cross_a_dialect_boundary() {
    let mut resolver = InMemoryChannelResolver::new();
    for (model, channel, provider) in [
        ("t-openai", "k-openai", Provider::OpenAi),
        ("t-codex", "k-codex", Provider::Codex),
        ("t-claude", "k-claude", Provider::Claude),
        ("t-gemini", "k-gemini", Provider::Gemini),
        ("t-vertex", "k-vertex", Provider::Vertex),
    ] {
        resolver = resolver
            .with_model(model, [channel])
            .with_channel(channel, provider);
    }

    let mut seen = 0;
    for (path, surface) in [
        ("/v1/chat/completions", Surface::OpenAiCompletions),
        ("/v1/responses", Surface::OpenAiResponses),
        ("/v1/messages", Surface::AnthropicMessages),
    ] {
        for provider in Provider::ALL {
            if cell(surface, provider) != Cell::Translate {
                continue;
            }
            let model = format!("t-{}", provider.as_str());
            let body = serde_json::to_vec(&serde_json::json!({ "model": model })).unwrap();
            let (_, route) = dispatch(path, &body, Some(&resolver));
            let Route::Translate { upstream } = route else {
                panic!("{path} × {} 必须走转义", provider.as_str())
            };
            assert_eq!(upstream, upstream_dialect(surface, provider));
            seen += 1;
        }
    }
    assert_eq!(seen, 7, "需要转义的格数变了（P1 四格 + P2 三格）");
}

/// 请求体只被解析一次，结果被后续每一环复用（缺陷 #15）。
///
/// 守护的 bug：在链路的第二环再解一遍 JSON。今天 `hold.rs:866` 与
/// `routes.rs:632` 各解一遍，流式还有第三遍（`common.rs:252`）——
/// 一个 900 KB 的 body 解析三遍。这条测试把「一次」这件事钉在类型上：
/// 上游选择与矩阵派发拿到的都是同一个 `RequestSpec`，
/// 它们的签名里没有任何一个能接受原始字节的参数。
#[test]
fn the_body_is_parsed_once_and_the_result_is_what_everyone_downstream_uses() {
    let body = br#"{"model":"gpt-5","stream":true,"max_output_tokens":9}"#;
    let (spec, _) = dispatch("/v1/responses", body, None);

    assert_eq!(spec.model(), Some("gpt-5"));
    assert!(spec.stream);
    assert_eq!(spec.max_tokens, Some(9));
    assert!(spec.body_visible);

    // 同一个 spec 直接喂给下游两环，不需要再看一眼原始字节。
    let selection = select(spec.surface, spec.model(), None);
    assert!(!selection.candidates.is_empty());
    assert!(matches!(
        route(spec.surface, selection.candidates[0], spec.model()),
        Route::Passthrough { .. }
    ));
}

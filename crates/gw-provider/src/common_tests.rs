//! Unit tests for [`crate::common`].
//!
//! The include_usage rewrite is covered by an exhaustive matrix over its
//! generator space. The stream machinery these used to also cover is gone —
//! frame relaying and side-band usage now live in `gw-relay`, which tests them.

use super::*;
use serde_json::json;

// --- include_usage 定点插入（审计缺陷 #4）---------------------------------------

/// 把两段拼回一个完整 body，供断言检查。
/// 生产路径走 [`crate::RoutePlan::splice`]，同样是这两段。
fn joined(spliced: &Spliced) -> Vec<u8> {
    let mut out = Vec::with_capacity(spliced.len());
    out.extend_from_slice(&spliced.prefix);
    out.extend_from_slice(&spliced.rest);
    out
}

/// 枚举而非抽样：3 种 `stream` × 5 种 `stream_options` × 2 组无关兄弟字段。
fn include_usage_matrix() -> Vec<Value> {
    let stream_variants: [Option<Value>; 3] = [Some(json!(true)), Some(json!(false)), None];
    let options_variants: [Option<Value>; 5] = [
        None,
        Some(json!({})),
        Some(json!({"include_usage": true})),
        Some(json!({"include_usage": false})),
        Some(json!({"include_usage": false, "include_input_tokens": true})),
    ];
    let sibling_variants = [
        json!({}),
        json!({
            "model": "gpt-4o-mini",
            "max_tokens": 512,
            "temperature": 0.25,
            "messages": [{"role": "user", "content": "hello"}]
        }),
    ];

    let mut out = Vec::new();
    for stream in &stream_variants {
        for options in &options_variants {
            for siblings in &sibling_variants {
                let mut body = siblings.as_object().cloned().unwrap_or_default();
                if let Some(stream) = stream {
                    body.insert("stream".to_owned(), stream.clone());
                }
                if let Some(options) = options {
                    body.insert("stream_options".to_owned(), options.clone());
                }
                out.push(Value::Object(body));
            }
        }
    }
    out
}

/// 插入条件的全量矩阵。三条性质，都不核对任何字面量：
///
/// 1. 只有「流式 **且** 客户端没写 `stream_options`」才会动；
/// 2. 动过之后 `stream_options.include_usage` 为真；
/// 3. 幂等 —— 对插过的结果再插一次是 `None`（因为这时 `stream_options` 已经在了）。
#[test]
fn only_a_streaming_body_without_stream_options_is_touched() {
    for body in include_usage_matrix() {
        let payload = Bytes::from(serde_json::to_vec(&body).expect("fixture must serialise"));
        let streaming = body.get("stream") == Some(&Value::Bool(true));
        let client_wrote_options = body.get("stream_options").is_some();
        let expected = streaming && !client_wrote_options;

        let Some(spliced) = ensure_include_usage(&payload, Surface::OpenAiCompletions) else {
            assert!(!expected, "should have been spliced: {body}");
            continue;
        };
        assert!(expected, "must not have been touched: {body}");

        let rewritten: Value =
            serde_json::from_slice(&joined(&spliced)).expect("splice must stay JSON");
        assert_eq!(
            rewritten.pointer("/stream_options/include_usage"),
            Some(&Value::Bool(true)),
            "streaming payload did not gain include_usage: {body}"
        );
        assert!(
            ensure_include_usage(&Bytes::from(joined(&spliced)), Surface::OpenAiCompletions)
                .is_none(),
            "splicing twice is not idempotent: {body}"
        );
    }
}

/// **根除缺陷 #4 的第一条**：客户端显式写的 `include_usage: false` 曾被静默翻成
/// `true`，于是 SSE 末尾多出一帧 `{"choices":[],"usage":{…}}`，任何手写
/// `chunk["choices"][0]` 的客户端在那一帧上抛异常。
///
/// 现在「客户端写过 `stream_options`」本身就是终止条件 —— 连它写的是 true 还是
/// false 都不必看。
#[test]
fn a_client_that_wrote_stream_options_keeps_its_own_value() {
    let payload = Bytes::from(
        serde_json::to_vec(&json!({
            "stream": true,
            "model": "gpt-4o",
            "stream_options": {"include_usage": false}
        }))
        .unwrap(),
    );
    assert!(
        ensure_include_usage(&payload, Surface::OpenAiCompletions).is_none(),
        "客户端显式写的 include_usage 被网关改掉了"
    );
}

/// **根除缺陷 #4 的第二、第三条**：整体 JSON 往返会递归重排键序
/// （`serde_json` 的 `Map` 是 `BTreeMap`），并把超出 `f64` 精度的整数烧成科学计数法。
///
/// 定点插入的性质是：**除了插进去的那一段，原字节一个都没动**。
/// 这里用「把插入段从结果里去掉，应当还原出原 body」来测，不核对插入段长什么样。
#[test]
fn every_other_byte_survives_verbatim() {
    // 键序刻意不是字典序，且带一个 f64 存不下的整数。
    let original =
        br#"{"stream":true,"zzz":1,"seed":12345678901234567890,"aaa":[3,2,1],"model":"gpt-4o"}"#;
    let payload = Bytes::from_static(original);
    let spliced = ensure_include_usage(&payload, Surface::OpenAiCompletions).expect("must splice");
    let out = joined(&spliced);

    let inserted = out.len() - original.len();
    assert!(inserted > 0, "nothing was inserted");
    // 插入点在最外层 `{` 之后，所以去掉那一段就该逐字节还原。
    let mut restored = out.clone();
    restored.drain(1..=inserted);
    assert_eq!(
        restored,
        original.to_vec(),
        "定点插入之外的字节被改动了：{}",
        String::from_utf8_lossy(&out)
    );
}

/// 第二段是原 [`Bytes`] 的**零拷贝切片** —— 与原 body 共享同一块 allocation。
/// 这是「不做全量拷贝」的可观测判据。
#[test]
fn the_untouched_tail_shares_the_original_allocation() {
    let payload = Bytes::from(
        serde_json::to_vec(&json!({"stream": true, "model": "gpt-4o", "messages": []})).unwrap(),
    );
    let spliced = ensure_include_usage(&payload, Surface::OpenAiCompletions).expect("must splice");
    // fixture 的第一个字节就是 `{`，插入点紧随其后，所以未改动的那一段应当正好
    // 落在原 allocation 的第 1 个字节上 —— 复制过的话地址不可能对上。
    assert_eq!(
        spliced.rest.as_ptr() as usize,
        payload.as_ptr() as usize + 1,
        "tail was copied instead of sliced out of the original body"
    );
}

/// 改写是**建议性**的 —— 计费的 fallback 路径会补偿 —— 所以一个解析不了的 body
/// 必须原样通过，而不是让请求失败。
#[test]
fn unparseable_or_non_object_payloads_are_left_alone() {
    let cases: [(&str, &[u8]); 14] = [
        ("empty", b""),
        ("random bytes", &[0xff, 0x00, 0xab, 0x7f, 0x10]),
        ("plain text", b"hello, not json"),
        ("unopened brace", br#""stream":true}"#),
        ("truncated object", br#"{"stream":true"#),
        ("truncated nested", br#"{"stream":true,"stream_options":"#),
        ("unclosed string", br#"{"stream":"tr"#),
        ("invalid token", b"{not valid json}"),
        ("partial array", b"[1,2,"),
        ("json array top-level", b"[1,2,3]"),
        ("json primitive number", b"42"),
        ("json primitive string", br#""hello""#),
        ("json null", b"null"),
        ("stream as string", br#"{"stream":"true"}"#),
    ];
    for (name, input) in cases {
        assert!(
            ensure_include_usage(&Bytes::from_static(input), Surface::OpenAiCompletions).is_none(),
            "case {name} was modified"
        );
    }
}

/// **入口 B 绝不能被插 `stream_options`**：Responses API 不认识这个键，塞进去
/// 上游直接 400。缺陷 #1（打错端点）叠加缺陷 #4（还塞 `stream_options`）
/// 曾让入口 B 双重不可用。
#[test]
fn the_responses_surface_is_never_spliced() {
    for body in include_usage_matrix() {
        let payload = Bytes::from(serde_json::to_vec(&body).expect("fixture must serialise"));
        assert!(
            ensure_include_usage(&payload, Surface::OpenAiResponses).is_none(),
            "Responses 入口被插了 stream_options: {body}"
        );
    }
}

// --- 端点由入口决定（审计缺陷 #1 / S1）----------------------------------------

/// 两个入口拼出两个**不同**的端点。这是缺陷 #1 的判据：在此之前无论入口是什么，
/// executor 都只会拼 chat/completions。
///
/// 断言的是「路径以入口自己的那一段收尾」，不是核对一整条硬编码 URL。
#[test]
fn the_entry_point_decides_the_endpoint_not_the_provider() {
    for base in [
        "https://api.example.com",
        "https://api.example.com/",
        "https://api.example.com/v1",
        // 已经配成完整 chat 端点的 base 也要能服务 responses。
        "https://api.example.com/v1/chat/completions",
    ] {
        let chat = chat_completions_endpoint(base, &[]).expect("chat endpoint");
        let responses = responses_endpoint(base, &[]).expect("responses endpoint");
        assert_ne!(chat, responses, "base {base}: 两个入口拼出了同一个端点");
        assert!(
            chat.ends_with("/v1/chat/completions"),
            "base {base}: {chat}"
        );
        assert!(
            responses.ends_with("/v1/responses"),
            "base {base}: {responses}"
        );
    }
}

/// 入口从 [`SURFACE_PATH_METADATA_KEY`] 读，路径 → 入口的映射复用
/// [`Surface::from_path`]。键缺失时回落到 chat completions —— 那是本键存在之前的
/// 既有行为，所以对还没写这个键的调用方是严格加性的。
#[test]
fn the_surface_comes_from_the_inbound_path_and_defaults_to_chat() {
    let with = |value: Option<&str>| {
        let mut req = ProviderRequest::default();
        if let Some(value) = value {
            req.metadata
                .insert(SURFACE_PATH_METADATA_KEY.to_owned(), value.to_owned());
        }
        request_surface(&req)
    };

    assert_eq!(with(None), Surface::OpenAiCompletions, "缺失必须回落");
    assert_eq!(
        with(Some("/nope")),
        Surface::OpenAiCompletions,
        "不认识的路径必须回落，不许猜"
    );
    // 三个入口自己报出来的路径必须能被认回去 —— 不抄字面量，用 gw-relay 的映射对账。
    for surface in [
        Surface::OpenAiCompletions,
        Surface::OpenAiResponses,
        Surface::AnthropicMessages,
    ] {
        let path = ["/v1/chat/completions", "/v1/responses", "/v1/messages"]
            .into_iter()
            .find(|p| Surface::from_path(p) == Some(surface))
            .expect("gw-relay 认得三个入口");
        assert_eq!(with(Some(path)), surface);
        assert_eq!(
            with(Some(&format!("  {path} "))),
            surface,
            "两侧空白应被容忍"
        );
    }
}

// --- small helpers -----------------------------------------------------------

#[test]
fn token_estimate_rounds_up_to_the_next_whole_token() {
    assert_eq!(approximate_tokens_from_bytes(0), 0);
    // Monotonic, and never under-counts a partial token.
    let mut previous = 0;
    for size in 1..64usize {
        let got = approximate_tokens_from_bytes(size);
        assert!(got >= previous, "estimate must be monotonic in size");
        assert!(got * 4 >= size as i64, "estimate must not under-count");
        assert!((got - 1) * 4 < size as i64, "estimate must not over-count");
        previous = got;
    }
}

#[test]
fn failure_body_is_clipped_without_splitting_a_code_point() {
    let long = "é".repeat(8 * 1024);
    let clipped = truncate_failure_body(long.as_bytes());
    assert!(clipped.len() <= 4 * 1024);
    assert!(long.starts_with(&clipped) || clipped.ends_with('\u{fffd}'));
    // Short bodies survive intact, including non-UTF-8 ones.
    assert_eq!(truncate_failure_body(b"boom"), "boom");
    assert!(!truncate_failure_body(&[0xff, 0xfe]).is_empty());
}

#[test]
fn requested_model_prefers_the_translated_name_then_the_router_hint() {
    let mut req = ProviderRequest {
        model: "  gpt-4o  ".to_owned(),
        ..Default::default()
    };
    req.metadata
        .insert(REQUESTED_MODEL_METADATA_KEY.to_owned(), "alias".to_owned());
    assert_eq!(requested_model(&req), "gpt-4o");

    req.model = "   ".to_owned();
    assert_eq!(requested_model(&req), "alias");

    req.metadata.clear();
    assert_eq!(requested_model(&req), "");
}

#[test]
fn string_from_map_coerces_scalars_and_treats_null_as_absent() {
    let values = json!({
        "text": "  padded  ",
        "blank": "   ",
        "int": 42,
        "float_integral": 3.0,
        "float": 1.5,
        "yes": true,
        "nothing": null,
        "nested": {"a": 1}
    });
    assert_eq!(string_from_map(&values, "text").as_deref(), Some("padded"));
    assert_eq!(string_from_map(&values, "int").as_deref(), Some("42"));
    assert_eq!(
        string_from_map(&values, "float_integral").as_deref(),
        Some("3")
    );
    assert_eq!(string_from_map(&values, "float").as_deref(), Some("1.5"));
    assert_eq!(string_from_map(&values, "yes").as_deref(), Some("true"));
    assert_eq!(
        string_from_map(&values, "nothing"),
        None,
        "a JSON null is absent, not the literal string \"null\""
    );
    assert_eq!(
        string_from_map(&values, "blank"),
        None,
        "whitespace is not a credential"
    );
    assert_eq!(string_from_map(&values, "missing"), None);
    assert_eq!(
        string_from_map(&values, "nested").as_deref(),
        Some(r#"{"a":1}"#)
    );
    assert_eq!(string_from_map(&json!("not an object"), "k"), None);
}

#[test]
fn nested_string_reads_through_an_object_or_an_embedded_json_document() {
    let nested_obj = json!({"token_data": {"access_token": "  abc  "}});
    assert_eq!(
        nested_string(&nested_obj, "token_data", "access_token").as_deref(),
        Some("abc")
    );

    let embedded = json!({"token_data": " {\"access_token\":\"xyz\"} "});
    assert_eq!(
        nested_string(&embedded, "token_data", "access_token").as_deref(),
        Some("xyz")
    );

    for broken in [
        json!({"token_data": "{not json"}),
        json!({"token_data": 7}),
        json!({"token_data": null}),
        json!({}),
    ] {
        assert_eq!(
            nested_string(&broken, "token_data", "access_token"),
            None,
            "{broken}"
        );
    }
}

// --- endpoint construction ---------------------------------------------------

#[test]
fn every_base_url_shape_converges_on_one_endpoint() {
    let expected = "https://api.example.com/v1/chat/completions";
    for base in [
        "https://api.example.com",
        "https://api.example.com/",
        "https://api.example.com/v1",
        "https://api.example.com/v1/",
        "https://api.example.com/v1/chat/completions",
        "  https://api.example.com/v1/chat/completions/  ",
    ] {
        assert_eq!(
            chat_completions_endpoint(base, &[]).unwrap(),
            expected,
            "base {base}"
        );
    }
}

#[test]
fn inbound_query_parameters_are_appended_including_duplicate_keys() {
    let query = vec![
        ("beta".to_owned(), "1".to_owned()),
        ("tag".to_owned(), "a".to_owned()),
        ("tag".to_owned(), "b".to_owned()),
    ];
    let endpoint = chat_completions_endpoint("https://api.example.com/v1", &query).unwrap();
    let parsed = url::Url::parse(&endpoint).unwrap();
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(pairs, query, "order and duplicate keys must both survive");
}

#[test]
fn a_base_url_without_a_host_is_rejected() {
    for base in ["", "not-a-url", "/v1", "https://"] {
        assert!(
            chat_completions_endpoint(base, &[]).is_err(),
            "base {base:?} should not produce an endpoint"
        );
    }
}

// --- 密钥不许被 Debug 带出去 -------------------------------------------------

/// [`Redacted`] 的守护测试：**每一个** executor 的 `Debug` 都不许打出配置里的活密钥。
///
/// 密文是这条测试自己造的（生产源码里不存在这个串），所以断言的期望值来自输入
/// 而不是源码字面量（规范 2.11）。
///
/// 守护的 bug：给这些结构体加回 `#[derive(Debug)]`。那时一句
/// `tracing::debug!(?provider)`、一个带上下文的 `expect`，写进日志的就是一把
/// 能直接拿去用的上游 key —— 而日志落到哪、留多久、被谁看到，都不由本进程决定。
///
/// 逐个列出来而不是只测一个：这七个 executor 是同一条规矩的七处落点，
/// 新加第八个上游时漏掉脱敏，这条会红。
#[test]
fn no_executor_debug_carries_its_api_key() {
    const LIVE: &str = "sk-live-UNIQUE-KNIFE3-provider-4b81de";

    let cfg = ProviderConfig {
        base_url: "https://upstream.test/v1".to_owned(),
        api_key: LIVE.to_owned(),
        enabled: true,
    };

    let dumps: Vec<(&str, String)> = vec![
        ("ProviderConfig", format!("{cfg:?}")),
        (
            "ClaudeProvider",
            format!(
                "{:?}",
                crate::claude::ClaudeProvider::new(&cfg, 0).expect("claude")
            ),
        ),
        (
            "OpenAiCompatibleProvider",
            format!(
                "{:?}",
                crate::openai::OpenAiCompatibleProvider::new(&cfg, 0).expect("openai")
            ),
        ),
        (
            "GeminiProvider",
            format!(
                "{:?}",
                crate::gemini::GeminiProvider::new(&cfg, 0).expect("gemini")
            ),
        ),
        (
            "KiroProvider",
            format!(
                "{:?}",
                crate::kiro::KiroProvider::new(&cfg, 0).expect("kiro")
            ),
        ),
        (
            "CodexProvider",
            format!(
                "{:?}",
                crate::codex::CodexProvider::new(&cfg, 0).expect("codex")
            ),
        ),
        (
            "XaiProvider",
            format!("{:?}", crate::xai::XaiProvider::new(&cfg, 0).expect("xai")),
        ),
        (
            "VertexProvider",
            format!(
                "{:?}",
                crate::vertex::VertexProvider::new(&cfg, 0).expect("vertex")
            ),
        ),
    ];

    for (name, dump) in &dumps {
        assert!(
            !dump.contains(LIVE),
            "{name} 的 Debug 把活密钥打了出来：{dump}"
        );
        assert!(
            dump.contains(name),
            "脱敏不许把类型名一起吃掉，否则日志读不出这是谁：{dump}"
        );
    }

    // 掩码必须稳定：同一份配置两次 dump 一模一样，否则日志里同一把 key
    // 会变成两把，关联与去重全部失效。
    assert_eq!(format!("{cfg:?}"), dumps[0].1, "掩码不稳定");
}

/// [`crate::RoutePlan`] 是凭证在本 crate 里唯一的出口值，它的 `Debug` 同样不许漏。
///
/// 计划里的凭证是 `gw_relay::Credential`，脱敏收在**它**那一层，所以 `RoutePlan`
/// 照常 `derive(Debug)`。这条测的正是那个「照常」还成不成立 —— 谁要是往
/// `RoutePlan` 上加了第二个装密文的裸 `String` 字段，这条会红。
#[test]
fn a_route_plan_never_dumps_the_credential_it_carries() {
    const LIVE: &str = "sk-live-UNIQUE-KNIFE3-plan-e5d9a2";

    for credential in [
        gw_relay::Credential::Bearer(LIVE.to_owned()),
        gw_relay::Credential::XApiKey(LIVE.to_owned()),
        gw_relay::Credential::GoogleApiKey(LIVE.to_owned()),
    ] {
        let plan = crate::RoutePlan {
            provider: PROVIDER_OPENAI,
            endpoint: url::Url::parse("https://upstream.test/v1/chat/completions")
                .expect("测试用的 endpoint"),
            credential,
            headers: http::HeaderMap::new(),
            body: None,
            timeouts: relay_timeouts(DEFAULT_TIMEOUT),
            dialect: gw_relay::UpstreamDialect::OpenAiChat,
        };
        let dump = format!("{plan:?}");
        assert!(!dump.contains(LIVE), "RoutePlan 把活密钥打了出来：{dump}");
    }
}

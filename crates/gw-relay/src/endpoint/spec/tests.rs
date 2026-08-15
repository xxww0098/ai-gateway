//! OWNER: worker `relay-endpoints`。
//!
//! 规范 2.11：测的是**性质**，不是字面量的复述。每条测试的 doc 写明它守护的 bug ——
//! 那个 bug 都已经被塞回去跑过一遍、确认测试真的会红，然后才还原。

use http::{HeaderMap, HeaderValue, Method, header};

use super::*;

fn json_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h
}

fn with_accept(accept: &str) -> HeaderMap {
    let mut h = json_headers();
    h.insert(header::ACCEPT, HeaderValue::from_str(accept).unwrap());
    h
}

const ENTRY_PATHS: [&str; 3] = ["/v1/chat/completions", "/v1/responses", "/v1/messages"];

fn all_surfaces() -> Vec<Surface> {
    ENTRY_PATHS
        .iter()
        .map(|p| Surface::from_path(p).expect("三个入口路径必须可识别"))
        .collect()
}

/// 路径先于方法被判定：一个不存在的路径拿到的是 404，不是 405。
///
/// 守护的 bug：把方法校验提到路径校验之前。那样 `GET /v1/embeddings`
/// 会拿到 405，而 405 的含义是「这个路径存在，只是方法不对」——
/// 等于向外泄漏一个已经被硬删的路由确实存在过。
#[test]
fn an_unknown_path_is_not_told_which_methods_it_would_accept() {
    let err = validate(&Method::GET, "/v1/embeddings", &json_headers())
        .expect_err("已删除的入口不该通过校验");
    assert_eq!(
        err,
        SurfaceError::UnknownPath,
        "不存在的路径必须先被判成 UnknownPath，不能先回答方法问题"
    );
}

/// 被收敛掉的入口在校验层是 404，而不是别的什么。
///
/// 其中 `/v1beta/**` 是**已知且已接受的缺口**：面板
/// `QuickIntegrationPanel.tsx:80` 还在把 `${origin}/v1beta` 印给用户，前端冻结改不了。
/// 这条测试钉的是「网关这边确实彻底没有这个前缀」，不是在为那行文案辩护。
///
/// 守护的 bug：给 `validate` 加一条兜底把不认识的 `/v1*` 归到某个入口。
/// 那会让 Gemini 原生 body 静默走进 OpenAI 方言，拿一个上游 400 而不是网关 404。
#[test]
fn the_removed_surfaces_are_gone_rather_than_redirected() {
    for path in [
        "/v1beta/models/gemini-2.5-pro:generateContent",
        "/v1beta/models",
        "/v1/completions",
        "/v1/embeddings",
        "/v1/models/gemini-2.5-pro",
        "/v1/chat/completions/",
    ] {
        assert_eq!(
            validate(&Method::POST, path, &json_headers()),
            Err(SurfaceError::UnknownPath),
            "{path} 必须 404，不许被兜底猜成某个入口"
        );
    }
}

/// 三个入口都只接受 POST。
///
/// 守护的 bug：某个入口漏配了方法白名单，于是 `GET /v1/messages` 带着空 body
/// 走完鉴权与计费链。
#[test]
fn every_entry_takes_exactly_one_method() {
    for path in ENTRY_PATHS {
        assert!(validate(&Method::POST, path, &json_headers()).is_ok());
        for m in [Method::GET, Method::PUT, Method::DELETE, Method::PATCH] {
            assert_eq!(
                validate(&m, path, &json_headers()),
                Err(SurfaceError::MethodNotAllowed),
                "{m} {path} 不该被接受"
            );
        }
    }
}

/// 非 JSON 的 content-type 被**拒绝**，而不是静默降级成一份全零的 peek。
///
/// 守护的 bug（今天就在生产里）：`parse_body_peek`（`hold.rs:757-762`）在
/// content-type 不含 `json` 子串时直接放弃解析并返回全零 peek，于是
/// `model=""`、`stream=false`、`max_tokens=0` —— 请求不被拒，而是带着空模型名
/// 走完整个计费与派发链，最后打到一个由端点默认值猜出来的上游。
#[test]
fn a_non_json_content_type_is_refused_instead_of_zeroed() {
    let path = ENTRY_PATHS[0];
    for good in [
        "application/json",
        "application/json; charset=utf-8",
        "Application/JSON",
        "  application/json  ; boundary=x",
        "application/vnd.anthropic+json",
    ] {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_str(good).unwrap());
        assert!(
            validate(&Method::POST, path, &h).is_ok(),
            "{good} 应当被接受"
        );
    }
    for bad in [
        "text/plain",
        "application/x-www-form-urlencoded",
        "application/jsonp",
        "multipart/form-data; boundary=x",
        "",
    ] {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_str(bad).unwrap());
        assert_eq!(
            validate(&Method::POST, path, &h),
            Err(SurfaceError::UnsupportedMediaType),
            "{bad:?} 不该被当成 JSON"
        );
    }
    assert_eq!(
        validate(&Method::POST, path, &HeaderMap::new()),
        Err(SurfaceError::UnsupportedMediaType),
        "缺失 content-type 时网关不替客户端编一个"
    );
}

/// 三个入口对**同一份 body** 得出的流式判定完全一致。
///
/// 这是「判定规则写死成一处实现」这条要求的可执行形式：
/// `anthropic-messages` 与 `openai-responses` 的规则必须一致。
///
/// 守护的 bug：给某个入口加一条 per-surface 分支（例如「Responses 入口改看
/// `Accept`」）。判定分岔之后，同一个客户端换个入口就会拿到不同的响应框架，
/// 而预扣估算也跟着 `stream` 走，直接变成 402 误判。
#[test]
fn the_three_entries_agree_on_what_streaming_means() {
    for body in [
        br#"{"model":"m","stream":true}"#.as_slice(),
        br#"{"model":"m","stream":false}"#.as_slice(),
        br#"{"model":"m"}"#.as_slice(),
        br#"{"stream":true,"model":"m","max_tokens":1}"#.as_slice(),
    ] {
        let decisions: Vec<bool> = all_surfaces()
            .into_iter()
            .map(|s| RequestSpec::parse(s, Some(body)).stream)
            .collect();
        assert!(
            decisions.windows(2).all(|w| w[0] == w[1]),
            "三入口对同一份 body 给出了不同的流式判定：{decisions:?}"
        );
    }
}

/// body 说不出 `stream: true` 的每一种情形，一律是非流式（规则 S1）。
///
/// 守护的 bug：把「解析失败」当成「未知」再去别处（`Accept`、URL）找答案。
/// 判定一旦有第二个来源，网关的判断就可能与上游的判断不一致 ——
/// 而上游只看 body。不一致的那一刻，网关就被迫自己把 JSON 切成 SSE 帧。
#[test]
fn anything_that_is_not_a_body_saying_true_is_not_streaming() {
    let s = Surface::AnthropicMessages;
    for body in [
        br#"{"model":"m"}"#.as_slice(),
        br#"{"stream":null}"#.as_slice(),
        br#"{"stream":"true"}"#.as_slice(),
        br#"{"stream":1}"#.as_slice(),
        br#"[]"#.as_slice(),
        b"not json at all".as_slice(),
        b"".as_slice(),
    ] {
        assert!(
            !RequestSpec::parse(s, Some(body)).stream,
            "{:?} 不该被判成流式",
            String::from_utf8_lossy(body)
        );
    }
    assert!(
        !RequestSpec::parse(s, None).stream,
        "看不见 body 时必须按非流式处理，而不是猜一个"
    );
}

/// 三种方言的输出上限字段都被认，且「没说」与「说了 0」能分开。
///
/// 守护的 bug（今天就在生产里）：`parse_body_peek`（`hold.rs:765-775`）只认
/// `max_tokens` 与 `max_completion_tokens`，于是每个 `/v1/responses` 请求的
/// `max_output_tokens` 都被丢掉、上限当成 0，预扣退化成保守估算、过度冻结余额。
/// 另一半守护的是把 `Option<i64>` 摊平成 `i64`：那样「客户端没说」与
/// 「客户端说了 0」会合并成同一个数，预扣从「按估算」静默变成「按 0」。
#[test]
fn every_dialect_spelling_of_the_output_cap_is_read_and_zero_is_not_absence() {
    let s = Surface::OpenAiResponses;
    for body in [
        br#"{"max_tokens":7}"#.as_slice(),
        br#"{"max_completion_tokens":7}"#.as_slice(),
        br#"{"max_output_tokens":7}"#.as_slice(),
    ] {
        assert_eq!(
            RequestSpec::parse(s, Some(body)).max_tokens,
            Some(7),
            "{} 里的输出上限没被读到",
            String::from_utf8_lossy(body)
        );
    }
    assert_eq!(
        RequestSpec::parse(s, Some(br#"{"model":"m"}"#)).max_tokens,
        None,
        "客户端没说上限"
    );
    assert_eq!(
        RequestSpec::parse(s, Some(br#"{"max_output_tokens":0}"#)).max_tokens,
        Some(0),
        "客户端明确说了 0，这与「没说」不是一回事"
    );
}

/// 「看不见 body」与「body 是空的」必须能分开。
///
/// 守护的 bug：给流式 body 的 peek 返回一个空切片当成解析输入。
/// 那样一个 500 MiB 的流式请求会被计费当成一个空 body（估算 0 token），
/// 而调用方连「我看不见」这件事都不知道。
#[test]
fn an_unreadable_body_is_marked_invisible_not_empty() {
    let s = Surface::OpenAiCompletions;
    assert!(!RequestSpec::parse(s, None).body_visible);
    assert!(!RequestSpec::parse(s, Some(b"\xff\xfe binary")).body_visible);

    let empty_object = RequestSpec::parse(s, Some(b"{}"));
    assert!(
        empty_object.body_visible,
        "`{{}}` 是看得见的、合法的、恰好没写任何字段的 body"
    );
    assert!(empty_object.model.is_none());
}

/// 模型名逐字节原样保留。
///
/// 守护的 bug：在 peek 阶段顺手 `trim()` / `to_lowercase()` 模型名。
/// 那个被改过的名字会被路由与计费两边使用，而 body 里送给上游的仍是原名 ——
/// 计价按一个模型、请求按另一个模型，两边永远对不上账。
#[test]
fn the_model_name_is_read_verbatim() {
    for raw in [" GPT-5 ", "Claude-Opus-4.5", "渠道/模型-名", "a\tb"] {
        let body = serde_json::to_vec(&serde_json::json!({ "model": raw })).unwrap();
        assert_eq!(
            RequestSpec::parse(Surface::OpenAiCompletions, Some(&body)).model(),
            Some(raw),
            "模型名被改写了"
        );
    }
}

/// `Accept` 永不参与判定（规则 S2），冲突时以 body 为准并告警（规则 S3）。
///
/// 守护的 bug：让 `Accept: text/event-stream` 把一个 `stream:false` 的请求
/// 提升为流式。网关会回 `text/event-stream` 头并进流式分支，而上游按 body 返回
/// 一次性 JSON —— 网关被迫自己把 JSON 切成 SSE 帧，凭空发明一次转义，
/// 并且掩盖了客户端与上游之间的真实分歧。
#[test]
fn accept_cannot_flip_the_decision_but_the_disagreement_is_reported() {
    let path = ENTRY_PATHS[2];
    let asked_json = RequestSpec::parse(Surface::AnthropicMessages, Some(br#"{"stream":false}"#));
    let asked_sse = RequestSpec::parse(Surface::AnthropicMessages, Some(br#"{"stream":true}"#));

    assert!(!asked_json.stream, "Accept 不该改变判定");
    assert!(accept_conflicts_with_body(
        &asked_json,
        &with_accept("text/event-stream"),
        path
    ));
    assert!(accept_conflicts_with_body(
        &asked_sse,
        &with_accept("application/json"),
        path
    ));

    for (spec, accept) in [
        (&asked_json, "application/json"),
        (&asked_sse, "text/event-stream"),
        (&asked_sse, "text/event-stream, application/json"),
        (&asked_json, "*/*"),
        (&asked_sse, "*/*"),
    ] {
        assert!(
            !accept_conflicts_with_body(spec, &with_accept(accept), path),
            "Accept: {accept} 与 stream={} 并不冲突，不该告警",
            spec.stream
        );
    }
    assert!(
        !accept_conflicts_with_body(&asked_sse, &json_headers(), path),
        "没有 Accept 头就没有分歧可言"
    );
}

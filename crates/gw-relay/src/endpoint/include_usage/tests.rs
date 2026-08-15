//! OWNER: worker `relay-endpoints`。
//!
//! 规范 2.11：这里**不抄**插入的那段字节，也不断言它有多少个字节。
//! 测的是缺陷 #4 的三条实测后果各自不再发生：键序不变、数字格式不变、
//! 客户端写的 `include_usage:false` 不被翻成 `true`；外加两条形状性质：
//! 拼出来仍是合法 JSON，且原字节逐位保留。

use super::*;
use crate::contract::Surface;

fn spec_for(body: &[u8], stream: bool) -> RequestSpec {
    let mut spec = RequestSpec::parse(Surface::OpenAiCompletions, Some(body));
    // `parse` 已经从 body 里读出了 stream；这里允许测试单独指定，
    // 用来覆盖「body 说非流式」这一支。
    spec.stream = stream;
    // 顶层不是对象的 body 解析不出来，`body_visible` 会是 false —— 那样
    // splice 会因为「看不见」提前返回，测不到「顶层不是对象」这一支。
    // 这里强制置真，把两条拒绝理由分开测。
    spec.body_visible = true;
    spec
}

fn splice(body: &Bytes) -> Option<Spliced> {
    let spec = spec_for(body, true);
    splice_include_usage(
        body,
        &spec,
        UpstreamDialect::OpenAiChat,
        IncludeUsagePolicy::Force,
    )
}

fn joined(s: &Spliced) -> Vec<u8> {
    [s.prefix.as_ref(), s.rest.as_ref()].concat()
}

/// 一份刻意难看的 body：键序非字典序、大整数超过 f64 精度、嵌套对象、
/// `\uXXXX` 转义（含代理对）、带尾随零的浮点。整体 round-trip 会把它们全改一遍
/// —— 键序重排、`seed` 落进 f64、`é` 被展开成原始 UTF-8、`1.50` 变 `1.5`。
const HOSTILE: &[u8] = br#"{"zeta":1,"seed":12345678901234567890,"alpha":{"z":1,"a":2},"model":"gpt-5","stream":true,"note":"\u00e9\ud83d\ude00","temp":1.50}"#;

/// 客户端写的字节**逐位保留**，只在最外层 `{` 之后多出一段前缀。
///
/// 守护的 bug（今天就在生产里）：`ensure_include_usage()`（`common.rs:248-268`）
/// 做整体 JSON round-trip，于是 `serde_json` 的 `BTreeMap` 把递归键序重排成字典序，
/// `"seed": 12345678901234567890` 落进 f64 变成 `1.2345678901234568e+22`（上游按浮点
/// 收 seed，可复现性没了），`1.50` 变成 `1.5`。
#[test]
fn the_clients_bytes_survive_bit_for_bit() {
    let body = Bytes::from_static(HOSTILE);
    let spliced = splice(&body).expect("缺 stream_options 的流式请求应当被插入");
    let out = joined(&spliced);

    assert!(
        out.ends_with(&body[1..]),
        "最外层 `{{` 之后的每一个字节都必须原样保留"
    );
    assert!(out.starts_with(&body[..1]), "开头的 `{{` 被动过了");
    assert!(out.len() > body.len(), "什么都没插进去");
    assert_eq!(
        spliced.rest,
        body.slice(1..),
        "尾段必须是原 Bytes 的切片，不是一份新拷贝"
    );
    assert_eq!(spliced.len(), out.len(), "长度自述与实际字节数对不上");
}

/// 拼出来是合法 JSON，原有的每个键值都还在，而且 `include_usage` 是 true。
///
/// 守护的 bug：插入点算错（例如插在第一个 `{` 之前，或漏掉分隔逗号）。
/// 那会产出一段坏 JSON，上游直接 400 —— 而客户端写的东西完全没问题。
#[test]
fn the_result_is_valid_json_that_asks_for_usage() {
    let body = Bytes::from_static(HOSTILE);
    let original: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let out = joined(&splice(&body).expect("应当被插入"));
    let patched: serde_json::Value = serde_json::from_slice(&out).expect("拼出来必须是合法 JSON");

    assert_eq!(
        patched["stream_options"]["include_usage"],
        serde_json::Value::Bool(true)
    );
    for (k, v) in original.as_object().unwrap() {
        assert_eq!(&patched[k], v, "原有字段 {k} 的值被改动了");
    }
}

/// 空对象不会被插出一个尾随逗号。
///
/// 守护的 bug：无条件插入带逗号的那一段。`{}` 会变成
/// `{"stream_options":{...},}` —— 非法 JSON，上游 400。
/// 这是一个只在最小 body 上才出现的边界，正常请求永远测不到它。
#[test]
fn an_empty_object_does_not_grow_a_trailing_comma() {
    for raw in [
        b"{}".as_slice(),
        b"{ }".as_slice(),
        b"{\n\t}".as_slice(),
        b"  {}".as_slice(),
    ] {
        let body = Bytes::copy_from_slice(raw);
        let out = joined(&splice(&body).expect("空对象也该被插入"));
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap_or_else(|e| {
            panic!(
                "{:?} 插入后变成了非法 JSON: {e}",
                String::from_utf8_lossy(raw)
            )
        });
        assert_eq!(
            parsed["stream_options"]["include_usage"],
            serde_json::Value::Bool(true)
        );
    }
}

/// 客户端自己写了 `stream_options` 就**一个字节都不动** —— 包括写了 `false`。
///
/// 守护的 bug（今天就在生产里）：`common.rs:262` 是无条件 `insert`，把客户端
/// 显式写的 `include_usage:false` 静默翻成 `true`。SSE 末尾于是多出一帧
/// `data: {"choices":[],"usage":{...}}`，任何手写 `chunk["choices"][0]` 的客户端
/// 在这一帧上抛 `IndexError` / `undefined is not an object`。
#[test]
fn a_client_that_already_spoke_is_never_overridden() {
    for raw in [
        br#"{"stream":true,"stream_options":{"include_usage":false}}"#.as_slice(),
        br#"{"stream":true,"stream_options":{"include_usage":true}}"#.as_slice(),
        br#"{"stream":true,"stream_options":null}"#.as_slice(),
    ] {
        let body = Bytes::copy_from_slice(raw);
        assert!(
            splice(&body).is_none(),
            "{} 已经写了 stream_options，网关不该再碰它",
            String::from_utf8_lossy(raw)
        );
    }
}

/// 嵌在字符串里的 `stream_options` 不算数。
///
/// 守护的 bug：用 `body.windows(n).any(...)` 做子串扫描代替顶层键判定。
/// 一个用户问「what does stream_options do?」的对话请求会被误判为
/// 「客户端已经写了」，于是不插入 —— usage 缺失，这一次请求静默落 fallback 计费。
#[test]
fn the_phrase_appearing_inside_a_message_is_not_a_top_level_key() {
    let body = Bytes::from_static(
        br#"{"stream":true,"messages":[{"role":"user","content":"what does stream_options do?"}]}"#,
    );
    let spliced = splice(&body).expect("顶层并没有 stream_options，应当插入");
    let patched: serde_json::Value = serde_json::from_slice(&joined(&spliced)).unwrap();
    assert_eq!(
        patched["stream_options"]["include_usage"],
        serde_json::Value::Bool(true)
    );
}

/// 只有「OpenAI Chat 上游 + 流式」这一种组合会被碰。
///
/// 其中 `OpenAiResponses` 是重点：Responses API 不认识 `stream_options`，
/// 塞进去上游直接 400。今天正是这么坏的 —— 缺陷 #1（打错端点）叠加缺陷 #4
/// （还塞 `stream_options`），入口 B 双重不可用。
///
/// 守护的 bug：把判定条件从「上游方言」放宽成「provider 属于 OpenAI 系」。
#[test]
fn only_streaming_openai_chat_upstreams_are_touched() {
    let body = Bytes::from_static(br#"{"stream":true}"#);
    for upstream in [
        UpstreamDialect::OpenAiChat,
        UpstreamDialect::OpenAiResponses,
        UpstreamDialect::AnthropicMessages,
        UpstreamDialect::GoogleGenerateContent,
    ] {
        for stream in [true, false] {
            let spec = spec_for(&body, stream);
            let touched =
                splice_include_usage(&body, &spec, upstream, IncludeUsagePolicy::Force).is_some();
            let should = stream && upstream == UpstreamDialect::OpenAiChat;
            assert_eq!(touched, should, "{upstream:?} / stream={stream} 的处理不对");
        }
    }
}

/// 「尊重客户端」这条路径**完全不碰**请求体。
///
/// 这是「透传优先于计费」的字面执行：部署方宁可少收准一点也不愿意让网关碰
/// 请求体时，代价是明确的、可预期的（usage 缺失 → fallback 计费），
/// 而不是像今天那样静默失真。
///
/// 守护的 bug：把开关做成「插入但记一个标记」。那不叫尊重客户端，
/// 字节照样被改了。
#[test]
fn respecting_the_client_means_not_touching_a_single_byte() {
    for raw in [HOSTILE, br#"{"stream":true}"#.as_slice(), b"{}".as_slice()] {
        let body = Bytes::copy_from_slice(raw);
        let spec = spec_for(&body, true);
        assert!(
            splice_include_usage(
                &body,
                &spec,
                UpstreamDialect::OpenAiChat,
                IncludeUsagePolicy::RespectClient
            )
            .is_none(),
            "RespectClient 下不该产生任何改写"
        );
    }
}

/// 看不见的 body、以及顶层不是对象的 body，一律不碰。
///
/// 守护的 bug：对一个顶层是数组的 body 也去插入。那会产出
/// `[{"stream_options":...}, ...]` 之类的坏字节，把一个本来只会被上游
/// 干脆拒绝的畸形请求，变成一个由网关制造的畸形请求。
#[test]
fn an_unreadable_or_non_object_body_is_left_alone() {
    let mut invisible = RequestSpec::parse(Surface::OpenAiCompletions, None);
    assert!(!invisible.body_visible);
    invisible.stream = true;
    assert!(
        splice_include_usage(
            &Bytes::from_static(b"{}"),
            &invisible,
            UpstreamDialect::OpenAiChat,
            IncludeUsagePolicy::Force
        )
        .is_none(),
        "body 看不见时不该改写它"
    );

    for raw in [b"[1,2]".as_slice(), b"\"str\"".as_slice(), b"".as_slice()] {
        let body = Bytes::copy_from_slice(raw);
        let spec = spec_for(&body, true);
        assert!(
            splice_include_usage(
                &body,
                &spec,
                UpstreamDialect::OpenAiChat,
                IncludeUsagePolicy::Force
            )
            .is_none(),
            "{:?} 顶层不是对象，不该被插入",
            String::from_utf8_lossy(raw)
        );
    }
}

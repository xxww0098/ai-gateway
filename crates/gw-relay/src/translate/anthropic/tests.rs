//! OWNER: worker `relay-anthropic`。
//!
//! 规范 2.11：**不许把源码里的字面量抄进断言**。所以这里测的全是
//! **不写死在源码里的性质**：
//!
//! - **往返不丢**：OpenAI 请求 → Anthropic → 再翻回 OpenAI，语义摘要逐字相等。
//!   这条比逐字节比对一串期望字符串强得多 —— 它能抓到单向测试根本看不见的字段遗漏。
//! - **分片不丢**：把所有 `input_json_delta` / `arguments` 增量拼起来，
//!   必须与上游发出的分片拼起来完全一致（工具参数分片累积是最容易写错的地方）。
//! - **分块无关**：同一条 SSE 流，整块喂与逐字节喂，产出必须逐帧相等。
//! - **序列合法**：`message_start` 先于任何 `content_block_delta`；
//!   每个 `content_block_start` 有配对的 `content_block_stop`；
//!   `message_stop` / `data: [DONE]` 最后且只有一次；`delta.role` 只在首帧。
//! - **缺失与零可分**：上游没给 usage 与上游给了 0，产出必须不同。
//! - **边界被拒**：`n>1` / `logprobs` / `response_format` / `seed` / `top_k` /
//!   `thinking` / 未知字段一律 400，而不是静默丢。
//!
//! 固定用例的**形状**取自 `crates/gw-provider/src/{openai_tests.rs, claude/tests.rs}`
//! 里的真实上游响应（只读引用，未改动那两个文件）。

use serde_json::{Value, json};

use super::{AnthropicToOpenAi, DEFAULT_MAX_TOKENS, OpenAiToAnthropic};
use crate::contract::{StreamTranslator, TranslateError, Translator};

const MODEL: &str = "upstream-model-under-test";

// ===================================================================== 工具

fn req(t: &impl Translator, body: &Value) -> Value {
    let out = t
        .translate_request(MODEL, body.to_string().as_bytes())
        .expect("translate_request");
    serde_json::from_slice(&out).expect("产出必须是合法 JSON")
}

fn req_err(t: &impl Translator, body: &Value) -> TranslateError {
    t.translate_request(MODEL, body.to_string().as_bytes())
        .expect_err("这个字段必须被拒，不许静默丢")
}

fn resp(t: &impl Translator, body: &Value) -> Value {
    let out = t
        .translate_response(body.to_string().as_bytes())
        .expect("translate_response");
    serde_json::from_slice(&out).expect("产出必须是合法 JSON")
}

/// 独立的 SSE 解析器：**不复用被测代码的解析器**，否则两边一起错就测不出来。
fn parse_frames(frames: &[bytes::Bytes]) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    for frame in frames {
        let text = std::str::from_utf8(frame).expect("SSE 帧必须是 UTF-8");
        assert!(text.ends_with("\n\n"), "每一帧都必须以空行收尾: {text:?}");
        let mut event = None;
        let mut data = String::new();
        for line in text.trim_end_matches('\n').split('\n') {
            if let Some(v) = line.strip_prefix("event: ") {
                event = Some(v.to_owned());
            } else if let Some(v) = line.strip_prefix("data: ") {
                data.push_str(v);
            }
        }
        out.push((event, data));
    }
    out
}

fn json_frames(frames: &[bytes::Bytes]) -> Vec<Value> {
    parse_frames(frames)
        .into_iter()
        .filter(|(_, d)| d != "[DONE]")
        .map(|(_, d)| serde_json::from_str(&d).expect("data 必须是合法 JSON"))
        .collect()
}

/// 把一条 SSE 流整块喂进去。
fn drive(t: &mut dyn StreamTranslator, wire: &str) -> Vec<bytes::Bytes> {
    let mut out = t.push(wire.as_bytes()).expect("push");
    out.extend(t.finish().expect("finish"));
    out
}

/// 同一条流逐字节喂进去。用于证明产出与网络分块无关。
fn drive_bytewise(t: &mut dyn StreamTranslator, wire: &str) -> Vec<bytes::Bytes> {
    let mut out = Vec::new();
    for b in wire.as_bytes() {
        out.extend(t.push(std::slice::from_ref(b)).expect("push"));
    }
    out.extend(t.finish().expect("finish"));
    out
}

// ============================================ 固定用例（形状取自 gw-provider 测试）

/// 形状对齐 `crates/gw-provider/src/claude/tests.rs:330-340`
/// （`message_start` 带 `input_tokens`、`message_delta` 带 `output_tokens`）。
const CLAUDE_STREAM: &str = concat!(
    "event: message_start\n",
    r#"data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-x","content":[],"usage":{"input_tokens":1200,"output_tokens":1}}}"#,
    "\n\n",
    "event: content_block_start\n",
    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}"#,
    "\n\n",
    "event: content_block_stop\n",
    r#"data: {"type":"content_block_stop","index":0}"#,
    "\n\n",
    "event: content_block_start\n",
    r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_9","name":"get_weather","input":{}}}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"loc"}}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"ation\":\"SF\"}"}}"#,
    "\n\n",
    "event: content_block_stop\n",
    r#"data: {"type":"content_block_stop","index":1}"#,
    "\n\n",
    "event: message_delta\n",
    r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":37}}"#,
    "\n\n",
    "event: message_stop\n",
    r#"data: {"type":"message_stop"}"#,
    "\n\n",
);

/// 上游发出的工具参数分片，拼起来就是完整的 JSON。断言用它，不用源码里的字面量。
const CLAUDE_TOOL_FRAGMENTS: [&str; 2] = ["{\"loc", "ation\":\"SF\"}"];

/// 形状对齐 `crates/gw-provider/src/usage_tests.rs:227-230`
/// （末帧 usage + `data: [DONE]` 收尾）。
const OPENAI_STREAM: &str = concat!(
    r#"data: {"id":"chatcmpl-7","object":"chat.completion.chunk","created":1,"model":"gpt-x","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
    "\n\n",
    r#"data: {"id":"chatcmpl-7","object":"chat.completion.chunk","created":1,"model":"gpt-x","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
    "\n\n",
    r#"data: {"id":"chatcmpl-7","object":"chat.completion.chunk","created":1,"model":"gpt-x","choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":null}]}"#,
    "\n\n",
    r#"data: {"id":"chatcmpl-7","object":"chat.completion.chunk","created":1,"model":"gpt-x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_2","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}"#,
    "\n\n",
    r#"data: {"id":"chatcmpl-7","object":"chat.completion.chunk","created":1,"model":"gpt-x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"loc"}}]}}]}"#,
    "\n\n",
    r#"data: {"id":"chatcmpl-7","object":"chat.completion.chunk","created":1,"model":"gpt-x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ation\":\"SF\"}"}}]}}]}"#,
    "\n\n",
    r#"data: {"id":"chatcmpl-7","object":"chat.completion.chunk","created":1,"model":"gpt-x","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
    "\n\n",
    r#"data: {"id":"chatcmpl-7","object":"chat.completion.chunk","created":1,"model":"gpt-x","choices":[],"usage":{"prompt_tokens":120,"completion_tokens":8,"prompt_tokens_details":{"cached_tokens":40},"completion_tokens_details":{"reasoning_tokens":30}}}"#,
    "\n\n",
    "data: [DONE]\n\n",
);

const OPENAI_TOOL_FRAGMENTS: [&str; 2] = ["{\"loc", "ation\":\"SF\"}"];

// ================================================ 请求：OpenAI → Anthropic

#[test]
fn system_role_is_hoisted_to_the_top_level_because_anthropic_has_no_system_role() {
    let marker = "系统提示的原文";
    let out = req(
        &OpenAiToAnthropic,
        &json!({
            "model": "ignored",
            "messages": [
                {"role": "system", "content": marker},
                {"role": "user", "content": "hi"},
            ],
        }),
    );

    let roles: Vec<&str> = out["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["role"].as_str())
        .collect();
    assert!(
        !roles.contains(&"system"),
        "Anthropic 没有 system role，它不许留在 messages[] 里：{roles:?}"
    );
    assert!(
        out["system"].to_string().contains(marker),
        "系统提示必须出现在顶层 system 里，实际：{}",
        out["system"]
    );
}

#[test]
fn max_tokens_is_supplied_when_missing_and_never_overridden_when_present() {
    let base = json!({"model": "ignored", "messages": [{"role": "user", "content": "hi"}]});

    let defaulted = req(&OpenAiToAnthropic, &base);
    assert_eq!(
        defaulted["max_tokens"].as_i64(),
        Some(DEFAULT_MAX_TOKENS),
        "Anthropic 的 max_tokens 是必填，缺失时必须补上"
    );

    // 客户端写了就必须原样透传 —— 补默认值不能变成覆盖客户端的值。
    for asked in [1_i64, DEFAULT_MAX_TOKENS - 1, DEFAULT_MAX_TOKENS * 3] {
        let mut body = base.clone();
        body["max_tokens"] = json!(asked);
        assert_eq!(
            req(&OpenAiToAnthropic, &body)["max_tokens"].as_i64(),
            Some(asked)
        );

        // `max_completion_tokens` 是 OpenAI 的新名字，优先级更高。
        let mut newer = base.clone();
        newer["max_tokens"] = json!(asked + 7);
        newer["max_completion_tokens"] = json!(asked);
        assert_eq!(
            req(&OpenAiToAnthropic, &newer)["max_tokens"].as_i64(),
            Some(asked)
        );
    }
}

#[test]
fn semantic_openai_fields_without_an_anthropic_counterpart_are_rejected_not_dropped() {
    let base = json!({"model": "ignored", "messages": [{"role": "user", "content": "hi"}]});
    // 每一条都是「静默丢会让客户端拿到 200 却收到错东西」的字段。
    let poisons = [
        ("n", json!(5)),
        ("logprobs", json!(true)),
        ("top_logprobs", json!(3)),
        ("response_format", json!({"type": "json_object"})),
        ("seed", json!(42)),
        ("frequency_penalty", json!(0.5)),
        ("presence_penalty", json!(-1.5)),
        ("logit_bias", json!({"1234": -100})),
        ("reasoning_effort", json!("high")),
        ("service_tier", json!("flex")),
        ("a_field_that_does_not_exist_upstream", json!(1)),
    ];
    for (key, value) in poisons {
        let mut body = base.clone();
        body[key] = value;
        let err = req_err(&OpenAiToAnthropic, &body);
        assert!(
            matches!(err, TranslateError::Unsupported(_)),
            "`{key}` 必须判 Unsupported（→400），实际：{err:?}"
        );
        assert!(
            err.to_string().contains(key),
            "错误信息必须点名是哪个字段，否则客户端无从下手：{err}"
        );
    }
}

#[test]
fn identity_valued_fields_are_allowed_through_because_dropping_them_changes_nothing() {
    let base = json!({"model": "ignored", "messages": [{"role": "user", "content": "hi"}]});
    for (key, value) in [
        ("n", json!(1)),
        ("logprobs", json!(false)),
        ("frequency_penalty", json!(0)),
        ("presence_penalty", json!(0.0)),
        ("response_format", json!({"type": "text"})),
        ("service_tier", json!("auto")),
        ("stream_options", json!({"include_usage": false})),
        ("seed", Value::Null),
    ] {
        let mut body = base.clone();
        body[key] = value;
        req(&OpenAiToAnthropic, &body); // 不 panic 即通过
    }
}

#[test]
fn consecutive_tool_results_merge_into_one_user_message_so_roles_still_alternate() {
    // 一个 assistant 回合并发调两个工具，OpenAI 侧是两条 role=tool 消息。
    // Anthropic 侧必须挤进**一条** user 消息，否则上游 400 "roles must alternate"。
    let out = req(
        &OpenAiToAnthropic,
        &json!({
            "model": "ignored",
            "messages": [
                {"role": "user", "content": "q"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "a", "type": "function", "function": {"name": "f", "arguments": "{}"}},
                    {"id": "b", "type": "function", "function": {"name": "g", "arguments": "{}"}},
                ]},
                {"role": "tool", "tool_call_id": "a", "content": "ra"},
                {"role": "tool", "tool_call_id": "b", "content": "rb"},
            ],
        }),
    );

    let roles: Vec<&str> = out["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["role"].as_str())
        .collect();
    assert!(
        roles.windows(2).all(|w| w[0] != w[1]),
        "相邻同 role 的消息会被 Anthropic 直接 400：{roles:?}"
    );

    // 合并不许吃掉任何一条 tool_result。
    let ids: Vec<String> = out["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["type"] == "tool_result")
        .map(|b| b["tool_use_id"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(ids.len(), 2, "两条 tool_result 一条都不能丢：{ids:?}");
}

// ================================================ 请求：Anthropic → OpenAI

#[test]
fn top_level_system_becomes_the_first_message_because_openai_has_no_system_field() {
    let marker = "系统提示的原文";
    let out = req(
        &AnthropicToOpenAi,
        &json!({
            "model": "ignored",
            "max_tokens": 16,
            "system": marker,
            "messages": [{"role": "user", "content": "hi"}],
        }),
    );
    assert!(out.get("system").is_none(), "OpenAI 顶层没有 system 字段");
    let first = &out["messages"][0];
    assert_eq!(first["role"].as_str(), Some("system"));
    assert!(first["content"].to_string().contains(marker));
}

#[test]
fn semantic_anthropic_fields_without_an_openai_counterpart_are_rejected_not_dropped() {
    let base = json!({
        "model": "ignored",
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "hi"}],
    });
    let poisons = [
        ("top_k", json!(40)),
        (
            "thinking",
            json!({"type": "enabled", "budget_tokens": 1024}),
        ),
        ("mcp_servers", json!([{"name": "x"}])),
        ("a_field_that_does_not_exist_upstream", json!(1)),
    ];
    for (key, value) in poisons {
        let mut body = base.clone();
        body[key] = value;
        let err = req_err(&AnthropicToOpenAi, &body);
        assert!(
            matches!(err, TranslateError::Unsupported(_)),
            "`{key}` 必须判 Unsupported（→400），实际：{err:?}"
        );
        assert!(err.to_string().contains(key), "错误信息必须点名字段：{err}");
    }

    // thinking / redacted_thinking **内容块**同样不许放行：块上的 signature
    // 是多轮 extended thinking 的凭据，过一遍 OpenAI 就回不来了。
    let mut with_block = base.clone();
    with_block["messages"] = json!([{
        "role": "assistant",
        "content": [{"type": "thinking", "thinking": "…", "signature": "sig"}],
    }]);
    assert!(matches!(
        req_err(&AnthropicToOpenAi, &with_block),
        TranslateError::Unsupported(_)
    ));
}

#[test]
fn tool_results_are_emitted_before_the_remaining_user_content() {
    // OpenAI 要求 role=tool 紧跟带 tool_calls 的 assistant 消息之后。
    // 一条 Anthropic user 消息里 tool_result 与 text 混排时，顺序不能反。
    let out = req(
        &AnthropicToOpenAi,
        &json!({
            "model": "ignored",
            "max_tokens": 16,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "还有一句话"},
                    {"type": "tool_result", "tool_use_id": "t1", "content": "结果"},
                ],
            }],
        }),
    );
    let roles: Vec<&str> = out["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["role"].as_str())
        .collect();
    let tool_at = roles.iter().position(|r| *r == "tool");
    let user_at = roles.iter().position(|r| *r == "user");
    assert!(
        tool_at < user_at,
        "tool 消息必须排在剩余 user 内容之前：{roles:?}"
    );
}

// ============================================================ 往返（最强的一条）

/// 一个 OpenAI 请求的**语义摘要**。往返测试比对的是它，不是字节 ——
/// 字节会因为 `"Hi"` 与 `[{"type":"text","text":"Hi"}]` 这种同义写法而不等，
/// 但**语义**一个字都不许丢。
fn summarize_openai(v: &Value) -> Value {
    let messages: Vec<Value> = v["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            json!({
                "role": m["role"],
                "text": flatten_text(&m["content"]),
                "tool_call_id": m.get("tool_call_id").cloned().unwrap_or(Value::Null),
                "images": collect_images(&m["content"]),
                "tool_calls": m.get("tool_calls").and_then(Value::as_array).map(|calls| {
                    calls.iter().map(|c| json!({
                        "id": c["id"],
                        "name": c["function"]["name"],
                        // arguments 是「字符串里的 JSON」：比对解析后的值，
                        // 否则键序或空白的差异会造成假阴性。
                        "input": serde_json::from_str::<Value>(
                            c["function"]["arguments"].as_str().unwrap_or("{}")
                        ).unwrap_or(Value::Null),
                    })).collect::<Vec<_>>()
                }),
            })
        })
        .collect();
    let tools: Vec<Value> = v["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|t| {
            json!({
                "name": t["function"]["name"],
                "description": t["function"]["description"],
                "parameters": t["function"]["parameters"],
            })
        })
        .collect();
    json!({
        "messages": messages,
        "tools": tools,
        "max_tokens": v.get("max_tokens").or_else(|| v.get("max_completion_tokens")).cloned(),
        "temperature": v.get("temperature").cloned(),
        "top_p": v.get("top_p").cloned(),
        "stop": normalize_stop(v.get("stop")),
        "tool_choice": v.get("tool_choice").cloned(),
        "parallel_tool_calls": v.get("parallel_tool_calls").cloned(),
        "user": v.get("user").cloned(),
        "stream": v.get("stream").cloned(),
    })
}

fn flatten_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}

fn collect_images(content: &Value) -> Vec<String> {
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p["image_url"]["url"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// `stop` 在 OpenAI 侧可以是字符串或数组，两种写法语义相同。
fn normalize_stop(stop: Option<&Value>) -> Value {
    match stop {
        Some(Value::String(s)) => json!([s]),
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

#[test]
fn an_openai_request_survives_a_round_trip_through_anthropic_without_losing_semantics() {
    let original = json!({
        "model": "whatever",
        "messages": [
            {"role": "system", "content": "你是一个助手"},
            {"role": "user", "content": [
                {"type": "text", "text": "看这张图"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJD"}},
            ]},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call_1", "type": "function",
                 "function": {"name": "get_weather", "arguments": "{\"city\":\"SF\",\"unit\":\"c\"}"}},
            ]},
            {"role": "tool", "tool_call_id": "call_1", "content": "18°C"},
            {"role": "user", "content": "谢谢"},
        ],
        "tools": [{"type": "function", "function": {
            "name": "get_weather",
            "description": "查天气",
            "parameters": {"type": "object", "properties": {"city": {"type": "string"}},
                           "required": ["city"]},
        }}],
        "tool_choice": "required",
        "parallel_tool_calls": false,
        "max_tokens": 512,
        "temperature": 0.3,
        "top_p": 0.9,
        "stop": ["<<END>>"],
        "user": "tenant-42",
        "stream": true,
    });

    let as_anthropic = req(&OpenAiToAnthropic, &original);
    let back = req(&AnthropicToOpenAi, &as_anthropic);

    assert_eq!(
        summarize_openai(&back),
        summarize_openai(&original),
        "往返之后语义摘要必须逐字相等 —— 不相等就是有字段在某个方向上被丢了。\
         中间的 Anthropic 形状：{as_anthropic}"
    );
}

// ================================================ 响应（非流式）

#[test]
fn anthropic_response_usage_lands_on_the_openai_counterparts() {
    let (input, output, cached) = (1200_i64, 37_i64, 40_i64);
    let out = resp(
        &OpenAiToAnthropic,
        &json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-x",
            "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": input, "output_tokens": output,
                      "cache_read_input_tokens": cached},
        }),
    );
    assert_eq!(out["usage"]["prompt_tokens"].as_i64(), Some(input));
    assert_eq!(out["usage"]["completion_tokens"].as_i64(), Some(output));
    assert_eq!(out["usage"]["total_tokens"].as_i64(), Some(input + output));
    assert_eq!(
        out["usage"]["prompt_tokens_details"]["cached_tokens"].as_i64(),
        Some(cached)
    );
}

#[test]
fn a_missing_usage_is_distinguishable_from_a_zero_usage() {
    let body = |usage: Value| {
        let mut v = json!({
            "id": "msg_1", "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
        });
        if !usage.is_null() {
            v["usage"] = usage;
        }
        v
    };
    let missing = resp(&OpenAiToAnthropic, &body(Value::Null));
    let zero = resp(
        &OpenAiToAnthropic,
        &body(json!({"input_tokens": 0, "output_tokens": 0})),
    );
    assert!(
        missing.get("usage").is_none(),
        "上游没给 usage 时不许凭空补 0 —— 那会让计费错过 fallback 分支"
    );
    assert_eq!(zero["usage"]["prompt_tokens"].as_i64(), Some(0));
}

#[test]
fn stop_reason_and_finish_reason_round_trip_on_the_four_shared_meanings() {
    // 「结束了 / 到长度上限了 / 要调工具了 / 被拦了」这四种含义两边都有，
    // 翻过去再翻回来必须回到同一个桶。这是一条不写死在源码里的性质。
    let content = json!([{"type": "text", "text": "x"}]);
    for reason in ["end_turn", "max_tokens", "tool_use", "refusal"] {
        let as_openai = resp(
            &OpenAiToAnthropic,
            &json!({"id": "m", "content": content, "stop_reason": reason}),
        );
        let finish = as_openai["choices"][0]["finish_reason"].clone();
        let back = resp(
            &AnthropicToOpenAi,
            &json!({"id": "m", "choices": [{"index": 0,
                "message": {"role": "assistant", "content": "x"},
                "finish_reason": finish}]}),
        );
        assert_eq!(
            back["stop_reason"].as_str(),
            Some(reason),
            "stop_reason `{reason}` 在两次转义之后没回到原来的桶"
        );
    }
}

#[test]
fn a_tool_call_survives_the_response_round_trip_with_its_arguments_intact() {
    let input = json!({"city": "SF", "nested": {"a": [1, 2, 3]}});
    let as_openai = resp(
        &OpenAiToAnthropic,
        &json!({
            "id": "m", "model": "claude-x", "stop_reason": "tool_use",
            "content": [{"type": "tool_use", "id": "toolu_1", "name": "f", "input": input}],
        }),
    );
    let call = &as_openai["choices"][0]["message"]["tool_calls"][0];
    let parsed: Value = serde_json::from_str(
        call["function"]["arguments"]
            .as_str()
            .expect("arguments 必须是字符串"),
    )
    .expect("arguments 必须是合法 JSON");
    assert_eq!(parsed, input, "工具参数在 object ↔ string 之间不许丢");

    let back = resp(&AnthropicToOpenAi, &as_openai);
    assert_eq!(back["content"][0]["input"], input);
    assert_eq!(back["content"][0]["id"], call["id"]);
}

#[test]
fn an_upstream_error_body_is_re_enveloped_into_the_entry_dialect() {
    // 客户端 SDK 只会解析它自己那套错误结构，回一个陌生结构会被渲染成无字红叉。
    let marker = "上游说的原话";
    let to_openai = resp(
        &OpenAiToAnthropic,
        &json!({"type": "error", "error": {"type": "rate_limit_error", "message": marker}}),
    );
    assert_eq!(to_openai["error"]["message"].as_str(), Some(marker));
    assert!(
        to_openai.get("type").is_none(),
        "OpenAI 的错误信封没有顶层 type"
    );

    let to_anthropic = resp(
        &AnthropicToOpenAi,
        &json!({"error": {"type": "invalid_request_error", "message": marker}}),
    );
    assert_eq!(to_anthropic["type"].as_str(), Some("error"));
    assert_eq!(to_anthropic["error"]["message"].as_str(), Some(marker));
}

// ================================================ 流式：Anthropic → OpenAI

#[test]
fn openai_frames_carry_role_only_in_the_first_frame_and_end_with_exactly_one_done() {
    let mut t = OpenAiToAnthropic.stream_translator();
    let frames = drive(t.as_mut(), CLAUDE_STREAM);
    let parsed = parse_frames(&frames);

    let done: Vec<usize> = parsed
        .iter()
        .enumerate()
        .filter(|(_, (_, d))| d == "[DONE]")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(done, vec![parsed.len() - 1], "`[DONE]` 必须最后且只有一次");

    let with_role: Vec<usize> = json_frames(&frames)
        .iter()
        .enumerate()
        .filter(|(_, v)| v["choices"][0]["delta"].get("role").is_some())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(with_role, vec![0], "`delta.role` 只许在首帧出现");
}

#[test]
fn anthropic_tool_argument_fragments_reach_openai_without_loss() {
    let mut t = OpenAiToAnthropic.stream_translator();
    let frames = drive(t.as_mut(), CLAUDE_STREAM);
    let glued: String = json_frames(&frames)
        .iter()
        .filter_map(|v| {
            v["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"]
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(
        glued,
        CLAUDE_TOOL_FRAGMENTS.concat(),
        "所有 arguments 增量拼起来必须等于上游 partial_json 拼起来的结果"
    );
    assert!(
        serde_json::from_str::<Value>(&glued).is_ok(),
        "拼出来的必须是完整的 JSON，缺一个分片就不是了"
    );
}

#[test]
fn anthropic_usage_split_across_first_and_last_frame_is_fully_recovered() {
    let mut t = OpenAiToAnthropic.stream_translator();
    drive(t.as_mut(), CLAUDE_STREAM);
    let usage = t
        .usage()
        .expect("走转义路径时 usage() 取代 UsageProbe，绝不能是 None");
    // input_tokens 只在首帧 message_start，output_tokens 只在末帧 message_delta。
    // 任何一边丢了都会让计费落 fallback。
    assert!(usage.input_tokens.is_some(), "首帧的 input_tokens 丢了");
    assert!(usage.output_tokens.is_some(), "末帧的 output_tokens 丢了");
    assert!(!usage.is_empty());
}

#[test]
fn an_anthropic_mid_stream_error_event_is_surfaced_not_swallowed_as_a_clean_eof() {
    // 缺陷 #6：静默收尾会让客户端把截断的回答当成模型的完整回答。
    let mut t = OpenAiToAnthropic.stream_translator();
    let head = CLAUDE_STREAM.split("event: message_delta").next().unwrap();
    t.push(head.as_bytes()).expect("前半段正常");
    let err = t
        .push(
            concat!(
                "event: error\n",
                r#"data: {"type":"error","error":{"type":"overloaded_error"}}"#,
                "\n\n",
            )
            .as_bytes(),
        )
        .expect_err("中途报错必须上抛，不能补一帧 [DONE] 假装干净结束");
    assert!(matches!(err, TranslateError::UpstreamShape(_)));
}

// ================================================ 流式：OpenAI → Anthropic

/// Anthropic 方言的流式序列合法性 —— 规范 2.11 要的「不写死在源码里的性质」。
fn assert_anthropic_sequence_is_legal(frames: &[bytes::Bytes]) {
    let events: Vec<String> = parse_frames(frames)
        .into_iter()
        .map(|(e, _)| e.expect("Anthropic 的每一帧都必须带 event: 行"))
        .collect();
    let values = json_frames(frames);

    assert_eq!(
        events.iter().filter(|e| *e == "message_start").count(),
        1,
        "message_start 只许有一次：{events:?}"
    );
    let start = events.iter().position(|e| e == "message_start").unwrap();
    let first_delta = events.iter().position(|e| e == "content_block_delta");
    assert!(
        first_delta.is_none_or(|d| start < d),
        "message_start 必须先于任何 content_block_delta：{events:?}"
    );

    assert_eq!(
        events.iter().filter(|e| *e == "message_stop").count(),
        1,
        "message_stop 只许有一次：{events:?}"
    );
    assert_eq!(
        events.last().map(String::as_str),
        Some("message_stop"),
        "message_stop 必须是最后一帧：{events:?}"
    );

    // 每个 content_block_start 必须有配对的 content_block_stop，且 index 对得上。
    let mut open: Option<i64> = None;
    for (event, value) in events.iter().zip(&values) {
        let index = value["index"].as_i64();
        match event.as_str() {
            "content_block_start" => {
                assert!(
                    open.is_none(),
                    "上一个 content block 没关就开了新的：{events:?}"
                );
                open = index;
            }
            "content_block_delta" => assert_eq!(
                index, open,
                "content_block_delta 落在了一个没开的 block 上：{events:?}"
            ),
            "content_block_stop" => {
                assert_eq!(index, open, "content_block_stop 的 index 与 start 对不上");
                open = None;
            }
            _ => {}
        }
    }
    assert!(
        open.is_none(),
        "有 content_block_start 没有配对的 stop：{events:?}"
    );
}

#[test]
fn synthesized_anthropic_frames_form_a_legal_event_sequence() {
    let mut t = AnthropicToOpenAi.stream_translator();
    let frames = drive(t.as_mut(), OPENAI_STREAM);
    assert_anthropic_sequence_is_legal(&frames);
}

#[test]
fn openai_tool_argument_fragments_accumulate_into_input_json_delta_without_loss() {
    let mut t = AnthropicToOpenAi.stream_translator();
    let frames = drive(t.as_mut(), OPENAI_STREAM);
    let glued: String = json_frames(&frames)
        .iter()
        .filter_map(|v| v["delta"]["partial_json"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        glued,
        OPENAI_TOOL_FRAGMENTS.concat(),
        "所有 partial_json 拼起来必须等于上游 arguments 分片拼起来的结果"
    );
    assert!(serde_json::from_str::<Value>(&glued).is_ok());
}

#[test]
fn openai_last_frame_usage_is_fully_recovered_including_reasoning_tokens() {
    let mut t = AnthropicToOpenAi.stream_translator();
    drive(t.as_mut(), OPENAI_STREAM);
    let usage = t.usage().expect("usage() 取代 UsageProbe，绝不能是 None");
    for (name, value) in [
        ("input_tokens", usage.input_tokens),
        ("output_tokens", usage.output_tokens),
        ("cached_tokens", usage.cached_tokens),
        ("reasoning_tokens", usage.reasoning_tokens),
    ] {
        assert!(value.is_some(), "上游末帧给了 {name}，转义器不许丢");
    }
}

#[test]
fn a_stream_that_never_starts_produces_no_frames_at_all() {
    // 上游一帧都没给时，凭空造一个 message_start/message_stop 会让客户端
    // 以为拿到了一个空回答。什么都不发，让上层看见「上游什么都没给」。
    let mut t = AnthropicToOpenAi.stream_translator();
    assert!(drive(t.as_mut(), "").is_empty());
    assert!(t.usage().is_none(), "没有 usage 时必须是 None，不是全零");
}

// ============================================================ 分块无关性

#[test]
fn stream_output_is_independent_of_how_the_upstream_bytes_are_chunked() {
    // SSE 事件边界与 TCP 分段没有任何关系。逐字节喂与整块喂必须产出同一串帧 ——
    // 这条挂了就意味着某个跨分块的事件被吞了或被切成了两半。
    for (whole, bytewise) in [
        (
            drive(
                OpenAiToAnthropic.stream_translator().as_mut(),
                CLAUDE_STREAM,
            ),
            drive_bytewise(
                OpenAiToAnthropic.stream_translator().as_mut(),
                CLAUDE_STREAM,
            ),
        ),
        (
            drive(
                AnthropicToOpenAi.stream_translator().as_mut(),
                OPENAI_STREAM,
            ),
            drive_bytewise(
                AnthropicToOpenAi.stream_translator().as_mut(),
                OPENAI_STREAM,
            ),
        ),
    ] {
        assert!(!whole.is_empty());
        assert_eq!(
            parse_frames(&whole),
            parse_frames(&bytewise),
            "整块喂与逐字节喂产出不一致"
        );
    }
}

#[test]
fn a_translator_reports_the_grid_cell_it_covers() {
    // 派发靠这两个方法把转义器与格子对上，接反了会静默走错上游。
    assert_eq!(
        OpenAiToAnthropic.surface(),
        crate::Surface::OpenAiCompletions
    );
    assert_eq!(
        OpenAiToAnthropic.to_dialect(),
        crate::UpstreamDialect::AnthropicMessages
    );
    assert_eq!(
        AnthropicToOpenAi.surface(),
        crate::Surface::AnthropicMessages
    );
    assert_eq!(
        AnthropicToOpenAi.to_dialect(),
        crate::UpstreamDialect::OpenAiChat
    );
}

#[test]
fn a_late_system_message_is_rejected_instead_of_being_hoisted() {
    let body = serde_json::json!({
        "model": "m",
        "messages": [
            {"role": "user", "content": "first"},
            {"role": "system", "content": "late instruction"}
        ]
    });
    let err = OpenAiToAnthropic
        .translate_request("m", body.to_string().as_bytes())
        .expect_err("late system changes ordering");
    assert!(matches!(err, TranslateError::Unsupported(_)));
}

#[test]
fn tool_arguments_must_encode_an_object_for_anthropic() {
    let body = serde_json::json!({
        "model": "m",
        "messages": [{
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call-1",
                "type": "function",
                "function": {"name": "f", "arguments": "[1,2,3]"}
            }]
        }]
    });
    let err = OpenAiToAnthropic
        .translate_request("m", body.to_string().as_bytes())
        .expect_err("Anthropic tool input is an object");
    assert!(matches!(err, TranslateError::Malformed(_)));
}

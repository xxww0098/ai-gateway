//! OWNER: worker `relay-google`。
//!
//! 规范 2.11：**不许把源码里的字面量抄进断言**。这里测的全是**不写死在源码里
//! 的性质**：
//!
//! - **字节逐位相等**：喂进去的文本，转一圈出来必须一字不差；
//! - **多值不丢**：3 个工具进去必须 3 个出来，4 个采样参数进去必须 4 个落地，
//!   usage 的计数分散在多帧里也一个都不能少；
//! - **边界被拒**：未知字段、认不出的 tool id、远程图片 URL 必须 400，
//!   而不是被静默丢掉；
//! - **缺失与零可分**：`candidatesTokenCount: 0` 与「压根没给」是两件事；
//! - **序列合法性**：产出的帧序列在**目标方言里**合法（`message_start` 先于
//!   任何 delta、`message_stop` 最后且只有一次、block index 严格递增）。
//!   这一条不比对任何期望字符串 —— 它检查的是一条不变量，不是一份抄件。

use bytes::Bytes;
use serde_json::{Value, json};

use super::{AnthropicToGoogle, OpenAiToGoogle};
use crate::contract::{RelayUsage, TranslateError, Translator};

// ===================================================================== 夹具

/// 真实形状的 Google `alt=sse` 帧序列。
///
/// 形状取自 `crates/gw-provider/src/{gemini,vertex}/tests.rs` 里已有的上游
/// 响应固定用例（只读引用）：`data:` 前缀 + `candidates[].content.parts[]` +
/// `usageMetadata` 的四个 `*TokenCount` 键。usage 是**累计量**，故意拆成
/// 「首帧只有 prompt、末帧才补齐」—— 这正是合并逻辑最容易出错的地方。
fn google_sse_frames() -> Vec<Vec<u8>> {
    vec![
        concat!(
            r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"Hel"}]}}],"#,
            r#""usageMetadata":{"promptTokenCount":11},"modelVersion":"gemini-2.5-flash"}"#,
            "\n\n"
        )
        .into(),
        // 上游的注释帧 / 心跳。目标方言里没有对应物 —— 必须产出零帧。
        ": keep-alive\n\n".into(),
        concat!(
            r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"lo, world"}]}}]}"#,
            "\n\n"
        )
        .into(),
        // 末帧**不带** `promptTokenCount`。上游按帧给部分计数是真实行为
        // （见 `gw-provider/src/vertex/tests.rs` 里那两帧不同字段集的 usage），
        // 而这正是「整体替换」式合并会把首帧的 input 计数抹掉的地方。
        concat!(
            r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"!"}]},"#,
            r#""finishReason":"STOP"}],"usageMetadata":{"candidatesTokenCount":7,"#,
            r#""cachedContentTokenCount":3,"thoughtsTokenCount":2}}"#,
            "\n\n"
        )
        .into(),
    ]
}

/// 上面那串帧里模型真正说出来的话。断言用它做**字节逐位相等**，
/// 而不是抄一份期望的输出 JSON。
const SPOKEN: &str = "Hello, world!";

/// 非流式的真实形状响应。
fn google_response(finish: &str) -> Vec<u8> {
    json!({
        "candidates": [{
            "content": { "role": "model", "parts": [{ "text": SPOKEN }] },
            "finishReason": finish,
        }],
        "usageMetadata": {
            "promptTokenCount": 11,
            "candidatesTokenCount": 7,
            "cachedContentTokenCount": 3,
        },
        "modelVersion": "gemini-2.5-flash",
    })
    .to_string()
    .into_bytes()
}

// ================================================================= 小工具

fn to_google(t: &dyn Translator, body: &Value) -> Value {
    let out = t
        .translate_request("some-model", body.to_string().as_bytes())
        .expect("request should translate");
    serde_json::from_slice(&out).expect("translated request must be JSON")
}

fn to_google_err(t: &dyn Translator, body: &Value) -> TranslateError {
    t.translate_request("some-model", body.to_string().as_bytes())
        .expect_err("this request must be rejected, not silently accepted")
}

fn from_google(t: &dyn Translator, body: &[u8]) -> Value {
    let out = t
        .translate_response(body)
        .expect("response should translate");
    serde_json::from_slice(&out).expect("translated response must be JSON")
}

/// 把产出的帧拆成 `(event 名, data 载荷)`，顺便断言每一帧都是合法 SSE。
fn decode(frames: &[Bytes]) -> Vec<(Option<String>, Value)> {
    frames
        .iter()
        .map(|frame| {
            let text = std::str::from_utf8(frame).expect("frame must be UTF-8");
            assert!(
                text.ends_with("\n\n"),
                "every SSE frame must be terminated by a blank line"
            );
            let mut event = None;
            let mut data = String::new();
            for line in text.lines() {
                if let Some(name) = line.strip_prefix("event: ") {
                    event = Some(name.to_owned());
                }
                if let Some(payload) = line.strip_prefix("data: ") {
                    data.push_str(payload);
                }
            }
            let value = serde_json::from_str(&data).unwrap_or_else(|_| Value::String(data.clone()));
            (event, value)
        })
        .collect()
}

/// 跑完整条流：逐帧 `push`，再 `finish`。
fn run_stream(t: &dyn Translator) -> (Vec<(Option<String>, Value)>, Option<RelayUsage>) {
    let mut st = t.stream_translator();
    let mut frames = Vec::new();
    for (idx, raw) in google_sse_frames().iter().enumerate() {
        let produced = st
            .push(raw)
            .expect("push should not fail on a real Google frame");
        if idx == 1 {
            assert!(
                produced.is_empty(),
                "an upstream comment frame has no counterpart downstream: zero frames is legal"
            );
        }
        frames.extend(produced);
    }
    frames.extend(st.finish().expect("finish should not fail"));
    (decode(&frames), st.usage())
}

// ============================================================ 请求 · 共通

/// 两个转义器都必须声明自己覆盖的那一格，且都指向同一个上游方言
/// （2 个转义器覆盖 4 格的前提）。
#[test]
fn both_translators_target_the_same_upstream_dialect() {
    let a = OpenAiToGoogle;
    let c = AnthropicToGoogle;
    assert_eq!(a.to_dialect(), c.to_dialect());
    assert_ne!(
        a.surface(),
        c.surface(),
        "they must cover different entry surfaces, otherwise one of the four cells is unreachable"
    );
}

/// `model` 属于 URL，不属于 body。塞进 GenerateContent 的 body 里，
/// Google 会以 `Unknown name "model"` 拒收整个请求。
#[test]
fn model_never_leaks_into_the_google_body() {
    let openai = to_google(
        &OpenAiToGoogle,
        &json!({ "model": "gpt-4o", "messages": [{ "role": "user", "content": "hi" }] }),
    );
    let anthropic = to_google(
        &AnthropicToGoogle,
        &json!({
            "model": "claude-x", "max_tokens": 8,
            "messages": [{ "role": "user", "content": "hi" }],
        }),
    );
    assert!(openai.get("model").is_none());
    assert!(anthropic.get("model").is_none());
}

/// 未知顶层字段是**边界**，必须被拒。静默丢一个有语义的字段是审计报告里
/// 反复强调的那类错误 —— 比一个 400 坏得多。
#[test]
fn unknown_top_level_fields_are_rejected_not_dropped() {
    for (t, body) in [
        (
            &OpenAiToGoogle as &dyn Translator,
            json!({
                "messages": [{ "role": "user", "content": "hi" }],
                "logit_bias": { "42": -100 },
            }),
        ),
        (
            &AnthropicToGoogle as &dyn Translator,
            json!({
                "max_tokens": 8,
                "messages": [{ "role": "user", "content": "hi" }],
                "mcp_servers": [{ "url": "https://example.invalid" }],
            }),
        ),
    ] {
        assert!(
            matches!(to_google_err(t, &body), TranslateError::Unsupported(_)),
            "an unmappable field must surface as Unsupported so the caller can answer 400"
        );
    }
}

/// 白名单上的装饰性字段不能把请求打成 400 —— 它们是 SDK 无脑带上的，
/// 拒掉等于这一格对真实客户端不可用。
#[test]
fn decorative_fields_do_not_break_the_request() {
    to_google(
        &OpenAiToGoogle,
        &json!({
            "messages": [{ "role": "user", "content": "hi", "name": "alice" }],
            "stream": true,
            "stream_options": { "include_usage": true },
            "user": "tenant-7",
            "parallel_tool_calls": true,
        }),
    );
    to_google(
        &AnthropicToGoogle,
        &json!({
            "max_tokens": 8,
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": "hi", "cache_control": { "type": "ephemeral" } },
                { "type": "thinking", "thinking": "…", "signature": "abc" },
            ]}],
            "stream": true,
            "metadata": { "user_id": "u1" },
        }),
    );
}

/// 非默认值的并行工具开关**不是**装饰性的 —— Google 没这个旋钮，只能 400。
/// 这条和上一条是一对：同一个字段，默认值放行、非默认值拒绝。
#[test]
fn non_default_parallel_tool_switches_are_rejected() {
    assert!(matches!(
        to_google_err(
            &OpenAiToGoogle,
            &json!({
                "messages": [{ "role": "user", "content": "hi" }],
                "parallel_tool_calls": false,
            })
        ),
        TranslateError::Unsupported(_)
    ));
    assert!(matches!(
        to_google_err(
            &AnthropicToGoogle,
            &json!({
                "max_tokens": 8,
                "messages": [{ "role": "user", "content": "hi" }],
                "tool_choice": { "type": "auto", "disable_parallel_tool_use": true },
            })
        ),
        TranslateError::Unsupported(_)
    ));
}

// ============================================================ 请求 · 形状

/// `role: assistant` 必须变成 `model`，且 `assistant` 这个词一个都不能剩下。
#[test]
fn assistant_role_becomes_model_and_never_survives() {
    for (t, body) in [
        (
            &OpenAiToGoogle as &dyn Translator,
            json!({ "messages": [
                { "role": "user", "content": "q" },
                { "role": "assistant", "content": "a" },
                { "role": "user", "content": "q2" },
            ]}),
        ),
        (
            &AnthropicToGoogle as &dyn Translator,
            json!({ "max_tokens": 8, "messages": [
                { "role": "user", "content": "q" },
                { "role": "assistant", "content": "a" },
                { "role": "user", "content": "q2" },
            ]}),
        ),
    ] {
        let out = to_google(t, &body);
        let contents = out["contents"].as_array().expect("contents[]");
        assert_eq!(contents.len(), 3, "no message may be dropped");
        let roles: Vec<&str> = contents
            .iter()
            .map(|c| c["role"].as_str().expect("role"))
            .collect();
        assert!(
            roles.iter().all(|r| *r == "user" || *r == "model"),
            "GenerateContent only knows `user` and `model`, got {roles:?}"
        );
        assert_eq!(roles.iter().filter(|r| **r == "model").count(), 1);
    }
}

/// system 指令必须离开 `contents[]`，落到 `systemInstruction`，且文本一字不差。
#[test]
fn system_moves_out_of_contents_byte_for_byte() {
    let marker = "you are a \u{6d4b}\u{8bd5} bot";
    let openai = to_google(
        &OpenAiToGoogle,
        &json!({ "messages": [
            { "role": "system", "content": marker },
            { "role": "user", "content": "hi" },
        ]}),
    );
    let anthropic = to_google(
        &AnthropicToGoogle,
        &json!({
            "max_tokens": 8, "system": marker,
            "messages": [{ "role": "user", "content": "hi" }],
        }),
    );
    for out in [&openai, &anthropic] {
        assert_eq!(out["contents"].as_array().expect("contents[]").len(), 1);
        assert_eq!(out["systemInstruction"]["parts"][0]["text"], json!(marker));
    }
}

/// 四个采样参数进去，四个都要落在 `generationConfig` 里 —— 多值不丢。
/// 断言的是「四个互不相同的输入值都能在输出里找到」，不比对键名以外的东西。
#[test]
fn every_sampling_knob_reaches_generation_config() {
    let openai = to_google(
        &OpenAiToGoogle,
        &json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "temperature": 0.11, "top_p": 0.22, "max_tokens": 33, "stop": ["zzz"],
        }),
    );
    let cfg = &openai["generationConfig"];
    let flat = cfg.to_string();
    for needle in ["0.11", "0.22", "33", "zzz"] {
        assert!(
            flat.contains(needle),
            "generationConfig lost {needle}: {flat}"
        );
    }

    let anthropic = to_google(
        &AnthropicToGoogle,
        &json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "temperature": 0.11, "top_p": 0.22, "top_k": 7, "max_tokens": 33,
        }),
    );
    let flat = anthropic["generationConfig"].to_string();
    for needle in ["0.11", "0.22", "7", "33"] {
        assert!(
            flat.contains(needle),
            "generationConfig lost {needle}: {flat}"
        );
    }
}

/// `stop` 的字符串态与数组态都要落成 Google 的数组，且元素不丢。
#[test]
fn stop_accepts_both_shapes_without_losing_entries() {
    let one = to_google(
        &OpenAiToGoogle,
        &json!({ "messages": [{ "role": "user", "content": "x" }], "stop": "END" }),
    );
    let many = to_google(
        &OpenAiToGoogle,
        &json!({
            "messages": [{ "role": "user", "content": "x" }],
            "stop": ["A", "B", "C"],
        }),
    );
    assert_eq!(one["generationConfig"]["stopSequences"], json!(["END"]));
    assert_eq!(
        many["generationConfig"]["stopSequences"]
            .as_array()
            .expect("stopSequences")
            .len(),
        3
    );
}

/// `n = 1`（默认值）放行，`n > 1` 必须拒 —— 多候选在流式方向翻不出去，
/// 悄悄只回第一个就是丢掉了客户端真的要过的东西。
#[test]
fn multiple_candidates_are_rejected_rather_than_silently_truncated() {
    to_google(
        &OpenAiToGoogle,
        &json!({ "messages": [{ "role": "user", "content": "x" }], "n": 1 }),
    );
    assert!(matches!(
        to_google_err(
            &OpenAiToGoogle,
            &json!({ "messages": [{ "role": "user", "content": "x" }], "n": 3 })
        ),
        TranslateError::Unsupported(_)
    ));
}

/// 3 个工具进去必须 3 个 functionDeclaration 出来，名字一个不丢。
#[test]
fn every_tool_reaches_function_declarations() {
    let names = ["alpha", "beta", "gamma"];
    let openai = to_google(
        &OpenAiToGoogle,
        &json!({
            "messages": [{ "role": "user", "content": "x" }],
            "tools": names.map(|n| json!({
                "type": "function",
                "function": { "name": n, "parameters": { "type": "object" } },
            })).to_vec(),
        }),
    );
    let anthropic = to_google(
        &AnthropicToGoogle,
        &json!({
            "max_tokens": 8,
            "messages": [{ "role": "user", "content": "x" }],
            "tools": names.map(|n| json!({
                "name": n, "input_schema": { "type": "object" },
            })).to_vec(),
        }),
    );
    for out in [&openai, &anthropic] {
        let decls = out["tools"][0]["functionDeclarations"]
            .as_array()
            .expect("functionDeclarations[]");
        let got: Vec<&str> = decls
            .iter()
            .map(|d| d["name"].as_str().expect("name"))
            .collect();
        assert_eq!(got, names, "tool names must survive one-for-one");
    }
}

/// 工具结果靠**名字**回到 Google。名字来自上一轮的 tool_call，
/// 认不回来就必须 400 —— 编一个名字会让模型收到它没调用过的工具的结果。
#[test]
fn tool_results_resolve_by_name_or_are_rejected() {
    let resolved = to_google(
        &OpenAiToGoogle,
        &json!({ "messages": [
            { "role": "user", "content": "weather?" },
            { "role": "assistant", "tool_calls": [{
                "id": "call_abc", "type": "function",
                "function": { "name": "get_weather", "arguments": "{\"city\":\"SF\"}" },
            }]},
            { "role": "tool", "tool_call_id": "call_abc", "content": "sunny" },
        ]}),
    );
    let last = resolved["contents"]
        .as_array()
        .expect("contents[]")
        .last()
        .expect("last");
    assert_eq!(
        last["parts"][0]["functionResponse"]["name"],
        json!("get_weather")
    );

    assert!(matches!(
        to_google_err(
            &OpenAiToGoogle,
            &json!({ "messages": [
                { "role": "tool", "tool_call_id": "call_missing", "content": "?" },
            ]})
        ),
        TranslateError::Unsupported(_)
    ));
    assert!(matches!(
        to_google_err(
            &AnthropicToGoogle,
            &json!({ "max_tokens": 8, "messages": [
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_missing", "content": "?" },
                ]},
            ]})
        ),
        TranslateError::Unsupported(_)
    ));
}

/// OpenAI 的 `arguments` 是 JSON **字符串**，Google 的 `args` 是**对象**。
/// 转过去之后必须是对象，且内容一字不差。
#[test]
fn tool_call_arguments_are_reparsed_into_an_object() {
    let out = to_google(
        &OpenAiToGoogle,
        &json!({ "messages": [{ "role": "assistant", "tool_calls": [{
            "id": "c1", "type": "function",
            "function": { "name": "f", "arguments": "{\"n\":1,\"s\":\"x\"}" },
        }]}]}),
    );
    let args = &out["contents"][0]["parts"][0]["functionCall"]["args"];
    assert!(args.is_object(), "args must be an object, got {args}");
    assert_eq!(args["n"], json!(1));
    assert_eq!(args["s"], json!("x"));
}

/// Anthropic 的四种 block 都要有落点，一个都不能丢。
#[test]
fn anthropic_content_blocks_all_reach_parts() {
    let out = to_google(
        &AnthropicToGoogle,
        &json!({ "max_tokens": 8, "messages": [
            { "role": "user", "content": [
                { "type": "text", "text": "look" },
                { "type": "image", "source": {
                    "type": "base64", "media_type": "image/png", "data": "QUJD",
                }},
            ]},
            { "role": "assistant", "content": [
                { "type": "tool_use", "id": "toolu_1", "name": "probe", "input": { "q": 1 } },
            ]},
            { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "content": "done" },
            ]},
        ]}),
    );
    let contents = out["contents"].as_array().expect("contents[]");
    assert_eq!(contents.len(), 3);
    assert_eq!(contents[0]["parts"].as_array().expect("parts").len(), 2);
    assert_eq!(contents[0]["parts"][1]["inlineData"]["data"], json!("QUJD"));
    assert_eq!(
        contents[1]["parts"][0]["functionCall"]["name"],
        json!("probe")
    );
    assert_eq!(
        contents[2]["parts"][0]["functionResponse"]["name"],
        json!("probe")
    );
}

/// 远程图片是边界：Google 收不了，替客户端去下载是网关不该有的副作用。
#[test]
fn remote_images_are_rejected_on_both_surfaces() {
    assert!(matches!(
        to_google_err(
            &OpenAiToGoogle,
            &json!({ "messages": [{ "role": "user", "content": [
                { "type": "image_url", "image_url": { "url": "https://example.invalid/a.png" } },
            ]}]})
        ),
        TranslateError::Unsupported(_)
    ));
    assert!(matches!(
        to_google_err(
            &AnthropicToGoogle,
            &json!({ "max_tokens": 8, "messages": [{ "role": "user", "content": [
                { "type": "image", "source": { "type": "url", "url": "https://x.invalid/a.png" } },
            ]}]})
        ),
        TranslateError::Unsupported(_)
    ));
}

/// `is_error: true` 不能丢：模型看到「工具报错」和看到「工具返回了这段文本」
/// 会走完全不同的下一步。
#[test]
fn tool_result_error_flag_survives_translation() {
    let build = |is_error: bool| {
        to_google(
            &AnthropicToGoogle,
            &json!({ "max_tokens": 8, "messages": [
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "t1", "name": "f", "input": {} },
                ]},
                { "role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": "t1",
                    "content": "boom", "is_error": is_error,
                }]},
            ]}),
        )
    };
    assert_ne!(
        build(true)["contents"][1]["parts"][0]["functionResponse"]["response"],
        build(false)["contents"][1]["parts"][0]["functionResponse"]["response"],
        "an errored tool result must not look identical to a successful one"
    );
}

// ======================================================== 响应 · 非流式

/// 模型说的话必须**一字不差**地到达客户端方言的正确位置。
#[test]
fn spoken_text_survives_byte_for_byte() {
    let openai = from_google(&OpenAiToGoogle, &google_response("STOP"));
    let anthropic = from_google(&AnthropicToGoogle, &google_response("STOP"));
    assert_eq!(openai["choices"][0]["message"]["content"], json!(SPOKEN));
    assert_eq!(anthropic["content"][0]["text"], json!(SPOKEN));
    assert_eq!(anthropic["content"][0]["type"], json!("text"));
}

/// 三个 Google 终止原因在目标方言里必须**互不相同** —— 否则「说完了」、
/// 「被截断」、「被安全策略拦下」在客户端看来是同一件事。
/// 断言的是可区分性，不是任何一个具体字符串。
#[test]
fn distinct_google_finish_reasons_stay_distinguishable() {
    let openai: Vec<Value> = ["STOP", "MAX_TOKENS", "SAFETY"]
        .iter()
        .map(|r| {
            from_google(&OpenAiToGoogle, &google_response(r))["choices"][0]["finish_reason"].clone()
        })
        .collect();
    let anthropic: Vec<Value> = ["STOP", "MAX_TOKENS", "SAFETY"]
        .iter()
        .map(|r| from_google(&AnthropicToGoogle, &google_response(r))["stop_reason"].clone())
        .collect();
    for mapped in [&openai, &anthropic] {
        assert!(mapped.iter().all(Value::is_string));
        assert_ne!(mapped[0], mapped[1]);
        assert_ne!(mapped[1], mapped[2]);
        assert_ne!(mapped[0], mapped[2]);
    }
}

/// functionCall 必须让 finish reason 变成「去调工具」，否则客户端不会去读
/// tool_calls，一次工具调用就变成一个空回答。
#[test]
fn a_function_call_changes_the_finish_reason() {
    let body = json!({
        "candidates": [{
            "content": { "role": "model", "parts": [
                { "functionCall": { "name": "f", "args": { "a": 1 } } },
            ]},
            "finishReason": "STOP",
        }],
    })
    .to_string();
    let openai = from_google(&OpenAiToGoogle, body.as_bytes());
    let anthropic = from_google(&AnthropicToGoogle, body.as_bytes());
    assert_ne!(
        openai["choices"][0]["finish_reason"],
        from_google(&OpenAiToGoogle, &google_response("STOP"))["choices"][0]["finish_reason"]
    );
    assert_ne!(
        anthropic["stop_reason"],
        from_google(&AnthropicToGoogle, &google_response("STOP"))["stop_reason"]
    );
    // 工具名与参数本身也不能丢。
    assert_eq!(
        openai["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
        json!("f")
    );
    assert_eq!(anthropic["content"][0]["input"]["a"], json!(1));
}

/// prompt 被安全策略整体拦下时 `candidates` 是空的，拦截原因只在
/// `promptFeedback` 里。不读它就会把「被拦截」翻成「模型什么都没说」。
#[test]
fn a_blocked_prompt_is_not_reported_as_an_empty_answer() {
    let blocked = json!({ "promptFeedback": { "blockReason": "SAFETY" } }).to_string();
    let silent = json!({ "candidates": [{ "content": { "parts": [] } }] }).to_string();
    assert_ne!(
        from_google(&OpenAiToGoogle, blocked.as_bytes())["choices"][0]["finish_reason"],
        from_google(&OpenAiToGoogle, silent.as_bytes())["choices"][0]["finish_reason"],
    );
    assert_ne!(
        from_google(&AnthropicToGoogle, blocked.as_bytes())["stop_reason"],
        from_google(&AnthropicToGoogle, silent.as_bytes())["stop_reason"],
    );
}

/// 上游给的不是 GenerateContent 形状时，是**上游/转义器**的 bug，
/// 不是客户端的错 —— 错误类型要能分开，因为这两者对应的 HTTP 状态不同。
#[test]
fn a_broken_upstream_body_is_not_blamed_on_the_client() {
    assert!(matches!(
        OpenAiToGoogle
            .translate_response(b"<html>502</html>")
            .expect_err("must fail"),
        TranslateError::UpstreamShape(_)
    ));
    assert!(matches!(
        AnthropicToGoogle
            .translate_request("m", b"<html>")
            .expect_err("must fail"),
        TranslateError::Malformed(_)
    ));
}

// ========================================================== 响应 · 流式

/// OpenAI 方向的序列合法性：每帧都是 `data:`，`[DONE]` 只有一次且在最后，
/// `delta.role` 只出现在第一帧。
#[test]
fn openai_stream_sequence_is_legal() {
    let (frames, _) = run_stream(&OpenAiToGoogle);
    assert!(
        frames.iter().all(|(event, _)| event.is_none()),
        "the OpenAI dialect has no `event:` lines"
    );
    let done: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, (_, v))| v.as_str() == Some("[DONE]"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(done.len(), 1, "`[DONE]` must appear exactly once");
    assert_eq!(done[0], frames.len() - 1, "`[DONE]` must be the last frame");

    let with_role: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, (_, v))| !v["choices"][0]["delta"]["role"].is_null())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        with_role,
        vec![0],
        "`delta.role` belongs to the first chunk only"
    );
}

/// 流过去的文本必须**一字不差**地重组出来。
#[test]
fn openai_stream_reassembles_the_spoken_text() {
    let (frames, _) = run_stream(&OpenAiToGoogle);
    let text: String = frames
        .iter()
        .filter_map(|(_, v)| v["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(text, SPOKEN);
}

/// 流式方向**不合成**客户端没要过的 usage chunk（审计缺陷 #4 里让手写
/// `chunk.choices[0]` 的客户端抛 `IndexError` 的正是那一帧）。
/// 每一个非 `[DONE]` 帧都必须有 `choices[0]`。
#[test]
fn openai_stream_never_emits_a_choiceless_chunk() {
    let (frames, _) = run_stream(&OpenAiToGoogle);
    for (_, value) in &frames {
        if value.as_str() == Some("[DONE]") {
            continue;
        }
        assert!(
            value["choices"][0].is_object(),
            "a chunk without choices[0] breaks clients that index it blindly: {value}"
        );
    }
}

/// Anthropic 方向的序列合法性 —— 本任务里最硬的一条不变量。
///
/// 检查的全是**跨帧**的性质，一条都不比对期望字符串：
/// `message_start` 最先且只有一次、`message_stop` 最后且只有一次、
/// 每个 delta 落在一对 start/stop 之间、index 从 0 起严格递增。
#[test]
fn anthropic_stream_sequence_is_legal() {
    let (frames, _) = run_stream(&AnthropicToGoogle);
    let events: Vec<&str> = frames
        .iter()
        .map(|(e, _)| {
            e.as_deref()
                .expect("the Anthropic dialect requires an `event:` line")
        })
        .collect();

    assert_eq!(events.first(), Some(&"message_start"));
    assert_eq!(events.iter().filter(|e| **e == "message_start").count(), 1);
    assert_eq!(events.last(), Some(&"message_stop"));
    assert_eq!(events.iter().filter(|e| **e == "message_stop").count(), 1);
    assert_eq!(
        events.iter().filter(|e| **e == "message_delta").count(),
        1,
        "exactly one message_delta carries the stop reason"
    );

    // 每一帧的 `type` 必须和 `event:` 名一致，否则按 data 分派的客户端会错乱。
    for (event, value) in &frames {
        assert_eq!(value["type"].as_str(), event.as_deref());
    }

    let mut open: Option<i64> = None;
    let mut highest: Option<i64> = None;
    for (event, value) in &frames {
        let index = value["index"].as_i64();
        match event.as_deref() {
            Some("content_block_start") => {
                assert!(open.is_none(), "a block was started while another was open");
                let index = index.expect("content_block_start needs an index");
                assert_eq!(
                    index,
                    highest.map_or(0, |h| h + 1),
                    "block indices must start at 0 and increase by one"
                );
                highest = Some(index);
                open = Some(index);
            }
            Some("content_block_delta") => {
                assert_eq!(open, index, "a delta must fall inside its own open block");
            }
            Some("content_block_stop") => {
                assert_eq!(open.take(), index, "stop must close the block that is open");
            }
            _ => {}
        }
    }
    assert!(open.is_none(), "every content block must be closed");
}

/// 流过去的文本必须一字不差地重组出来（Anthropic 方向）。
#[test]
fn anthropic_stream_reassembles_the_spoken_text() {
    let (frames, _) = run_stream(&AnthropicToGoogle);
    let text: String = frames
        .iter()
        .filter_map(|(_, v)| v["delta"]["text"].as_str())
        .collect();
    assert_eq!(text, SPOKEN);
}

/// 一帧内容都没有的流仍然必须给出一个语法完整的信封 ——
/// 否则就是审计缺陷 #6 那种「干净的 EOF」：客户端拿到截断响应却不报错。
#[test]
fn an_empty_stream_still_produces_a_complete_envelope() {
    let mut st = AnthropicToGoogle.stream_translator();
    let frames = decode(&st.finish().expect("finish"));
    let events: Vec<&str> = frames
        .iter()
        .map(|(e, _)| e.as_deref().expect("event"))
        .collect();
    assert_eq!(events.first(), Some(&"message_start"));
    assert_eq!(events.last(), Some(&"message_stop"));

    let mut st = OpenAiToGoogle.stream_translator();
    let frames = decode(&st.finish().expect("finish"));
    assert_eq!(frames.last().expect("last").1.as_str(), Some("[DONE]"));
    assert!(
        frames
            .iter()
            .any(|(_, v)| v["choices"][0]["finish_reason"].is_string()),
        "an OpenAI client waits for a non-null finish_reason before it considers the turn over"
    );
}

/// 重复收尾必须幂等 —— 第二个 `message_stop` 会被客户端当成下一条消息的开头。
#[test]
fn finishing_twice_does_not_emit_a_second_message_stop() {
    let mut st = AnthropicToGoogle.stream_translator();
    assert!(!st.finish().expect("first finish").is_empty());
    assert!(st.finish().expect("second finish").is_empty());
}

// ============================================================ usage

/// usage 的计数分散在多帧里（首帧只有 prompt、末帧才补齐），
/// **一个都不能丢**。丢了就落 fallback 结算，直接违反「计费语义不变」。
#[test]
fn usage_counts_survive_being_spread_across_frames() {
    for t in [&OpenAiToGoogle as &dyn Translator, &AnthropicToGoogle] {
        let (_, usage) = run_stream(t);
        let usage = usage.expect("the upstream sent usageMetadata, so usage must be Some");
        assert!(
            usage.input_tokens.is_some(),
            "promptTokenCount arrived in the first frame only"
        );
        assert!(
            usage.output_tokens.is_some(),
            "candidatesTokenCount arrived in the last frame"
        );
        assert!(
            usage.cached_tokens.is_some(),
            "cachedContentTokenCount must not be dropped"
        );
        assert!(
            usage.reasoning_tokens.is_some(),
            "thoughtsTokenCount must not be dropped"
        );
    }
}

/// 上游给的是**原始值**，转义器不许在计费口径上做算术。
/// 断言方式：把同一份 `usageMetadata` 喂给两个方向，计费拿到的必须完全一样 ——
/// 客户端信封的方言差异不许渗进 [`RelayUsage`]。
#[test]
fn billing_usage_is_dialect_independent() {
    let (_, openai) = run_stream(&OpenAiToGoogle);
    let (_, anthropic) = run_stream(&AnthropicToGoogle);
    assert_eq!(openai, anthropic);
}

/// 「缺失」与「零」必须能分开：`0` 是上游说「产出了 0 个 token」，
/// 缺失是上游根本没说 —— 后者要走 fallback 结算，前者不能。
#[test]
fn a_missing_count_is_not_a_zero_count() {
    let zero = concat!(
        r#"data: {"usageMetadata":{"promptTokenCount":0,"candidatesTokenCount":0}}"#,
        "\n\n"
    );
    let absent = concat!(r#"data: {"usageMetadata":{"promptTokenCount":0}}"#, "\n\n");

    let mut st = OpenAiToGoogle.stream_translator();
    st.push(zero.as_bytes()).expect("push");
    let zero = st.usage().expect("usage");

    let mut st = OpenAiToGoogle.stream_translator();
    st.push(absent.as_bytes()).expect("push");
    let absent = st.usage().expect("usage");

    assert_eq!(zero.output_tokens, Some(0));
    assert_eq!(absent.output_tokens, None);
    assert_ne!(zero, absent);
}

/// 上游一个计数都没给时 `usage()` 必须是 `None`，而不是一个全零的
/// [`RelayUsage`] —— 全零会被结算当成真实计量，租户白嫖。
#[test]
fn no_usage_metadata_means_none_not_zero() {
    let mut st = AnthropicToGoogle.stream_translator();
    st.push(br#"data: {"candidates":[{"content":{"parts":[{"text":"x"}]}}]}"#)
        .expect("push");
    st.finish().expect("finish");
    assert_eq!(st.usage(), None);
}

/// 多行 `data:` 与一次喂进多个事件块都不能把两帧的载荷粘成一个 JSON。
#[test]
fn the_frame_reader_handles_multiline_and_multiframe_input() {
    let mut st = OpenAiToGoogle.stream_translator();
    let two = concat!(
        r#"data: {"candidates":[{"content":{"parts":[{"text":"a"}]}}]}"#,
        "\n\n",
        r#"data: {"candidates":[{"content":{"parts":[{"text":"b"}]}}]}"#,
        "\n\n",
    );
    let frames = decode(&st.push(two.as_bytes()).expect("two frames in one buffer"));
    let text: String = frames
        .iter()
        .filter_map(|(_, v)| v["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(text, "ab");
}

mod stream_framing;

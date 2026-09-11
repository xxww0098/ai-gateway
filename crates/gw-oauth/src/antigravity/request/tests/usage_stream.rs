use serde_json::json;

use super::{
    antigravity_to_openai, cached_tokens_of, events_to_openai_chunks, google_event,
    incremental_suffix,
};

#[test]
fn cached_content_token_count_maps_to_cached_tokens() {
    let out = antigravity_to_openai(
        &json!({
            "response": {
                "candidates": [{"content": {"parts": [{"text": "hi"}]}, "finishReason": "STOP"}],
                "usageMetadata": {
                    "promptTokenCount": 25598,
                    "candidatesTokenCount": 4768,
                    "totalTokenCount": 30366,
                    "cachedContentTokenCount": 18360
                }
            }
        }),
        Some("gemini-3.7-flash-high"),
        None,
        None,
    );
    assert_eq!(
        out["usage"]["prompt_tokens_details"]["cached_tokens"],
        18360
    );
    assert_eq!(out["usage"]["completion_tokens"], 4768);

    let snake = antigravity_to_openai(
        &json!({
            "response": {
                "candidates": [{"content": {"parts": [{"text": "hi"}]}}],
                "usageMetadata": {
                    "promptTokenCount": 100,
                    "cached_content_token_count": 80,
                    "candidatesTokenCount": 1
                }
            }
        }),
        Some("gemini-3.7-flash-high"),
        None,
        None,
    );
    assert_eq!(snake["usage"]["prompt_tokens_details"]["cached_tokens"], 80);

    let details = antigravity_to_openai(
        &json!({
            "response": {
                "candidates": [{"content": {"parts": [{"text": "hi"}]}}],
                "usageMetadata": {
                    "promptTokenCount": 50,
                    "cacheTokensDetails": [{"tokenCount": 20}, {"token_count": 10}],
                    "candidatesTokenCount": 1
                }
            }
        }),
        Some("gemini-3.7-flash-high"),
        None,
        None,
    );
    assert_eq!(
        details["usage"]["prompt_tokens_details"]["cached_tokens"],
        30
    );

    let zero = antigravity_to_openai(
        &json!({
            "response": {
                "candidates": [{"content": {"parts": [{"text": "hi"}]}}],
                "usageMetadata": {
                    "promptTokenCount": 10,
                    "cachedContentTokenCount": 0,
                    "candidatesTokenCount": 1
                }
            }
        }),
        Some("gemini-3.7-flash-high"),
        None,
        None,
    );
    assert_eq!(zero["usage"]["prompt_tokens_details"]["cached_tokens"], 0);

    assert_eq!(
        cached_tokens_of(&json!({"cache_read_tokens": 12})),
        Some(12)
    );
    assert_eq!(cached_tokens_of(&json!({"cacheReadTokens": 34})), Some(34));
    assert_eq!(
        cached_tokens_of(&json!({"cachedContentTokenCount": 9, "cacheReadTokens": 99})),
        Some(9)
    );
}

#[test]
fn thoughts_count_as_completion_tokens_and_stay_out_of_visible_text() {
    let usage = json!({
        "promptTokenCount": 120,
        "candidatesTokenCount": 18,
        "thoughtsTokenCount": 42,
        "totalTokenCount": 180
    });
    let out = antigravity_to_openai(
        &json!({
            "response": {
                "candidates": [{"content": {"parts": [{"text": "hi"}]}, "finishReason": "STOP"}],
                "usageMetadata": usage
            }
        }),
        Some("gemini-3.7-flash-high"),
        Some("chatcmpl-usage"),
        None,
    );
    assert_eq!(out["usage"]["completion_tokens"], 60);
    assert_eq!(
        out["usage"]["completion_tokens_details"]["reasoning_tokens"],
        42
    );

    let chunks = events_to_openai_chunks(
        &[
            google_event(
                json!([{"text": "planning the answer", "thought": true}]),
                None,
                None,
            ),
            google_event(
                json!([
                    {"text": "planning the answer", "thought": true},
                    {"text": "可见正文从这里开始"}
                ]),
                Some(usage),
                Some("STOP"),
            ),
        ],
        Some("gemini-3.7-flash-high"),
        Some("chatcmpl-thought"),
        None,
    );
    let contents: Vec<&str> = chunks
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(contents, vec!["可见正文从这里开始"]);
    assert!(!contents.iter().any(|t| t.contains("planning")));
    let terminal = chunks.last().expect("terminal");
    assert_eq!(terminal["choices"][0]["finish_reason"], "stop");
    assert_eq!(terminal["usage"]["completion_tokens"], 60);
}

#[test]
fn cumulative_google_sse_becomes_incremental_openai_deltas() {
    let chunks = events_to_openai_chunks(
        &[
            google_event(json!([{"text": "Hello"}]), None, None),
            google_event(json!([{"text": "Hello world"}]), None, None),
        ],
        Some("gemini-3.7-flash-high"),
        Some("chatcmpl-delta"),
        None,
    );
    assert_eq!(chunks[0]["choices"][0]["delta"]["content"], "Hello");
    assert_eq!(chunks[1]["choices"][0]["delta"]["content"], " world");
    assert!(
        !chunks
            .iter()
            .any(|c| c["choices"][0]["delta"]["content"] == "Hello world")
    );
    let terminal = chunks.last().expect("terminal");
    assert_eq!(terminal["choices"][0]["delta"], json!({}));
    assert!(terminal.get("usage").is_none() || terminal["usage"].is_null());

    assert_eq!(incremental_suffix("Hello world", "Hello"), " world");
    assert_eq!(incremental_suffix("Hi", "Hello world"), "Hi");
    assert_eq!(incremental_suffix("", "Hello"), "");
}

#[test]
fn tool_call_stream_finish_reason_is_tool_calls() {
    let chunks = events_to_openai_chunks(
        &[
            google_event(
                json!([{"functionCall": {"name": "Read", "args": {"path": "a.ts"}}}]),
                None,
                None,
            ),
            google_event(
                json!([{"functionCall": {"name": "Read", "args": {"path": "a.ts"}}}]),
                None,
                Some("STOP"),
            ),
        ],
        Some("gemini-3.7-flash-high"),
        Some("chatcmpl-tools"),
        None,
    );
    let tool = chunks
        .iter()
        .find(|c| c["choices"][0]["delta"].get("tool_calls").is_some())
        .expect("tool chunk");
    assert_eq!(
        tool["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
        "Read"
    );
    assert_eq!(
        chunks
            .iter()
            .filter(|c| c["choices"][0]["delta"].get("tool_calls").is_some())
            .count(),
        1
    );
    assert_eq!(
        chunks.last().expect("t")["choices"][0]["finish_reason"],
        "tool_calls"
    );
}

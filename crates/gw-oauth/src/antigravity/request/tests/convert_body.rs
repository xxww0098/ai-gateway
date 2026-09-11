use serde_json::json;

use crate::antigravity::BODY_USER_AGENT;
use crate::antigravity::cache::ANTIGRAVITY_STABLE_SESSION;

use super::{
    contents_of, convert, isolated, json_blob, max_output_tokens, openai_to_antigravity, request_of,
};

#[test]
fn empty_project_is_rejected_and_body_is_hub_shaped() {
    let _lock = isolated();
    let err = openai_to_antigravity(
        &json!({"model": "claude-sonnet-4-6", "messages": []}),
        "  ",
        None,
    )
    .expect_err("empty project");
    assert!(err.to_string().contains("project_id"), "{err}");

    let body = convert(
        json!({
            "model": "claude-sonnet-4-6",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "hi"}
            ]
        }),
        "proj-1",
    );
    assert_eq!(body["userAgent"], BODY_USER_AGENT);
    assert_eq!(body["requestType"], "agent");
    assert_eq!(request_of(&body)["systemInstruction"]["role"], "user");
    assert_eq!(
        request_of(&body)["systemInstruction"]["parts"][0]["text"],
        "You are helpful."
    );
    assert_eq!(contents_of(&body)[0]["role"], "user");
    assert_eq!(
        request_of(&body)["toolConfig"]["functionCallingConfig"]["mode"],
        "VALIDATED"
    );
    assert!(request_of(&body).get("implicitCacheConfig").is_none());
    assert!(request_of(&body).get("cachedContent").is_none());
    let session = request_of(&body)["sessionId"].as_str().expect("sessionId");
    assert!(
        !session.starts_with('-') || !session[1..].chars().all(|c| c.is_ascii_digit()),
        "must not stamp Date.now(): {session}"
    );
    assert!(
        body["requestId"]
            .as_str()
            .is_some_and(|id| id.starts_with("agent-"))
    );
}

#[test]
fn session_id_prefers_caller_pin_and_never_date_now() {
    let _lock = isolated();
    let pinned = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-dsh-1",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(request_of(&pinned)["sessionId"], "session-dsh-1");

    let from_cache = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "prompt_cache_key": "cache-key-9",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(request_of(&from_cache)["sessionId"], "cache-key-9");

    let explicit = openai_to_antigravity(
        &json!({
            "model": "gemini-3.7-flash-high",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
        Some("session 772f/foo"),
    )
    .expect("explicit");
    assert_eq!(request_of(&explicit)["sessionId"], "session-772f-foo");
}

#[test]
fn fallback_session_id_is_per_model() {
    let _lock = isolated();
    let flash = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    let sonnet = convert(
        json!({
            "model": "claude-sonnet-4-6",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    let flash_id = request_of(&flash)["sessionId"].as_str().expect("id");
    let sonnet_id = request_of(&sonnet)["sessionId"].as_str().expect("id");
    assert_ne!(flash_id, sonnet_id);
    assert!(flash_id.starts_with(ANTIGRAVITY_STABLE_SESSION));
    assert!(flash_id.contains("gemini-3.7-flash-high"));
    assert!(sonnet_id.contains("claude-sonnet-4-6"));
}

#[test]
fn extra_system_snapshot_is_a_trailing_user_turn() {
    let _lock = isolated();
    let first = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-cache-1",
            "messages": [
                {"role": "system", "content": "You are an AI agent."},
                {"role": "user", "content": "analyze the repo"}
            ]
        }),
        "p",
    );
    assert_eq!(
        request_of(&first)["systemInstruction"]["parts"][0]["text"],
        "You are an AI agent."
    );

    let later = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-cache-1",
            "messages": [
                {"role": "system", "content": "You are an AI agent."},
                {"role": "system", "content": "Current runtime context. This snapshot supersedes earlier runtime-context snapshots."},
                {"role": "user", "content": "analyze the repo"},
                {"role": "assistant", "content": "ok"}
            ]
        }),
        "p",
    );
    assert_eq!(
        request_of(&later)["systemInstruction"]["parts"][0]["text"],
        "You are an AI agent."
    );
    let last = contents_of(&later).last().expect("turn");
    assert_eq!(last["role"], "user");
    assert!(
        last["parts"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Current runtime context")),
        "{last}"
    );
}

#[test]
fn first_turn_must_be_user_and_system_is_not_replaced() {
    let _lock = isolated();
    let body = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "messages": [{"role": "assistant", "content": "I am ready."}]
        }),
        "p",
    );
    assert_eq!(contents_of(&body)[0]["role"], "user");
    assert_eq!(contents_of(&body)[0]["parts"][0]["text"], "Hello");
    assert_eq!(contents_of(&body)[1]["role"], "model");

    let with_system = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "messages": [
                {"role": "system", "content": "You are an AI agent."},
                {"role": "user", "content": "hi"}
            ]
        }),
        "p",
    );
    let blob = json_blob(&request_of(&with_system)["systemInstruction"]);
    assert!(!blob.contains("pair programming"));
    assert_eq!(
        request_of(&with_system)["systemInstruction"]["role"],
        "user"
    );
}

#[test]
fn max_output_tokens_is_clamped_per_runtime_family() {
    let _lock = isolated();
    let over = 999_999i64;
    let flash = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "max_tokens": over,
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    let pro = convert(
        json!({
            "model": "gemini-pro-agent",
            "max_tokens": over,
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    let claude = convert(
        json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": over,
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    let flash_n = request_of(&flash)["generationConfig"]["maxOutputTokens"]
        .as_i64()
        .expect("n");
    let pro_n = request_of(&pro)["generationConfig"]["maxOutputTokens"]
        .as_i64()
        .expect("n");
    let claude_n = request_of(&claude)["generationConfig"]["maxOutputTokens"]
        .as_i64()
        .expect("n");
    assert!(flash_n < over && pro_n < over && claude_n < over);
    assert_ne!(flash_n, pro_n);
    assert_eq!(
        convert(
            json!({
                "model": "gemini-3.7-flash-high",
                "max_tokens": 16,
                "messages": [{"role": "user", "content": "hi"}]
            }),
            "p",
        )["request"]["generationConfig"]["maxOutputTokens"],
        16
    );
    assert!(max_output_tokens("gemini-3.7-flash-high") >= max_output_tokens("gpt-oss-120b-medium"));
}

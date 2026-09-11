use serde_json::json;

use super::{normalize_completions, normalize_request};

#[test]
fn anthropic_default_strips_prompt_cache_key_and_pins_user_id() {
    let out = normalize_request(json!({
        "model": "glm-5.3",
        "session_id": "sess-glm",
        "prompt_cache_key": "should-not-leave",
        "prompt_cache_retention": "24h",
        "system": "You are an AI agent.",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    assert!(out.get("prompt_cache_key").is_none());
    assert!(out.get("prompt_cache_retention").is_none());
    assert!(out.get("session_id").is_none());
    assert_eq!(out["metadata"]["user_id"], "sess-glm");
    assert!(out["max_tokens"].as_i64().is_some_and(|n| n > 0));
}

#[test]
fn existing_anthropic_user_id_wins_over_session_id() {
    let out = normalize_request(json!({
        "metadata": {"user_id": "keep-me"},
        "session_id": "other",
        "prompt_cache_key": "drop",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "hi"}]
    }));
    assert_eq!(out["metadata"]["user_id"], "keep-me");
    assert!(out.get("prompt_cache_key").is_none());
    assert_eq!(out["max_tokens"], 1024);
}

#[test]
fn five_three_and_flash_force_thinking_turbo_does_not() {
    let five = normalize_request(json!({
        "model": "GLM-5.3",
        "max_tokens": 8,
        "messages": [{"role": "user", "content": "hi"}]
    }));
    assert_eq!(five["thinking"]["type"], "enabled");
    assert_eq!(five["thinking"]["clear_thinking"], false);

    let flash = normalize_request(json!({
        "model": "glm-5.3-flash",
        "max_tokens": 8,
        "messages": [{"role": "user", "content": "hi"}]
    }));
    assert_eq!(flash["thinking"]["type"], "enabled");
    assert_eq!(flash["thinking"]["clear_thinking"], false);

    let idle = normalize_request(json!({
        "model": "glm-5-turbo",
        "max_tokens": 8,
        "messages": [{"role": "user", "content": "hi"}]
    }));
    assert!(idle.get("thinking").is_none());

    let off = normalize_request(json!({
        "model": "glm-5-turbo",
        "max_tokens": 8,
        "thinking": {"type": "disabled"},
        "messages": [{"role": "user", "content": "hi"}]
    }));
    assert_eq!(off["thinking"]["type"], "disabled");
    assert!(off["thinking"].get("clear_thinking").is_none());
}

#[test]
fn missing_or_non_positive_max_tokens_becomes_a_positive_default() {
    let missing = normalize_request(json!({"model": "glm-5.3"}));
    let zero = normalize_request(json!({"model": "glm-5.3", "max_tokens": 0}));
    let kept = normalize_request(json!({"model": "glm-5.3", "max_tokens": 2048}));
    let default = missing["max_tokens"].as_i64().expect("default");
    assert!(default > 0);
    assert_eq!(zero["max_tokens"].as_i64(), Some(default));
    assert_eq!(kept["max_tokens"], 2048);
}

#[test]
fn completions_leftover_maps_unknown_roles_to_system() {
    let out = normalize_completions(json!({
        "model": "glm-5.3-flash",
        "prompt_cache_key": "drop-me",
        "messages": [
            {"role": "developer", "content": "You are DSH."},
            {"role": "user", "content": "hello"},
            {"role": "tool", "tool_call_id": "c1", "content": "ok"}
        ]
    }));
    assert_eq!(out["messages"][0]["role"], "system");
    assert_eq!(out["messages"][1]["role"], "user");
    assert_eq!(out["messages"][2]["role"], "tool");
    assert!(out.get("prompt_cache_key").is_none());
    assert_eq!(out["thinking"]["clear_thinking"], false);
}

#[test]
fn completions_copies_assistant_reasoning_only_when_content_missing() {
    let out = normalize_completions(json!({
        "model": "glm-5.3",
        "messages": [
            {"role": "assistant", "content": "done", "reasoning": "counted"},
            {"role": "assistant", "content": "kept", "reasoning_content": "official", "reasoning": "alias"}
        ]
    }));
    assert_eq!(out["messages"][0]["reasoning_content"], "counted");
    assert_eq!(out["messages"][1]["reasoning_content"], "official");
}

#[test]
fn completions_does_not_rewrite_input_only_developer_roles() {
    let out = normalize_completions(json!({
        "model": "glm-5.3",
        "input": [{"role": "developer", "content": "sys"}]
    }));
    assert_eq!(out["input"][0]["role"], "developer");
}

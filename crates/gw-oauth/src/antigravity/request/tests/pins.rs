use serde_json::json;

use super::{convert, grep_tool, isolated, read_tool, request_of};

#[test]
fn tools_pin_reuses_first_json_when_names_and_schemas_match() {
    let _lock = isolated();
    let first = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-tools-1",
            "tools": [read_tool(), grep_tool()],
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    let shuffled = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-tools-1",
            "tools": [
                {"type": "function", "function": {"name": "Grep", "parameters": {"properties": {"q": {"type": "string"}}, "type": "object"}}},
                {"type": "function", "function": {"name": "Read", "parameters": {"required": ["path"], "properties": {"path": {"type": "string"}}, "type": "object"}}}
            ],
            "messages": [{"role": "user", "content": "again"}]
        }),
        "p",
    );
    assert_eq!(request_of(&shuffled)["tools"], request_of(&first)["tools"]);
    assert_eq!(
        request_of(&shuffled)["tools"][0]["functionDeclarations"][0]["name"],
        "Read"
    );

    let removed = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-tools-1",
            "tools": [read_tool()],
            "messages": [{"role": "user", "content": "only read"}]
        }),
        "p",
    );
    assert_eq!(
        request_of(&removed)["tools"][0]["functionDeclarations"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_ne!(request_of(&removed)["tools"], request_of(&first)["tools"]);
}

#[test]
fn thinking_config_is_sticky_first_and_never_adds_implicit_cache() {
    let _lock = isolated();
    let sent = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-think-on",
            "reasoning_effort": "high",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    let omitted = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-think-on",
            "messages": [{"role": "user", "content": "again"}]
        }),
        "p",
    );
    assert_eq!(
        request_of(&sent)["generationConfig"]["thinkingConfig"]["thinkingLevel"],
        "high"
    );
    assert_eq!(
        request_of(&omitted)["generationConfig"]["thinkingConfig"],
        request_of(&sent)["generationConfig"]["thinkingConfig"]
    );
    assert!(request_of(&omitted).get("implicitCacheConfig").is_none());

    let first_omit = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-think-off",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    let later_effort = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-think-off",
            "reasoning_effort": "low",
            "messages": [{"role": "user", "content": "again"}]
        }),
        "p",
    );
    assert!(
        request_of(&first_omit)["generationConfig"]
            .get("thinkingConfig")
            .is_none()
    );
    assert!(
        request_of(&later_effort)["generationConfig"]
            .get("thinkingConfig")
            .is_none()
    );
}

#[test]
fn claude_and_gpt_oss_omit_thinking_and_picker_id_is_not_rewritten() {
    let _lock = isolated();
    let claude = convert(
        json!({
            "model": "claude-sonnet-4-6",
            "reasoning_effort": "high",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(claude["model"], "claude-sonnet-4-6");
    assert!(
        request_of(&claude)["generationConfig"]
            .get("thinkingConfig")
            .is_none()
    );

    let oss = convert(
        json!({
            "model": "gpt-oss-120b-medium",
            "reasoning_effort": "medium",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(oss["model"], "gpt-oss-120b-medium");
    assert!(
        request_of(&oss)["generationConfig"]
            .get("thinkingConfig")
            .is_none()
    );

    let flash = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "reasoning_effort": "low",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(flash["model"], "gemini-3.7-flash-high");
    assert_eq!(
        request_of(&flash)["generationConfig"]["thinkingConfig"]["thinkingLevel"],
        "low"
    );

    let agent = convert(
        json!({
            "model": "gemini-pro-agent",
            "reasoning_effort": "high",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(agent["model"], "gemini-pro-agent");
    assert!(
        request_of(&agent)["generationConfig"]["thinkingConfig"]
            .get("thinkingLevel")
            .is_none()
    );
    assert!(
        request_of(&agent)["generationConfig"]["thinkingConfig"]["thinkingBudget"]
            .as_i64()
            .is_some_and(|n| n > 0)
    );
}

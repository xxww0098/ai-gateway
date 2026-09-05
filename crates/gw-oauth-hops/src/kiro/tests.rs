use super::{KIRO_STABLE_SESSION, conversation_id, plan};
use crate::rewrite::HopInput;
use serde_json::json;

/// Switching the model must not reuse another model's AWS conversation.
/// The same (session, model) pair is stable across calls — never `Date.now()`.
#[test]
fn conversation_id_includes_model_and_is_stable() {
    let body = json!({ "session_id": "sess-1", "model": "claude-sonnet-4" });
    let a = conversation_id(&body, None, None);
    let b = conversation_id(&body, None, None);
    assert_eq!(a, b);
    let other = conversation_id(&body, None, Some("gpt-5.4"));
    assert_ne!(a, other);
    assert!(a.contains("sess-1") || a.ends_with("claude-sonnet-4") || a.contains(':'));
}

/// No DSH id → the stable constant, not a generated timestamp.
#[test]
fn missing_id_falls_back_to_a_stable_constant() {
    let id = conversation_id(&json!({}), None, None);
    assert!(id.starts_with(KIRO_STABLE_SESSION) || id == KIRO_STABLE_SESSION);
    let again = conversation_id(&json!({}), None, None);
    assert_eq!(id, again);
}

/// Native headers are CodeWhisperer, not Codex / Grok, and carry no bearer.
#[test]
fn identity_headers_are_kiro_wire_not_codex() {
    let hop = plan(&HopInput::default(), None);
    assert_eq!(
        hop.headers
            .get("x-amz-target")
            .and_then(|v| v.to_str().ok()),
        Some(super::KIRO_AMZ_TARGET)
    );
    assert!(!hop.headers.contains_key("session-id"));
    assert!(!hop.headers.contains_key("x-grok-conv-id"));
    assert!(!hop.headers.contains_key(http::header::AUTHORIZATION));
}

/// Two plans of the same input agree on the conversation id.
#[test]
fn plan_id_is_deterministic() {
    let body = serde_json::to_vec(&json!({ "session_id": "s", "model": "m" })).unwrap();
    let a = plan(
        &HopInput {
            body: &body,
            ..HopInput::default()
        },
        None,
    );
    let b = plan(
        &HopInput {
            body: &body,
            ..HopInput::default()
        },
        None,
    );
    assert_eq!(a.cache_session_id, b.cache_session_id);
}

/// Chat Completions become conversationState. System parks as user+ack.
#[test]
fn openai_chat_becomes_conversation_state() {
    let body = serde_json::to_vec(&json!({
        "model": "claude-sonnet-4",
        "session_id": "k-1",
        "messages": [
            { "role": "system", "content": "be brief" },
            { "role": "user", "content": "hi" }
        ]
    }))
    .unwrap();
    let hop = plan(
        &HopInput {
            body: &body,
            ..HopInput::default()
        },
        None,
    );
    let value: serde_json::Value = serde_json::from_slice(hop.body.as_deref().unwrap()).unwrap();
    let state = &value["conversationState"];
    assert!(state["conversationId"].as_str().unwrap().contains("k-1"));
    assert_eq!(
        state["currentMessage"]["userInputMessage"]["content"],
        json!("hi")
    );
    let history = state["history"].as_array().unwrap();
    assert_eq!(history[0]["userInputMessage"]["content"], json!("be brief"));
    assert_eq!(
        history[1]["assistantResponseMessage"]["content"],
        json!(super::KIRO_SYSTEM_ACK)
    );
}

/// Already-native bodies are not wrapped again.
#[test]
fn native_conversation_state_is_left_alone() {
    let body = serde_json::to_vec(&json!({
        "conversationState": { "conversationId": "keep" }
    }))
    .unwrap();
    let hop = plan(
        &HopInput {
            body: &body,
            ..HopInput::default()
        },
        None,
    );
    assert!(hop.body.is_none());
}

/// Piped OpenAI tool ids remap to a stable tooluse_ id both ways.
#[test]
fn compound_tool_ids_remap_stably() {
    let long = "call_abc|fc_def_this_is_way_too_long_for_kiro_wire_ids";
    let a = super::translate::normalize_tool_use_id(Some(long)).unwrap();
    let b = super::translate::normalize_tool_use_id(Some(long)).unwrap();
    assert_eq!(a, b);
    assert!(a.starts_with("tooluse_"));
    assert!(a.len() <= 64);
}

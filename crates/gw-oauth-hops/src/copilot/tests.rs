use super::{COPILOT_STABLE_SESSION, plan};
use crate::rewrite::HopInput;
use serde_json::json;

fn header_str<'a>(headers: &'a http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Official Copilot always sends an interaction id. Missing DSH session
/// still stamps the stable constant, never a timestamp.
#[test]
fn interaction_id_is_stable_and_not_codex() {
    let a = plan(&HopInput::default(), None);
    let b = plan(&HopInput::default(), None);
    assert_eq!(
        header_str(&a.headers, "x-interaction-id"),
        Some(COPILOT_STABLE_SESSION)
    );
    assert_eq!(a.cache_session_id, b.cache_session_id);
    assert!(!a.headers.contains_key("session-id"));
    assert!(!a.headers.contains_key("x-grok-conv-id"));
    assert!(!a.headers.contains_key(http::header::AUTHORIZATION));
}

/// GPT hops drop max-token caps. A tool-turn sets initiator=agent.
#[test]
fn gpt_drops_max_tokens_and_tool_turn_is_agent() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-4.1",
        "session_id": "cp-9",
        "max_tokens": 99,
        "messages": [
            { "role": "user", "content": "hi" },
            { "role": "tool", "content": "ok" }
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
    assert!(value.get("max_tokens").is_none());
    assert!(value.get("session_id").is_none());
    assert_eq!(header_str(&hop.headers, "x-initiator"), Some("agent"));
    assert_eq!(header_str(&hop.headers, "x-interaction-id"), Some("cp-9"));
}

/// An image part opts into the vision header.
#[test]
fn vision_part_sets_the_vision_header() {
    let body = serde_json::to_vec(&json!({
        "messages": [{
            "role": "user",
            "content": [{ "type": "image_url", "image_url": { "url": "data:," } }]
        }]
    }))
    .unwrap();
    let hop = plan(
        &HopInput {
            body: &body,
            ..HopInput::default()
        },
        None,
    );
    assert_eq!(
        header_str(&hop.headers, "copilot-vision-request"),
        Some("true")
    );
}

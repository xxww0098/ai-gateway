use super::{GLM_STABLE_SESSION, plan, plan_anthropic};
use crate::pin::PrefixPins;
use crate::rewrite::HopInput;
use serde_json::json;

fn header_str<'a>(headers: &'a http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Sticky routing is `x-session-id`, not Codex `session-id`.
#[test]
fn session_header_is_not_codex() {
    let hop = plan(&HopInput::default(), None);
    assert_eq!(
        header_str(&hop.headers, "x-session-id"),
        Some(GLM_STABLE_SESSION)
    );
    assert!(!hop.headers.contains_key("session-id"));
    assert!(!hop.headers.contains_key("x-grok-conv-id"));
    assert!(!hop.headers.contains_key(http::header::AUTHORIZATION));
    assert!(hop.headers.contains_key("x-zcode-app-version"));
}

/// Completions pin `user` and force thinking on GLM-5.3.
#[test]
fn completions_sets_user_and_forced_thinking() {
    let body = serde_json::to_vec(&json!({
        "model": "glm-5.3",
        "session_id": "g-1",
        "prompt_cache_key": "nope",
        "messages": [{ "role": "user", "content": "hi" }],
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
    assert_eq!(value["user"], json!("g-1"));
    assert!(value.get("prompt_cache_key").is_none());
    assert_eq!(value["thinking"]["clear_thinking"], json!(false));
    assert_eq!(value["thinking"]["type"], json!("enabled"));
}

/// Anthropic hop stamps cache_control on the first system block and user_id.
#[test]
fn anthropic_stamps_cache_control_and_user_id() {
    let body = serde_json::to_vec(&json!({
        "system": "be brief",
        "messages": [{ "role": "user", "content": "hi" }],
    }))
    .unwrap();
    let hop = plan_anthropic(
        &HopInput {
            body: &body,
            ..HopInput::default()
        },
        None,
    );
    let value: serde_json::Value = serde_json::from_slice(hop.body.as_deref().unwrap()).unwrap();
    assert_eq!(
        value["system"][0]["cache_control"]["type"],
        json!("ephemeral")
    );
    assert_eq!(value["metadata"]["user_id"], json!(GLM_STABLE_SESSION));
    assert!(hop.headers.contains_key("anthropic-version"));
    assert!(value.get("max_tokens").is_some());
}

/// Extra Completions system text parks at the suffix when pins are provided.
#[test]
fn extra_system_parks_at_the_suffix() {
    let mut pins = PrefixPins::new();
    let first = serde_json::to_vec(&json!({
        "user": "conv-g",
        "messages": [
            { "role": "system", "content": "be brief" },
            { "role": "user", "content": "hi" }
        ]
    }))
    .unwrap();
    let _ = plan(
        &HopInput {
            body: &first,
            ..HopInput::default()
        },
        Some(&mut pins),
    );
    let second = serde_json::to_vec(&json!({
        "user": "conv-g",
        "messages": [
            { "role": "system", "content": "be brief\n\nthis snapshot" },
            { "role": "user", "content": "hi" }
        ]
    }))
    .unwrap();
    let hop = plan(
        &HopInput {
            body: &second,
            ..HopInput::default()
        },
        Some(&mut pins),
    );
    let value: serde_json::Value = serde_json::from_slice(hop.body.as_deref().unwrap()).unwrap();
    let messages = value["messages"].as_array().unwrap();
    assert_eq!(messages[0]["content"], json!("be brief"));
    assert_eq!(messages.last().unwrap()["role"], json!("system"));
}

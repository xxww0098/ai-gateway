use super::{KIMI_STABLE_SESSION, plan};
use crate::pin::PrefixPins;
use crate::rewrite::HopInput;
use serde_json::json;

fn header_str<'a>(headers: &'a http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Kimi does not sticky-route on Codex / Grok headers, and never carries a bearer.
#[test]
fn identity_excludes_codex_grok_and_authorization() {
    let hop = plan(&HopInput::default(), None);
    assert!(hop.headers.contains_key("user-agent"));
    assert!(hop.headers.contains_key("x-msh-platform"));
    assert!(!hop.headers.contains_key("session-id"));
    assert!(!hop.headers.contains_key("x-grok-conv-id"));
    assert!(!hop.headers.contains_key(http::header::AUTHORIZATION));
    assert_eq!(hop.cache_session_id.as_deref(), Some(KIMI_STABLE_SESSION));
}

/// Codex cache fields leave the Completions body.
#[test]
fn cache_fields_leave_the_body() {
    let body = serde_json::to_vec(&json!({
        "session_id": "k-1",
        "prompt_cache_key": "ignored",
        "prompt_cache_retention": "24h",
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
    assert!(value.get("session_id").is_none());
    assert!(value.get("prompt_cache_key").is_none());
    assert!(value.get("prompt_cache_retention").is_none());
    assert_eq!(hop.cache_session_id.as_deref(), Some("k-1"));
}

/// A later snapshot that grows the leading system parks at the suffix.
#[test]
fn extra_system_parks_at_the_suffix() {
    let mut pins = PrefixPins::new();
    let first = serde_json::to_vec(&json!({
        "session_id": "conv-k",
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
        "session_id": "conv-k",
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
    assert_ne!(messages.last().unwrap()["content"], json!("be brief"));
    assert_eq!(
        header_str(&hop.headers, "x-msh-platform"),
        Some(super::KIMI_PLATFORM)
    );
}

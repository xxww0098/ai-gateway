use super::{CURSOR_CLIENT_TYPE, CURSOR_STABLE_SESSION, conversation_id, plan};
use crate::rewrite::HopInput;
use serde_json::json;

fn header_str<'a>(headers: &'a http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Client type is CLI OAuth. `sdk` is the API-key Agent.create path.
#[test]
fn client_type_is_cli_not_sdk() {
    let hop = plan(&HopInput::default(), None);
    assert_eq!(
        header_str(&hop.headers, "x-cursor-client-type"),
        Some(CURSOR_CLIENT_TYPE)
    );
    assert_ne!(
        header_str(&hop.headers, "x-cursor-client-type"),
        Some("sdk")
    );
    assert!(!hop.headers.contains_key(http::header::AUTHORIZATION));
    assert!(!hop.headers.contains_key("session-id"));
}

/// Same (session, model) pair is stable. Fast suffix does not fork the pin.
#[test]
fn conversation_id_is_stable_across_fast_suffix() {
    let body = json!({ "session_id": "c-1", "model": "composer-2.5" });
    let a = conversation_id(&body, None, None);
    let b = conversation_id(&body, None, Some("composer-2.5-fast"));
    assert_eq!(a, b);
    assert_ne!(a, CURSOR_STABLE_SESSION);
    let other = conversation_id(&body, None, Some("grok-4.6"));
    assert_ne!(a, other);
}

/// Caller-owned request id is replayed on both Cursor sticky headers.
#[test]
fn request_id_is_caller_owned() {
    let hop = plan(
        &HopInput {
            request_id: Some("run-7"),
            ..HopInput::default()
        },
        None,
    );
    assert_eq!(header_str(&hop.headers, "x-request-id"), Some("run-7"));
    assert_eq!(
        header_str(&hop.headers, "x-original-request-id"),
        Some("run-7")
    );
}

/// Codex cache fields and `service_tier` leave the JSON.
#[test]
fn cache_fields_leave_the_body() {
    let body = serde_json::to_vec(&json!({
        "prompt_cache_key": "nope",
        "service_tier": "fast",
        "model": "composer-2.5",
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
    assert!(value.get("prompt_cache_key").is_none());
    assert!(value.get("service_tier").is_none());
}

/// Chat Completions with messages become a Connect frame, not JSON.
#[test]
fn chat_messages_become_connect_protobuf() {
    let body = serde_json::to_vec(&json!({
        "model": "composer-2.5",
        "session_id": "c-9",
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
    let bytes = hop.body.expect("protobuf body");
    assert_eq!(bytes[0], 0, "connect flag none");
    let len = u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 5 + len);
    assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_err());
}

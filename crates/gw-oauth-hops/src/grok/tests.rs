use super::{GROK_STABLE_SESSION, plan};
use crate::rewrite::HopInput;
use serde_json::json;

fn header_str<'a>(headers: &'a http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// conv-id and session-id are the same shard key. Codex `session-id` is a
/// different backend and must not appear.
#[test]
fn affinity_headers_match_and_exclude_codex() {
    let body = serde_json::to_vec(&json!({
        "model": "grok-4.6",
        "prompt_cache_key": "conv-9",
        "session_id": "dsh-other",
    }))
    .unwrap();
    let hop = plan(&HopInput {
        body: &body,
        request_id: Some("req-fixed"),
        ..HopInput::default()
    });
    let conv = header_str(&hop.headers, "x-grok-conv-id").unwrap();
    assert_eq!(header_str(&hop.headers, "x-grok-session-id"), Some(conv));
    assert_eq!(header_str(&hop.headers, "x-grok-req-id"), Some("req-fixed"));
    assert!(!hop.headers.contains_key("session-id"));
    assert!(!hop.headers.contains_key("thread-id"));
    assert!(!hop.headers.contains_key(http::header::AUTHORIZATION));
}

/// No DSH id → the stable constant, never a timestamp. Two plans with the
/// same empty body share that constant (so a cache can exist at all).
#[test]
fn missing_id_falls_back_to_a_stable_constant() {
    let a = plan(&HopInput::default());
    let b = plan(&HopInput::default());
    assert_eq!(a.cache_session_id.as_deref(), Some(GROK_STABLE_SESSION));
    assert_eq!(a.cache_session_id, b.cache_session_id);
    assert_eq!(
        header_str(&a.headers, "x-grok-conv-id"),
        Some(GROK_STABLE_SESSION)
    );
}

/// `session_id` / retention / options leave the body. `prompt_cache_key`
/// becomes the conversation id.
#[test]
fn body_drops_fields_grok_does_not_speak() {
    let body = serde_json::to_vec(&json!({
        "session_id": "dsh-sess",
        "prompt_cache_retention": "24h",
        "prompt_cache_options": { "max": 1 },
    }))
    .unwrap();
    let hop = plan(&HopInput {
        body: &body,
        ..HopInput::default()
    });
    let rewritten = hop.body.expect("stripping fields is a rewrite");
    let value: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
    assert!(value.get("session_id").is_none());
    assert!(value.get("prompt_cache_retention").is_none());
    assert!(value.get("prompt_cache_options").is_none());
    assert_eq!(value["prompt_cache_key"], json!("dsh-sess"));
}

/// A supplied request id is reused on retries of the same attempt; a missing
/// one is omitted (the planner mints, this crate does not).
#[test]
fn request_id_is_per_attempt_not_the_shard_key() {
    let hop = plan(&HopInput {
        request_id: Some("same-attempt"),
        retry_attempt: 2,
        ..HopInput::default()
    });
    assert_eq!(
        header_str(&hop.headers, "x-grok-req-id"),
        Some("same-attempt")
    );
    assert_eq!(
        header_str(&hop.headers, "x-grok-transient-retry"),
        Some("2")
    );
    assert_ne!(
        header_str(&hop.headers, "x-grok-req-id"),
        header_str(&hop.headers, "x-grok-conv-id")
    );

    let missing = plan(&HopInput::default());
    assert!(!missing.headers.contains_key("x-grok-req-id"));
}

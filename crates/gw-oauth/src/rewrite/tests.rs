use serde_json::json;

use super::{HeaderExtra, cache_headers, rewrite_body};
use crate::Family;

fn body_with_dsh_cache_fields() -> serde_json::Value {
    json!({
        "model": "probe",
        "session_id": "sess-1",
        "prompt_cache_key": "pin-1",
        "prompt_cache_retention": "24h",
        "messages": [{"role": "user", "content": "hi"}]
    })
}

#[test]
fn codex_keeps_prompt_cache_key_and_drops_session_id() {
    let out = rewrite_body(Family::Codex, body_with_dsh_cache_fields(), None);
    assert!(out.payload.get("session_id").is_none());
    assert_eq!(out.payload["prompt_cache_key"], "pin-1");
    let headers = cache_headers(Family::Codex, out.cache_session_id.as_deref(), &HeaderExtra::default());
    assert_eq!(headers.get("session-id").map(|v| v.as_bytes()), Some(b"pin-1".as_slice()));
    assert_eq!(headers.get("thread-id").map(|v| v.as_bytes()), Some(b"pin-1".as_slice()));
    assert!(headers.get("x-grok-conv-id").is_none());
}

#[test]
fn grok_uses_grok_headers_not_codex_session_id() {
    let out = rewrite_body(Family::Grok, body_with_dsh_cache_fields(), None);
    assert!(out.payload.get("session_id").is_none());
    let headers = cache_headers(
        Family::Grok,
        out.cache_session_id.as_deref(),
        &HeaderExtra {
            req_id: Some("req-1".to_owned()),
            model: Some("grok-4.6".to_owned()),
            retry_attempt: 0,
        },
    );
    assert!(headers.get("session-id").is_none());
    assert!(headers.get("x-client-request-id").is_none());
    assert_eq!(headers.get("x-grok-conv-id").map(|v| v.as_bytes()), Some(b"pin-1".as_slice()));
    assert_eq!(headers.get("x-grok-session-id").map(|v| v.as_bytes()), Some(b"pin-1".as_slice()));
}

#[test]
fn glm_strips_codex_fields() {
    let out = rewrite_body(Family::Glm, body_with_dsh_cache_fields(), None);
    assert!(out.payload.get("prompt_cache_key").is_none());
    assert!(out.payload.get("prompt_cache_retention").is_none());
    let headers = cache_headers(Family::Glm, out.cache_session_id.as_deref(), &HeaderExtra::default());
    assert!(headers.get("session-id").is_none());
    assert!(headers.get("x-grok-conv-id").is_none());
}

#[test]
fn ollama_does_not_write_a_sticky_header() {
    let out = rewrite_body(Family::Ollama, body_with_dsh_cache_fields(), None);
    assert!(out.payload.get("prompt_cache_key").is_none());
    let headers = cache_headers(Family::Ollama, out.cache_session_id.as_deref(), &HeaderExtra::default());
    assert!(headers.is_empty());
}

#[test]
fn copilot_writes_interaction_id_not_codex_headers() {
    let out = rewrite_body(Family::Copilot, body_with_dsh_cache_fields(), None);
    let headers = cache_headers(Family::Copilot, out.cache_session_id.as_deref(), &HeaderExtra::default());
    assert!(headers.get("session-id").is_none());
    assert!(headers.get("x-interaction-id").is_some());
}

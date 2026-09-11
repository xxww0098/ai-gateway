use serde_json::json;

use super::{COPILOT_STABLE_SESSION, apply_cache, cache_headers};

#[test]
fn missing_pin_still_writes_interaction_id() {
    let out = apply_cache(json!({}));
    assert_eq!(out.cache_session_id.as_deref(), Some(COPILOT_STABLE_SESSION));
    let headers = cache_headers(out.cache_session_id.as_deref());
    assert_eq!(headers.get("x-interaction-id").unwrap(), COPILOT_STABLE_SESSION);
    assert!(headers.get("session-id").is_none());
    assert!(headers.get("x-grok-conv-id").is_none());
}

#[test]
fn strips_codex_fields() {
    let out = apply_cache(json!({"prompt_cache_key": "p", "session_id": "s"}));
    assert!(out.payload.get("prompt_cache_key").is_none());
    assert_eq!(out.cache_session_id.as_deref(), Some("s"));
}

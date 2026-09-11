use serde_json::json;

use super::{OLLAMA_STABLE_SESSION, apply_cache};

#[test]
fn strips_codex_fields_and_does_not_invent_cached_tokens() {
    let out = apply_cache(json!({
        "prompt_cache_key": "x",
        "session_id": "s",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    assert!(out.payload.get("prompt_cache_key").is_none());
    assert!(out.payload.get("session_id").is_none());
    assert!(out.payload.get("cached_tokens").is_none());
    assert_eq!(out.cache_session_id.as_deref(), Some("s"));
}

#[test]
fn missing_pin_is_analyzer_constant() {
    let out = apply_cache(json!({}));
    assert_eq!(out.cache_session_id.as_deref(), Some(OLLAMA_STABLE_SESSION));
}

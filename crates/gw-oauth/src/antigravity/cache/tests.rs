use serde_json::json;

use super::{ANTIGRAVITY_STABLE_SESSION, apply_cache};

#[test]
fn fallback_session_includes_model() {
    let out = apply_cache(json!({"model": "gemini-3.8-flash-high"}), None);
    assert_eq!(
        out.cache_session_id.as_deref(),
        Some(&format!("{ANTIGRAVITY_STABLE_SESSION}:gemini-3.8-flash-high")[..])
    );
}

#[test]
fn caller_session_id_is_kept_as_is() {
    let out = apply_cache(json!({"session_id": "llm-session", "model": "other"}), None);
    assert_eq!(out.cache_session_id.as_deref(), Some("llm-session"));
    assert!(out.payload.get("prompt_cache_key").is_none());
}

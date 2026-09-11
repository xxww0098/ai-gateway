use serde_json::json;

use super::apply_cache;

#[test]
fn strips_codex_and_grok_fields() {
    let out = apply_cache(json!({
        "prompt_cache_key": "k",
        "session_id": "s",
        "prompt_cache_retention": "24h"
    }));
    assert!(out.payload.get("prompt_cache_key").is_none());
    assert!(out.payload.get("session_id").is_none());
    assert_eq!(out.cache_session_id.as_deref(), Some("s"));
}

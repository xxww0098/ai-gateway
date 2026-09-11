use serde_json::json;

use super::{apply_anthropic_cache, session_headers};

#[test]
fn anthropic_cache_drops_codex_fields_and_pins_user_id() {
    let out = apply_anthropic_cache(json!({
        "prompt_cache_key": "pin-glm",
        "session_id": "sess",
        "prompt_cache_retention": "24h",
        "system": "hello"
    }));
    assert!(out.payload.get("prompt_cache_key").is_none());
    assert!(out.payload.get("session_id").is_none());
    assert_eq!(out.payload["metadata"]["user_id"], "sess");
    let headers = session_headers(out.cache_session_id.as_deref());
    assert_eq!(headers.get("x-session-id").unwrap(), "sess");
    assert!(headers.get("session-id").is_none());
}

use serde_json::json;

use super::{apply_cache, cache_headers, cache_session_id};

#[test]
fn empty_or_non_id_becomes_none() {
    assert!(cache_session_id(None).is_none());
    assert!(cache_session_id(Some("")).is_none());
    assert!(cache_session_id(Some("   ")).is_none());
}

#[test]
fn apply_cache_strips_session_id_and_keeps_one_pin() {
    let out = apply_cache(json!({
        "prompt_cache_key": "conv-1",
        "session_id": "dsh-other",
        "model": "gpt-5.4"
    }));
    assert!(out.payload.get("session_id").is_none());
    assert_eq!(out.cache_session_id.as_deref(), Some("conv-1"));
    let headers = cache_headers(out.cache_session_id.as_deref());
    assert_eq!(headers.get("session-id").unwrap(), "conv-1");
    assert_eq!(headers.get("thread-id").unwrap(), "conv-1");
    assert_eq!(headers.get("x-client-request-id").unwrap(), "conv-1");
    assert!(headers.get("x-grok-conv-id").is_none());
}

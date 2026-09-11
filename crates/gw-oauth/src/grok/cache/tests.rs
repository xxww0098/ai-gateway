use serde_json::json;

use super::{GROK_STABLE_SESSION, affinity_headers, apply_cache, cache_session_id};
use crate::rewrite::HeaderExtra;

#[test]
fn missing_pin_falls_back_to_stable_constant() {
    let out = apply_cache(json!({"model": "grok-4.6"}));
    assert_eq!(out.cache_session_id.as_deref(), Some(GROK_STABLE_SESSION));
    assert!(out.payload.get("session_id").is_none());
}

#[test]
fn affinity_headers_are_not_codex_session_id() {
    let headers = affinity_headers(
        cache_session_id(Some("conv-9")).as_deref(),
        &HeaderExtra {
            req_id: Some("req-9".to_owned()),
            model: Some("grok-4.6".to_owned()),
            retry_attempt: 1,
        },
    );
    assert!(headers.get("session-id").is_none());
    assert!(headers.get("x-client-request-id").is_none());
    assert_eq!(headers.get("x-grok-conv-id").unwrap(), "conv-9");
    assert_eq!(headers.get("x-grok-req-id").unwrap(), "req-9");
    assert_eq!(headers.get("x-grok-transient-retry").unwrap(), "1");
}

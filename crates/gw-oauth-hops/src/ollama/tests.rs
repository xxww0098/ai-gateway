use super::{OLLAMA_STABLE_SESSION, plan};
use crate::rewrite::HopInput;
use serde_json::json;

/// No identity headers, no bearer, no Codex / Grok sticky names.
#[test]
fn hop_is_strip_only() {
    let hop = plan(&HopInput::default(), None);
    assert!(hop.headers.is_empty());
    assert_eq!(hop.cache_session_id.as_deref(), Some(OLLAMA_STABLE_SESSION));
    assert!(!hop.headers.contains_key(http::header::AUTHORIZATION));
}

/// Cache fields leave. Two empty plans share the stable analyzer id.
#[test]
fn cache_fields_leave_and_id_is_stable() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-oss:120b-cloud",
        "session_id": "ol-1",
        "prompt_cache_key": "nope",
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
    assert!(value.get("session_id").is_none());
    assert!(value.get("prompt_cache_key").is_none());
    assert_eq!(hop.cache_session_id.as_deref(), Some("ol-1"));
    let again = plan(&HopInput::default(), None);
    assert_eq!(
        again.cache_session_id.as_deref(),
        Some(OLLAMA_STABLE_SESSION)
    );
}

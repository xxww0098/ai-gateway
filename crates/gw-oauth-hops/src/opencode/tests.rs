use super::{OPENCODE_STABLE_SESSION, plan};
use crate::rewrite::HopInput;
use serde_json::json;

/// Zen 401s any bearer. Codex / Grok sticky headers are not a thing here.
#[test]
fn never_sends_authorization_or_foreign_cache_headers() {
    let hop = plan(&HopInput::default(), None);
    assert!(!hop.headers.contains_key(http::header::AUTHORIZATION));
    assert!(!hop.headers.contains_key("session-id"));
    assert!(!hop.headers.contains_key("x-grok-conv-id"));
    assert!(hop.headers.contains_key("user-agent"));
    assert!(hop.headers.contains_key("http-referer"));
    assert_eq!(
        hop.cache_session_id.as_deref(),
        Some(OPENCODE_STABLE_SESSION)
    );
}

/// Cache fields are stripped, not forwarded as a fake conversation id.
#[test]
fn cache_fields_leave_the_body() {
    let body = serde_json::to_vec(&json!({
        "model": "big-pickle",
        "session_id": "oc-1",
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
    assert_eq!(value["model"], json!("big-pickle"));
}

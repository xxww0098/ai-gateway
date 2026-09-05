use super::{ANTIGRAVITY_STABLE_SESSION, conversation_id, plan};
use crate::rewrite::HopInput;
use serde_json::json;

/// Chat hops do not send the onboardUser `x-goog-api-client` fingerprint.
#[test]
fn chat_identity_omits_goog_api_client_and_codex() {
    let hop = plan(&HopInput::default(), None);
    assert!(hop.headers.contains_key("user-agent"));
    assert!(!hop.headers.contains_key("x-goog-api-client"));
    assert!(!hop.headers.contains_key("session-id"));
    assert!(!hop.headers.contains_key(http::header::AUTHORIZATION));
}

/// Fallback ids include the model so two pickers cannot share a pin.
#[test]
fn fallback_id_includes_the_model() {
    let a = conversation_id(&json!({ "model": "gemini-3-flash" }), None, None);
    let b = conversation_id(&json!({ "model": "gemini-3-pro-high" }), None, None);
    assert_ne!(a, b);
    assert!(a.starts_with(ANTIGRAVITY_STABLE_SESSION) || a.contains("gemini"));
}

/// `sessionId` is stamped; Codex `session_id` leaves.
#[test]
fn session_id_is_the_gemini_field() {
    let body = serde_json::to_vec(&json!({
        "session_id": "ag-1",
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
    assert_eq!(value["sessionId"], json!("ag-1"));
    assert!(value.get("prompt_cache_key").is_none());
}

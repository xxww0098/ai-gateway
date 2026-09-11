use serde_json::json;

use super::{KIRO_STABLE_SESSION, apply_cache, pin_system_prefix, reset_system_pins};

#[test]
fn conversation_id_includes_model_and_never_date_now() {
    let out = apply_cache(
        json!({"model": "claude-sonnet-4", "session_id": "sess-k"}),
        None,
    );
    let id = out.cache_session_id.expect("pin");
    assert!(id.contains("claude-sonnet-4"), "{id}");
    assert!(id.starts_with("sess-k"), "{id}");
    assert!(
        !id.chars().all(|c| c.is_ascii_digit()),
        "must not be a Date.now() millisecond stamp: {id}"
    );
}

#[test]
fn missing_pin_is_stable_constant_plus_model() {
    let out = apply_cache(json!({"model": "opus-4.6"}), None);
    assert_eq!(
        out.cache_session_id.as_deref(),
        Some(&format!("{KIRO_STABLE_SESSION}:opus-4.6")[..])
    );
    assert!(out.payload.get("prompt_cache_key").is_none());
}

#[test]
fn later_system_snapshot_is_extra_not_a_rewrite_of_the_pin() {
    reset_system_pins();
    let id = "session-pin:claude-sonnet-5";
    let first = pin_system_prefix(id, "You are DSH.");
    assert_eq!(first.pinned, "You are DSH.");
    assert!(first.extra.is_empty());
    let second = pin_system_prefix(
        id,
        "You are DSH.\nThis snapshot supersedes the previous context.",
    );
    assert_eq!(second.pinned, first.pinned);
    assert_ne!(second.extra, second.pinned);
    assert!(!second.extra.is_empty());
    assert!(!second.extra.contains("You are DSH."));
}

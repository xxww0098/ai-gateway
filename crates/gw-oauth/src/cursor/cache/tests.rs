use super::{
    CURSOR_STABLE_SESSION, apply_cache, conversation_id, peel_fast_suffix, pin_system_prefix,
    reset_system_pins, stable_id,
};
use serde_json::json;

#[test]
fn fast_suffix_is_peeled_from_the_pin_model() {
    let (id, fast) = peel_fast_suffix("composer-2.5-fast");
    assert_eq!(id, "composer-2.5");
    assert!(fast);
    let out = apply_cache(json!({"model": "composer-2.5-fast", "session_id": "c1"}));
    assert_eq!(out.cache_session_id.as_deref(), Some("c1:composer-2.5"));
    assert!(out.payload.get("service_tier").is_none());
}

#[test]
fn stable_id_is_deterministic_and_not_random_uuid() {
    let a = stable_id(&["hello", "turn"]);
    let b = stable_id(&["hello", "turn"]);
    let c = stable_id(&["hello", "other"]);
    assert_eq!(a, b);
    assert_ne!(a, c);
    let parts: Vec<&str> = a.split('-').collect();
    assert_eq!(parts.len(), 5, "UUID layout, but hashed: {a}");
}

#[test]
fn apply_cache_strips_codex_fields() {
    let out = apply_cache(json!({
        "session_id": "sess-cursor",
        "prompt_cache_key": "codex-style",
        "prompt_cache_retention": "24h",
        "prompt_cache_options": {"max_age": 1},
        "service_tier": "priority",
        "model": "composer-2",
    }));
    assert!(out.payload.get("prompt_cache_key").is_none());
    assert!(out.payload.get("prompt_cache_retention").is_none());
    assert!(out.payload.get("prompt_cache_options").is_none());
    assert!(out.payload.get("service_tier").is_none());
    assert_eq!(
        out.cache_session_id.as_deref(),
        Some("sess-cursor:composer-2")
    );
}

#[test]
fn conversation_id_peels_fast_and_is_stable_across_turns() {
    let first = conversation_id(&json!({"session_id": "sess-cursor", "model": "gpt-5.5-fast"}));
    let second = conversation_id(&json!({"session_id": "sess-cursor", "model": "gpt-5.5"}));
    assert_eq!(first, second);
    assert!(!first.ends_with("-fast"));
    assert_ne!(
        conversation_id(&json!({"model": "composer-2"})),
        conversation_id(&json!({"model": "gpt-5.5"})),
    );
    assert_eq!(conversation_id(&json!({})), CURSOR_STABLE_SESSION);
    assert!(!conversation_id(&json!({})).starts_with('-'));
}

#[test]
fn later_system_snapshot_is_extra_not_a_rewrite() {
    reset_system_pins();
    let pin = pin_system_prefix("sess-cursor:composer-2", "You are DSH.");
    let extra = pin_system_prefix("sess-cursor:composer-2", "You are DSH.\nSnapshot");
    assert_eq!(pin.0, "You are DSH.");
    assert_eq!(extra.0, "You are DSH.");
    assert_eq!(extra.1, "Snapshot");
    reset_system_pins();
}

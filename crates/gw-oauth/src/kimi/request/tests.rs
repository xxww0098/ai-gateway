use serde_json::{Map, Value, json};

use super::{REASONING, apply_thinking, apply_thinking_for};

#[test]
fn picker_effort_lands_inside_thinking_not_on_the_wire_key() {
    let on = apply_thinking(json!({"model": "k3", "reasoning_effort": "low"}));
    assert_eq!(on["thinking"]["type"], "enabled");
    assert_eq!(on["thinking"]["effort"], "low");
    assert!(on.get("reasoning_effort").is_none());

    let mapped = apply_thinking(json!({"model": "k3", "reasoning_effort": "medium"}));
    assert_eq!(mapped["thinking"]["effort"], "high");

    let off = apply_thinking(json!({"model": "k3", "reasoning_effort": "off"}));
    assert_eq!(off["thinking"], json!({"type": "disabled"}));
    assert!(off.get("reasoning_effort").is_none());
}

#[test]
fn unknown_model_does_not_invent_thinking() {
    let out = apply_thinking(json!({
        "model": "not-a-kimi-coding-model",
        "reasoning_effort": "high",
        "thinking": {"type": "enabled", "effort": "high"}
    }));
    assert!(out.get("thinking").is_none());
    assert!(out.get("reasoning_effort").is_none());
}

#[test]
fn advertised_map_without_off_still_disables_when_family_default_has_off() {
    let mut efforts = Map::new();
    efforts.insert("low".to_owned(), Value::String("low".to_owned()));
    let out = apply_thinking_for(
        json!({"model": "custom", "reasoning_effort": "off"}),
        Some(&efforts),
    );
    assert!(REASONING.iter().any(|(key, wire)| *key == "off" && *wire == "off"));
    assert_eq!(out["thinking"], json!({"type": "disabled"}));
}

#[test]
fn vendor_spelling_is_accepted_as_a_value() {
    let out = apply_thinking(json!({"model": "kimi-for-coding", "reasoning_effort": "max"}));
    assert_eq!(out["thinking"]["effort"], "max");
}

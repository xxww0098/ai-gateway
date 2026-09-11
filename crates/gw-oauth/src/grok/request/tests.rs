//! Properties of Grok Responses body shaping.
//!
//! Extra DSH snapshots must not rewrite the leading system prefix; they park
//! at the input suffix. Top-level `instructions` is never introduced.
//! `service_tier` is never forwarded.

use serde_json::{Value, json};

use super::normalize_request;

fn conv(id: &str) -> String {
    format!("grok-req-{id}")
}

#[test]
fn leading_developer_becomes_system_in_input_not_instructions() {
    let out = normalize_request(json!({
        "model": "grok-4.6",
        "session_id": conv("lead"),
        "input": [
            {"role": "developer", "content": "You are DSH."},
            {"role": "user", "content": [{"type": "input_text", "text": "hi"}]}
        ]
    }));
    assert!(out.get("instructions").is_none());
    let input = out["input"].as_array().expect("input");
    assert_eq!(input[0]["role"], "system");
    assert_eq!(input[0]["content"], "You are DSH.");
    assert_eq!(input[1]["role"], "user");
}

#[test]
fn extra_leading_developer_parks_at_the_input_suffix() {
    let key = conv("park");
    let _first = normalize_request(json!({
        "model": "grok-4.6",
        "prompt_cache_key": key,
        "input": [
            {"role": "developer", "content": "You are DSH."},
            {"role": "user", "content": [{"type": "input_text", "text": "hi"}]}
        ]
    }));
    let out = normalize_request(json!({
        "model": "grok-4.6",
        "prompt_cache_key": key,
        "input": [
            {"role": "developer", "content": "You are DSH.\n\nPlan: toggle all skills."},
            {"role": "user", "content": [{"type": "input_text", "text": "hi"}]},
            {"role": "assistant", "content": [{"type": "output_text", "text": "ok"}]}
        ]
    }));
    assert!(out.get("instructions").is_none());
    let input = out["input"].as_array().expect("input");
    assert_eq!(input[0]["role"], "system");
    assert_eq!(input[0]["content"], "You are DSH.");
    assert_eq!(input[1]["role"], "user");
    assert_eq!(input[2]["role"], "assistant");
    assert_eq!(input[3]["role"], "developer");
    assert_eq!(input[3]["content"][0]["type"], "input_text");
    assert_eq!(input[3]["content"][0]["text"], "Plan: toggle all skills.");
}

#[test]
fn a_wholly_different_leading_snapshot_still_leaves_history_in_the_middle() {
    let key = conv("rebuild");
    let _first = normalize_request(json!({
        "model": "grok-4.6",
        "session_id": key,
        "input": [
            {"role": "system", "content": "You are DSH."},
            {"role": "user", "content": "hi"}
        ]
    }));
    let out = normalize_request(json!({
        "model": "grok-4.6",
        "session_id": key,
        "input": [
            {"role": "system", "content": "Session header rebuilt."},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "ok"}
        ]
    }));
    let input = out["input"].as_array().expect("input");
    assert_eq!(input[0]["content"], "You are DSH.");
    assert_eq!(input[1]["role"], "user");
    assert_eq!(input[2]["role"], "assistant");
    assert_eq!(input[3]["role"], "developer");
    assert_eq!(input[3]["content"][0]["text"], "Session header rebuilt.");
}

#[test]
fn keyless_requests_do_not_share_a_pin() {
    let first = normalize_request(json!({
        "model": "grok-4.6",
        "input": [
            {"role": "developer", "content": "You are DSH."},
            {"role": "user", "content": "hi"}
        ]
    }));
    let second = normalize_request(json!({
        "model": "grok-4.6",
        "input": [
            {"role": "developer", "content": "Other snapshot."},
            {"role": "user", "content": "hi"}
        ]
    }));
    assert_eq!(first["input"][0]["content"], "You are DSH.");
    assert_eq!(second["input"][0]["content"], "Other snapshot.");
    let second_input = second["input"].as_array().expect("input");
    assert_eq!(
        second_input.last().map(|v| &v["role"]),
        Some(&json!("user"))
    );
}

#[test]
fn non_array_input_is_left_in_place() {
    let payload = json!({
        "model": "grok-4.6",
        "session_id": conv("text"),
        "input": "just text"
    });
    let out = normalize_request(payload.clone());
    assert_eq!(out["input"], payload["input"]);
}

#[test]
fn service_tier_is_stripped_and_fast_suffix_is_peeled() {
    let out = normalize_request(json!({
        "model": "grok-4.6-fast",
        "service_tier": "priority",
        "input": [{"role": "user", "content": "hi"}]
    }));
    assert!(out.get("service_tier").is_none());
    let model = out["model"].as_str().expect("model");
    assert!(!model.to_ascii_lowercase().ends_with("-fast"), "{model}");
    assert!(out.get("instructions").is_none());
}

#[test]
fn non_object_payloads_pass_through() {
    assert_eq!(normalize_request(Value::Null), Value::Null);
    assert_eq!(normalize_request(json!([1, 2])), json!([1, 2]));
}

use serde_json::{Value, json};

use super::normalize_request;

fn conversation_roles(input: &[Value]) -> Vec<&str> {
    input
        .iter()
        .map(|item| item.get("role").and_then(Value::as_str).unwrap_or(""))
        .collect()
}

#[test]
fn lifts_leading_developer_into_instructions() {
    let developer = "You are DSH.";
    let out = normalize_request(json!({
        "input": [
            { "role": "developer", "content": developer },
            { "role": "user", "content": [{ "type": "input_text", "text": "hi" }] },
        ],
    }));
    assert_eq!(out["instructions"], developer);
    let input = out["input"].as_array().expect("input");
    assert_eq!(conversation_roles(input), ["user"]);
    assert!(
        out["include"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}

#[test]
fn defaults_instructions_when_none_are_present() {
    let out = normalize_request(json!({
        "input": [{ "role": "user", "content": "hi" }],
    }));
    assert!(
        out["instructions"]
            .as_str()
            .is_some_and(|text| !text.trim().is_empty())
    );
}

#[test]
fn strips_pro_mode_and_leaves_the_effort_alone() {
    let effort = "max";
    let out = normalize_request(json!({
        "reasoning": { "effort": effort, "mode": "pro", "summary": "auto" },
    }));
    assert_eq!(out["reasoning"]["effort"], effort);
    assert_eq!(out["reasoning"]["summary"], "auto");
    assert!(out["reasoning"].get("mode").is_none());
}

#[test]
fn drops_public_only_fields_and_default_service_tier() {
    let out = normalize_request(json!({
        "service_tier": "default",
        "max_output_tokens": 128000,
        "prompt_cache_options": { "mode": "explicit" },
        "prompt_cache_retention": "24h",
        "safety_identifier": "sid",
    }));
    assert!(out.get("service_tier").is_none());
    assert_eq!(out["store"], false);
    assert!(out.get("max_output_tokens").is_none());
    assert!(out.get("prompt_cache_options").is_none());
    assert!(out.get("prompt_cache_retention").is_none());
    assert!(out.get("safety_identifier").is_none());
}

#[test]
fn maps_fast_to_priority_and_forces_store_false() {
    let out = normalize_request(json!({
        "service_tier": "fast",
        "store": true,
    }));
    assert_eq!(out["service_tier"], "priority");
    assert_eq!(out["store"], false);
}

#[test]
fn strips_duplicate_leading_developer_once_instructions_exist() {
    let instructions = "You are DSH.";
    let out = normalize_request(json!({
        "instructions": instructions,
        "input": [
            { "role": "developer", "content": instructions },
            { "role": "user", "content": [{ "type": "input_text", "text": "hi" }] },
        ],
    }));
    assert_eq!(out["instructions"], instructions);
    assert_eq!(
        conversation_roles(out["input"].as_array().expect("input")),
        ["user"]
    );
}

#[test]
fn parks_extra_leading_developer_after_the_conversation() {
    let instructions = "You are DSH.";
    let extra = "Plan: toggle all skills.";
    let out = normalize_request(json!({
        "instructions": instructions,
        "input": [
            { "role": "developer", "content": format!("{instructions}\n\n{extra}") },
            { "role": "user", "content": [{ "type": "input_text", "text": "hi" }] },
            { "role": "assistant", "content": [{ "type": "output_text", "text": "ok" }] },
        ],
    }));
    assert_eq!(out["instructions"], instructions);
    let input = out["input"].as_array().expect("input");
    assert_eq!(
        conversation_roles(input),
        ["user", "assistant", "developer"]
    );
    assert_eq!(
        input[2]["content"],
        json!([{ "type": "input_text", "text": extra }])
    );
}

#[test]
fn lifts_a_run_of_leading_developer_items_then_parks_the_extra() {
    let instructions = "You are DSH.";
    let extra = "Plan: toggle all skills.";
    let out = normalize_request(json!({
        "instructions": instructions,
        "input": [
            { "role": "developer", "content": instructions },
            { "role": "developer", "content": extra },
            { "role": "user", "content": [{ "type": "input_text", "text": "hi" }] },
            { "role": "assistant", "content": [{ "type": "output_text", "text": "ok" }] },
        ],
    }));
    assert_eq!(out["instructions"], instructions);
    let input = out["input"].as_array().expect("input");
    assert_eq!(
        conversation_roles(input),
        ["user", "assistant", "developer"]
    );
    assert_eq!(
        input[2]["content"],
        json!([{ "type": "input_text", "text": extra }])
    );
}

#[test]
fn parks_a_wholly_different_leading_system_so_history_still_leads() {
    let instructions = "You are DSH.";
    let rebuilt = "Session header rebuilt.";
    let out = normalize_request(json!({
        "instructions": instructions,
        "input": [
            { "role": "system", "content": rebuilt },
            { "role": "user", "content": "hi" },
            { "role": "assistant", "content": "ok" },
        ],
    }));
    assert_eq!(out["instructions"], instructions);
    let input = out["input"].as_array().expect("input");
    assert_eq!(
        conversation_roles(input),
        ["user", "assistant", "developer"]
    );
    assert_eq!(
        input[2]["content"],
        json!([{ "type": "input_text", "text": rebuilt }])
    );
}

#[test]
fn does_not_lift_a_developer_item_after_the_conversation_has_started() {
    let later = json!({ "role": "developer", "content": "Plan: later" });
    let out = normalize_request(json!({
        "instructions": "You are DSH.",
        "input": [
            { "role": "user", "content": "hi" },
            later,
        ],
    }));
    assert_eq!(
        out["input"],
        json!([
            { "role": "user", "content": "hi" },
            { "role": "developer", "content": "Plan: later" },
        ])
    );
}

#[test]
fn does_not_duplicate_extra_when_instructions_already_contain_the_prefix() {
    let extra = "Plan: already in instructions.";
    let instructions = format!("You are DSH.\n\n{extra}");
    let out = normalize_request(json!({
        "instructions": instructions,
        "input": [
            { "role": "developer", "content": "You are DSH." },
            { "role": "user", "content": "hi" },
        ],
    }));
    assert_eq!(out["instructions"], instructions);
    assert_eq!(out["input"], json!([{ "role": "user", "content": "hi" }]));
}

#[test]
fn non_object_bodies_pass_through() {
    let array = json!([{ "role": "user" }]);
    assert_eq!(normalize_request(array.clone()), array);
    let text = Value::String("x".to_owned());
    assert_eq!(normalize_request(text.clone()), text);
}

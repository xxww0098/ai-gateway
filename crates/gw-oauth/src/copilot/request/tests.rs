use serde_json::json;

use super::{apply_thinking, map_usage};

#[test]
fn advertised_effort_is_kept_and_gpt_max_tokens_are_stripped() {
    let on = apply_thinking(json!({
        "model": "gpt-5.5",
        "reasoning_effort": "high",
        "max_tokens": 128
    }));
    assert_eq!(on["reasoning_effort"], "high");
    assert!(on.get("max_tokens").is_none());

    let off = apply_thinking(json!({"model": "gpt-5.5", "reasoning_effort": "off"}));
    assert!(off.get("reasoning_effort").is_none());
}

#[test]
fn non_gpt_without_effort_map_keeps_max_tokens() {
    let claude = apply_thinking(json!({
        "model": "claude-sonnet-4.6",
        "reasoning_effort": "high",
        "max_tokens": 64
    }));
    assert!(claude.get("reasoning_effort").is_none());
    assert_eq!(claude["max_tokens"], 64);
}

#[test]
fn cache_read_fields_become_cached_tokens() {
    let usage = map_usage(json!({"prompt_tokens": 10, "cache_read_input_tokens": 4}));
    assert_eq!(usage["prompt_tokens_details"]["cached_tokens"], 4);
    let untouched = map_usage(json!({"prompt_tokens": 3}));
    assert!(untouched.get("prompt_tokens_details").is_none());
}

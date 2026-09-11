//! Shape openai-responses bodies for xAI Grok.
//!
//! grok-build sends `instructions: null` and keeps system in `input`. Prefix
//! cache stays hot only when later turns replay that order byte for byte.
//! Extra DSH snapshots park at the input suffix. Never lift into top-level
//! `instructions`. Never write `service_tier` (Grok Fast is a no-op).

use serde_json::{Value, json};

use super::cache::{conversation_id, pin_system_prefix};

#[cfg(test)]
mod tests;

const FAST_SUFFIX: &str = "-fast";

/// Pin leading system/developer, park extras at the input suffix, peel a
/// leftover `-fast` model suffix, and drop `service_tier`.
#[must_use]
pub fn normalize_request(payload: Value) -> Value {
    if !payload.is_object() {
        return payload;
    }
    let mut next = payload;
    peel_fast_model(&mut next);
    if let Some(obj) = next.as_object_mut() {
        obj.remove("service_tier");
    }
    stabilize_input(next)
}

fn peel_fast_model(payload: &mut Value) {
    let Some(model) = payload.get("model").and_then(Value::as_str) else {
        return;
    };
    if !model.to_ascii_lowercase().ends_with(FAST_SUFFIX) || model.len() <= FAST_SUFFIX.len() {
        return;
    }
    let peeled = model[..model.len() - FAST_SUFFIX.len()].to_owned();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("model".to_owned(), Value::String(peeled));
    }
}

fn stabilize_input(mut next: Value) -> Value {
    let Some(input) = next.get("input").and_then(Value::as_array).cloned() else {
        return next;
    };
    let conv = conversation_id(&next);
    let (lifted, rest) = split_leading_instructions(&input);
    let (pinned, extra) = pin_system_prefix(&conv, &lifted);
    let mut items = Vec::with_capacity(rest.len() + 2);
    if !pinned.is_empty() {
        items.push(json!({"role": "system", "content": pinned}));
    }
    items.extend(rest);
    if !extra.is_empty() {
        items.push(json!({
            "role": "developer",
            "content": [{"type": "input_text", "text": extra}],
        }));
    }
    if let Some(obj) = next.as_object_mut() {
        obj.insert("input".to_owned(), Value::Array(items));
    }
    next
}

fn split_leading_instructions(input: &[Value]) -> (String, Vec<Value>) {
    let mut lifted = Vec::new();
    let mut rest = Vec::new();
    for item in input {
        if rest.is_empty() && is_instruction_role(item) {
            let text = instruction_text(item);
            if !text.is_empty() {
                lifted.push(text);
            }
            continue;
        }
        rest.push(item.clone());
    }
    (lifted.join("\n\n"), rest)
}

fn is_instruction_role(item: &Value) -> bool {
    matches!(
        item.get("role").and_then(Value::as_str),
        Some("system" | "developer")
    )
}

fn instruction_text(item: &Value) -> String {
    match item.get("content") {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|part| match part {
                Value::String(text) => text.as_str(),
                Value::Object(obj) => obj.get("text").and_then(Value::as_str).unwrap_or(""),
                _ => "",
            })
            .collect::<String>()
            .trim()
            .to_owned(),
        _ => String::new(),
    }
}

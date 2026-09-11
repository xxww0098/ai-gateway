//! Copilot Completions hop: keep advertised `reasoning_effort`, strip GPT max tokens.

use serde_json::{Map, Value};

#[cfg(test)]
mod tests;

/// Picker keys that Copilot GPT-5.x advertises.
pub const REASONING: &[(&str, &str)] = &[("low", "low"), ("medium", "medium"), ("high", "high")];

const REASONING_MODELS: &[&str] = &["gpt-5.4", "gpt-5.5"];

/// Keep OpenAI `reasoning_effort` when the catalog advertises it. GPT ids drop max tokens.
#[must_use]
pub fn apply_thinking(payload: Value) -> Value {
    apply_thinking_for(payload, None)
}

/// Same as [`apply_thinking`], with an optional advertised effort map.
#[must_use]
pub fn apply_thinking_for(payload: Value, advertised: Option<&Map<String, Value>>) -> Value {
    let mut next = payload;
    let Some(object) = next.as_object_mut() else {
        return next;
    };
    let model = object.get("model").and_then(Value::as_str).unwrap_or("").to_owned();
    let efforts = advertised.cloned().or_else(|| advertised_for_model(&model));
    let effort = object.get("reasoning_effort").cloned();
    if let Some(efforts) = efforts {
        match effort.as_ref().and_then(|value| wire_effort(value, &efforts)) {
            Some(wire) => {
                object.insert("reasoning_effort".to_owned(), Value::String(wire));
            }
            None => {
                object.remove("reasoning_effort");
            }
        }
    } else {
        object.remove("reasoning_effort");
    }
    if is_gpt_model(&model) {
        object.remove("max_tokens");
        object.remove("max_completion_tokens");
        object.remove("max_output_tokens");
    }
    next
}

/// Map `cache_read_*` onto OpenAI `prompt_tokens_details.cached_tokens`.
#[must_use]
pub fn map_usage(usage: Value) -> Value {
    let Some(object) = usage.as_object() else {
        return usage;
    };
    let cached = object
        .get("prompt_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_i64)
        .or_else(|| object.get("cached_tokens").and_then(Value::as_i64))
        .or_else(|| object.get("cache_read_input_tokens").and_then(Value::as_i64))
        .or_else(|| object.get("cache_read_tokens").and_then(Value::as_i64));
    let Some(cached) = cached.filter(|n| *n >= 0) else {
        return usage;
    };
    let mut next = usage;
    let details = match next.get("prompt_tokens_details") {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    let mut details = details;
    if details.get("cached_tokens").and_then(Value::as_i64).is_none() {
        details.insert("cached_tokens".to_owned(), Value::from(cached));
    }
    if let Some(object) = next.as_object_mut() {
        object.insert("prompt_tokens_details".to_owned(), Value::Object(details));
    }
    next
}

fn advertised_for_model(id: &str) -> Option<Map<String, Value>> {
    if !REASONING_MODELS.contains(&id) {
        return None;
    }
    let mut map = Map::new();
    for (key, wire) in REASONING {
        map.insert((*key).to_owned(), Value::String((*wire).to_owned()));
    }
    Some(map)
}

fn wire_effort(value: &Value, efforts: &Map<String, Value>) -> Option<String> {
    if is_off(value) {
        return None;
    }
    let text = value.as_str()?.trim();
    if let Some(Value::String(wire)) = efforts.get(text) {
        return Some(wire.clone());
    }
    efforts
        .values()
        .find_map(|item| item.as_str().filter(|wire| *wire == text).map(str::to_owned))
}

fn is_off(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::String(text) => matches!(text.trim(), "off" | "none" | "disabled" | ""),
        _ => false,
    }
}

fn is_gpt_model(id: &str) -> bool {
    id.to_ascii_lowercase().contains("gpt")
}

//! Shape bodies for Zhipu Coding Plan.
//!
//! Anthropic Messages is the default hop. Completions `paas/v4` is leftover:
//! unknown instructional roles (`developer`) become `system` (400 `1214`).

use serde_json::{Map, Value};

use super::{apply_anthropic_cache, apply_cache};

#[cfg(test)]
mod tests;

const CHAT_ROLES: [&str; 4] = ["system", "user", "assistant", "tool"];

/// Anthropic requires a positive `max_tokens`; GLM Coding Plan default.
pub const DEFAULT_MAX_TOKENS: i64 = 128_000;

/// GLM-5.3 / Flash cannot turn thinking off (`type: disabled` 400s). Turbo is hybrid.
#[must_use]
pub fn forced_thinking_model(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    id == "glm-5.3" || id.starts_with("glm-5.3-")
}

/// Anthropic default hop: `max_tokens`, thinking, then this family's cache.
#[must_use]
pub fn normalize_request(payload: Value) -> Value {
    normalize_anthropic(payload)
}

/// Completions leftover until the next catalog sync.
#[must_use]
pub fn normalize_completions(payload: Value) -> Value {
    let Some(obj) = payload.as_object() else {
        return payload;
    };
    let mut next = obj.clone();
    if let Some(Value::Array(messages)) = next.get_mut("messages") {
        for message in messages.iter_mut() {
            rewrite_chat_message(message);
        }
    }
    let cached = apply_cache(Value::Object(next)).payload;
    apply_thinking(cached)
}

fn normalize_anthropic(payload: Value) -> Value {
    let Some(obj) = payload.as_object() else {
        return payload;
    };
    let mut next = obj.clone();
    if !positive_number(next.get("max_tokens")) {
        next.insert("max_tokens".to_owned(), Value::from(DEFAULT_MAX_TOKENS));
    }
    let cached = apply_anthropic_cache(Value::Object(next)).payload;
    apply_thinking(cached)
}

fn rewrite_chat_message(message: &mut Value) {
    let Some(obj) = message.as_object_mut() else {
        return;
    };
    if let Some(role) = obj.get("role").and_then(Value::as_str)
        && !CHAT_ROLES.contains(&role)
    {
        obj.insert("role".to_owned(), Value::String("system".to_owned()));
    }
    if obj.get("role").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    let has_reasoning_content = match obj.get("reasoning_content") {
        None | Some(Value::Null) => false,
        Some(_) => true,
    };
    if has_reasoning_content {
        return;
    }
    if let Some(reasoning) = obj.get("reasoning").cloned().filter(|v| !v.is_null()) {
        obj.insert("reasoning_content".to_owned(), reasoning);
    }
}

fn apply_thinking(payload: Value) -> Value {
    let Some(obj) = payload.as_object() else {
        return payload;
    };
    let mut next = obj.clone();
    let forced = next
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(forced_thinking_model);
    let current = match next.get("thinking") {
        Some(Value::Object(map)) => Some(map.clone()),
        _ => None,
    };
    if forced {
        let mut thinking = current.unwrap_or_else(Map::new);
        thinking.insert("type".to_owned(), Value::String("enabled".to_owned()));
        thinking.insert("clear_thinking".to_owned(), Value::Bool(false));
        next.insert("thinking".to_owned(), Value::Object(thinking));
        return Value::Object(next);
    }
    if let Some(mut thinking) = current
        && thinking.get("type").and_then(Value::as_str) != Some("disabled")
    {
        thinking.insert("clear_thinking".to_owned(), Value::Bool(false));
        next.insert("thinking".to_owned(), Value::Object(thinking));
    }
    Value::Object(next)
}

fn positive_number(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_f64)
        .is_some_and(|n| n.is_finite() && n > 0.0)
}

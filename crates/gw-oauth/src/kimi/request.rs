//! Map picker `reasoning_effort` onto Kimi `thinking` / `thinking.effort`.

use serde_json::{Map, Value};

#[cfg(test)]
mod tests;

/// Picker keys → Kimi `thinking.effort`. Vendor `none` is never a key.
pub const REASONING: &[(&str, &str)] = &[
    ("off", "off"),
    ("minimal", "low"),
    ("low", "low"),
    ("medium", "high"),
    ("high", "high"),
    ("xhigh", "max"),
    ("max", "max"),
];

const THINKING_MODELS: &[&str] = &["kimi-for-coding", "kimi-for-coding-highspeed", "k3"];

/// Official Kimi Code accepts effort only inside `thinking`.
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
    let effort = object.remove("reasoning_effort");
    let efforts = advertised.cloned().or_else(|| advertised_for_model(object.get("model")));
    let Some(efforts) = efforts else {
        object.remove("thinking");
        return next;
    };
    let wire = match effort.as_ref() {
        None => None,
        Some(value) if is_off(value) => Some("off".to_owned()),
        Some(value) => wire_effort(value, &efforts).or_else(|| wire_effort(&Value::String("medium".to_owned()), &efforts)),
    };
    let Some(wire) = wire else {
        return next;
    };
    if wire == "off" || is_off(&Value::String(wire.clone())) {
        if efforts.contains_key("off") || REASONING.iter().any(|(_, w)| *w == "off") {
            object.insert("thinking".to_owned(), serde_json::json!({"type": "disabled"}));
        } else {
            object.remove("thinking");
        }
        return next;
    }
    object.insert(
        "thinking".to_owned(),
        serde_json::json!({"type": "enabled", "effort": wire}),
    );
    next
}

fn advertised_for_model(model: Option<&Value>) -> Option<Map<String, Value>> {
    let id = model.and_then(Value::as_str).unwrap_or("");
    if !THINKING_MODELS.contains(&id) {
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
        return Some("off".to_owned());
    }
    let text = value.as_str()?.trim();
    if text.is_empty() {
        return Some("off".to_owned());
    }
    if let Some(Value::String(wire)) = efforts.get(text) {
        return Some(wire.clone());
    }
    efforts.values().find_map(|item| {
        item.as_str()
            .filter(|wire| *wire == text)
            .map(str::to_owned)
    })
}

fn is_off(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::String(text) => matches!(text.trim(), "off" | "none" | "disabled" | ""),
        _ => false,
    }
}

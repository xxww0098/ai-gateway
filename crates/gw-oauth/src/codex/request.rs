//! Shape a generic openai-responses body for chatgpt.com Codex.
//!
//! Cache matches the longest stable prefix of `instructions` then `input`.
//! Extra leading developer/system items are parked at the input suffix.

use serde_json::{Map, Value};

#[cfg(test)]
mod tests;

/// Lift leading system/developer into `instructions`, park extras at the
/// input suffix, force `store: false`, and drop public-API-only fields.
#[must_use]
pub fn normalize_request(body: Value) -> Value {
    let Value::Object(map) = body else {
        return body;
    };
    let mut next = Value::Object(map);
    stabilize_input_prefix(&mut next);
    let Some(obj) = next.as_object_mut() else {
        return next;
    };

    if let Some(Value::Object(reasoning)) = obj.get_mut("reasoning")
        && let Some("standard" | "pro") = reasoning.get("mode").and_then(Value::as_str)
    {
        reasoning.remove("mode");
    }

    let tier = obj
        .get("service_tier")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match tier.as_deref() {
        Some("fast") => {
            obj.insert(
                "service_tier".to_owned(),
                Value::String("priority".to_owned()),
            );
        }
        Some("default" | "auto") => {
            obj.remove("service_tier");
        }
        _ => {}
    }

    // ChatGPT Codex Responses 400 unless store is false (Codex CLI sets this
    // false on every non-Azure request).
    obj.insert("store".to_owned(), Value::Bool(false));

    // gpt-5.6 rejects prompt_cache_retention / prompt_cache_options (Codex #39397).
    obj.remove("prompt_cache_options");
    obj.remove("prompt_cache_retention");
    obj.remove("safety_identifier");
    obj.remove("max_output_tokens");

    let include_ok = obj
        .get("include")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty());
    if !include_ok {
        obj.insert(
            "include".to_owned(),
            Value::Array(vec![Value::String(
                "reasoning.encrypted_content".to_owned(),
            )]),
        );
    }

    next
}

fn stabilize_input_prefix(next: &mut Value) {
    let Some(obj) = next.as_object_mut() else {
        return;
    };
    if !obj.get("input").is_some_and(Value::is_array) {
        set_default_or_trim_instructions(obj);
        return;
    }
    let input = obj
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (lifted, rest) = lift_instructions(&input);
    let existing = obj
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or_default()
        .to_owned();

    if existing.is_empty() {
        let instructions = if lifted.is_empty() {
            "You are a helpful assistant.".to_owned()
        } else {
            lifted
        };
        obj.insert("instructions".to_owned(), Value::String(instructions));
        obj.insert("input".to_owned(), Value::Array(rest));
        return;
    }

    obj.insert("instructions".to_owned(), Value::String(existing.clone()));
    if lifted.is_empty() || lifted == existing {
        obj.insert("input".to_owned(), Value::Array(rest));
        return;
    }

    let extra = if let Some(suffix) = lifted.strip_prefix(&existing) {
        suffix.trim_start_matches('\n').trim().to_owned()
    } else if existing.starts_with(&lifted) {
        String::new()
    } else {
        lifted
    };
    let mut input = rest;
    if !extra.is_empty() {
        input.push(developer_item(&extra));
    }
    obj.insert("input".to_owned(), Value::Array(input));
}

fn set_default_or_trim_instructions(obj: &mut Map<String, Value>) {
    match obj.get("instructions").and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => {
            obj.insert(
                "instructions".to_owned(),
                Value::String(text.trim().to_owned()),
            );
        }
        _ => {
            obj.insert(
                "instructions".to_owned(),
                Value::String("You are a helpful assistant.".to_owned()),
            );
        }
    }
}

fn lift_instructions(input: &[Value]) -> (String, Vec<Value>) {
    let mut lifted = Vec::new();
    let mut rest = Vec::new();
    for item in input {
        if rest.is_empty()
            && item
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(is_instruction_role)
        {
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

fn is_instruction_role(role: &str) -> bool {
    role == "system" || role == "developer"
}

fn instruction_text(item: &Value) -> String {
    match item.get("content") {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|part| match part {
                Value::String(text) => text.as_str(),
                Value::Object(map) => map.get("text").and_then(Value::as_str).unwrap_or(""),
                _ => "",
            })
            .collect::<String>()
            .trim()
            .to_owned(),
        _ => String::new(),
    }
}

fn developer_item(text: &str) -> Value {
    Value::Object(Map::from_iter([
        ("role".to_owned(), Value::String("developer".to_owned())),
        (
            "content".to_owned(),
            Value::Array(vec![Value::Object(Map::from_iter([
                ("type".to_owned(), Value::String("input_text".to_owned())),
                ("text".to_owned(), Value::String(text.to_owned())),
            ]))]),
        ),
    ]))
}

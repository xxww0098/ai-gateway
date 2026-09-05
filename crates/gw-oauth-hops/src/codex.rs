//! ChatGPT Codex hop. Identity matches Codex CLI; cache is `session-id` /
//! `thread-id` / `x-client-request-id` all equal to `prompt_cache_key`.
//!
//! chatgpt.com 400s on inbound `session_id`. This hop copies it onto the
//! cache key then strips it. No Grok / Kiro headers.

use http::HeaderMap;
use serde_json::{Value, json};

use crate::id::{first_cache_id, sanitize_cache_id};
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, insert_owned, insert_static, parse_object, string_field,
};

/// Codex CLI originator. Token endpoint and Responses both expect this pair
/// with [`CODEX_USER_AGENT`].
pub const CODEX_ORIGINATOR: &str = "codex_cli_rs";
/// Pinned Codex CLI version the hop fingerprints as.
pub const CODEX_CLIENT_VERSION: &str = "0.153.4";
/// `originator/version`.
pub const CODEX_USER_AGENT: &str = "codex_cli_rs/0.153.4";
/// Experimental Responses beta the CLI sends.
pub const CODEX_OPENAI_BETA: &str = "responses=experimental";
/// chatgpt.com 400s without a top-level `instructions` string.
const CODEX_DEFAULT_INSTRUCTIONS: &str = "You are a helpful assistant.";

/// Plan Codex identity + cache headers and shape the Responses body.
#[must_use]
pub fn plan(input: &HopInput<'_>) -> HopRewrite {
    let original = parse_object(input.body);
    let mut next = original.clone();

    let cache_session_id = first_cache_id([
        string_field(&next, "prompt_cache_key"),
        string_field(&next, "session_id"),
    ]);

    stabilize_instructions(&mut next);
    apply_wire_rules(&mut next);

    match &cache_session_id {
        Some(id) => next["prompt_cache_key"] = Value::String(id.clone()),
        None => {
            if let Value::Object(map) = &mut next {
                map.remove("prompt_cache_key");
            }
        }
    }
    if let Value::Object(map) = &mut next {
        map.remove("session_id");
    }

    let mut headers = HeaderMap::new();
    insert_static(&mut headers, "originator", CODEX_ORIGINATOR);
    insert_static(&mut headers, "user-agent", CODEX_USER_AGENT);
    insert_static(&mut headers, "openai-version", CODEX_CLIENT_VERSION);
    insert_static(&mut headers, "openai-beta", CODEX_OPENAI_BETA);

    if let Some(account) = input.account_id.and_then(sanitize_cache_id) {
        insert_owned(&mut headers, "chatgpt-account-id", &account);
    }
    if let Some(id) = &cache_session_id {
        insert_owned(&mut headers, "session-id", id);
        insert_owned(&mut headers, "thread-id", id);
        insert_owned(&mut headers, "x-client-request-id", id);
    }

    let model = input
        .model
        .and_then(sanitize_cache_id)
        .or_else(|| string_field(&original, "model").and_then(sanitize_cache_id));
    if let Some(model) = model {
        let tier = input
            .service_tier
            .and_then(sanitize_cache_id)
            .or_else(|| string_field(&next, "service_tier").and_then(sanitize_cache_id));
        let hint = match tier {
            Some(tier) => format!("model={model};tier={tier}"),
            None => format!("model={model}"),
        };
        insert_owned(&mut headers, "x-codex-routing-hint", &hint);
    }

    debug_assert!(
        !headers.contains_key(http::header::AUTHORIZATION),
        "hop headers must not carry a credential"
    );

    HopRewrite {
        headers,
        body: body_if_changed(&original, next),
        cache_session_id,
    }
}

fn is_instruction_role(item: &Value) -> bool {
    matches!(
        item.get("role").and_then(Value::as_str),
        Some("system" | "developer")
    )
}

fn part_text(part: &Value) -> String {
    match part {
        Value::String(text) => text.clone(),
        Value::Object(_) => part
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

fn instruction_text(item: &Value) -> String {
    match item.get("content") {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(part_text)
            .collect::<String>()
            .trim()
            .to_owned(),
        _ => String::new(),
    }
}

fn lift_instructions(input: &[Value]) -> (String, Vec<Value>) {
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

fn developer_item(text: &str) -> Value {
    json!({
        "role": "developer",
        "content": [{ "type": "input_text", "text": text }]
    })
}

fn stabilize_instructions(next: &mut Value) {
    let Some(input) = next.get("input").cloned() else {
        let existing = next
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        next["instructions"] = Value::String(if existing.is_empty() {
            CODEX_DEFAULT_INSTRUCTIONS.to_owned()
        } else {
            existing.to_owned()
        });
        return;
    };
    let Value::Array(items) = input else {
        let existing = next
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        next["instructions"] = Value::String(if existing.is_empty() {
            CODEX_DEFAULT_INSTRUCTIONS.to_owned()
        } else {
            existing.to_owned()
        });
        return;
    };

    let (lifted, rest) = lift_instructions(&items);
    let existing = next
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_owned();

    if existing.is_empty() {
        next["instructions"] = Value::String(if lifted.is_empty() {
            CODEX_DEFAULT_INSTRUCTIONS.to_owned()
        } else {
            lifted
        });
        next["input"] = Value::Array(rest);
        return;
    }

    next["instructions"] = Value::String(existing.clone());
    if lifted.is_empty() || lifted == existing {
        next["input"] = Value::Array(rest);
        return;
    }

    let extra = if lifted.starts_with(&existing) {
        lifted[existing.len()..]
            .trim_start_matches('\n')
            .trim()
            .to_owned()
    } else if existing.starts_with(&lifted) {
        String::new()
    } else {
        lifted
    };
    let mut input = rest;
    if !extra.is_empty() {
        input.push(developer_item(&extra));
    }
    next["input"] = Value::Array(input);
}

fn apply_wire_rules(next: &mut Value) {
    if let Some(Value::Object(map)) = next.get_mut("reasoning")
        && matches!(
            map.get("mode").and_then(Value::as_str),
            Some("standard" | "pro")
        )
    {
        map.remove("mode");
    }

    match next.get("service_tier").and_then(Value::as_str) {
        Some("fast") => next["service_tier"] = Value::String("priority".into()),
        Some("default" | "auto") => {
            if let Value::Object(map) = next {
                map.remove("service_tier");
            }
        }
        _ => {}
    }

    next["store"] = Value::Bool(false);
    if let Value::Object(map) = next {
        map.remove("prompt_cache_options");
        map.remove("prompt_cache_retention");
        map.remove("safety_identifier");
        map.remove("max_output_tokens");
    }

    let include_empty = match next.get("include") {
        Some(Value::Array(items)) => items.is_empty(),
        Some(_) => true,
        None => true,
    };
    if include_empty {
        next["include"] = json!(["reasoning.encrypted_content"]);
    }
}

#[cfg(test)]
mod tests;

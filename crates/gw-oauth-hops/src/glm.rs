//! Z.AI / BigModel Coding Plan hop. Sticky routing is OpenAI `user` /
//! Anthropic `metadata.user_id` plus `x-session-id`. No Codex
//! `prompt_cache_key`, no Grok `x-grok-conv-id`. Per-attempt trace ids
//! are caller-owned (this crate does not mint them).

use http::HeaderMap;
use serde_json::{Value, json};

use crate::id::first_cache_id;
use crate::pin::PrefixPins;
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, insert_owned, insert_static, park_leading_system,
    parse_object, remove_keys, string_field,
};

/// Analyzer + header fallback. Never a timestamp.
pub const GLM_STABLE_SESSION: &str = "dsh-glm";
/// ZCode Desktop 3.10.1 UA (`eao`/`rao`).
pub const GLM_USER_AGENT: &str = "ZCode/3.10.1 ai-sdk/anthropic/3.0.81";
/// `X-ZCode-App-Version`.
pub const GLM_APP_VERSION: &str = "3.10.1";
/// `X-ZCode-Agent`.
pub const GLM_AGENT: &str = "glm";
/// Desktop referer.
pub const GLM_REFERER: &str = "https://zcode.z.ai";
/// `X-Title`.
pub const GLM_TITLE: &str = "Z Code";
/// Anthropic Messages version ZCode sends.
pub const GLM_ANTHROPIC_VERSION: &str = "2023-06-01";

/// Conversation id: `user` / session / cache key / stable.
#[must_use]
pub fn conversation_id(body: &Value, explicit: Option<&str>) -> String {
    first_cache_id([
        explicit,
        string_field(body, "user"),
        string_field(body, "session_id"),
        string_field(body, "prompt_cache_key"),
        body.get("metadata")
            .and_then(|meta| string_field(meta, "user_id")),
    ])
    .unwrap_or_else(|| GLM_STABLE_SESSION.to_owned())
}

fn forced_thinking(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    id == "glm-5.3" || id.starts_with("glm-5.3-")
}

fn apply_thinking(next: &mut Value) {
    let model = string_field(next, "model").unwrap_or("");
    let forced = forced_thinking(model);
    let current = next.get("thinking").cloned();
    let disabled = current
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        == Some("disabled");
    if forced || (current.is_some() && !disabled) {
        let mut thinking = match current {
            Some(Value::Object(map)) => Value::Object(map),
            _ => json!({}),
        };
        if let Value::Object(map) = &mut thinking {
            if forced {
                map.insert("type".into(), json!("enabled"));
            }
            map.insert("clear_thinking".into(), json!(false));
        }
        next["thinking"] = thinking;
    }
}

fn identity_headers(session_id: &str, request_id: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    insert_static(&mut headers, "user-agent", GLM_USER_AGENT);
    insert_static(&mut headers, "x-zcode-app-version", GLM_APP_VERSION);
    insert_static(&mut headers, "x-zcode-agent", GLM_AGENT);
    insert_static(&mut headers, "http-referer", GLM_REFERER);
    insert_static(&mut headers, "referer", GLM_REFERER);
    insert_static(&mut headers, "x-title", GLM_TITLE);
    insert_owned(&mut headers, "x-session-id", session_id);
    if let Some(id) = request_id.map(str::trim).filter(|id| !id.is_empty()) {
        insert_owned(&mut headers, "x-request-id", id);
    }
    headers
}

/// Completions hop (`/api/coding/paas/v4/chat/completions`).
#[must_use]
pub fn plan(input: &HopInput<'_>, pins: Option<&mut PrefixPins>) -> HopRewrite {
    let original = parse_object(input.body);
    let mut next = original.clone();
    let cache_session_id = conversation_id(&next, input.session_id);
    remove_keys(
        &mut next,
        &[
            "prompt_cache_key",
            "prompt_cache_retention",
            "prompt_cache_options",
        ],
    );
    if let Some(messages) = next.get("messages").and_then(Value::as_array).cloned() {
        next["messages"] = Value::Array(park_leading_system(
            &messages,
            &cache_session_id,
            pins,
            &[GLM_STABLE_SESSION],
        ));
    }
    if string_field(&next, "user").is_none() {
        next["user"] = Value::String(cache_session_id.clone());
    }
    apply_thinking(&mut next);

    let headers = identity_headers(&cache_session_id, input.request_id);
    debug_assert!(!headers.contains_key(http::header::AUTHORIZATION));
    debug_assert!(!headers.contains_key("session-id"));

    HopRewrite {
        headers,
        body: body_if_changed(&original, next),
        cache_session_id: Some(cache_session_id),
    }
}

fn with_cache_control(blocks: Vec<Value>) -> Vec<Value> {
    blocks
        .into_iter()
        .enumerate()
        .map(|(index, mut block)| {
            if index == 0
                && let Value::Object(map) = &mut block
            {
                map.insert("cache_control".into(), json!({ "type": "ephemeral" }));
            }
            block
        })
        .collect()
}

fn system_blocks(system: &Value) -> Vec<Value> {
    match system {
        Value::String(text) if !text.trim().is_empty() => {
            vec![json!({ "type": "text", "text": text.trim() })]
        }
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) if !text.trim().is_empty() => {
                    Some(json!({ "type": "text", "text": text.trim() }))
                }
                Value::Object(_) => part
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(|text| json!({ "type": "text", "text": text })),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Anthropic Messages hop. First system block gets `cache_control`.
#[must_use]
pub fn plan_anthropic(input: &HopInput<'_>, pins: Option<&mut PrefixPins>) -> HopRewrite {
    let original = parse_object(input.body);
    let mut next = original.clone();
    let cache_session_id = conversation_id(&next, input.session_id);
    remove_keys(
        &mut next,
        &[
            "prompt_cache_key",
            "prompt_cache_retention",
            "prompt_cache_options",
        ],
    );

    if let Some(system) = next.get("system").cloned() {
        let blocks = system_blocks(&system);
        let text = blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut extra = String::new();
        let mut pinned = text.clone();
        if let Some(pins) = pins {
            let pin_id = format!("{cache_session_id}\0anthropic");
            let result = pins.pin(&pin_id, &text, &[GLM_STABLE_SESSION]);
            pinned = result.pinned;
            extra = result.extra;
            if result.fresh {
                next["system"] = Value::Array(with_cache_control(blocks));
            } else {
                let mut restored = vec![json!({ "type": "text", "text": pinned })];
                if !extra.is_empty() {
                    restored.push(json!({ "type": "text", "text": extra }));
                }
                next["system"] = Value::Array(with_cache_control(restored));
            }
        } else if !blocks.is_empty() {
            next["system"] = Value::Array(with_cache_control(blocks));
        }
        let _ = (pinned, extra);
    }

    let max_ok = next
        .get("max_tokens")
        .and_then(Value::as_u64)
        .is_some_and(|n| n > 0);
    if !max_ok {
        next["max_tokens"] = json!(128_000);
    }

    let mut metadata = match next.get("metadata") {
        Some(Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    if metadata
        .get("user_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        metadata.insert("user_id".into(), Value::String(cache_session_id.clone()));
    }
    next["metadata"] = Value::Object(metadata);
    apply_thinking(&mut next);

    let mut headers = identity_headers(&cache_session_id, input.request_id);
    insert_static(&mut headers, "anthropic-version", GLM_ANTHROPIC_VERSION);

    HopRewrite {
        headers,
        body: body_if_changed(&original, next),
        cache_session_id: Some(cache_session_id),
    }
}

#[cfg(test)]
mod tests;

//! Common hop input / output. Families fill this; they do not share helpers.

use bytes::Bytes;
use http::HeaderMap;
use serde_json::{Value, json};

use crate::pin::PrefixPins;

/// What a planner knows about one inbound request, without a socket.
#[derive(Debug, Clone, Default)]
pub struct HopInput<'a> {
    /// Raw inbound JSON (OpenAI-shaped or already vendor-shaped).
    pub body: &'a [u8],
    /// ChatGPT account id (Codex). Absent → that header is omitted.
    pub account_id: Option<&'a str>,
    /// Grok JWT `sub` (optional). Absent → `x-userid` is omitted.
    pub user_id: Option<&'a str>,
    /// Model slug for routing hints / conversation suffix.
    pub model: Option<&'a str>,
    /// Codex `service_tier` (`fast` / `priority`), if the body asked for one.
    pub service_tier: Option<&'a str>,
    /// Per-attempt id (`x-grok-req-id`, Cursor `x-request-id`). Caller-owned.
    pub request_id: Option<&'a str>,
    /// Retry count. Grok stamps `x-grok-transient-retry` when > 0.
    pub retry_attempt: u32,
    /// Explicit conversation id, outranking body `session_id`.
    pub session_id: Option<&'a str>,
    /// Kiro `profileArn`. Absent → omitted from the CodeWhisperer body.
    pub profile_arn: Option<&'a str>,
    /// Antigravity Cloud Code `project`. Missing → generateContent is not built.
    pub project_id: Option<&'a str>,
}

/// Planned hop: identity + cache headers, optional rewritten body.
///
/// `headers` never carries `Authorization`. `body` is `None` when the JSON
/// value did not change, so the relay can forward the inbound `Bytes` by
/// refcount.
#[derive(Debug, Clone)]
pub struct HopRewrite {
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
    pub cache_session_id: Option<String>,
}

impl HopRewrite {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            headers: HeaderMap::new(),
            body: None,
            cache_session_id: None,
        }
    }
}

/// Parse inbound JSON. Non-JSON becomes an empty object so header-only hops
/// still run.
#[must_use]
pub fn parse_object(body: &[u8]) -> Value {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => Value::Object(serde_json::Map::new()),
    }
}

/// String field if it is a non-empty string.
#[must_use]
pub fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Serialize `next` only when it differs from `original` as JSON values.
#[must_use]
pub fn body_if_changed(original: &Value, next: Value) -> Option<Bytes> {
    if original == &next {
        None
    } else {
        Some(Bytes::from(serde_json::to_vec(&next).unwrap_or_default()))
    }
}

pub(crate) fn insert_static(headers: &mut HeaderMap, name: &'static str, value: &'static str) {
    headers.insert(name, http::HeaderValue::from_static(value));
}

pub(crate) fn insert_owned(headers: &mut HeaderMap, name: &'static str, value: &str) {
    let Ok(value) = http::HeaderValue::from_str(value) else {
        return;
    };
    headers.insert(name, value);
}

pub(crate) fn remove_keys(value: &mut Value, keys: &[&str]) {
    if let Value::Object(map) = value {
        for key in keys {
            map.remove(*key);
        }
    }
}

pub(crate) fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|part| match part {
                Value::String(text) => text.clone(),
                Value::Object(_) => part
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                _ => String::new(),
            })
            .collect(),
        _ => String::new(),
    }
}

/// Pin the leading `system` run. Fresh pins keep the original objects;
/// later DSH snapshots park as a trailing system message.
pub(crate) fn park_leading_system(
    messages: &[Value],
    conversation_id: &str,
    pins: Option<&mut PrefixPins>,
    skip_ids: &[&str],
) -> Vec<Value> {
    let mut head_len = 0;
    while head_len < messages.len()
        && messages[head_len].get("role").and_then(Value::as_str) == Some("system")
    {
        head_len += 1;
    }
    if head_len == 0 {
        return messages.to_vec();
    }
    let Some(pins) = pins else {
        return messages.to_vec();
    };
    let text = messages[..head_len]
        .iter()
        .map(message_text)
        .filter(|chunk| !chunk.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let result = pins.pin(conversation_id, &text, skip_ids);
    if result.fresh {
        return messages.to_vec();
    }
    let mut out = Vec::with_capacity(messages.len() - head_len + 2);
    out.push(json!({ "role": "system", "content": result.pinned }));
    out.extend(messages[head_len..].iter().cloned());
    if !result.extra.is_empty() {
        out.push(json!({ "role": "system", "content": result.extra }));
    }
    out
}

//! Common hop input / output. Families fill this; they do not share helpers.

use bytes::Bytes;
use http::HeaderMap;
use serde_json::Value;

/// What a planner knows about one inbound request, without a socket.
#[derive(Debug, Clone, Default)]
pub struct HopInput<'a> {
    /// Raw inbound JSON (OpenAI-shaped or already vendor-shaped).
    pub body: &'a [u8],
    /// ChatGPT account id (Codex). Absent → that header is omitted.
    pub account_id: Option<&'a str>,
    /// Grok JWT `sub` (optional). Absent → `x-userid` is omitted.
    pub user_id: Option<&'a str>,
    /// Model slug for routing hints / Kiro conversation suffix.
    pub model: Option<&'a str>,
    /// Codex `service_tier` (`fast` / `priority`), if the body asked for one.
    pub service_tier: Option<&'a str>,
    /// Grok `x-grok-req-id`. Missing → a fresh UUID (one per planned attempt).
    pub request_id: Option<&'a str>,
    /// Retry count. Grok stamps `x-grok-transient-retry` when > 0.
    pub retry_attempt: u32,
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

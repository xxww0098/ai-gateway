//! GitHub Copilot prefix-hash + `X-Interaction-Id`. Do not import this from another family.

use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use crate::CacheRewrite;

#[cfg(test)]
mod tests;

/// Fallback **is** written as `X-Interaction-Id` (official always sends a session id).
pub const COPILOT_STABLE_SESSION: &str = "dsh-copilot";

/// Sanitize a Copilot interaction id.
#[must_use]
pub fn cache_session_id(key: Option<&str>) -> Option<String> {
    let key = key?;
    let cleaned: String = key
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.chars().take(64).collect())
    }
}

/// Strip Codex/Grok fields. Sticky id is a header, not a body field.
#[must_use]
pub fn apply_cache(payload: Value) -> CacheRewrite {
    let pin = payload
        .get("session_id")
        .and_then(Value::as_str)
        .and_then(|s| cache_session_id(Some(s)))
        .or_else(|| {
            payload
                .get("prompt_cache_key")
                .and_then(Value::as_str)
                .and_then(|s| cache_session_id(Some(s)))
        })
        .unwrap_or_else(|| COPILOT_STABLE_SESSION.to_owned());
    let mut next = payload;
    if let Some(obj) = next.as_object_mut() {
        obj.remove("prompt_cache_key");
        obj.remove("prompt_cache_retention");
        obj.remove("prompt_cache_options");
        obj.remove("session_id");
    }
    CacheRewrite {
        payload: next,
        cache_session_id: Some(pin),
    }
}

/// `X-Interaction-Id` only. Do not invent `X-Interaction-Type: agent-session-name-generation`.
#[must_use]
pub fn cache_headers(cache_session_id: Option<&str>) -> HeaderMap {
    let session = cache_session_id
        .and_then(|s| self::cache_session_id(Some(s)))
        .unwrap_or_else(|| COPILOT_STABLE_SESSION.to_owned());
    let Ok(value) = HeaderValue::from_str(&session) else {
        return HeaderMap::new();
    };
    let mut headers = HeaderMap::new();
    if let Ok(name) = HeaderName::from_bytes(b"x-interaction-id") {
        headers.insert(name, value);
    }
    headers
}

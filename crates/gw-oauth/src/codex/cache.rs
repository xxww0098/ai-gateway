//! ChatGPT Codex prompt cache. Do not import this from another family.

use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use crate::CacheRewrite;

#[cfg(test)]
mod tests;

/// Sanitize a Codex cache id. Empty / non-string → `None`.
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

/// Copy `prompt_cache_key` / `session_id` onto the pin, then drop `session_id`
/// (chatgpt.com 400s on that field).
#[must_use]
pub fn apply_cache(payload: Value) -> CacheRewrite {
    let mut next = payload;
    let pin = next
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .and_then(|s| cache_session_id(Some(s)))
        .or_else(|| {
            next.get("session_id")
                .and_then(Value::as_str)
                .and_then(|s| cache_session_id(Some(s)))
        });
    if let Some(obj) = next.as_object_mut() {
        match &pin {
            Some(id) => {
                obj.insert("prompt_cache_key".to_owned(), Value::String(id.clone()));
            }
            None => {
                obj.remove("prompt_cache_key");
            }
        }
        obj.remove("session_id");
    }
    CacheRewrite {
        payload: next,
        cache_session_id: pin,
    }
}

/// `session-id` = `thread-id` = `x-client-request-id` = the pin.
#[must_use]
pub fn cache_headers(cache_session_id: Option<&str>) -> HeaderMap {
    let Some(id) = cache_session_id.filter(|s| !s.is_empty()) else {
        return HeaderMap::new();
    };
    let Ok(value) = HeaderValue::from_str(id) else {
        return HeaderMap::new();
    };
    let mut headers = HeaderMap::new();
    for name in ["session-id", "thread-id", "x-client-request-id"] {
        if let Ok(header) = HeaderName::from_bytes(name.as_bytes()) {
            headers.insert(header, value.clone());
        }
    }
    headers
}

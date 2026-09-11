//! Z.AI Coding Plan implicit prefix cache. Do not import this from another family.

use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value};

use crate::CacheRewrite;

#[cfg(test)]
mod tests;

/// Sanitize a GLM session id.
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

fn strip_codex_fields(obj: &mut Map<String, Value>) {
    obj.remove("prompt_cache_key");
    obj.remove("prompt_cache_retention");
    obj.remove("prompt_cache_options");
}

fn pin_of(payload: &Value) -> Option<String> {
    payload
        .get("metadata")
        .and_then(|m| m.get("user_id"))
        .and_then(Value::as_str)
        .and_then(|s| cache_session_id(Some(s)))
        .or_else(|| {
            payload
                .get("session_id")
                .and_then(Value::as_str)
                .and_then(|s| cache_session_id(Some(s)))
        })
        .or_else(|| {
            payload
                .get("prompt_cache_key")
                .and_then(Value::as_str)
                .and_then(|s| cache_session_id(Some(s)))
        })
        .or_else(|| {
            payload
                .get("user")
                .and_then(Value::as_str)
                .and_then(|s| cache_session_id(Some(s)))
        })
}

/// Completions leftover: drop Codex fields, pin `user`.
#[must_use]
pub fn apply_cache(payload: Value) -> CacheRewrite {
    let pin = pin_of(&payload);
    let mut next = payload;
    if let Some(obj) = next.as_object_mut() {
        strip_codex_fields(obj);
        if let Some(id) = &pin
            && (obj.get("user").is_none()
                || obj.get("user") == Some(&Value::Null)
                || obj.get("user") == Some(&Value::String(String::new())))
        {
            obj.insert("user".to_owned(), Value::String(id.clone()));
        }
        obj.remove("session_id");
    }
    CacheRewrite {
        payload: next,
        cache_session_id: pin,
    }
}

/// Anthropic default hop: `metadata.user_id` + drop Codex fields.
#[must_use]
pub fn apply_anthropic_cache(payload: Value) -> CacheRewrite {
    let pin = pin_of(&payload);
    let mut next = payload;
    if let Some(obj) = next.as_object_mut() {
        strip_codex_fields(obj);
        obj.remove("session_id");
        if let Some(id) = &pin {
            let mut metadata = match obj.remove("metadata") {
                Some(Value::Object(map)) => map,
                _ => Map::new(),
            };
            if metadata
                .get("user_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
            {
                metadata.insert("user_id".to_owned(), Value::String(id.clone()));
            }
            obj.insert("metadata".to_owned(), Value::Object(metadata));
        }
    }
    CacheRewrite {
        payload: next,
        cache_session_id: pin,
    }
}

/// `x-session-id` only — never Codex `session-id` or Grok conv headers.
#[must_use]
pub fn session_headers(cache_session_id: Option<&str>) -> HeaderMap {
    let Some(id) = cache_session_id.filter(|s| !s.is_empty()) else {
        return HeaderMap::new();
    };
    let Ok(value) = HeaderValue::from_str(id) else {
        return HeaderMap::new();
    };
    let mut headers = HeaderMap::new();
    if let Ok(name) = HeaderName::from_bytes(b"x-session-id") {
        headers.insert(name, value);
    }
    headers
}

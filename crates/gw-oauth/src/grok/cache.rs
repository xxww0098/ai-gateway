//! xAI Grok prompt cache. Do not import this from another family.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use crate::CacheRewrite;
use crate::rewrite::HeaderExtra;

#[cfg(test)]
mod tests;

/// Fallback pin when the caller sent neither `session_id` nor `prompt_cache_key`.
pub const GROK_STABLE_SESSION: &str = "dsh-grok";

const PIN_CAP: usize = 64;
static SYSTEM_PINS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn pins() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    SYSTEM_PINS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Sanitize a Grok cache id.
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

/// Drop in-process prefix pins (tests).
pub fn reset_system_pins() {
    pins().clear();
}

/// Pin the first leading system/developer blob per conversation.
#[must_use]
pub fn pin_system_prefix(conversation_id: &str, system_text: &str) -> (String, String) {
    if system_text.is_empty() {
        return (String::new(), String::new());
    }
    if conversation_id.is_empty() || conversation_id == GROK_STABLE_SESSION {
        return (system_text.to_owned(), String::new());
    }
    let mut pins = pins();
    if let Some(existing) = pins.get(conversation_id) {
        if existing == system_text || existing.starts_with(system_text) {
            return (existing.clone(), String::new());
        }
        let extra = if system_text.starts_with(existing.as_str()) {
            system_text[existing.len()..]
                .trim_start_matches('\n')
                .trim()
                .to_owned()
        } else {
            system_text.to_owned()
        };
        return (existing.clone(), extra);
    }
    if pins.len() >= PIN_CAP
        && let Some(first) = pins.keys().next().cloned()
    {
        pins.remove(&first);
    }
    pins.insert(conversation_id.to_owned(), system_text.to_owned());
    (system_text.to_owned(), String::new())
}

/// Conversation id: `prompt_cache_key` then `session_id` then [`GROK_STABLE_SESSION`].
#[must_use]
pub fn conversation_id(payload: &Value) -> String {
    payload
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .and_then(|s| cache_session_id(Some(s)))
        .or_else(|| {
            payload
                .get("session_id")
                .and_then(Value::as_str)
                .and_then(|s| cache_session_id(Some(s)))
        })
        .unwrap_or_else(|| GROK_STABLE_SESSION.to_owned())
}

/// grok-build `GrokRequestHeaders`. Never copies Codex `session-id`.
#[must_use]
pub fn affinity_headers(cache_session_id: Option<&str>, extra: &HeaderExtra) -> HeaderMap {
    let Some(id) = cache_session_id.filter(|s| !s.is_empty()) else {
        return HeaderMap::new();
    };
    let Ok(id_value) = HeaderValue::from_str(id) else {
        return HeaderMap::new();
    };
    let mut headers = HeaderMap::new();
    let insert = |headers: &mut HeaderMap, name: &str, value: HeaderValue| {
        if let Ok(header) = HeaderName::from_bytes(name.as_bytes()) {
            headers.insert(header, value);
        }
    };
    insert(&mut headers, "x-grok-conv-id", id_value.clone());
    insert(&mut headers, "x-grok-session-id", id_value);
    let req_id = extra
        .req_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned);
    if let Ok(value) = HeaderValue::from_str(&req_id) {
        insert(&mut headers, "x-grok-req-id", value);
    }
    if let Some(model) = extra
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Ok(value) = HeaderValue::from_str(model)
    {
        insert(&mut headers, "x-grok-model-override", value);
    }
    if extra.retry_attempt > 0
        && let Ok(value) = HeaderValue::from_str(&extra.retry_attempt.to_string())
    {
        insert(&mut headers, "x-grok-transient-retry", value);
    }
    headers
}

/// Stamp `prompt_cache_key`, drop DSH `session_id` and retention fields.
#[must_use]
pub fn apply_cache(payload: Value) -> CacheRewrite {
    let mut next = payload;
    let pin = conversation_id(&next);
    if let Some(obj) = next.as_object_mut() {
        obj.insert("prompt_cache_key".to_owned(), Value::String(pin.clone()));
        obj.remove("session_id");
        obj.remove("prompt_cache_retention");
        obj.remove("prompt_cache_options");
    }
    CacheRewrite {
        payload: next,
        cache_session_id: Some(pin),
    }
}

//! Google Antigravity implicit prompt cache. Do not import this from another family.

use serde_json::Value;

use crate::CacheRewrite;

#[cfg(test)]
mod tests;

/// Bare fallback. Not written into the pin map; chat uses `dsh-antigravity:<model>`.
pub const ANTIGRAVITY_STABLE_SESSION: &str = "dsh-antigravity";

/// Sanitize an Antigravity session id.
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

fn append_model(base: String, model_id: Option<&str>) -> String {
    if base != ANTIGRAVITY_STABLE_SESSION {
        return base;
    }
    let Some(model) = cache_session_id(model_id) else {
        return base;
    };
    let room = 64usize.saturating_sub(1 + model.len());
    if room < 1 {
        return model.chars().take(64).collect();
    }
    let prefix: String = base.chars().take(room).collect();
    format!("{prefix}:{model}")
}

/// Sticky `request.sessionId`. Fallback is `dsh-antigravity:<model>`.
#[must_use]
pub fn session_id_of(payload: &Value, explicit: Option<&str>) -> String {
    let base = cache_session_id(explicit)
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
        .unwrap_or_else(|| ANTIGRAVITY_STABLE_SESSION.to_owned());
    append_model(base, payload.get("model").and_then(Value::as_str))
}

/// Strip Codex/Grok fields. Session id is computed for `request.sessionId` (request.rs).
#[must_use]
pub fn apply_cache(payload: Value, explicit: Option<&str>) -> CacheRewrite {
    let pin = session_id_of(&payload, explicit);
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

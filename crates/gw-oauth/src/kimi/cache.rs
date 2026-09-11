//! Kimi Code prefix-hash cache. Do not import this from another family.

use serde_json::Value;

use crate::CacheRewrite;

#[cfg(test)]
mod tests;

/// Analyzer-only fallback. **Not** written upstream.
pub const KIMI_STABLE_SESSION: &str = "dsh-kimi";

/// Sanitize a Kimi analyzer id.
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

/// Strip Codex/Grok fields. Extra system parking is request.rs.
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
        .unwrap_or_else(|| KIMI_STABLE_SESSION.to_owned());
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

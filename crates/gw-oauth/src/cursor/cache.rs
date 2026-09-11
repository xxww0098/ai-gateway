//! Cursor Agent conversation cache. Do not import this from another family.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::CacheRewrite;

#[cfg(test)]
mod tests;

/// Bare fallback. Chat uses `dsh-cursor:<model>`.
pub const CURSOR_STABLE_SESSION: &str = "dsh-cursor";
const FAST_SUFFIX: &str = "-fast";
const PIN_CAP: usize = 64;

static SYSTEM_PINS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn pins() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    SYSTEM_PINS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Sanitize a Cursor conversation id fragment.
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

/// Host-side Fast picker suffix. Not Codex `service_tier`.
#[must_use]
pub fn peel_fast_suffix(model_id: &str) -> (String, bool) {
    let raw = model_id.trim();
    let lower = raw.to_ascii_lowercase();
    if !lower.ends_with(FAST_SUFFIX) {
        return (raw.to_owned(), false);
    }
    let peeled = &raw[..raw.len() - FAST_SUFFIX.len()];
    if peeled.is_empty() {
        (raw.to_owned(), false)
    } else {
        (peeled.to_owned(), true)
    }
}

/// Deterministic UUID for conversation blobs. Same seed → same id.
#[must_use]
pub fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update([0]);
        }
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn append_model(base: String, model_id: Option<&str>) -> String {
    let model = model_id
        .map(peel_fast_suffix)
        .and_then(|(id, _)| cache_session_id(Some(&id)));
    let Some(model) = model else {
        return base;
    };
    if base == model || base.ends_with(&format!(":{model}")) {
        return base;
    }
    let room = 64usize.saturating_sub(1 + model.len());
    if room < 1 {
        return model.chars().take(64).collect();
    }
    let prefix: String = base.chars().take(room).collect();
    format!("{prefix}:{model}")
}

/// Sticky `conversation_id` = pin plus family model (Fast suffix peeled).
#[must_use]
pub fn conversation_id(payload: &Value) -> String {
    conversation_id_with(payload, None)
}

/// Same as [`conversation_id`], with an already-chosen pin taking precedence.
#[must_use]
pub fn conversation_id_with(payload: &Value, explicit: Option<&str>) -> String {
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
        .unwrap_or_else(|| CURSOR_STABLE_SESSION.to_owned());
    append_model(base, payload.get("model").and_then(Value::as_str))
}

/// Drop in-process prefix pins (tests).
pub fn reset_system_pins() {
    pins().clear();
}

/// Pin the first system blob per conversation; later snapshots become `extra`.
#[must_use]
pub fn pin_system_prefix(conversation_id: &str, system_text: &str) -> (String, String) {
    if system_text.is_empty() {
        return (String::new(), String::new());
    }
    if conversation_id.is_empty() || conversation_id == CURSOR_STABLE_SESSION {
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

/// Strip Codex/Grok fields and `service_tier`.
#[must_use]
pub fn apply_cache(payload: Value) -> CacheRewrite {
    let pin = conversation_id(&payload);
    let mut next = payload;
    if let Some(obj) = next.as_object_mut() {
        obj.remove("prompt_cache_retention");
        obj.remove("prompt_cache_options");
        obj.remove("prompt_cache_key");
        obj.remove("service_tier");
        obj.remove("session_id");
    }
    CacheRewrite {
        payload: next,
        cache_session_id: Some(pin),
    }
}

//! AWS Kiro / CodeWhisperer conversation cache. Do not import this from another family.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use crate::CacheRewrite;

#[cfg(test)]
mod tests;

/// Fallback when the caller sent no pin. Bare value is not written with a model.
pub const KIRO_STABLE_SESSION: &str = "dsh-kiro";

const SYSTEM_PIN_CAP: usize = 64;

struct PinMap {
    order: VecDeque<String>,
    values: HashMap<String, String>,
}

thread_local! {
    static SYSTEM_PINS: RefCell<PinMap> = RefCell::new(PinMap {
        order: VecDeque::new(),
        values: HashMap::new(),
    });
}

/// First system blob for this conversation, plus any later DSH snapshot extra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPin {
    pub pinned: String,
    pub extra: String,
}

/// Drop this thread's system pins. Tests call this so snapshots do not leak.
pub fn reset_system_pins() {
    SYSTEM_PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        pins.order.clear();
        pins.values.clear();
    });
}

/// Pin the first system blob per conversationId. Extra / changed snapshots
/// are parked after history (user+ack), never on currentMessage.
#[must_use]
pub fn pin_system_prefix(conversation_id: &str, system_text: &str) -> SystemPin {
    let text = system_text.to_owned();
    if text.is_empty() {
        return SystemPin {
            pinned: String::new(),
            extra: String::new(),
        };
    }
    if conversation_id.is_empty() || conversation_id == KIRO_STABLE_SESSION {
        return SystemPin {
            pinned: text,
            extra: String::new(),
        };
    }
    SYSTEM_PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        if let Some(existing) = pins.values.get(conversation_id).cloned() {
            if existing == text || existing.starts_with(&text) {
                return SystemPin {
                    pinned: existing,
                    extra: String::new(),
                };
            }
            let extra = if let Some(rest) = text.strip_prefix(&existing) {
                rest.trim_start_matches('\n').trim().to_owned()
            } else {
                text
            };
            return SystemPin {
                pinned: existing,
                extra,
            };
        }
        if pins.values.len() >= SYSTEM_PIN_CAP
            && let Some(first) = pins.order.pop_front()
        {
            pins.values.remove(&first);
        }
        pins.order.push_back(conversation_id.to_owned());
        pins.values.insert(conversation_id.to_owned(), text.clone());
        SystemPin {
            pinned: text,
            extra: String::new(),
        }
    })
}
/// Sanitize a Kiro conversation id fragment.
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
    let Some(model) = cache_session_id(model_id) else {
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

/// Sticky id is pin **plus model**, never `Date.now()`.
#[must_use]
pub fn conversation_id(payload: &Value, explicit: Option<&str>) -> String {
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
        .unwrap_or_else(|| KIRO_STABLE_SESSION.to_owned());
    append_model(base, payload.get("model").and_then(Value::as_str))
}

/// Strip Codex/Grok fields. Conversation id is computed, not written as those fields.
#[must_use]
pub fn apply_cache(payload: Value, explicit: Option<&str>) -> CacheRewrite {
    let pin = conversation_id(&payload, explicit);
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

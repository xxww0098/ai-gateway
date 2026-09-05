//! Per-conversation system-prefix pins.
//!
//! The caller owns the map (one per tenant, never a process-global). The first
//! system blob for a conversation sticks; later DSH snapshots come back as
//! `extra` so the family can park them at the suffix. Stable fallback ids
//! (`dsh-grok`, `dsh-kiro`, …) skip the map — pinning a shared constant would
//! leak one tenant's prefix onto the next.

use std::collections::HashMap;

/// First-seen prefix plus any extra text that must not rewrite it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinResult {
    pub pinned: String,
    pub extra: String,
}

/// Bounded map of conversation-id → first system text.
#[derive(Debug, Clone)]
pub struct PrefixPins {
    map: HashMap<String, String>,
    cap: usize,
}

impl Default for PrefixPins {
    fn default() -> Self {
        Self::new()
    }
}

impl PrefixPins {
    /// Cap matches the DSH plugin: 64 live conversations.
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            cap: 64,
        }
    }

    /// Pin `text` under `conversation_id`, unless the id is a family-wide
    /// fallback listed in `skip_ids`.
    #[must_use]
    pub fn pin(&mut self, conversation_id: &str, text: &str, skip_ids: &[&str]) -> PinResult {
        if text.is_empty() {
            return PinResult {
                pinned: String::new(),
                extra: String::new(),
            };
        }
        if conversation_id.is_empty() || skip_ids.contains(&conversation_id) {
            return PinResult {
                pinned: text.to_owned(),
                extra: String::new(),
            };
        }
        if let Some(existing) = self.map.get(conversation_id) {
            if existing == text || existing.starts_with(text) {
                return PinResult {
                    pinned: existing.clone(),
                    extra: String::new(),
                };
            }
            let extra = if text.starts_with(existing.as_str()) {
                text[existing.len()..]
                    .trim_start_matches('\n')
                    .trim()
                    .to_owned()
            } else {
                text.to_owned()
            };
            return PinResult {
                pinned: existing.clone(),
                extra,
            };
        }
        if self.map.len() >= self.cap
            && let Some(first) = self.map.keys().next().cloned()
        {
            self.map.remove(&first);
        }
        self.map.insert(conversation_id.to_owned(), text.to_owned());
        PinResult {
            pinned: text.to_owned(),
            extra: String::new(),
        }
    }

    /// Drop every pin. Tests call this between cases.
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests;

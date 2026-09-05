//! Ollama Cloud hop. Official `/v1/chat/completions` has no documented
//! conversation / shard field. Strip Codex / Grok keys; do not invent a
//! sticky id or cache-read field. Auth is a Bearer the planner sets.

use serde_json::Value;

use crate::id::first_cache_id;
use crate::pin::PrefixPins;
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, parse_object, remove_keys, string_field,
};

/// Analyzer-only fallback. Never written onto the JSON.
pub const OLLAMA_STABLE_SESSION: &str = "dsh-ollama";

/// Analyzer id only.
#[must_use]
pub fn conversation_id(body: &Value, explicit: Option<&str>) -> String {
    first_cache_id([
        explicit,
        string_field(body, "session_id"),
        string_field(body, "prompt_cache_key"),
    ])
    .unwrap_or_else(|| OLLAMA_STABLE_SESSION.to_owned())
}

/// Plan a strip-only hop. No identity headers — the plugin sends Bearer
/// alone, and this crate never carries a credential.
#[must_use]
pub fn plan(input: &HopInput<'_>, _pins: Option<&mut PrefixPins>) -> HopRewrite {
    let original = parse_object(input.body);
    let mut next = original.clone();
    let cache_session_id = conversation_id(&next, input.session_id);
    remove_keys(
        &mut next,
        &[
            "prompt_cache_key",
            "prompt_cache_retention",
            "prompt_cache_options",
            "session_id",
        ],
    );
    HopRewrite {
        headers: http::HeaderMap::new(),
        body: body_if_changed(&original, next),
        cache_session_id: Some(cache_session_id),
    }
}

#[cfg(test)]
mod tests;

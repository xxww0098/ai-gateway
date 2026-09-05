//! OpenCode Free (Zen) hop. Anonymous relay — never send Authorization.
//! Zen has no documented conversation / shard field. Strip Codex / Grok
//! cache keys only; do not invent `prompt_cache_key`.

use http::HeaderMap;
use serde_json::Value;

use crate::id::first_cache_id;
use crate::pin::PrefixPins;
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, insert_static, parse_object, remove_keys, string_field,
};

/// Analyzer-only fallback. Never written onto the JSON.
pub const OPENCODE_STABLE_SESSION: &str = "dsh-opencode";
/// Relay 401s an unrecognized bearer; UA/referer identify this hop instead.
pub const OPENCODE_USER_AGENT: &str = "dsh-plugin-oauth-subs";
/// `http-referer` the official Zen docs accept.
pub const OPENCODE_REFERER: &str = "https://github.com/xxww0098/dsh-plugin-oauth-subs";
/// `x-title`.
pub const OPENCODE_TITLE: &str = "dsh-plugin-oauth-subs";

/// Analyzer id only. Not a sticky shard key.
#[must_use]
pub fn conversation_id(body: &Value, explicit: Option<&str>) -> String {
    first_cache_id([
        explicit,
        string_field(body, "session_id"),
        string_field(body, "prompt_cache_key"),
    ])
    .unwrap_or_else(|| OPENCODE_STABLE_SESSION.to_owned())
}

/// Plan OpenCode identity. `pins` is accepted for the family signature and
/// ignored — Zen has no documented prefix pin.
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

    let mut headers = HeaderMap::new();
    insert_static(&mut headers, "user-agent", OPENCODE_USER_AGENT);
    insert_static(&mut headers, "http-referer", OPENCODE_REFERER);
    insert_static(&mut headers, "x-title", OPENCODE_TITLE);

    debug_assert!(!headers.contains_key(http::header::AUTHORIZATION));
    debug_assert!(!headers.contains_key("session-id"));
    debug_assert!(!headers.contains_key("x-grok-conv-id"));

    HopRewrite {
        headers,
        body: body_if_changed(&original, next),
        cache_session_id: Some(cache_session_id),
    }
}

#[cfg(test)]
mod tests;

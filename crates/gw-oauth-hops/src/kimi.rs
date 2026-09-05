//! Moonshot Kimi Code hop. Implicit prefix hash; no Codex `prompt_cache_key`
//! and no Grok `x-grok-conv-id`. Device fingerprint stays off this crate
//! (hostname / hashed machine id belong to the planner).

use http::HeaderMap;
use serde_json::Value;

use crate::id::first_cache_id;
use crate::pin::PrefixPins;
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, insert_static, park_leading_system, parse_object,
    remove_keys, string_field,
};

/// When DSH sends neither `session_id` nor `prompt_cache_key`.
pub const KIMI_STABLE_SESSION: &str = "dsh-kimi";
/// Plugin UA. Kimi Code matches on `x-msh-*`, not Pi's UA.
pub const KIMI_USER_AGENT: &str = "dsh-plugin-oauth-subs";
/// `x-msh-platform`.
pub const KIMI_PLATFORM: &str = "dsh";

/// Conversation id: session, else cache key, else stable.
#[must_use]
pub fn conversation_id(body: &Value, explicit: Option<&str>) -> String {
    first_cache_id([
        explicit,
        string_field(body, "session_id"),
        string_field(body, "prompt_cache_key"),
    ])
    .unwrap_or_else(|| KIMI_STABLE_SESSION.to_owned())
}

/// Plan Kimi identity + strip Codex/Grok cache fields. Optional prefix pin.
#[must_use]
pub fn plan(input: &HopInput<'_>, pins: Option<&mut PrefixPins>) -> HopRewrite {
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
    if let Some(messages) = next.get("messages").and_then(Value::as_array).cloned() {
        next["messages"] = Value::Array(park_leading_system(
            &messages,
            &cache_session_id,
            pins,
            &[KIMI_STABLE_SESSION],
        ));
    }

    let mut headers = HeaderMap::new();
    insert_static(&mut headers, "user-agent", KIMI_USER_AGENT);
    insert_static(&mut headers, "x-msh-platform", KIMI_PLATFORM);
    insert_static(&mut headers, "x-msh-version", KIMI_USER_AGENT);

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

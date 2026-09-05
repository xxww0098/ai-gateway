//! xAI Grok hop. Sticky routing is the grok-build header set, not Codex
//! `session-id`. Missing DSH ids fall back to [`GROK_STABLE_SESSION`].

use http::HeaderMap;
use serde_json::Value;

use crate::id::first_cache_id;
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, insert_owned, insert_static, parse_object, string_field,
};

/// When DSH sends neither `session_id` nor `prompt_cache_key`.
pub const GROK_STABLE_SESSION: &str = "dsh-grok";
/// Pinned grok-cli version the hop fingerprints as.
pub const GROK_CLIENT_VERSION: &str = "0.2.93";
/// `grok-cli/<version>`.
pub const GROK_USER_AGENT: &str = "grok-cli/0.2.93";
/// grok-cli token-auth marker.
pub const GROK_TOKEN_AUTH: &str = "xai-grok-cli";

/// Conversation id for this body: cache key, else session id, else stable.
#[must_use]
pub fn conversation_id(body: &Value, explicit: Option<&str>) -> String {
    first_cache_id([
        explicit,
        string_field(body, "prompt_cache_key"),
        string_field(body, "session_id"),
    ])
    .unwrap_or_else(|| GROK_STABLE_SESSION.to_owned())
}

/// Plan Grok identity + affinity headers. Never copies Codex cache headers.
///
/// `x-grok-req-id` is caller-owned (one id per planned attempt). This crate
/// does not mint UUIDs — that would pull an RNG into a planning crate.
#[must_use]
pub fn plan(input: &HopInput<'_>) -> HopRewrite {
    let original = parse_object(input.body);
    let mut next = original.clone();
    let cache_session_id = conversation_id(&next, None);

    next["prompt_cache_key"] = Value::String(cache_session_id.clone());
    if let Value::Object(map) = &mut next {
        map.remove("session_id");
        map.remove("prompt_cache_retention");
        map.remove("prompt_cache_options");
    }

    let mut headers = HeaderMap::new();
    insert_static(&mut headers, "user-agent", GROK_USER_AGENT);
    insert_static(&mut headers, "x-xai-token-auth", GROK_TOKEN_AUTH);
    insert_owned(&mut headers, "x-grok-conv-id", &cache_session_id);
    insert_owned(&mut headers, "x-grok-session-id", &cache_session_id);

    if let Some(req_id) = input.request_id.map(str::trim).filter(|id| !id.is_empty()) {
        insert_owned(&mut headers, "x-grok-req-id", req_id);
    }

    if let Some(model) = input
        .model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| string_field(&original, "model"))
    {
        insert_owned(&mut headers, "x-grok-model-override", model);
    }
    if input.retry_attempt > 0 {
        insert_owned(
            &mut headers,
            "x-grok-transient-retry",
            &input.retry_attempt.to_string(),
        );
    }
    if let Some(user) = input.user_id.map(str::trim).filter(|s| !s.is_empty()) {
        insert_owned(&mut headers, "x-userid", user);
    }

    debug_assert!(
        !headers.contains_key(http::header::AUTHORIZATION),
        "hop headers must not carry a credential"
    );
    debug_assert!(
        !headers.contains_key("session-id"),
        "Codex session-id must not be copied onto Grok"
    );

    HopRewrite {
        headers,
        body: body_if_changed(&original, next),
        cache_session_id: Some(cache_session_id),
    }
}

#[cfg(test)]
mod tests;

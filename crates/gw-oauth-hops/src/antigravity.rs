//! Google Antigravity (hub) hop. Sticky identity is request `sessionId`,
//! not Codex headers and not `implicitCacheConfig`. Full OpenAI ↔
//! generateContent translation stays in the planner.

use http::HeaderMap;
use serde_json::Value;

use crate::id::{first_cache_id, sanitize_cache_id};
use crate::pin::PrefixPins;
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, insert_static, parse_object, remove_keys, string_field,
};

/// When DSH sends neither `session_id` nor `prompt_cache_key`.
pub const ANTIGRAVITY_STABLE_SESSION: &str = "dsh-antigravity";
/// Pinned hub UA (Antigravity.app 2.11.0). Chat hops send UA only —
/// no `x-goog-api-client` (that is onboardUser).
pub const ANTIGRAVITY_USER_AGENT: &str = "antigravity/hub/2.11.0 linux/x64";
/// Body `userAgent` / metadata ide type.
pub const ANTIGRAVITY_BODY_USER_AGENT: &str = "antigravity";

fn append_model_on_stable(base: &str, model_id: Option<&str>) -> String {
    if base != ANTIGRAVITY_STABLE_SESSION {
        return base.to_owned();
    }
    let Some(model) = model_id.and_then(sanitize_cache_id) else {
        return base.to_owned();
    };
    if base == model || base.ends_with(&format!(":{model}")) {
        return base.to_owned();
    }
    let room = 64usize.saturating_sub(1 + model.len());
    if room < 1 {
        return model.chars().take(64).collect();
    }
    let head: String = base.chars().take(room).collect();
    format!("{head}:{model}")
}

/// Session id: explicit / session / cache key / stable, plus model on the
/// fallback so two pickers cannot share a pin.
#[must_use]
pub fn conversation_id(body: &Value, explicit: Option<&str>, model: Option<&str>) -> String {
    let base = first_cache_id([
        explicit,
        string_field(body, "session_id"),
        string_field(body, "prompt_cache_key"),
        string_field(body, "sessionId"),
    ])
    .unwrap_or_else(|| ANTIGRAVITY_STABLE_SESSION.to_owned());
    let model = model.or_else(|| string_field(body, "model"));
    append_model_on_stable(&base, model)
}

/// Plan hub chat identity and stamp `sessionId` onto the body.
#[must_use]
pub fn plan(input: &HopInput<'_>, _pins: Option<&mut PrefixPins>) -> HopRewrite {
    let original = parse_object(input.body);
    let mut next = original.clone();
    let cache_session_id = conversation_id(&next, input.session_id, input.model);
    remove_keys(
        &mut next,
        &[
            "prompt_cache_key",
            "prompt_cache_retention",
            "prompt_cache_options",
            "session_id",
        ],
    );
    next["sessionId"] = Value::String(cache_session_id.clone());
    if next.get("userAgent").is_none() {
        next["userAgent"] = Value::String(ANTIGRAVITY_BODY_USER_AGENT.to_owned());
    }

    let mut headers = HeaderMap::new();
    insert_static(&mut headers, "user-agent", ANTIGRAVITY_USER_AGENT);

    debug_assert!(!headers.contains_key(http::header::AUTHORIZATION));
    debug_assert!(!headers.contains_key("session-id"));
    debug_assert!(!headers.contains_key("x-goog-api-client"));

    HopRewrite {
        headers,
        body: body_if_changed(&original, next),
        cache_session_id: Some(cache_session_id),
    }
}

#[cfg(test)]
mod tests;

//! Cursor AgentService hop. Conversation id lives on the protobuf body
//! (not planned here). HTTP sticky headers are `x-request-id` +
//! `x-original-request-id`. Client type is CLI OAuth, never `sdk`.

use http::HeaderMap;
use serde_json::Value;

use crate::id::{first_cache_id, sanitize_cache_id};
use crate::pin::PrefixPins;
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, insert_owned, insert_static, parse_object, remove_keys,
    string_field,
};

pub mod proto;
pub mod translate;

pub use translate::openai_to_cursor;

/// When DSH sends neither `session_id` nor `prompt_cache_key`.
pub const CURSOR_STABLE_SESSION: &str = "dsh-cursor";
/// Official CLI fingerprint (Rahularya01/pi-cursor h2-session).
pub const CURSOR_CLIENT_VERSION: &str = "cli-2026.07.23-e383d2b";
/// `x-cursor-client-type`. Not `sdk`.
pub const CURSOR_CLIENT_TYPE: &str = "cli";
/// Host-side Fast picker suffix. Not Codex `service_tier`.
pub const CURSOR_FAST_SUFFIX: &str = "-fast";

pub(crate) fn peel_fast(model_id: &str) -> &str {
    let raw = model_id.trim();
    let suffix = CURSOR_FAST_SUFFIX;
    if raw.len() >= suffix.len() {
        let tail = &raw[raw.len() - suffix.len()..];
        if tail.eq_ignore_ascii_case(suffix) {
            let peeled = &raw[..raw.len() - suffix.len()];
            if !peeled.is_empty() {
                return peeled;
            }
        }
    }
    raw
}

fn append_model(base: &str, model_id: Option<&str>) -> String {
    let Some(model) = model_id.map(peel_fast).and_then(sanitize_cache_id) else {
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

/// Conversation id: session / cache key / stable, plus peeled model.
#[must_use]
pub fn conversation_id(body: &Value, explicit: Option<&str>, model: Option<&str>) -> String {
    let base = first_cache_id([
        explicit,
        string_field(body, "session_id"),
        string_field(body, "prompt_cache_key"),
    ])
    .unwrap_or_else(|| CURSOR_STABLE_SESSION.to_owned());
    let model = model.or_else(|| string_field(body, "model"));
    append_model(&base, model)
}

/// Plan Cursor CLI identity. Chat Completions with `messages` become a
/// Connect-framed AgentService/Run body.
#[must_use]
pub fn plan(input: &HopInput<'_>, pins: Option<&mut PrefixPins>) -> HopRewrite {
    let original = parse_object(input.body);
    let mut next = original.clone();
    let cache_session_id = conversation_id(&next, input.session_id, input.model);
    remove_keys(
        &mut next,
        &[
            "prompt_cache_key",
            "prompt_cache_retention",
            "prompt_cache_options",
            "service_tier",
        ],
    );

    let mut headers = HeaderMap::new();
    insert_static(&mut headers, "connect-protocol-version", "1");
    insert_static(&mut headers, "te", "trailers");
    insert_static(&mut headers, "x-ghost-mode", "true");
    insert_static(
        &mut headers,
        "x-cursor-client-version",
        CURSOR_CLIENT_VERSION,
    );
    insert_static(&mut headers, "x-cursor-client-type", CURSOR_CLIENT_TYPE);
    if let Some(id) = input.request_id.map(str::trim).filter(|id| !id.is_empty()) {
        insert_owned(&mut headers, "x-request-id", id);
        insert_owned(&mut headers, "x-original-request-id", id);
    }

    debug_assert!(!headers.contains_key(http::header::AUTHORIZATION));
    debug_assert!(!headers.contains_key("session-id"));
    debug_assert!(!headers.contains_key("x-grok-conv-id"));

    if let Some(bytes) = openai_to_cursor(&original, input.session_id, input.model, pins) {
        return HopRewrite {
            headers,
            body: Some(bytes::Bytes::from(bytes)),
            cache_session_id: Some(cache_session_id),
        };
    }

    HopRewrite {
        headers,
        body: body_if_changed(&original, next),
        cache_session_id: Some(cache_session_id),
    }
}

#[cfg(test)]
mod tests;

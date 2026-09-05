//! AWS Kiro / CodeWhisperer hop.
//!
//! Cache affinity is `conversationState.conversationId` (id + model). There is
//! no Codex `prompt_cache_key` and no Grok `x-grok-conv-id`. Identity headers
//! assume the native event-stream wire. OpenAI chat bodies are translated.

use http::HeaderMap;
use serde_json::Value;

use crate::id::{first_cache_id, sanitize_cache_id};
use crate::pin::PrefixPins;
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, insert_static, parse_object, string_field,
};

pub mod translate;

pub use translate::{KIRO_CHAT_ORIGIN, KIRO_SYSTEM_ACK, kiro_to_openai, openai_to_kiro};

/// When DSH sends neither `session_id` nor `prompt_cache_key`.
pub const KIRO_STABLE_SESSION: &str = "dsh-kiro";
/// CodeWhisperer streaming target.
pub const KIRO_AMZ_TARGET: &str = "AmazonCodeWhispererStreamingService.GenerateAssistantResponse";
/// Event-stream accept type.
pub const KIRO_EVENTSTREAM_TYPE: &str = "application/vnd.amazon.eventstream";
/// Amazon JSON content type.
pub const KIRO_AMZ_JSON_TYPE: &str = "application/x-amz-json-1.0";

fn append_model(base: &str, model_id: Option<&str>) -> String {
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

/// Conversation id: explicit / session / cache key / stable, plus model.
#[must_use]
pub fn conversation_id(body: &Value, explicit: Option<&str>, model: Option<&str>) -> String {
    let base = first_cache_id([
        explicit,
        string_field(body, "session_id"),
        string_field(body, "prompt_cache_key"),
    ])
    .unwrap_or_else(|| KIRO_STABLE_SESSION.to_owned());
    let model = model.or_else(|| string_field(body, "model"));
    append_model(&base, model)
}

/// Native CodeWhisperer identity headers. Not for OpenAI-shaped payloads.
#[must_use]
pub fn identity_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    insert_static(&mut headers, "accept", KIRO_EVENTSTREAM_TYPE);
    insert_static(&mut headers, "content-type", KIRO_AMZ_JSON_TYPE);
    insert_static(&mut headers, "x-amz-target", KIRO_AMZ_TARGET);
    insert_static(&mut headers, "x-amzn-kiro-agent-mode", "vibe");
    headers
}

/// Plan cache id + native identity headers. Chat Completions bodies become
/// `conversationState`; already-native bodies are forwarded unchanged.
#[must_use]
pub fn plan(input: &HopInput<'_>, pins: Option<&mut PrefixPins>) -> HopRewrite {
    let original = parse_object(input.body);
    let cache_session_id = conversation_id(&original, input.session_id, input.model);
    let next = openai_to_kiro(
        &original,
        input.session_id,
        input.profile_arn,
        input.model,
        pins,
    )
    .unwrap_or_else(|| original.clone());
    HopRewrite {
        headers: identity_headers(),
        body: body_if_changed(&original, next),
        cache_session_id: Some(cache_session_id),
    }
}

#[cfg(test)]
mod tests;

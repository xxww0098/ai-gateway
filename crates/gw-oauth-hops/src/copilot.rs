//! GitHub Copilot Completions hop. Sticky id is `x-interaction-id`.
//! Never copies Codex `session-id` or Grok `x-grok-conv-id`.

use http::HeaderMap;
use serde_json::Value;

use crate::id::first_cache_id;
use crate::pin::PrefixPins;
use crate::rewrite::{
    HopInput, HopRewrite, body_if_changed, insert_owned, insert_static, park_leading_system,
    parse_object, remove_keys, string_field,
};

/// Written as `x-interaction-id` when DSH omits a session.
pub const COPILOT_STABLE_SESSION: &str = "dsh-copilot";
/// VS Code Copilot Chat UA.
pub const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
/// `editor-version`.
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.107.0";
/// `editor-plugin-version`.
pub const COPILOT_EDITOR_PLUGIN: &str = "copilot-chat/0.35.0";
/// `copilot-integration-id`. OpenCode uses the same vscode-chat marker.
pub const COPILOT_INTEGRATION_ID: &str = "vscode-chat";
/// `x-github-api-version`.
pub const COPILOT_API_VERSION: &str = "2026-06-01";

/// Conversation id: session, else cache key, else stable.
#[must_use]
pub fn conversation_id(body: &Value, explicit: Option<&str>) -> String {
    first_cache_id([
        explicit,
        string_field(body, "session_id"),
        string_field(body, "prompt_cache_key"),
    ])
    .unwrap_or_else(|| COPILOT_STABLE_SESSION.to_owned())
}

fn has_vision(messages: &[Value]) -> bool {
    messages.iter().any(|message| {
        message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("image_url" | "image")
                    ) || part.get("image_url").is_some()
                })
            })
    })
}

fn initiator(messages: &[Value]) -> &'static str {
    match messages
        .last()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
    {
        Some("tool" | "assistant") => "agent",
        _ => "user",
    }
}

fn is_gpt(model: &str) -> bool {
    model.to_ascii_lowercase().contains("gpt")
}

/// Plan Copilot identity + interaction id. GPT models drop max-token caps
/// (official Copilot Chat omits them).
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
        let vision = has_vision(&messages);
        let who = initiator(&messages);
        next["messages"] = Value::Array(park_leading_system(
            &messages,
            &cache_session_id,
            pins,
            &[COPILOT_STABLE_SESSION],
        ));

        let model = input
            .model
            .or_else(|| string_field(&original, "model"))
            .unwrap_or("");
        if is_gpt(model) {
            remove_keys(
                &mut next,
                &["max_tokens", "max_completion_tokens", "max_output_tokens"],
            );
        }

        let mut headers = HeaderMap::new();
        insert_static(&mut headers, "user-agent", COPILOT_USER_AGENT);
        insert_static(&mut headers, "editor-version", COPILOT_EDITOR_VERSION);
        insert_static(&mut headers, "editor-plugin-version", COPILOT_EDITOR_PLUGIN);
        insert_static(
            &mut headers,
            "copilot-integration-id",
            COPILOT_INTEGRATION_ID,
        );
        insert_static(&mut headers, "openai-intent", "conversation-edits");
        insert_static(&mut headers, "x-github-api-version", COPILOT_API_VERSION);
        insert_owned(&mut headers, "x-interaction-id", &cache_session_id);
        insert_static(&mut headers, "x-initiator", who);
        if vision {
            insert_static(&mut headers, "copilot-vision-request", "true");
        }

        debug_assert!(!headers.contains_key(http::header::AUTHORIZATION));
        debug_assert!(!headers.contains_key("session-id"));

        return HopRewrite {
            headers,
            body: body_if_changed(&original, next),
            cache_session_id: Some(cache_session_id),
        };
    }

    let mut headers = HeaderMap::new();
    insert_static(&mut headers, "user-agent", COPILOT_USER_AGENT);
    insert_static(&mut headers, "editor-version", COPILOT_EDITOR_VERSION);
    insert_static(&mut headers, "editor-plugin-version", COPILOT_EDITOR_PLUGIN);
    insert_static(
        &mut headers,
        "copilot-integration-id",
        COPILOT_INTEGRATION_ID,
    );
    insert_static(&mut headers, "openai-intent", "conversation-edits");
    insert_static(&mut headers, "x-github-api-version", COPILOT_API_VERSION);
    insert_owned(&mut headers, "x-interaction-id", &cache_session_id);
    insert_static(&mut headers, "x-initiator", "user");

    HopRewrite {
        headers,
        body: body_if_changed(&original, next),
        cache_session_id: Some(cache_session_id),
    }
}

#[cfg(test)]
mod tests;

//! OpenAI chat/completions ↔ CodeWhisperer GenerateAssistantResponse.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{KIRO_STABLE_SESSION, conversation_id};
use crate::pin::PrefixPins;

/// Official kiro.rs ack after a parked system user turn.
pub const KIRO_SYSTEM_ACK: &str = "I will follow these instructions.";
/// Chat origin on `userInputMessage`.
pub const KIRO_CHAT_ORIGIN: &str = "AI_EDITOR";

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn flatten_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.as_str()),
                Value::Object(_) => part
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn try_json(value: &Value) -> Value {
    match value {
        Value::Object(_) => value.clone(),
        Value::String(text) if text.trim().is_empty() => json!({}),
        Value::String(text) => {
            serde_json::from_str(text).unwrap_or_else(|_| json!({ "result": text }))
        }
        other => json!({ "result": other }),
    }
}

fn is_kiro_tool_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

/// IDs Kiro already accepts stay. Compound OpenAI ids get a stable sha256 remap.
#[must_use]
pub fn normalize_tool_use_id(id: Option<&str>) -> Option<String> {
    let raw = trimmed(id)?;
    if raw.starts_with("tooluse_") && is_kiro_tool_id(raw) {
        return Some(raw.to_owned());
    }
    let stripped = raw
        .strip_prefix("tooluse_")
        .unwrap_or(raw)
        .strip_prefix("toolu_")
        .or_else(|| raw.strip_prefix("call_"))
        .or_else(|| raw.strip_prefix("tool_"))
        .unwrap_or(raw.strip_prefix("tooluse_").unwrap_or(raw));
    let prefixed = if raw.starts_with("tooluse_") {
        raw.to_owned()
    } else {
        format!("tooluse_{stripped}")
    };
    if is_kiro_tool_id(&prefixed) {
        return Some(prefixed);
    }
    let digest = Sha256::digest(raw.as_bytes());
    let short = URL_SAFE_NO_PAD.encode(digest)[..32].to_owned();
    Some(format!("tooluse_{short}"))
}

fn openai_tools(payload: &Value) -> Option<Value> {
    let tools = payload.get("tools")?.as_array()?;
    let mapped: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            let fn_obj = tool.get("function").unwrap_or(tool);
            let name = trimmed(fn_obj.get("name").and_then(Value::as_str))?;
            let mut spec = json!({ "name": name, "inputSchema": { "json": fn_obj.get("parameters").cloned().unwrap_or_else(|| json!({ "type": "object", "properties": {} })) } });
            if let Some(desc) = trimmed(fn_obj.get("description").and_then(Value::as_str)) {
                spec["description"] = json!(desc);
            }
            Some(json!({ "toolSpecification": spec }))
        })
        .collect();
    if mapped.is_empty() {
        None
    } else {
        Some(Value::Array(mapped))
    }
}

/// AWS 400s a tool_use without an immediately following tool_result.
#[must_use]
pub fn relocate_displaced_tool_results(messages: &[Value]) -> Vec<Value> {
    let mut pending: Vec<Value> = messages.to_vec();
    let mut out = Vec::new();
    while !pending.is_empty() {
        let message = pending.remove(0);
        let calls = if message.get("role").and_then(Value::as_str) == Some("assistant") {
            message
                .get("tool_calls")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        out.push(message);
        for call in calls {
            let Some(id) = trimmed(call.get("id").and_then(Value::as_str)) else {
                continue;
            };
            if let Some(at) = pending.iter().position(|row| {
                row.get("role").and_then(Value::as_str) == Some("tool")
                    && trimmed(row.get("tool_call_id").and_then(Value::as_str)) == Some(id)
            }) {
                out.push(pending.remove(at));
            }
        }
    }
    out
}

fn assistant_history(message: &Value) -> Value {
    let content = flatten_content(message.get("content").unwrap_or(&Value::Null));
    let mut row = json!({ "content": content });
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        let uses: Vec<Value> = calls
            .iter()
            .filter_map(|call| {
                let name = trimmed(
                    call.get("function")
                        .and_then(|fn_obj| fn_obj.get("name"))
                        .and_then(Value::as_str)
                        .or_else(|| call.get("name").and_then(Value::as_str)),
                )?;
                let tool_use_id = normalize_tool_use_id(call.get("id").and_then(Value::as_str))?;
                let args = call
                    .get("function")
                    .and_then(|fn_obj| fn_obj.get("arguments"))
                    .or_else(|| call.get("input"))
                    .unwrap_or(&Value::Null);
                let input = match args {
                    Value::String(text) => try_json(&Value::String(text.clone())),
                    Value::Object(_) => args.clone(),
                    _ => json!({}),
                };
                Some(json!({ "toolUseId": tool_use_id, "name": name, "input": input }))
            })
            .collect();
        if !uses.is_empty() {
            row["toolUses"] = Value::Array(uses);
        }
    }
    if row
        .get("content")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
        && row.get("toolUses").is_none()
    {
        row["content"] = json!(".");
    }
    json!({ "assistantResponseMessage": row })
}

fn user_history(content: &str, model_id: &str, tool_results: Option<&[Value]>) -> Value {
    let mut context = json!({});
    if let Some(results) = tool_results.filter(|rows| !rows.is_empty()) {
        context["toolResults"] = Value::Array(results.to_vec());
    }
    json!({
        "userInputMessage": {
            "content": if content.is_empty() && tool_results.is_some_and(|rows| !rows.is_empty()) {
                ""
            } else if content.is_empty() {
                "."
            } else {
                content
            },
            "userInputMessageContext": context,
            "origin": KIRO_CHAT_ORIGIN,
            "modelId": model_id,
        }
    })
}

fn push_system_pair(history: &mut Vec<Value>, text: &str, model_id: &str) {
    if text.is_empty() {
        return;
    }
    history.push(user_history(text, model_id, None));
    history.push(json!({ "assistantResponseMessage": { "content": KIRO_SYSTEM_ACK } }));
}

fn last_has_tool_uses(history: &[Value]) -> bool {
    history
        .last()
        .and_then(|row| row.get("assistantResponseMessage"))
        .and_then(|row| row.get("toolUses"))
        .and_then(Value::as_array)
        .is_some_and(|uses| !uses.is_empty())
}

fn park_extra(
    history: &mut Vec<Value>,
    extra: &str,
    model_id: &str,
    current_has_tool_results: bool,
) {
    if extra.is_empty() {
        return;
    }
    if current_has_tool_results && last_has_tool_uses(history) {
        let mut pair = Vec::new();
        push_system_pair(&mut pair, extra, model_id);
        let at = history.len().saturating_sub(1);
        history.splice(at..at, pair);
        return;
    }
    push_system_pair(history, extra, model_id);
}

/// Chat Completions body → CodeWhisperer `conversationState`. Missing model
/// yields `None` so the hop stays header-only.
#[must_use]
pub fn openai_to_kiro(
    payload: &Value,
    conversation: Option<&str>,
    profile_arn: Option<&str>,
    model: Option<&str>,
    pins: Option<&mut PrefixPins>,
) -> Option<Value> {
    if payload.get("conversationState").is_some() {
        return None;
    }
    let model_id = model
        .and_then(|id| trimmed(Some(id)).map(str::to_owned))
        .or_else(|| trimmed(payload.get("model").and_then(Value::as_str)).map(str::to_owned))?;
    let messages = relocate_displaced_tool_results(
        payload
            .get("messages")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    );

    let mut history = Vec::new();
    let mut system_parts = Vec::new();
    let mut pending_user: Option<String> = None;
    let mut pending_assistant: Option<Value> = None;
    let mut pending_tool_results: Vec<Value> = Vec::new();

    for message in &messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        match role {
            "user" => {
                if let Some(assistant) = pending_assistant.take() {
                    history.push(assistant);
                }
                if pending_user.is_some() || !pending_tool_results.is_empty() {
                    let user = pending_user.take().unwrap_or_default();
                    history.push(user_history(&user, &model_id, Some(&pending_tool_results)));
                    pending_tool_results.clear();
                }
                pending_user = Some(flatten_content(
                    message.get("content").unwrap_or(&Value::Null),
                ));
            }
            "assistant" => {
                if let Some(assistant) = pending_assistant.take() {
                    history.push(assistant);
                }
                if pending_user.is_some() || !pending_tool_results.is_empty() {
                    let user = pending_user.take().unwrap_or_default();
                    history.push(user_history(&user, &model_id, Some(&pending_tool_results)));
                    pending_tool_results.clear();
                }
                pending_assistant = Some(assistant_history(message));
            }
            "tool" => {
                if let Some(tool_use_id) =
                    normalize_tool_use_id(message.get("tool_call_id").and_then(Value::as_str))
                {
                    pending_tool_results.push(json!({
                        "toolUseId": tool_use_id,
                        "content": [{ "json": try_json(message.get("content").unwrap_or(&Value::Null)) }],
                        "status": "success",
                    }));
                }
            }
            _ => {
                let text = flatten_content(message.get("content").unwrap_or(&Value::Null));
                if !text.is_empty() {
                    system_parts.push(text);
                }
            }
        }
    }
    if let Some(assistant) = pending_assistant.take() {
        history.push(assistant);
    }

    let resolved_id = conversation_id(payload, conversation, Some(&model_id));
    let system_text = system_parts.join("\n");
    let (pinned, extra) = match pins {
        Some(pins) => {
            let result = pins.pin(&resolved_id, &system_text, &[KIRO_STABLE_SESSION]);
            (result.pinned, result.extra)
        }
        None => (system_text, String::new()),
    };
    let mut full_history = Vec::new();
    push_system_pair(&mut full_history, &pinned, &model_id);
    full_history.extend(history);
    park_extra(
        &mut full_history,
        &extra,
        &model_id,
        !pending_tool_results.is_empty(),
    );

    let mut user_context = json!({ "envState": { "operatingSystem": "linux" } });
    if let Some(tools) = openai_tools(payload) {
        user_context["tools"] = tools;
    }
    if !pending_tool_results.is_empty() {
        user_context["toolResults"] = Value::Array(pending_tool_results.clone());
    }

    let mut content = pending_user.unwrap_or_default();
    if content.is_empty() && pending_tool_results.is_empty() {
        content = trimmed(payload.get("input").and_then(Value::as_str))
            .unwrap_or(".")
            .to_owned();
    }

    let mut body = json!({
        "conversationState": {
            "conversationId": resolved_id,
            "history": full_history,
            "currentMessage": {
                "userInputMessage": {
                    "content": content,
                    "userInputMessageContext": user_context,
                    "origin": KIRO_CHAT_ORIGIN,
                    "modelId": model_id,
                }
            },
            "chatTriggerType": "MANUAL",
            "agentTaskType": "vibe",
        }
    });
    if let Some(arn) = trimmed(profile_arn) {
        body["profileArn"] = json!(arn);
    }
    Some(body)
}

/// Fold collected assistant text / tools into an OpenAI chat.completion.
#[must_use]
pub fn kiro_to_openai(
    text: &str,
    thinking: &str,
    tool_calls: &[Value],
    model: Option<&str>,
) -> Value {
    let mut message = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) } });
    if !thinking.is_empty() {
        message["reasoning_content"] = json!(thinking);
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls.to_vec());
    }
    json!({
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": if tool_calls.is_empty() { "stop" } else { "tool_calls" },
        }]
    })
}

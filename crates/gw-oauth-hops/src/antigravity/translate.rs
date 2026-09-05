//! OpenAI chat/completions ↔ Cloud Code generateContent.

use serde_json::{Value, json};

use super::{ANTIGRAVITY_BODY_USER_AGENT, ANTIGRAVITY_STABLE_SESSION, conversation_id};
use crate::pin::PrefixPins;

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn is_claude(model: &str) -> bool {
    model.starts_with("claude-")
}

fn is_gpt_oss(model: &str) -> bool {
    model.starts_with("gpt-oss-")
}

/// Gemini 3 / gemini-pro-agent need thoughtSignature on functionCall groups.
#[must_use]
pub fn requires_thought_signature(model: &str) -> bool {
    if !model.starts_with("gemini-") {
        return false;
    }
    if let Some(rest) = model.strip_prefix("gemini-")
        && let Some((n, _)) = rest.split_once(|c: char| !c.is_ascii_digit())
        && let Ok(major) = n.parse::<u32>()
    {
        return major >= 3;
    }
    if let Some(n) = model.strip_prefix("gemini-").and_then(|s| {
        s.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<u32>()
            .ok()
    }) {
        return n >= 3;
    }
    true
}

fn parts_from_content(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) if !text.is_empty() => vec![json!({ "text": text })],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(text) if !text.is_empty() => Some(json!({ "text": text })),
                Value::Object(_) if item.get("type").and_then(Value::as_str) == Some("text") => {
                    item.get("text")
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                        .map(|t| json!({ "text": t }))
                }
                _ => None,
            })
            .collect(),
        Value::Null | Value::String(_) => Vec::new(),
        other => vec![json!({ "text": other.to_string() })],
    }
}

fn append_turn(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    if let Some(last) = contents.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
        && let Some(Value::Array(existing)) = last.get_mut("parts")
    {
        existing.extend(parts);
        return;
    }
    contents.push(json!({ "role": role, "parts": parts }));
}

fn thought_signature_of(call: &Value) -> Option<&str> {
    trimmed(call.get("thoughtSignature").and_then(Value::as_str))
        .or_else(|| trimmed(call.get("thought_signature").and_then(Value::as_str)))
        .or_else(|| {
            call.get("extra_content")
                .or_else(|| call.get("extra_body"))
                .and_then(|extra| extra.get("google").or(Some(extra)))
                .and_then(|google| {
                    trimmed(google.get("thought_signature").and_then(Value::as_str))
                        .or_else(|| trimmed(google.get("thoughtSignature").and_then(Value::as_str)))
                })
        })
}

fn function_call_part(call: &Value) -> Option<Value> {
    let fn_obj = call.get("function").unwrap_or(call);
    let name = trimmed(fn_obj.get("name").and_then(Value::as_str))?;
    let args = match fn_obj.get("arguments") {
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or_else(|_| json!({})),
        Some(obj) if obj.is_object() => obj.clone(),
        _ => json!({}),
    };
    let mut part = json!({ "functionCall": { "name": name, "args": args } });
    if let Some(sig) = thought_signature_of(call) {
        part["thoughtSignature"] = json!(sig);
        part["functionCall"]["thoughtSignature"] = json!(sig);
    }
    Some(part)
}

fn function_response_part(message: &Value) -> Value {
    let name = trimmed(message.get("name").and_then(Value::as_str)).unwrap_or("tool");
    let response = match message.get("content") {
        Some(Value::String(text)) => {
            serde_json::from_str(text).unwrap_or_else(|_| json!({ "result": text }))
        }
        Some(obj) if obj.is_object() => obj.clone(),
        Some(other) => json!({ "result": other }),
        None => json!({}),
    };
    json!({ "functionResponse": { "name": name, "response": response } })
}

fn tool_declarations(tools: Option<&Value>, model: &str) -> Option<Value> {
    let tools = tools.and_then(Value::as_array).filter(|t| !t.is_empty())?;
    let legacy = is_claude(model) || is_gpt_oss(model);
    let decls: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            let fn_obj = tool.get("function").unwrap_or(tool);
            let name = trimmed(fn_obj.get("name").and_then(Value::as_str))?;
            let mut decl = json!({ "name": name });
            if let Some(desc) = trimmed(fn_obj.get("description").and_then(Value::as_str)) {
                decl["description"] = json!(desc);
            }
            if let Some(schema) = fn_obj.get("parameters") {
                if legacy {
                    decl["parameters"] = schema.clone();
                } else {
                    decl["parametersJsonSchema"] = schema.clone();
                }
            }
            Some(decl)
        })
        .collect();
    if decls.is_empty() {
        None
    } else {
        Some(json!([{ "functionDeclarations": decls }]))
    }
}

fn tool_choice_mode(choice: Option<&Value>) -> &'static str {
    let value = choice
        .and_then(Value::as_str)
        .or_else(|| choice.and_then(|v| v.get("type")).and_then(Value::as_str));
    match value {
        Some("none") => "NONE",
        Some("required" | "any" | "function") => "ANY",
        _ => "AUTO",
    }
}

fn thinking_config(model: &str, effort: Option<&str>) -> Option<Value> {
    if is_claude(model) || is_gpt_oss(model) {
        return None;
    }
    let e = effort.map(str::trim).filter(|s| !s.is_empty())?;
    if e == "off" {
        return None;
    }
    Some(json!({ "includeThoughts": true, "thinkingLevel": e }))
}

/// Chat Completions → generateContent. Missing project or model yields `None`.
#[must_use]
pub fn openai_to_antigravity(
    payload: &Value,
    project_id: Option<&str>,
    session_id: Option<&str>,
    model: Option<&str>,
    pins: Option<&mut PrefixPins>,
) -> Option<Value> {
    if payload.get("contents").is_some() && payload.get("request").is_some() {
        return None;
    }
    let project = trimmed(project_id).map(str::to_owned)?;
    let model_id = model
        .and_then(|id| trimmed(Some(id)).map(str::to_owned))
        .or_else(|| trimmed(payload.get("model").and_then(Value::as_str)).map(str::to_owned))?;
    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let pinned_session = conversation_id(payload, session_id, Some(&model_id));
    let mut system_parts = Vec::new();
    let mut contents = Vec::new();
    for message in messages {
        match message.get("role").and_then(Value::as_str) {
            Some("system" | "developer") => {
                system_parts.extend(parts_from_content(
                    message.get("content").unwrap_or(&Value::Null),
                ));
            }
            Some("tool") => {
                append_turn(&mut contents, "user", vec![function_response_part(message)]);
            }
            role => {
                let mut parts = Vec::new();
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        if let Some(part) = function_call_part(call) {
                            parts.push(part);
                        }
                    }
                }
                parts.extend(parts_from_content(
                    message.get("content").unwrap_or(&Value::Null),
                ));
                append_turn(
                    &mut contents,
                    if role == Some("assistant") {
                        "model"
                    } else {
                        "user"
                    },
                    parts,
                );
            }
        }
    }
    if contents.is_empty() {
        let text = trimmed(payload.get("input").and_then(Value::as_str)).unwrap_or("");
        append_turn(&mut contents, "user", vec![json!({ "text": text })]);
    }
    let system_text = system_parts
        .iter()
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n");
    let extra = if let Some(pins) = pins {
        let result = pins.pin(&pinned_session, &system_text, &[ANTIGRAVITY_STABLE_SESSION]);
        if !result.fresh && !result.extra.is_empty() {
            Some(result.extra)
        } else {
            None
        }
    } else {
        None
    };
    if contents
        .first()
        .and_then(|c| c.get("role"))
        .and_then(Value::as_str)
        == Some("model")
    {
        contents.insert(0, json!({ "role": "user", "parts": [{ "text": "Hello" }] }));
    }
    if let Some(extra) = extra {
        append_turn(&mut contents, "user", vec![json!({ "text": extra })]);
    }

    let mut request = json!({
        "contents": contents,
        "sessionId": pinned_session,
    });
    if !system_parts.is_empty() {
        request["systemInstruction"] = json!({ "role": "user", "parts": system_parts });
    }
    let tools = tool_declarations(payload.get("tools"), &model_id);
    if let Some(tools) = &tools {
        request["tools"] = tools.clone();
    }
    let mut generation = json!({});
    if let Some(max) = payload.get("max_tokens").and_then(Value::as_u64) {
        generation["maxOutputTokens"] = json!(max);
    }
    if let Some(thinking) = thinking_config(
        &model_id,
        payload.get("reasoning_effort").and_then(Value::as_str),
    ) {
        generation["thinkingConfig"] = thinking;
    }
    if generation.as_object().is_some_and(|m| !m.is_empty()) {
        request["generationConfig"] = generation;
    }
    if is_claude(&model_id) {
        request["toolConfig"] = json!({ "functionCallingConfig": { "mode": "VALIDATED" } });
    } else if tools.is_some() {
        request["toolConfig"] = json!({
            "functionCallingConfig": { "mode": tool_choice_mode(payload.get("tool_choice")) }
        });
    }

    Some(json!({
        "model": model_id,
        "project": project,
        "userAgent": ANTIGRAVITY_BODY_USER_AGENT,
        "requestType": "agent",
        "request": request,
    }))
}

fn collect_parts(body: &Value) -> (String, Vec<Value>, &'static str) {
    let response = body.get("response").unwrap_or(body);
    let parts = response
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        if part.get("thought") == Some(&json!(true)) {
            continue;
        }
        if let Some(t) = part.get("text").and_then(Value::as_str) {
            text.push_str(t);
        }
        if let Some(call) = part.get("functionCall") {
            let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
            tool_calls.push(json!({
                "id": call.get("name"),
                "type": "function",
                "function": {
                    "name": call.get("name").cloned().unwrap_or_else(|| json!("tool")),
                    "arguments": args.to_string(),
                }
            }));
        }
    }
    let finish = response
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str)
        .unwrap_or("");
    let reason = if finish.eq_ignore_ascii_case("MAX_TOKENS") {
        "length"
    } else if finish.to_ascii_uppercase().contains("TOOL") || finish == "MALFORMED_FUNCTION_CALL" {
        "tool_calls"
    } else {
        "stop"
    };
    (text, tool_calls, reason)
}

/// generateContent response → chat.completion.
#[must_use]
pub fn antigravity_to_openai(body: &Value, model: Option<&str>) -> Value {
    let (text, tool_calls, finish) = collect_parts(body);
    let mut message = json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { json!(text) },
    });
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls.clone());
    }
    json!({
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": if tool_calls.is_empty() { finish } else { "tool_calls" },
        }]
    })
}

//! Chat Completions ↔ Zen Responses (Muse Spark). Completions models keep
//! the chat body.

use serde_json::{Value, json};

/// Floor Zen accepts on `max_output_tokens`.
pub const OPENCODE_MIN_OUTPUT_TOKENS: u64 = 16;

/// Muse Spark is the Responses hop. Completions 500s those ids.
#[must_use]
pub fn is_responses_model(id: &str) -> bool {
    let slug = id
        .rsplit('/')
        .next()
        .unwrap_or(id)
        .trim()
        .to_ascii_lowercase();
    slug.starts_with("muse-spark")
}

fn is_plain(value: &Value) -> bool {
    value.is_object()
}

fn first_number(values: &[Option<&Value>]) -> Option<u64> {
    for value in values {
        match value {
            Some(Value::Number(n)) => {
                if let Some(u) = n.as_u64() {
                    return Some(u);
                }
                if let Some(f) = n.as_f64().filter(|f| f.is_finite() && *f >= 0.0) {
                    return Some(f as u64);
                }
            }
            Some(Value::String(s)) => {
                if let Ok(n) = s.trim().parse::<u64>() {
                    return Some(n);
                }
            }
            _ => {}
        }
    }
    None
}

fn content_to_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part {
                Value::String(text) => text.clone(),
                Value::Object(_) => part
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                _ => String::new(),
            })
            .collect(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn map_content(content: &Value, output: bool) -> Value {
    match content {
        Value::Null => json!(""),
        Value::String(_) => content.clone(),
        Value::Array(parts) => Value::Array(
            parts
                .iter()
                .map(|part| match part {
                    Value::String(text) => json!({
                        "type": if output { "output_text" } else { "input_text" },
                        "text": text,
                    }),
                    Value::Object(_)
                        if matches!(
                            part.get("type").and_then(Value::as_str),
                            Some("text" | "input_text" | "output_text") | None
                        ) =>
                    {
                        json!({
                            "type": if output { "output_text" } else { "input_text" },
                            "text": part.get("text").and_then(Value::as_str).unwrap_or(""),
                        })
                    }
                    Value::Object(_)
                        if matches!(
                            part.get("type").and_then(Value::as_str),
                            Some("image_url" | "input_image")
                        ) =>
                    {
                        let url = part.get("image_url").and_then(|url| {
                            url.as_str().map(str::to_owned).or_else(|| {
                                url.get("url").and_then(Value::as_str).map(str::to_owned)
                            })
                        });
                        match url {
                            Some(url) => json!({ "type": "input_image", "image_url": url }),
                            None => part.clone(),
                        }
                    }
                    other => other.clone(),
                })
                .collect(),
        ),
        other => json!(other.to_string()),
    }
}

fn chat_tool_to_responses(tool: &Value) -> Value {
    let Some(function) = tool.get("function") else {
        return tool.clone();
    };
    if tool.get("type").and_then(Value::as_str) != Some("function") || !function.is_object() {
        return tool.clone();
    }
    let mut next = json!({
        "type": "function",
        "name": function.get("name"),
        "description": function.get("description"),
        "parameters": function.get("parameters"),
    });
    if let Some(strict) = function.get("strict") {
        next["strict"] = strict.clone();
    }
    next
}

fn messages_to_input(messages: &[Value]) -> Vec<Value> {
    let mut items = Vec::new();
    for msg in messages {
        if !is_plain(msg) {
            continue;
        }
        match msg.get("role").and_then(Value::as_str) {
            Some("tool") => {
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": msg.get("tool_call_id").or_else(|| msg.get("id")),
                    "output": content_to_text(msg.get("content").unwrap_or(&Value::Null)),
                }));
            }
            Some("assistant")
                if msg
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .is_some_and(|calls| !calls.is_empty()) =>
            {
                let text = content_to_text(msg.get("content").unwrap_or(&Value::Null));
                if !text.is_empty() {
                    items.push(json!({ "role": "assistant", "content": text }));
                }
                for call in msg.get("tool_calls").and_then(Value::as_array).unwrap() {
                    let fn_obj = call.get("function").unwrap_or(&Value::Null);
                    let args = match fn_obj.get("arguments") {
                        Some(Value::String(text)) => text.clone(),
                        Some(other) => other.to_string(),
                        None => "{}".into(),
                    };
                    items.push(json!({
                        "type": "function_call",
                        "call_id": call.get("id"),
                        "name": fn_obj.get("name").or_else(|| call.get("name")),
                        "arguments": args,
                    }));
                }
            }
            _ => {
                let output = msg.get("role").and_then(Value::as_str) == Some("assistant");
                items.push(json!({
                    "role": msg.get("role"),
                    "content": map_content(msg.get("content").unwrap_or(&Value::Null), output),
                }));
            }
        }
    }
    items
}

/// Chat / leftover Completions body → Zen Responses. No Codex cache fields.
#[must_use]
pub fn chat_to_responses(payload: &Value) -> Value {
    let mut next = serde_json::Map::new();
    if let Some(model) = payload.get("model").and_then(Value::as_str) {
        next.insert("model".into(), json!(model));
    }
    if payload.get("input").is_some() {
        next.insert("input".into(), payload["input"].clone());
    } else if let Some(messages) = payload.get("messages").and_then(Value::as_array) {
        next.insert("input".into(), Value::Array(messages_to_input(messages)));
    } else if let Some(messages) = payload.get("messages").and_then(Value::as_str) {
        next.insert("input".into(), json!(messages));
    }
    if let Some(instructions) = payload.get("instructions").and_then(Value::as_str) {
        next.insert("instructions".into(), json!(instructions));
    }
    if let Some(max) = first_number(&[payload.get("max_output_tokens"), payload.get("max_tokens")])
    {
        next.insert(
            "max_output_tokens".into(),
            json!(max.max(OPENCODE_MIN_OUTPUT_TOKENS)),
        );
    }
    let effort = payload
        .get("reasoning")
        .and_then(|r| r.get("effort"))
        .cloned()
        .or_else(|| payload.get("reasoning_effort").cloned());
    if let Some(wire) = effort {
        let mut reasoning = match payload.get("reasoning") {
            Some(Value::Object(map)) => map.clone(),
            _ => serde_json::Map::new(),
        };
        reasoning.insert("effort".into(), wire);
        next.insert("reasoning".into(), Value::Object(reasoning));
    }
    if let Some(tools) = payload
        .get("tools")
        .and_then(Value::as_array)
        .filter(|t| !t.is_empty())
    {
        next.insert(
            "tools".into(),
            Value::Array(tools.iter().map(chat_tool_to_responses).collect()),
        );
    }
    if let Some(choice) = payload.get("tool_choice") {
        next.insert("tool_choice".into(), choice.clone());
    }
    if payload.get("stream") == Some(&json!(true)) {
        next.insert("stream".into(), json!(true));
    }
    if let Some(temp) = payload.get("temperature") {
        next.insert("temperature".into(), temp.clone());
    }
    Value::Object(next)
}

fn parts_text(parts: &Value, types: &[&str]) -> String {
    match parts {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.as_str()),
                Value::Object(_) => {
                    let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                    if types.contains(&kind) || kind.is_empty() {
                        part.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect(),
        _ => String::new(),
    }
}

fn collect_output(payload: &Value) -> (String, String, Vec<Value>) {
    let mut text = payload
        .get("output_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    if let Some(output) = payload.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("message") | None
                    if item.get("role").and_then(Value::as_str) == Some("assistant")
                        || item.get("type").and_then(Value::as_str) == Some("message") =>
                {
                    let chunk = parts_text(
                        item.get("content").unwrap_or(&Value::Null),
                        &["output_text", "text"],
                    );
                    if !chunk.is_empty() && !text.contains(&chunk) {
                        text.push_str(&chunk);
                    }
                }
                Some("reasoning") => {
                    let summary = parts_text(
                        item.get("summary").unwrap_or(&Value::Null),
                        &["summary_text", "text"],
                    );
                    let content = parts_text(
                        item.get("content").unwrap_or(&Value::Null),
                        &["reasoning_text", "text"],
                    );
                    reasoning.push_str(if !summary.is_empty() {
                        &summary
                    } else {
                        &content
                    });
                    if reasoning.is_empty()
                        && let Some(t) = item.get("text").and_then(Value::as_str)
                    {
                        reasoning.push_str(t);
                    }
                }
                Some("function_call") => {
                    let args = match item.get("arguments") {
                        Some(Value::String(text)) => text.clone(),
                        Some(other) => other.to_string(),
                        None => "{}".into(),
                    };
                    tool_calls.push(json!({
                        "id": item.get("call_id").or_else(|| item.get("id")),
                        "type": "function",
                        "function": {
                            "name": item.get("name").cloned().unwrap_or_else(|| json!("tool")),
                            "arguments": args,
                        }
                    }));
                }
                _ => {}
            }
        }
    }
    (text, reasoning, tool_calls)
}

fn map_usage(usage: Option<&Value>) -> Value {
    let Some(usage) = usage.filter(|value| value.is_object()) else {
        return json!({ "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 });
    };
    let prompt =
        first_number(&[usage.get("input_tokens"), usage.get("prompt_tokens")]).unwrap_or(0);
    let completion =
        first_number(&[usage.get("output_tokens"), usage.get("completion_tokens")]).unwrap_or(0);
    let total = first_number(&[usage.get("total_tokens")]).unwrap_or(prompt + completion);
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    })
}

fn is_chat_completion(payload: &Value) -> bool {
    payload.get("object").and_then(Value::as_str) == Some("chat.completion")
        || payload.get("choices").and_then(Value::as_array).is_some()
}

/// Fold a Responses body (or a chat.completion passthrough) into chat.completion.
#[must_use]
pub fn responses_to_chat(payload: &Value, model: Option<&str>) -> Value {
    if is_chat_completion(payload) {
        return payload.clone();
    }
    let (text, reasoning, tool_calls) = collect_output(payload);
    let mut message = json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { json!(text) },
    });
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls.clone());
    }
    json!({
        "id": payload.get("id").cloned().unwrap_or_else(|| json!("chatcmpl-opencode")),
        "object": "chat.completion",
        "model": payload.get("model").cloned().unwrap_or_else(|| json!(model)),
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": if tool_calls.is_empty() { "stop" } else { "tool_calls" },
        }],
        "usage": map_usage(payload.get("usage")),
    })
}

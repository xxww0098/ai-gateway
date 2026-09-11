//! OpenAI chat/completions ↔ CodeWhisperer `GenerateAssistantResponse`.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{Error, Session};

use super::cache::{conversation_id, pin_system_prefix};
use super::catalog::context_window_of;
use super::session::{extra_str, json_str, trimmed};

mod eventstream;
#[cfg(test)]
mod tests;

pub use eventstream::{encode_event_frame, encode_event_stream, parse_event_stream};

/// Official kiro.rs ack after a parked system user turn. Byte-stable.
pub const SYSTEM_ACK: &str = "I will follow these instructions.";
pub const CHAT_ORIGIN: &str = "AI_EDITOR";
pub const AMZ_TARGET: &str = "AmazonCodeWhispererStreamingService.GenerateAssistantResponse";
pub const EVENTSTREAM_TYPE: &str = "application/vnd.amazon.eventstream";
pub const AMZ_JSON_TYPE: &str = "application/x-amz-json-1.0";

fn is_tool_use_id(id: &str) -> bool {
    let len = id.len();
    (1..=64).contains(&len)
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        std::env::consts::OS
    }
}

#[must_use]
pub fn chat_url(session: &Session) -> String {
    let region = extra_str(session, "api_region");
    let region = if region.is_empty() {
        extra_str(session, "region")
    } else {
        region
    };
    let region = if region.is_empty() {
        super::DEFAULT_REGION.to_owned()
    } else {
        region
    };
    format!("https://q.{region}.amazonaws.com/")
}

#[must_use]
pub fn chat_headers(session: &Session) -> Map<String, Value> {
    let machine = extra_str(session, "machine_id");
    let mut headers = Map::new();
    headers.insert(
        "authorization".to_owned(),
        json!(format!("Bearer {}", session.access_token)),
    );
    headers.insert("accept".to_owned(), json!(EVENTSTREAM_TYPE));
    headers.insert("content-type".to_owned(), json!(AMZ_JSON_TYPE));
    headers.insert("x-amz-target".to_owned(), json!(AMZ_TARGET));
    headers.insert("x-amzn-kiro-agent-mode".to_owned(), json!("vibe"));
    if !machine.is_empty() {
        headers.insert(
            "user-agent".to_owned(),
            json!(format!("KiroIDE-{}-{machine}", super::USAGE_VERSION)),
        );
    }
    let method = extra_str(session, "auth_method");
    if method == "api_key" {
        headers.insert("tokentype".to_owned(), json!("API_KEY"));
    } else if method == "external_idp" {
        headers.insert("tokentype".to_owned(), json!("EXTERNAL_IDP"));
    }
    headers
}

fn flatten_content(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    let Some(items) = content.as_array() else {
        return if content.is_null() {
            String::new()
        } else {
            content.to_string()
        };
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(text) = item.as_str() {
                return Some(text.to_owned());
            }
            if item.get("type").and_then(Value::as_str) == Some("text") {
                return item.get("text").and_then(Value::as_str).map(str::to_owned);
            }
            None
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn try_json(value: &Value) -> Value {
    if value.is_object() && !value.is_array() {
        return value.clone();
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return json!({});
        }
        return match serde_json::from_str::<Value>(trimmed) {
            Ok(parsed) if parsed.is_object() && !parsed.is_array() => parsed,
            Ok(parsed) => json!({"result": parsed}),
            Err(_) => json!({"result": text}),
        };
    }
    json!({})
}

fn openai_tools_to_kiro(tools: Option<&Value>) -> Option<Vec<Value>> {
    let tools = tools.and_then(Value::as_array)?;
    if tools.is_empty() {
        return None;
    }
    let mapped: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            let fn_obj = tool.get("function").unwrap_or(tool);
            let name = trimmed(fn_obj.get("name").and_then(Value::as_str))?;
            let mut spec = Map::new();
            spec.insert("name".to_owned(), json!(name));
            if let Some(desc) = trimmed(fn_obj.get("description").and_then(Value::as_str)) {
                spec.insert("description".to_owned(), json!(desc));
            }
            let params = fn_obj
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            spec.insert("inputSchema".to_owned(), json!({"json": params}));
            Some(json!({"toolSpecification": spec}))
        })
        .collect();
    if mapped.is_empty() {
        None
    } else {
        Some(mapped)
    }
}

/// IDs Kiro already accepts stay (after call_/toolu_/tool_ → tooluse_). Compound
/// OpenAI Responses ids get a stable sha256 remap. Never `Date.now()`.
#[must_use]
pub fn normalize_tool_use_id(id: Option<&str>) -> Option<String> {
    let raw = trimmed(id)?;
    if raw.starts_with("tooluse_") && is_tool_use_id(&raw) {
        return Some(raw);
    }
    let prefixed = if raw.starts_with("tooluse_") {
        raw.clone()
    } else {
        let rest = raw
            .strip_prefix("toolu_")
            .or_else(|| raw.strip_prefix("call_"))
            .or_else(|| raw.strip_prefix("tool_"))
            .unwrap_or(&raw);
        format!("tooluse_{rest}")
    };
    if is_tool_use_id(&prefixed) {
        return Some(prefixed);
    }
    let digest = URL_SAFE_NO_PAD.encode(Sha256::digest(raw.as_bytes()));
    let digest: String = digest.chars().take(32).collect();
    Some(format!("tooluse_{digest}"))
}

/// Concurrent tools can interleave; AWS 400s a tool_use without an immediately
/// following tool_result. Pure reorder by id.
#[must_use]
pub fn relocate_displaced_tool_results(messages: Vec<Value>) -> Vec<Value> {
    let mut pending = messages;
    let mut out = Vec::new();
    while !pending.is_empty() {
        let message = pending.remove(0);
        let is_assistant = message.get("role").and_then(Value::as_str) == Some("assistant");
        let calls = message.get("tool_calls").and_then(Value::as_array).cloned();
        out.push(message);
        if !is_assistant {
            continue;
        }
        let Some(calls) = calls else {
            continue;
        };
        for call in calls {
            let Some(id) = trimmed(call.get("id").and_then(Value::as_str)) else {
                continue;
            };
            if let Some(at) = pending.iter().position(|row| {
                row.get("role").and_then(Value::as_str) == Some("tool")
                    && trimmed(row.get("tool_call_id").and_then(Value::as_str)).as_deref()
                        == Some(id.as_str())
            }) {
                out.push(pending.remove(at));
            }
        }
    }
    out
}

fn assistant_history_message(message: &Value) -> Value {
    let content = flatten_content(message.get("content"));
    let mut row = Map::new();
    row.insert("content".to_owned(), json!(content));
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array)
        && !calls.is_empty()
    {
        let uses: Vec<Value> = calls
            .iter()
            .filter_map(|call| {
                let name = trimmed(
                    call.get("function")
                        .and_then(|f| f.get("name"))
                        .or_else(|| call.get("name"))
                        .and_then(Value::as_str),
                )?;
                let tool_use_id = normalize_tool_use_id(call.get("id").and_then(Value::as_str))?;
                let args = call
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .or_else(|| call.get("input"))
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let input = if args.is_string() {
                    try_json(&args)
                } else if args.is_object() && !args.is_array() {
                    args
                } else {
                    json!({})
                };
                Some(json!({
                    "toolUseId": tool_use_id,
                    "name": name,
                    "input": input,
                }))
            })
            .collect();
        if !uses.is_empty() {
            row.insert("toolUses".to_owned(), json!(uses));
        }
    }
    if row.get("content").and_then(Value::as_str) == Some("") && !row.contains_key("toolUses") {
        row.insert("content".to_owned(), json!("."));
    }
    json!({"assistantResponseMessage": row})
}

fn user_history_message(
    content: &str,
    model_id: &str,
    origin: &str,
    tool_results: Option<&[Value]>,
) -> Value {
    let mut context = Map::new();
    if let Some(results) = tool_results.filter(|r| !r.is_empty()) {
        context.insert("toolResults".to_owned(), json!(results));
    }
    let body = if content.is_empty() && tool_results.is_none_or(|r| r.is_empty()) {
        "."
    } else {
        content
    };
    json!({
        "userInputMessage": {
            "content": body,
            "userInputMessageContext": context,
            "origin": origin,
            "modelId": model_id,
        }
    })
}

fn push_system_pair(history: &mut Vec<Value>, text: &str, model_id: &str, origin: &str) {
    if text.is_empty() {
        return;
    }
    history.push(user_history_message(text, model_id, origin, None));
    history.push(json!({"assistantResponseMessage": {"content": SYSTEM_ACK}}));
}

fn last_history_has_tool_uses(history: &[Value]) -> bool {
    history
        .last()
        .and_then(|row| row.get("assistantResponseMessage"))
        .and_then(|asst| asst.get("toolUses"))
        .and_then(Value::as_array)
        .is_some_and(|uses| !uses.is_empty())
}

fn park_system_extra(
    history: &mut Vec<Value>,
    extra: &str,
    model_id: &str,
    origin: &str,
    current_has_tool_results: bool,
) {
    if extra.is_empty() {
        return;
    }
    if current_has_tool_results && last_history_has_tool_uses(history) {
        let mut pair = Vec::new();
        push_system_pair(&mut pair, extra, model_id, origin);
        let at = history.len().saturating_sub(1);
        history.splice(at..at, pair);
        return;
    }
    push_system_pair(history, extra, model_id, origin);
}

fn is_system_role(role: Option<&str>) -> bool {
    match role {
        Some("system" | "developer") => true,
        Some("user" | "assistant" | "tool") => false,
        Some(_) => true,
        None => false,
    }
}

/// Translate OpenAI messages into `conversationState`. System is the first
/// history user+ack pair; tools sit on current `userInputMessageContext`.
pub fn openai_to_kiro(
    payload: &Value,
    conversation_pin: Option<&str>,
    profile_arn: Option<&str>,
) -> Result<Value, Error> {
    let model_id = trimmed(payload.get("model").and_then(Value::as_str))
        .ok_or_else(|| Error::Payload("kiro generateAssistantResponse requires a model".into()))?;
    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let messages = relocate_displaced_tool_results(messages);
    let origin = CHAT_ORIGIN;
    let mut history = Vec::new();
    let mut system_parts: Vec<String> = Vec::new();
    let mut pending_user: Option<String> = None;
    let mut pending_assistant: Option<Value> = None;
    let mut pending_tool_results: Vec<Value> = Vec::new();

    let flush_user = |history: &mut Vec<Value>,
                      pending_user: &mut Option<String>,
                      pending_tool_results: &mut Vec<Value>,
                      model_id: &str| {
        if pending_user.is_none() && pending_tool_results.is_empty() {
            return;
        }
        let content = pending_user.take().unwrap_or_default();
        let results = std::mem::take(pending_tool_results);
        history.push(user_history_message(
            &content,
            model_id,
            origin,
            if results.is_empty() {
                None
            } else {
                Some(&results)
            },
        ));
    };

    for message in &messages {
        let role = message.get("role").and_then(Value::as_str);
        if is_system_role(role) {
            let text = flatten_content(message.get("content"));
            if !text.is_empty() {
                system_parts.push(text);
            }
            continue;
        }
        if role == Some("user") {
            if let Some(asst) = pending_assistant.take() {
                history.push(asst);
            }
            flush_user(
                &mut history,
                &mut pending_user,
                &mut pending_tool_results,
                &model_id,
            );
            pending_user = Some(flatten_content(message.get("content")));
            continue;
        }
        if role == Some("assistant") {
            if let Some(asst) = pending_assistant.take() {
                history.push(asst);
            }
            flush_user(
                &mut history,
                &mut pending_user,
                &mut pending_tool_results,
                &model_id,
            );
            pending_assistant = Some(assistant_history_message(message));
            continue;
        }
        if role == Some("tool") {
            let Some(tool_use_id) =
                normalize_tool_use_id(message.get("tool_call_id").and_then(Value::as_str))
            else {
                continue;
            };
            pending_tool_results.push(json!({
                "toolUseId": tool_use_id,
                "content": [{"json": try_json(message.get("content").unwrap_or(&Value::Null))}],
                "status": "success",
            }));
        }
    }
    if let Some(asst) = pending_assistant.take() {
        history.push(asst);
    }

    let resolved_id = conversation_id(payload, conversation_pin);
    let pin = pin_system_prefix(&resolved_id, &system_parts.join("\n"));
    let mut parked = Vec::new();
    push_system_pair(&mut parked, &pin.pinned, &model_id, origin);
    let mut full_history = parked;
    full_history.append(&mut history);
    park_system_extra(
        &mut full_history,
        &pin.extra,
        &model_id,
        origin,
        !pending_tool_results.is_empty(),
    );

    let mut user_context = Map::new();
    user_context.insert("envState".to_owned(), json!({"operatingSystem": os_name()}));
    if let Some(tools) = openai_tools_to_kiro(payload.get("tools")) {
        user_context.insert("tools".to_owned(), json!(tools));
    }
    if !pending_tool_results.is_empty() {
        user_context.insert(
            "toolResults".to_owned(),
            json!(pending_tool_results.clone()),
        );
    }

    let mut content = pending_user.unwrap_or_default();
    if content.is_empty() && pending_tool_results.is_empty() {
        content =
            trimmed(payload.get("input").and_then(Value::as_str)).unwrap_or_else(|| ".".to_owned());
    }

    let mut body = json!({
        "conversationState": {
            "conversationId": resolved_id,
            "history": full_history,
            "currentMessage": {
                "userInputMessage": {
                    "content": content,
                    "userInputMessageContext": user_context,
                    "origin": origin,
                    "modelId": model_id,
                }
            },
            "chatTriggerType": "MANUAL",
            "agentTaskType": "vibe",
        }
    });
    if let Some(arn) =
        trimmed(profile_arn).or_else(|| json_str(payload, &["profileArn", "profile_arn"]))
    {
        body.as_object_mut()
            .expect("object")
            .insert("profileArn".to_owned(), json!(arn));
    }
    Ok(body)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopError {
    pub status: u16,
    pub code: &'static str,
    pub retry_after: Option<String>,
}

fn hop_error_blob(parsed: &Value, text: &str) -> String {
    let mut parts = vec![text.to_owned()];
    for key in ["reason", "message", "Message"] {
        if let Some(s) = parsed.get(key).and_then(Value::as_str) {
            parts.push(s.to_owned());
        }
    }
    if let Some(s) = parsed
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
    {
        parts.push(s.to_owned());
    }
    if let Some(s) = parsed.get("code").and_then(Value::as_str) {
        parts.push(s.to_owned());
    }
    parts.join("\n")
}

/// `MONTHLY_REQUEST_COUNT` is a client error (400), not a hammerable 429.
#[must_use]
pub fn classify_hop_error(
    status: u16,
    parsed: &Value,
    text: &str,
    retry_after: Option<&str>,
) -> HopError {
    let blob = hop_error_blob(parsed, text);
    let header_retry = retry_after
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if blob.contains("MONTHLY_REQUEST_COUNT") {
        return HopError {
            status: 400,
            code: "kiro_quota",
            retry_after: None,
        };
    }
    if blob.contains("INSUFFICIENT_MODEL_CAPACITY") {
        return HopError {
            status: 503,
            code: "kiro_capacity",
            retry_after: header_retry,
        };
    }
    if blob.contains("USER_REQUEST_RATE_EXCEEDED") {
        return HopError {
            status: 429,
            code: "kiro_rate",
            retry_after: header_retry,
        };
    }
    if status == 413
        || blob.contains("CONTENT_LENGTH_EXCEEDS_THRESHOLD")
        || blob.contains("Input is too long")
        || blob
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .any(|w| w == "TOO_BIG")
    {
        return HopError {
            status: if status == 413 { 413 } else { 400 },
            code: "kiro_too_big",
            retry_after: None,
        };
    }
    if status == 401 || status == 403 {
        return HopError {
            status: 400,
            code: "kiro_upstream",
            retry_after: None,
        };
    }
    HopError {
        status: if status >= 400 { status } else { 502 },
        code: "kiro_upstream",
        retry_after: header_retry,
    }
}

#[must_use]
pub fn client_error_status(status: u16, parsed: &Value, text: &str) -> u16 {
    classify_hop_error(status, parsed, text, None).status
}

fn number_field(object: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        let Some(value) = object.get(*key) else {
            continue;
        };
        if let Some(n) = value.as_f64().filter(|n| n.is_finite()) {
            return Some(n);
        }
        if let Some(s) = value.as_str().and_then(|s| trimmed(Some(s)))
            && let Ok(n) = s.parse::<f64>()
            && n.is_finite()
        {
            return Some(n);
        }
    }
    None
}

#[must_use]
pub fn map_usage(tokens: &Value) -> Option<Value> {
    if !tokens.is_object() {
        return None;
    }
    let cache_read = number_field(tokens, &["cacheReadInputTokens", "cache_read_input_tokens"]);
    let cache_write = number_field(
        tokens,
        &["cacheWriteInputTokens", "cache_write_input_tokens"],
    )
    .unwrap_or(0.0);
    let uncached =
        number_field(tokens, &["uncachedInputTokens", "uncached_input_tokens"]).unwrap_or(0.0);
    let output = number_field(tokens, &["outputTokens", "output_tokens"]).unwrap_or(0.0);
    let cached = cache_read.unwrap_or(0.0);
    let prompt = uncached + cached + cache_write;
    let total = number_field(tokens, &["totalTokens", "total_tokens"]).unwrap_or(prompt + output);
    let mut usage = json!({
        "prompt_tokens": prompt.round() as i64,
        "completion_tokens": output.round() as i64,
        "total_tokens": total.round() as i64,
    });
    if let Some(read) = cache_read
        && let Some(obj) = usage.as_object_mut()
    {
        obj.insert(
            "prompt_tokens_details".to_owned(),
            json!({"cached_tokens": read.round() as i64}),
        );
    }
    Some(usage)
}

fn usage_from_payload(data: &Value) -> Option<Value> {
    if !data.is_object() {
        return None;
    }
    if let Some(nested) = data.get("tokenUsage").filter(|v| v.is_object()) {
        return map_usage(nested);
    }
    if let Some(nested) = data.get("token_usage").filter(|v| v.is_object()) {
        return map_usage(nested);
    }
    if number_field(data, &["uncachedInputTokens", "uncached_input_tokens"]).is_some()
        || number_field(data, &["cacheReadInputTokens", "cache_read_input_tokens"]).is_some()
        || number_field(data, &["outputTokens", "output_tokens"]).is_some()
        || number_field(data, &["totalTokens", "total_tokens"]).is_some()
    {
        return map_usage(data);
    }
    None
}

fn has_real_usage(usage: Option<&Value>) -> bool {
    let Some(usage) = usage else { return false };
    usage
        .get("prompt_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        > 0
        || usage
            .get("completion_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            > 0
        || usage
            .get("total_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            > 0
        || usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0)
            > 0
}

#[must_use]
pub fn usage_from_context(percent: f64, model: &str, text: &str) -> Option<Value> {
    if !percent.is_finite() || percent <= 0.0 {
        return None;
    }
    let prompt = ((context_window_of(model) as f64) * percent / 100.0).round() as i64;
    if prompt <= 0 {
        return None;
    }
    let completion = if text.is_empty() {
        0
    } else {
        ((text.len() as i64 + 3) / 4).max(1)
    };
    Some(json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": prompt + completion,
    }))
}

const PAYLOAD_WRAPPERS: &[&str] = &[
    "assistantResponseEvent",
    "metadataEvent",
    "messageMetadataEvent",
    "contextUsageEvent",
    "meteringEvent",
    "toolUseEvent",
    "thinkingEvent",
    "reasoningEvent",
];

#[must_use]
pub fn unwrap_event_payload<'a>(payload: &'a Value, type_name: Option<&str>) -> &'a Value {
    if !payload.is_object() {
        return payload;
    }
    if let Some(t) = type_name
        && payload.get(t).is_some_and(Value::is_object)
    {
        return &payload[t];
    }
    for key in PAYLOAD_WRAPPERS {
        if payload.get(*key).is_some_and(Value::is_object) {
            return &payload[*key];
        }
    }
    payload
}

#[derive(Debug, Clone)]
pub struct StreamEvent {
    pub type_name: String,
    pub message_type: String,
    pub payload: Value,
}

fn thinking_text(type_name: &str, data: &Value) -> Option<String> {
    if !data.is_object() {
        return None;
    }
    let kind = type_name.to_ascii_lowercase();
    if (kind.contains("thinking") || kind.contains("reasoning"))
        && let Some(t) = ["text", "thinking", "content", "reasoningContent"]
            .iter()
            .find_map(|k| trimmed(data.get(*k).and_then(Value::as_str)))
    {
        return Some(t);
    }
    if data.get("text").and_then(Value::as_str).is_some()
        && data.get("content").is_none()
        && data.get("toolUseId").is_none()
        && data.get("name").is_none()
    {
        return trimmed(data.get("text").and_then(Value::as_str));
    }
    trimmed(data.get("thinking").and_then(Value::as_str))
        .or_else(|| trimmed(data.get("reasoningContent").and_then(Value::as_str)))
}

fn merge_text(previous: &str, chunk: &str) -> String {
    if chunk.is_empty() {
        return previous.to_owned();
    }
    if chunk.starts_with(previous) {
        return chunk.to_owned();
    }
    format!("{previous}{chunk}")
}

#[derive(Debug, Clone, Default)]
pub struct Collected {
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<Value>,
    pub usage: Option<Value>,
    pub context_percentage: Option<f64>,
    pub error: Option<String>,
}

#[must_use]
pub fn collect_events(events: &[StreamEvent]) -> Collected {
    let mut text = String::new();
    let mut thinking = String::new();
    let mut tool_calls: Vec<(String, String, String)> = Vec::new();
    let mut usage = None;
    let mut context_percentage = None;
    let mut error = None;
    for event in events {
        let data = unwrap_event_payload(&event.payload, Some(&event.type_name)).clone();
        if let Some(thought) = thinking_text(&event.type_name, &data) {
            thinking = merge_text(&thinking, &thought);
            continue;
        }
        if (event.type_name == "assistantResponseEvent"
            || data.get("content").and_then(Value::as_str).is_some())
            && let Some(chunk) = data
                .get("content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        {
            text = merge_text(&text, chunk);
        }
        if event.type_name == "toolUseEvent"
            && let Some(id) = json_str(&data, &["toolUseId", "tool_use_id"])
        {
            if !tool_calls.iter().any(|(existing, _, _)| existing == &id) {
                tool_calls.push((
                    id.clone(),
                    json_str(&data, &["name"]).unwrap_or_else(|| "tool".to_owned()),
                    String::new(),
                ));
            }
            if let Some(row) = tool_calls
                .iter_mut()
                .find(|(existing, _, _)| existing == &id)
            {
                if let Some(name) = json_str(&data, &["name"]) {
                    row.1 = name;
                }
                if data.get("stop").is_none_or(|s| s == &json!(false))
                    && let Some(input) = data.get("input")
                    && input != &json!("")
                    && !input.is_null()
                {
                    let piece = if let Some(s) = input.as_str() {
                        s.to_owned()
                    } else {
                        input.to_string()
                    };
                    row.2.push_str(&piece);
                }
            }
        }
        if let Some(next) = usage_from_payload(&data)
            && has_real_usage(Some(&next))
        {
            usage = Some(next);
        }
        if let Some(percent) = number_field(
            &data,
            &["contextUsagePercentage", "context_usage_percentage"],
        ) {
            context_percentage = Some(percent);
        }
        if event.type_name == "exception"
            || event.type_name == "invalidStateEvent"
            || event.message_type == "exception"
        {
            error = json_str(&data, &["message", "reason", "Message"])
                .or_else(|| Some(data.to_string()));
        }
    }
    Collected {
        text,
        thinking,
        tool_calls: tool_calls
            .into_iter()
            .map(|(id, name, args)| {
                json!({
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": if args.is_empty() { "{}".to_owned() } else { args }},
                })
            })
            .collect(),
        usage,
        context_percentage,
        error,
    }
}

#[must_use]
pub fn resolve_usage(collected: &Collected, model: &str) -> Value {
    if has_real_usage(collected.usage.as_ref()) {
        return collected.usage.clone().unwrap_or_else(|| json!({}));
    }
    usage_from_context(
        collected.context_percentage.unwrap_or(0.0),
        model,
        &collected.text,
    )
    .unwrap_or_else(|| json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}))
}

#[must_use]
pub fn to_openai(events: &[StreamEvent], model: &str, id: &str) -> Value {
    let collected = collect_events(events);
    let mut message = json!({"role": "assistant", "content": if collected.text.is_empty() { Value::Null } else { json!(collected.text) }});
    if !collected.thinking.is_empty()
        && let Some(obj) = message.as_object_mut()
    {
        obj.insert("reasoning_content".to_owned(), json!(collected.thinking));
    }
    if !collected.tool_calls.is_empty()
        && let Some(obj) = message.as_object_mut()
    {
        obj.insert("tool_calls".to_owned(), json!(collected.tool_calls));
    }
    let finish = if collected.tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    let mut out = json!({
        "id": id,
        "object": "chat.completion",
        "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
        "usage": resolve_usage(&collected, model),
    });
    if let Some(err) = &collected.error
        && let Some(obj) = out.as_object_mut()
    {
        obj.insert("error".to_owned(), json!({"message": err}));
    }
    out
}

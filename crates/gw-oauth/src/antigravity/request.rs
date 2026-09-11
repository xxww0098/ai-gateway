//! OpenAI chat/completions ↔ daily-cloudcode-pa `generateContent` (hub).
//!
//! Body always includes `project` + `model` + `userAgent: "antigravity"`.
//! Extra system snapshots become a trailing **user** turn. Chat headers are
//! User-Agent only — no `Client-Metadata` / `x-goog-api-client`.

use std::collections::HashMap;

use http::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use uuid::Uuid;

use super::cache::session_id_of;
use super::{BODY_USER_AGENT, DAILY_API_URL, PROD_API_URL, request_user_agent};
use crate::Error;

mod response;
mod schema;
mod state;

pub use response::{
    Collected, OpenaiStream, antigravity_to_openai, cached_tokens_of, collect_parts,
    events_to_openai_chunks, incremental_suffix, map_antigravity_usage,
};
pub use state::{reset_pins, reset_thought_signatures};

use self::schema::tool_declarations;
use self::state::{
    lookup_thought_signature, pin_system_instruction, pin_thinking, pin_tools,
    remember_thought_signature, thought_signature_of,
};

#[cfg(test)]
mod tests;

/// Daily hub first, then IDE prod — same order as chat / loadCodeAssist.
#[must_use]
pub fn cloud_code_fallbacks(url: &str) -> Vec<String> {
    cloud_code_fallbacks_between(url, DAILY_API_URL, PROD_API_URL)
}

/// Rewrite `url` onto `prod` when it lives on `daily`. Already-prod URLs stay put.
#[must_use]
pub fn cloud_code_fallbacks_between(url: &str, daily: &str, prod: &str) -> Vec<String> {
    if !url.starts_with(daily) {
        return vec![url.to_owned()];
    }
    vec![url.to_owned(), url.replacen(daily, prod, 1)]
}

/// 5xx (and only 5xx) may retry the prod host. 4xx must not.
#[must_use]
pub fn should_retry_on_prod(status: u16) -> bool {
    status >= 500
}

/// POST a hub Cloud Code RPC: daily, then IDE prod on transport / 5xx.
pub async fn fetch_cloud_code(
    client: &reqwest::Client,
    url: &str,
    headers: HeaderMap,
    body: Vec<u8>,
) -> Result<reqwest::Response, Error> {
    fetch_cloud_code_at(client, url, DAILY_API_URL, PROD_API_URL, headers, body).await
}

/// [`fetch_cloud_code`] with caller-chosen daily/prod bases (tests use loopback).
pub async fn fetch_cloud_code_at(
    client: &reqwest::Client,
    url: &str,
    daily: &str,
    prod: &str,
    headers: HeaderMap,
    body: Vec<u8>,
) -> Result<reqwest::Response, Error> {
    let urls = cloud_code_fallbacks_between(url, daily, prod);
    let mut last_error: Option<Error> = None;
    for (i, target) in urls.iter().enumerate() {
        let last = i + 1 == urls.len();
        match client
            .post(target)
            .headers(headers.clone())
            .body(body.clone())
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success()
                    || !should_retry_on_prod(response.status().as_u16())
                    || last
                {
                    return Ok(response);
                }
            }
            Err(err) => {
                last_error = Some(err.into());
                if last {
                    return Err(last_error.take().expect("just set"));
                }
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| Error::Payload("antigravity cloud code request failed".into())))
}

fn header_value(value: &str) -> Result<HeaderValue, Error> {
    HeaderValue::from_str(value).map_err(|_| Error::Payload("invalid header value".into()))
}

fn request_id() -> String {
    format!("agent-{}", Uuid::new_v4())
}

/// Chat / generateContent headers: User-Agent only, never `x-goog-api-client`.
pub fn chat_headers(access_token: &str) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        header_value(&format!("Bearer {access_token}"))?,
    );
    headers.insert(http::header::ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        http::header::USER_AGENT,
        header_value(&request_user_agent())?,
    );
    Ok(headers)
}

/// Unary generateContent URL on the daily hub.
#[must_use]
pub fn generate_url() -> String {
    format!("{DAILY_API_URL}/v1internal:generateContent")
}

/// Streaming generateContent URL on the daily hub.
#[must_use]
pub fn stream_url() -> String {
    format!("{DAILY_API_URL}/v1internal:streamGenerateContent?alt=sse")
}

/// Cloud Code 400s if `maxOutputTokens` exceeds the runtime-id cap.
#[must_use]
pub fn max_output_tokens(model: &str) -> i64 {
    if model.starts_with("claude-") {
        64_000
    } else if model.starts_with("gpt-oss-") {
        32_768
    } else if model.starts_with("gemini-3.1-pro") || model == "gemini-pro-agent" {
        65_535
    } else if model.starts_with("gemini-") {
        65_536
    } else {
        8192
    }
}

/// Gemini 3 / `gemini-pro-agent` need thoughtSignature on functionCall groups.
#[must_use]
pub fn gemini_requires_thought_signature(model: &str) -> bool {
    let Some(rest) = model.strip_prefix("gemini-") else {
        return false;
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return true;
    }
    digits.parse::<u32>().map(|n| n >= 3).unwrap_or(true)
}

/// Claude / GPT-OSS custom-tool bridge: `[A-Za-z0-9_-]`, cap 64.
#[must_use]
pub fn sanitize_tool_call_id(id: &str, fallback_name: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if !cleaned.is_empty() {
        return cleaned;
    }
    let fallback: String = fallback_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if fallback.is_empty() {
        "tool".to_owned()
    } else {
        fallback
    }
}

/// `FunctionResponse.response` is a singular protobuf Struct.
#[must_use]
pub fn function_response_payload(value: &Value) -> Value {
    if is_plain_object(value) {
        return value.clone();
    }
    if let Some(s) = value.as_str() {
        if s.trim().is_empty() {
            return json!({});
        }
        match serde_json::from_str::<Value>(s) {
            Ok(parsed) if is_plain_object(&parsed) => parsed,
            Ok(parsed) => json!({"result": parsed}),
            Err(_) => json!({"text": s}),
        }
    } else if value.is_null() {
        json!({})
    } else {
        json!({"result": value})
    }
}

/// Translate OpenAI chat/completions into Cloud Code `generateContent`.
pub fn openai_to_antigravity(
    payload: &Value,
    project_id: &str,
    session_id: Option<&str>,
) -> Result<Value, Error> {
    let project = trimmed(project_id)
        .ok_or_else(|| Error::Payload("antigravity generateContent requires project_id".into()))?;
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .and_then(trimmed)
        .ok_or_else(|| Error::Payload("antigravity generateContent requires a model".into()))?;
    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let pinned_session = session_id_of(payload, session_id);
    let requires_sig = gemini_requires_thought_signature(&model);
    let mut system_parts = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    let mut dropped_tool_call_ids: HashMap<String, String> = HashMap::new();

    for message in &messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        if role == "system" || role == "developer" {
            system_parts.extend(parts_from_content(message.get("content")));
            continue;
        }
        if role == "tool" {
            let name = message
                .get("name")
                .and_then(Value::as_str)
                .and_then(trimmed)
                .unwrap_or_else(|| "tool".to_owned());
            let raw_id = message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            if requires_sig
                && let Some(dropped_args) = dropped_tool_args(&dropped_tool_call_ids, raw_id, &name)
            {
                append_turn(
                    &mut contents,
                    "user",
                    vec![observation_part(
                        &name,
                        dropped_args,
                        &tool_result_text(message),
                    )],
                );
                continue;
            }
            append_turn(
                &mut contents,
                "user",
                vec![function_response_part(message, &model)],
            );
            continue;
        }
        let mut parts = Vec::new();
        let mut built = Vec::new();
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                if let Some(part) = function_call_part(call, &pinned_session, message, &model) {
                    built.push((part, call.clone()));
                }
            }
        }
        let group_is_signed = built.first().is_some_and(|(part, _)| {
            part.get("thoughtSignature")
                .and_then(Value::as_str)
                .is_some()
        });
        if requires_sig && !built.is_empty() && !group_is_signed {
            for (part, call) in &built {
                let name = part
                    .pointer("/functionCall/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let args = part
                    .pointer("/functionCall/args")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                remember_dropped_tool_call(
                    &mut dropped_tool_call_ids,
                    call.get("id").and_then(Value::as_str),
                    name,
                    &args,
                );
            }
        } else {
            for (part, _) in built {
                parts.push(part);
            }
        }
        parts.extend(parts_from_content(message.get("content")));
        let wire_role = if role == "assistant" { "model" } else { "user" };
        append_turn(&mut contents, wire_role, parts);
    }

    if contents.is_empty() {
        let text = payload
            .get("input")
            .and_then(Value::as_str)
            .and_then(trimmed)
            .unwrap_or_default();
        append_turn(&mut contents, "user", vec![json!({"text": text})]);
    }
    let pinned = pin_system_instruction(&pinned_session, &system_parts);
    if contents
        .first()
        .and_then(|c| c.get("role"))
        .and_then(Value::as_str)
        == Some("model")
    {
        contents.insert(0, json!({"role": "user", "parts": [{"text": "Hello"}]}));
    }
    if let Some(extra) = &pinned.extra {
        append_turn(&mut contents, "user", vec![json!({"text": extra})]);
    }

    let mut request = json!({
        "contents": merge_adjacent_contents(contents),
        "sessionId": pinned_session,
    });
    if !pinned.parts.is_empty() {
        request["systemInstruction"] = json!({"role": "user", "parts": pinned.parts});
    }
    let tools = pin_tools(
        &pinned_session,
        tool_declarations(payload.get("tools"), &model),
    );
    if let Some(tools) = &tools {
        request["tools"] = tools.clone();
    }
    let thinking = pin_thinking(
        &pinned_session,
        thinking_config(
            &model,
            payload
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .and_then(trimmed)
                .as_deref(),
        ),
    );
    let mut generation_config = json!({
        "maxOutputTokens": clamp_max_output_tokens(&model, payload.get("max_tokens")),
    });
    if let Some(thinking) = thinking {
        generation_config["thinkingConfig"] = thinking;
    }
    request["generationConfig"] = generation_config;
    if is_claude_model(&model) {
        request["toolConfig"] = json!({"functionCallingConfig": {"mode": "VALIDATED"}});
    } else if tools.is_some() {
        request["toolConfig"] = json!({
            "functionCallingConfig": {"mode": gemini_tool_choice_mode(payload.get("tool_choice"))}
        });
    }

    Ok(json!({
        "model": model,
        "project": project,
        "userAgent": BODY_USER_AGENT,
        "requestType": "agent",
        "requestId": request_id(),
        "request": request,
    }))
}

fn is_claude_model(model: &str) -> bool {
    model.starts_with("claude-")
}

fn is_gpt_oss_model(model: &str) -> bool {
    model.starts_with("gpt-oss-")
}

fn uses_legacy_tool_parameters(model: &str) -> bool {
    is_claude_model(model) || is_gpt_oss_model(model)
}

fn tool_call_id_needed(model: &str) -> bool {
    uses_legacy_tool_parameters(model)
}

fn clamp_max_output_tokens(model: &str, requested: Option<&Value>) -> i64 {
    let cap = max_output_tokens(model);
    let n = requested.and_then(as_f64).unwrap_or(0.0);
    if n.is_finite() && n > 0.0 {
        (n.round() as i64).min(cap)
    } else {
        cap
    }
}

fn function_call_part(
    call: &Value,
    session_id: &str,
    message: &Value,
    model: &str,
) -> Option<Value> {
    let name = call
        .pointer("/function/name")
        .and_then(Value::as_str)
        .and_then(trimmed)?;
    let args = try_json(call.pointer("/function/arguments").unwrap_or(&Value::Null));
    let shared = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .filter(|c| c.len() == 1)
        .and_then(|_| thought_signature_of(&[message]));
    let signature = thought_signature_of(&[call]).or(shared).or_else(|| {
        lookup_thought_signature(
            session_id,
            call.get("id").and_then(Value::as_str),
            &name,
            &args,
        )
    });
    let mut function_call = json!({"name": name, "args": args});
    if tool_call_id_needed(model) {
        function_call["id"] = json!(sanitize_tool_call_id(
            call.get("id").and_then(Value::as_str).unwrap_or(""),
            &name
        ));
    }
    let mut part = json!({"functionCall": function_call});
    if let Some(signature) = signature {
        part["thoughtSignature"] = json!(signature);
        remember_thought_signature(
            Some(session_id),
            call.get("id").and_then(Value::as_str),
            &name,
            &args,
            &signature,
        );
    }
    Some(part)
}

fn function_response_part(message: &Value, model: &str) -> Value {
    let name = message
        .get("name")
        .and_then(Value::as_str)
        .and_then(trimmed)
        .unwrap_or_else(|| "tool".to_owned());
    let mut function_response = json!({
        "name": name,
        "response": function_response_payload(message.get("content").unwrap_or(&Value::Null)),
    });
    if tool_call_id_needed(model) {
        function_response["id"] = json!(sanitize_tool_call_id(
            message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or(""),
            &name
        ));
    }
    json!({"functionResponse": function_response})
}

fn tool_result_text(message: &Value) -> String {
    let content = message.get("content").unwrap_or(&Value::Null);
    if let Some(s) = content.as_str() {
        return s.to_owned();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .or_else(|| item.get("text").and_then(Value::as_str).and_then(trimmed))
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    if content.is_null() {
        String::new()
    } else {
        content.to_string()
    }
}

fn remember_dropped_tool_call(
    dropped: &mut HashMap<String, String>,
    raw_id: Option<&str>,
    name: &str,
    args: &Value,
) {
    let args_text = args.to_string();
    if let Some(raw_id) = raw_id.filter(|s| !s.is_empty()) {
        dropped.insert(raw_id.to_owned(), args_text.clone());
        dropped.insert(sanitize_tool_call_id(raw_id, name), args_text);
    } else {
        dropped.insert(format!("empty:{name}"), args_text);
    }
}

fn dropped_tool_args<'a>(
    dropped: &'a HashMap<String, String>,
    raw_id: &str,
    name: &str,
) -> Option<&'a String> {
    let sanitized = sanitize_tool_call_id(raw_id, name);
    dropped
        .get(raw_id)
        .or_else(|| dropped.get(&sanitized))
        .or_else(|| {
            if raw_id.is_empty() {
                dropped.get(&format!("empty:{name}"))
            } else {
                None
            }
        })
}

fn observation_part(name: &str, dropped_args: &str, response_text: &str) -> Value {
    let label = if dropped_args == "{}" {
        format!("`{name}`")
    } else {
        format!("`{name}` ({dropped_args})")
    };
    json!({"text": format!("[Observation from {label}:\n{response_text}]")})
}

fn image_part(url: &str) -> Option<Value> {
    let raw = trimmed(url)?;
    if let Some(rest) = raw.strip_prefix("data:")
        && let Some((mime, data)) = rest.split_once(";base64,")
    {
        return Some(json!({"inlineData": {"mimeType": mime, "data": data}}));
    }
    Some(json!({"fileData": {"fileUri": raw}}))
}

fn parts_from_content(content: Option<&Value>) -> Vec<Value> {
    let Some(content) = content else {
        return Vec::new();
    };
    if let Some(s) = content.as_str() {
        return if s.is_empty() {
            Vec::new()
        } else {
            vec![json!({"text": s})]
        };
    }
    let Some(arr) = content.as_array() else {
        if content.is_null() {
            return Vec::new();
        }
        return vec![json!({"text": content.to_string()})];
    };
    let mut parts = Vec::new();
    for item in arr {
        if let Some(s) = item.as_str() {
            if !s.is_empty() {
                parts.push(json!({"text": s}));
            }
        } else if item.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(t) = item
                .get("text")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                parts.push(json!({"text": t}));
            }
        } else if item.get("type").and_then(Value::as_str) == Some("image_url") {
            let url = item
                .pointer("/image_url/url")
                .and_then(Value::as_str)
                .or_else(|| item.get("image_url").and_then(Value::as_str));
            if let Some(url) = url
                && let Some(part) = image_part(url)
            {
                parts.push(part);
            }
        }
    }
    parts
}

fn gemini_tool_choice_mode(tool_choice: Option<&Value>) -> &'static str {
    let value = tool_choice.and_then(Value::as_str).or_else(|| {
        tool_choice
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
    });
    match value {
        Some("none") => "NONE",
        Some("required" | "any" | "function") => "ANY",
        _ => "AUTO",
    }
}

fn uses_thinking_budget_wire(model: &str) -> bool {
    model.starts_with("gemini-3.5-flash")
        || model == "gemini-3-flash-agent"
        || model.starts_with("gemini-3.1-pro")
        || model == "gemini-pro-agent"
}

fn thinking_config(model: &str, effort: Option<&str>) -> Option<Value> {
    if is_claude_model(model) || is_gpt_oss_model(model) {
        return None;
    }
    let e = effort.and_then(trimmed);
    if uses_thinking_budget_wire(model) {
        let e = e?;
        if e == "off" {
            return None;
        }
        let high = e == "high" || e == "xhigh";
        if model.starts_with("gemini-3.1-pro") || model == "gemini-pro-agent" {
            return Some(
                json!({"includeThoughts": true, "thinkingBudget": if high { 10_001 } else { 1_001 }}),
            );
        }
        let budget = if high {
            10_000
        } else if e == "medium" {
            4_000
        } else {
            1_000
        };
        return Some(json!({"includeThoughts": true, "thinkingBudget": budget}));
    }
    e.map(|level| json!({"thinkingLevel": level}))
}

fn append_turn(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    if let Some(last) = contents.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
    {
        if let Some(existing) = last.get_mut("parts").and_then(Value::as_array_mut) {
            existing.extend(parts);
        }
        return;
    }
    contents.push(json!({"role": role, "parts": parts}));
}

fn merge_adjacent_contents(contents: Vec<Value>) -> Vec<Value> {
    let mut merged = Vec::new();
    for turn in contents {
        let role = turn.get("role").and_then(Value::as_str).unwrap_or("");
        let parts = turn
            .get("parts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if role.is_empty() || parts.is_empty() {
            continue;
        }
        append_turn(&mut merged, role, parts);
    }
    merged
}

fn try_json(value: &Value) -> Value {
    if value.is_object() {
        return value.clone();
    }
    if let Some(s) = value.as_str() {
        if s.trim().is_empty() {
            return json!({});
        }
        return serde_json::from_str(s).unwrap_or_else(|_| json!({"text": s}));
    }
    json!({})
}

fn is_plain_object(value: &Value) -> bool {
    value.is_object()
}

fn trimmed(value: &str) -> Option<String> {
    let t = value.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_owned())
    }
}

fn as_count(value: &Value) -> Option<i64> {
    if let Some(n) = as_f64(value)
        && n.is_finite()
        && n >= 0.0
    {
        return Some(n.round() as i64);
    }
    None
}

fn as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|n| n as f64))
        .or_else(|| value.as_u64().map(|n| n as f64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

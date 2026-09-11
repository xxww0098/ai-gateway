//! OpenAI-facing normalization: part collection, usage mapping, and streaming
//! chunk translation for generateContent responses.

use chrono::Utc;
use serde_json::{Map, Value, json};

use super::state::{
    attach_thought_signature_fields, remember_thought_signature, thought_signature_of,
};
use super::{as_count, sanitize_tool_call_id, trimmed};

fn finish_reason(raw: Option<&str>) -> String {
    let value = raw.unwrap_or("").to_ascii_uppercase();
    if value == "MAX_TOKENS" {
        "length".to_owned()
    } else if value.contains("TOOL") || value == "MALFORMED_FUNCTION_CALL" {
        "tool_calls".to_owned()
    } else {
        "stop".to_owned()
    }
}

fn openai_chunk(
    id: &str,
    model: &str,
    delta: Value,
    finish_reason: Value,
    usage: Option<Value>,
) -> Value {
    let mut chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason,
        }],
    });
    if let Some(usage) = usage {
        chunk["usage"] = usage;
    }
    chunk
}

/// Collect visible text / tool calls from a generateContent body.
#[must_use]
pub fn collect_parts(body: &Value, session_id: Option<&str>) -> Collected {
    let response = body.get("response").unwrap_or(body);
    let candidate = response
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|c| c.first());
    let parts = candidate
        .and_then(|c| c.pointer("/content/parts"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut pending_thought_sig: Option<String> = None;
    for part in &parts {
        let part_sig =
            thought_signature_of(&[part, part.get("functionCall").unwrap_or(&Value::Null)]);
        if part
            .get("thought")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if let Some(sig) = part_sig {
                pending_thought_sig = Some(sig);
            }
            continue;
        }
        if let Some(t) = part.get("text").and_then(Value::as_str) {
            text.push_str(t);
        }
        if let Some(name) = part
            .pointer("/functionCall/name")
            .and_then(Value::as_str)
            .and_then(trimmed)
        {
            let signature = part_sig.or_else(|| pending_thought_sig.take());
            pending_thought_sig = None;
            let fc = part.get("functionCall").unwrap_or(&Value::Null);
            let id = fc
                .get("id")
                .and_then(Value::as_str)
                .and_then(trimmed)
                .map(|raw| sanitize_tool_call_id(&raw, &name))
                .unwrap_or_else(|| format!("call_{}", tool_calls.len() + 1));
            let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
            let mut call = json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": args.to_string(),
                }
            });
            if let Some(signature) = signature {
                attach_thought_signature_fields(&mut call, &signature);
                remember_thought_signature(
                    session_id,
                    call.get("id").and_then(Value::as_str),
                    &name,
                    &args,
                    &signature,
                );
            }
            tool_calls.push(call);
        }
    }
    Collected {
        text,
        tool_calls,
        finish_reason: finish_reason(
            candidate
                .and_then(|c| c.get("finishReason"))
                .and_then(Value::as_str),
        ),
        raw_finish: candidate
            .and_then(|c| c.get("finishReason"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        usage: response
            .get("usageMetadata")
            .cloned()
            .or_else(|| body.get("usageMetadata").cloned()),
    }
}

/// Gemini usage plus CLI stats aliases (`cache_read_tokens` / `cacheReadTokens`).
#[must_use]
pub fn cached_tokens_of(usage: &Value) -> Option<i64> {
    if !usage.is_object() {
        return None;
    }
    for key in [
        "cachedContentTokenCount",
        "cached_content_token_count",
        "cachedTokenCount",
        "cached_tokens",
        "cache_read_tokens",
        "cacheReadTokens",
        "cacheReadInputTokens",
    ] {
        if let Some(n) = usage.get(key).and_then(as_count) {
            return Some(n);
        }
    }
    let details = usage
        .get("cacheTokensDetails")
        .or_else(|| usage.get("cache_tokens_details"))
        .and_then(Value::as_array)?;
    let mut sum = 0i64;
    let mut any = false;
    for row in details {
        let Some(next) = row
            .get("tokenCount")
            .or_else(|| row.get("token_count"))
            .and_then(as_count)
        else {
            continue;
        };
        sum += next;
        any = true;
    }
    any.then_some(sum)
}

/// OpenAI chat.completion usage. Thoughts count as completion tokens.
#[must_use]
pub fn map_antigravity_usage(usage: &Value) -> Option<Value> {
    if !usage.is_object() {
        return None;
    }
    let prompt = usage
        .get("promptTokenCount")
        .or_else(|| usage.get("prompt_token_count"))
        .and_then(as_count)
        .unwrap_or(0);
    let candidates = usage
        .get("candidatesTokenCount")
        .or_else(|| usage.get("candidates_token_count"))
        .and_then(as_count)
        .unwrap_or(0);
    let thoughts = usage
        .get("thoughtsTokenCount")
        .or_else(|| usage.get("thoughts_token_count"))
        .and_then(as_count)
        .unwrap_or(0);
    let completion = candidates + thoughts;
    let total = usage
        .get("totalTokenCount")
        .or_else(|| usage.get("total_token_count"))
        .and_then(as_count)
        .unwrap_or(prompt + completion);
    let mut mapped = json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    });
    if thoughts != 0 {
        mapped["completion_tokens_details"] = json!({"reasoning_tokens": thoughts});
    }
    if let Some(cached) = cached_tokens_of(usage) {
        mapped["prompt_tokens_details"] = json!({"cached_tokens": cached});
    }
    Some(mapped)
}

/// Google SSE is cumulative; OpenAI deltas are suffixes. A shorter later frame is a reset.
#[must_use]
pub fn incremental_suffix(next: &str, previous: &str) -> String {
    if next.is_empty() {
        return String::new();
    }
    if !previous.is_empty() && next.starts_with(previous) {
        next[previous.len()..].to_owned()
    } else {
        next.to_owned()
    }
}

/// Non-streaming OpenAI chat.completion.
#[must_use]
pub fn antigravity_to_openai(
    body: &Value,
    model: Option<&str>,
    id: Option<&str>,
    session_id: Option<&str>,
) -> Value {
    let collected = collect_parts(body, session_id);
    let mut message = json!({
        "role": "assistant",
        "content": if collected.text.is_empty() { Value::Null } else { Value::String(collected.text.clone()) },
    });
    if !collected.tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(collected.tool_calls.clone());
    }
    let finish = if collected.tool_calls.is_empty() {
        collected.finish_reason
    } else {
        "tool_calls".to_owned()
    };
    json!({
        "id": id.map_or_else(|| format!("chatcmpl-{}", Utc::now().timestamp_millis()), str::to_owned),
        "object": "chat.completion",
        "model": model.unwrap_or(""),
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish,
        }],
        "usage": map_antigravity_usage(collected.usage.as_ref().unwrap_or(&Value::Null)),
    })
}

/// Per-stream mapper: cumulative Google frames → incremental OpenAI chunks.
pub struct OpenaiStream {
    model: String,
    id: String,
    session_id: Option<String>,
    emitted_text: String,
    emitted_tool_args: Vec<Option<String>>,
    last_usage: Option<Value>,
    last_finish: String,
    saw_tools: bool,
}

impl OpenaiStream {
    #[must_use]
    pub fn new(model: Option<&str>, id: Option<&str>, session_id: Option<&str>) -> Self {
        Self {
            model: model.unwrap_or("").to_owned(),
            id: id
                .map(str::to_owned)
                .unwrap_or_else(|| format!("chatcmpl-{}", Utc::now().timestamp_millis())),
            session_id: session_id.map(str::to_owned),
            emitted_text: String::new(),
            emitted_tool_args: Vec::new(),
            last_usage: None,
            last_finish: "stop".to_owned(),
            saw_tools: false,
        }
    }

    /// Push one Google frame. `None` when the frame produced no visible delta.
    pub fn push(&mut self, body: &Value) -> Option<Value> {
        let collected = collect_parts(body, self.session_id.as_deref());
        if let Some(usage) = collected.usage {
            self.last_usage = Some(usage);
        }
        if collected.raw_finish.is_some() {
            self.last_finish = collected.finish_reason;
        }
        if !collected.tool_calls.is_empty() {
            self.saw_tools = true;
            self.last_finish = "tool_calls".to_owned();
        }
        let mut delta = Map::new();
        let text_delta = incremental_suffix(&collected.text, &self.emitted_text);
        if !text_delta.is_empty() {
            delta.insert("content".into(), Value::String(text_delta));
            self.emitted_text = collected.text;
        }
        if !collected.tool_calls.is_empty() {
            let mut calls = Vec::new();
            for (index, call) in collected.tool_calls.iter().enumerate() {
                let args = call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if self.emitted_tool_args.len() <= index {
                    self.emitted_tool_args.resize(index + 1, None);
                }
                let first = self.emitted_tool_args[index].is_none();
                let prev = self.emitted_tool_args[index].as_deref().unwrap_or("");
                let arg_delta = incremental_suffix(args, if first { "" } else { prev });
                if !first && arg_delta.is_empty() {
                    continue;
                }
                self.emitted_tool_args[index] = Some(args.to_owned());
                let mut next = json!({
                    "index": index,
                    "id": call.get("id").cloned().unwrap_or(Value::Null),
                    "type": "function",
                    "function": {
                        "name": call.pointer("/function/name").cloned().unwrap_or(Value::Null),
                        "arguments": arg_delta,
                    }
                });
                if first && let Some(sig) = call.get("thoughtSignature").and_then(Value::as_str) {
                    attach_thought_signature_fields(&mut next, sig);
                }
                calls.push(next);
            }
            if !calls.is_empty() {
                delta.insert("tool_calls".into(), Value::Array(calls));
            }
        }
        if !delta.contains_key("content") && !delta.contains_key("tool_calls") {
            return None;
        }
        Some(openai_chunk(
            &self.id,
            &self.model,
            Value::Object(delta),
            Value::Null,
            None,
        ))
    }

    /// Terminal OpenAI chunk with usage.
    #[must_use]
    pub fn finish(&self) -> Value {
        let finish = if self.saw_tools {
            "tool_calls"
        } else {
            self.last_finish.as_str()
        };
        openai_chunk(
            &self.id,
            &self.model,
            json!({}),
            Value::String(finish.to_owned()),
            self.last_usage.as_ref().and_then(map_antigravity_usage),
        )
    }
}

/// Map a list of Google SSE events into OpenAI chunks, including the terminal frame.
#[must_use]
pub fn events_to_openai_chunks(
    events: &[Value],
    model: Option<&str>,
    id: Option<&str>,
    session_id: Option<&str>,
) -> Vec<Value> {
    let mut stream = OpenaiStream::new(model, id, session_id);
    let mut chunks = Vec::new();
    for event in events {
        if let Some(chunk) = stream.push(event) {
            chunks.push(chunk);
        }
    }
    chunks.push(stream.finish());
    chunks
}

/// Visible text, tool calls, and usage pulled off one generateContent body.
#[derive(Debug, Clone)]
pub struct Collected {
    pub text: String,
    pub tool_calls: Vec<Value>,
    pub finish_reason: String,
    pub raw_finish: Option<String>,
    pub usage: Option<Value>,
}

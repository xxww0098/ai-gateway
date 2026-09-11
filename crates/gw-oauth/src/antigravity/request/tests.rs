use std::sync::{Mutex, MutexGuard};

use serde_json::{Value, json};

use super::{
    antigravity_to_openai, cached_tokens_of, chat_headers, cloud_code_fallbacks,
    cloud_code_fallbacks_between, events_to_openai_chunks, function_response_payload,
    gemini_requires_thought_signature, incremental_suffix, max_output_tokens,
    openai_to_antigravity, reset_pins, reset_thought_signatures, sanitize_tool_call_id,
    should_retry_on_prod,
};

mod convert_body;
mod hub_endpoints;
mod pins;
mod thought_signatures;
mod tools;
mod usage_stream;

fn isolated() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_pins();
    reset_thought_signatures();
    guard
}

fn convert(payload: Value, project: &str) -> Value {
    openai_to_antigravity(&payload, project, None).expect("convert")
}

fn request_of(body: &Value) -> &Value {
    body.get("request").expect("request")
}

fn contents_of(body: &Value) -> &[Value] {
    request_of(body)
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn roles_of(body: &Value) -> Vec<&str> {
    contents_of(body)
        .iter()
        .filter_map(|c| c.get("role").and_then(Value::as_str))
        .collect()
}

fn json_blob(value: &Value) -> String {
    value.to_string()
}

fn read_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "Read",
            "description": "read a file",
            "parameters": {
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }
        }
    })
}

fn grep_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "Grep",
            "description": "search",
            "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}
        }
    })
}

fn schema_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "Read",
            "description": "read a file",
            "parameters": {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$defs": {
                    "Path": {"type": "string", "format": "uri-reference", "nullable": true}
                },
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": {"$ref": "#/$defs/Path"},
                    "kind": {"anyOf": [{"type": "string"}, {"type": "null"}]},
                    "mode": {"type": ["string", "null"]},
                    "flag": {"type": "string", "enum": ["a", 1]}
                },
                "required": ["path"]
            }
        }
    })
}

fn google_event(parts: Value, usage: Option<Value>, finish: Option<&str>) -> Value {
    let mut candidate = json!({"content": {"parts": parts}});
    if let Some(finish) = finish {
        candidate["finishReason"] = json!(finish);
    }
    let mut response = json!({"candidates": [candidate]});
    if let Some(usage) = usage {
        response["usageMetadata"] = usage;
    }
    json!({"response": response})
}

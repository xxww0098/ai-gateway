//! OpenAI Completions ↔ Cursor AgentService/Run.

use std::collections::HashMap;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::cache::{conversation_id_with, peel_fast_suffix, pin_system_prefix, stable_id};
use super::proto::{
    McpTool, ModelParam, ServerMessage, TokenUsage, ToolResult, decode_agent_server_message,
    encode_agent_client_message, encode_agent_run_request, encode_assistant_step,
    encode_cancel_action, encode_conversation_state, encode_conversation_turn, encode_exec_throw,
    encode_json_value_bytes, encode_kv_client_message, encode_mcp_tool_step,
    encode_requested_model, encode_thinking_step, encode_user_message, frame_connect,
    split_connect_frames,
};
use super::{REASONING, wire_model_id};

#[cfg(test)]
mod tests;

/// Built AgentClientMessage plus the blob store the Run loop answers KV from.
#[derive(Debug, Clone)]
pub struct CursorRun {
    pub conversation_id: String,
    pub model_id: String,
    pub picker_model: String,
    pub system_prompt: String,
    pub pinned_system: String,
    pub extra_system: String,
    pub user_text: String,
    pub tools: Vec<(String, String)>,
    pub request_bytes: Vec<u8>,
    pub blob_store: HashMap<String, Vec<u8>>,
    pub stream: bool,
}

#[derive(Debug, Clone)]
struct Step {
    kind: StepKind,
    text: String,
    tool_call_id: String,
    tool_name: String,
    arguments: Value,
    result: Option<ToolResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepKind {
    Thinking,
    AssistantText,
    ToolCall,
}

impl Step {
    fn fingerprint(&self) -> Value {
        let mut obj = Map::new();
        obj.insert(
            "kind".to_owned(),
            Value::String(
                match self.kind {
                    StepKind::Thinking => "thinking",
                    StepKind::AssistantText => "assistantText",
                    StepKind::ToolCall => "toolCall",
                }
                .to_owned(),
            ),
        );
        if !self.text.is_empty() {
            obj.insert("text".to_owned(), Value::String(self.text.clone()));
        }
        if !self.tool_call_id.is_empty() {
            obj.insert(
                "toolCallId".to_owned(),
                Value::String(self.tool_call_id.clone()),
            );
        }
        if !self.tool_name.is_empty() {
            obj.insert("toolName".to_owned(), Value::String(self.tool_name.clone()));
        }
        if self.kind == StepKind::ToolCall {
            obj.insert("arguments".to_owned(), self.arguments.clone());
        }
        if let Some(result) = &self.result {
            obj.insert(
                "result".to_owned(),
                serde_json::json!({
                    "content": result.content,
                    "isError": result.is_error,
                }),
            );
        }
        Value::Object(obj)
    }
}

fn text_of(content: &Value) -> String {
    match content {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                if part.get("type").and_then(Value::as_str) == Some("text")
                    || part.get("text").and_then(Value::as_str).is_some()
                {
                    part.get("text").and_then(Value::as_str).map(str::to_owned)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_tool_args(raw: &Value) -> Value {
    match raw {
        Value::Object(_) => raw.clone(),
        Value::String(s) if s.trim().is_empty() => Value::Object(Map::new()),
        Value::String(s) => {
            serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({ "__raw": s }))
        }
        _ => Value::Object(Map::new()),
    }
}

fn store_blob(data: &[u8], blob_store: &mut HashMap<String, Vec<u8>>) -> Vec<u8> {
    let digest = Sha256::digest(data);
    blob_store.insert(hex::encode(digest), data.to_vec());
    digest.to_vec()
}

fn openai_tools(payload: &Value) -> Vec<McpTool> {
    let mut tools = Vec::new();
    let Some(list) = payload.get("tools").and_then(Value::as_array) else {
        return tools;
    };
    for tool in list {
        let fn_obj = tool.get("function").unwrap_or(tool);
        let Some(name) = fn_obj.get("name").and_then(Value::as_str) else {
            continue;
        };
        let description = fn_obj
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let schema = encode_json_value_bytes(
            fn_obj
                .get("parameters")
                .unwrap_or(&Value::Object(Map::new())),
        );
        tools.push(McpTool {
            name: name.to_owned(),
            tool_name: name.to_owned(),
            provider_identifier: "dsh".to_owned(),
            description,
            input_schema: Some(schema),
        });
    }
    tools
}

struct ParsedTurns {
    system_prompt: String,
    turns: Vec<Turn>,
    user_text: String,
    in_flight: Option<Turn>,
}

struct Turn {
    user_text: String,
    steps: Vec<Step>,
}

fn parse_turns(messages: &[Value]) -> ParsedTurns {
    let mut system_parts = Vec::new();
    let mut turns = Vec::new();
    let mut current: Option<Turn> = None;
    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
        if role == "system" || role == "developer" {
            let text = text_of(msg.get("content").unwrap_or(&Value::Null));
            if !text.is_empty() {
                system_parts.push(text);
            }
            continue;
        }
        if role == "user" {
            if let Some(done) = current.take() {
                turns.push(done);
            }
            current = Some(Turn {
                user_text: text_of(msg.get("content").unwrap_or(&Value::Null)),
                steps: Vec::new(),
            });
            continue;
        }
        let Some(cur) = current.as_mut() else {
            continue;
        };
        if role == "assistant" {
            if let Some(thinking) = msg.get("reasoning_content").and_then(Value::as_str)
                && !thinking.trim().is_empty()
            {
                cur.steps.push(Step {
                    kind: StepKind::Thinking,
                    text: thinking.to_owned(),
                    tool_call_id: String::new(),
                    tool_name: String::new(),
                    arguments: Value::Object(Map::new()),
                    result: None,
                });
            }
            let text = text_of(msg.get("content").unwrap_or(&Value::Null));
            if !text.is_empty() {
                cur.steps.push(Step {
                    kind: StepKind::AssistantText,
                    text,
                    tool_call_id: String::new(),
                    tool_name: String::new(),
                    arguments: Value::Object(Map::new()),
                    result: None,
                });
            }
            if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let fn_obj = call
                        .get("function")
                        .cloned()
                        .unwrap_or(Value::Object(Map::new()));
                    cur.steps.push(Step {
                        kind: StepKind::ToolCall,
                        text: String::new(),
                        tool_call_id: id,
                        tool_name: fn_obj
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_owned(),
                        arguments: parse_tool_args(fn_obj.get("arguments").unwrap_or(&Value::Null)),
                        result: None,
                    });
                }
            }
            continue;
        }
        if role == "tool" {
            let id = msg
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let result = ToolResult {
                content: text_of(msg.get("content").unwrap_or(&Value::Null)),
                is_error: msg.get("is_error").and_then(Value::as_bool) == Some(true),
            };
            if let Some(existing) = cur
                .steps
                .iter_mut()
                .find(|step| step.kind == StepKind::ToolCall && step.tool_call_id == id)
            {
                existing.result = Some(result);
            } else {
                cur.steps.push(Step {
                    kind: StepKind::ToolCall,
                    text: String::new(),
                    tool_call_id: id,
                    tool_name: String::new(),
                    arguments: Value::Object(Map::new()),
                    result: Some(result),
                });
            }
        }
    }
    let mut user_text = String::new();
    let mut in_flight = None;
    if let Some(cur) = current {
        let last_is_tool = cur
            .steps
            .last()
            .is_some_and(|last| last.kind == StepKind::ToolCall);
        if cur.steps.is_empty() || last_is_tool {
            user_text = cur.user_text.clone();
            in_flight = Some(cur);
        } else {
            turns.push(cur);
        }
    }
    ParsedTurns {
        system_prompt: system_parts.join("\n"),
        turns,
        user_text,
        in_flight,
    }
}

fn vendor_effort(value: &str) -> Option<String> {
    let key = value.trim();
    if key.is_empty() {
        return None;
    }
    for (from, to) in REASONING {
        if *from == key {
            return Some((*to).to_owned());
        }
    }
    Some(key.to_owned())
}

/// `RequestedModel.parameters` for reasoning + Fast.
#[must_use]
pub fn model_parameters(payload: &Value) -> Vec<ModelParam> {
    let mut parameters = Vec::new();
    if let Some(effort) = payload
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .and_then(vendor_effort)
    {
        parameters.push(ModelParam {
            id: "reasoning".to_owned(),
            value: effort,
        });
    }
    if let Some(model) = payload.get("model").and_then(Value::as_str)
        && peel_fast_suffix(model).1
    {
        parameters.push(ModelParam {
            id: "fast".to_owned(),
            value: "true".to_owned(),
        });
    }
    parameters
}

/// Completions payload → Connect `AgentClientMessage` bytes.
#[must_use]
pub fn openai_to_cursor(payload: &Value, conversation_id: Option<&str>) -> CursorRun {
    let resolved_id = conversation_id_with(payload, conversation_id);
    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let parsed = parse_turns(&messages);
    let (pinned, extra) = pin_system_prefix(&resolved_id, &parsed.system_prompt);
    let mut blob_store = HashMap::new();
    let system_prompt = if extra.is_empty() {
        pinned.clone()
    } else {
        format!("{pinned}\n\n{extra}")
    };
    let mut root_prompt_blobs = Vec::new();
    if !pinned.is_empty() {
        let blob = serde_json::to_vec(&serde_json::json!({
            "role": "system",
            "content": pinned,
        }))
        .unwrap_or_default();
        root_prompt_blobs.push(store_blob(&blob, &mut blob_store));
    }
    if !extra.is_empty() {
        let blob = serde_json::to_vec(&serde_json::json!({
            "role": "system",
            "content": extra,
        }))
        .unwrap_or_default();
        root_prompt_blobs.push(store_blob(&blob, &mut blob_store));
    }

    let mut turn_blobs = Vec::new();
    for (index, turn) in parsed.turns.iter().enumerate() {
        let steps_json =
            serde_json::to_string(&turn.steps.iter().map(Step::fingerprint).collect::<Vec<_>>())
                .unwrap_or_default();
        let index_s = index.to_string();
        let message_id = stable_id(&[&resolved_id, "turn", &index_s, &turn.user_text, &steps_json]);
        let user_blob = store_blob(
            &encode_user_message(&turn.user_text, &message_id, None),
            &mut blob_store,
        );
        let mut step_blobs = Vec::new();
        for step in &turn.steps {
            let encoded = match step.kind {
                StepKind::Thinking => encode_thinking_step(&step.text),
                StepKind::ToolCall => encode_mcp_tool_step(
                    &step.tool_name,
                    &step.tool_call_id,
                    &step.arguments,
                    step.result.as_ref(),
                ),
                StepKind::AssistantText => encode_assistant_step(&step.text),
            };
            step_blobs.push(store_blob(&encoded, &mut blob_store));
        }
        let request_id = stable_id(&[&resolved_id, "req", &index_s, &turn.user_text, &steps_json]);
        turn_blobs.push(store_blob(
            &encode_conversation_turn(&user_blob, &step_blobs, &request_id),
            &mut blob_store,
        ));
    }

    let picker_model = payload
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("composer-2")
        .to_owned();
    let model_id = wire_model_id(&picker_model);
    let in_flight_user = parsed.in_flight.as_ref().map(|t| t.user_text.as_str());
    let user_text = if !parsed.user_text.is_empty() {
        parsed.user_text.clone()
    } else if parsed
        .in_flight
        .as_ref()
        .is_some_and(|t| !t.steps.is_empty())
    {
        String::new()
    } else {
        parsed
            .turns
            .last()
            .map(|t| t.user_text.clone())
            .unwrap_or_default()
    };
    let current_text = if user_text.is_empty() {
        in_flight_user.unwrap_or("").to_owned()
    } else {
        user_text.clone()
    };
    let in_flight_steps = parsed
        .in_flight
        .as_ref()
        .map(|t| {
            serde_json::to_string(&t.steps.iter().map(Step::fingerprint).collect::<Vec<_>>())
                .unwrap_or_else(|_| "[]".to_owned())
        })
        .unwrap_or_else(|| "[]".to_owned());
    let user_message_id = stable_id(&[&resolved_id, "user", &current_text, &in_flight_steps]);
    let user_message = encode_user_message(&current_text, &user_message_id, None);
    let conversation_state = encode_conversation_state(&root_prompt_blobs, &turn_blobs, "dsh");
    let tools = openai_tools(payload);
    let requested = encode_requested_model(&model_id, false, &model_parameters(payload));
    let request_bytes = encode_agent_client_message(&encode_agent_run_request(
        &conversation_state,
        &user_message,
        Some(&requested),
        &resolved_id,
        &tools,
    ));

    CursorRun {
        conversation_id: resolved_id,
        model_id,
        picker_model,
        system_prompt,
        pinned_system: pinned,
        extra_system: extra,
        user_text: if user_text.is_empty() {
            in_flight_user.unwrap_or("").to_owned()
        } else {
            user_text
        },
        tools: tools
            .into_iter()
            .map(|tool| (tool.name, tool.description))
            .collect(),
        request_bytes,
        blob_store,
        stream: payload.get("stream").and_then(Value::as_bool) == Some(true),
    }
}

/// Map Cursor usage onto OpenAI `usage`.
#[must_use]
pub fn map_usage(usage: TokenUsage) -> Value {
    let prompt = usage.prompt_tokens.unwrap_or(0);
    let completion = usage.completion_tokens.unwrap_or(0);
    let cached = usage.cached_tokens.unwrap_or(0);
    let mut obj = serde_json::json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": prompt + completion,
    });
    if cached > 0
        && let Some(map) = obj.as_object_mut()
    {
        map.insert(
            "prompt_tokens_details".to_owned(),
            serde_json::json!({ "cached_tokens": cached }),
        );
    }
    obj
}

fn connect_error_message(parsed: &Value) -> Option<String> {
    let details = parsed.pointer("/error/details").and_then(Value::as_array);
    if let Some(details) = details {
        for entry in details {
            if let Some(text) = entry.pointer("/debug/details")
                && let Some(title) = text.get("title").and_then(Value::as_str)
            {
                return Some(match text.get("detail").and_then(Value::as_str) {
                    Some(detail) if !detail.is_empty() => format!("{title}: {detail}"),
                    _ => title.to_owned(),
                });
            }
        }
    }
    parsed
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("message").and_then(Value::as_str))
        .map(str::to_owned)
}

/// Decode Connect frames from a byte stream remainder.
#[must_use]
pub fn consume_frames(chunk: &[u8], rest: &[u8], out: &mut Vec<Consumed>) -> Vec<u8> {
    let mut joined = rest.to_vec();
    joined.extend_from_slice(chunk);
    let (frames, leftover) = split_connect_frames(&joined);
    for frame in frames {
        if frame.end {
            let text = String::from_utf8_lossy(&frame.payload);
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            let message = serde_json::from_str::<Value>(trimmed)
                .ok()
                .and_then(|parsed| connect_error_message(&parsed))
                .unwrap_or_else(|| trimmed.chars().take(300).collect());
            out.push(Consumed::Error { message });
            continue;
        }
        out.push(Consumed::Message(decode_agent_server_message(
            &frame.payload,
        )));
    }
    leftover
}

/// One decoded Connect frame from the AgentService stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Consumed {
    Message(ServerMessage),
    Error { message: String },
}

/// First Connect data frame for a built Run.
#[must_use]
pub fn first_connect_frame(built: &CursorRun) -> Vec<u8> {
    frame_connect(&built.request_bytes, false)
}

/// KV / exec / query replies the HTTP/2 Run loop writes back.
#[must_use]
pub fn reply_to_server(
    msg: &ServerMessage,
    blob_store: &mut HashMap<String, Vec<u8>>,
) -> Option<Vec<u8>> {
    match msg {
        ServerMessage::Kv {
            id,
            blob_id,
            blob_data,
            set,
        } => {
            let key = blob_id.as_deref().map(hex::encode).unwrap_or_default();
            if *set
                && let Some(data) = blob_data
                && !key.is_empty()
            {
                blob_store.insert(key.clone(), data.clone());
            }
            let data = if key.is_empty() {
                None
            } else {
                blob_store.get(&key).map(Vec::as_slice)
            };
            Some(frame_connect(
                &encode_kv_client_message(id.unwrap_or(0), data),
                false,
            ))
        }
        ServerMessage::Exec { id, .. } => Some(frame_connect(
            &encode_exec_throw(id.unwrap_or(0), "dsh owns tool execution"),
            false,
        )),
        ServerMessage::Query { .. } => Some(frame_connect(&encode_cancel_action(), false)),
        _ => None,
    }
}

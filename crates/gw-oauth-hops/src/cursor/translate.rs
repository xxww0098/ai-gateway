//! OpenAI chat/completions → Cursor AgentService/Run protobuf.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::conversation_id;
use super::proto::{
    McpTool, encode_agent_client_message, encode_agent_run_request, encode_assistant_step,
    encode_conversation_state, encode_conversation_turn, encode_json_value_bytes,
    encode_requested_model, encode_user_message, frame_connect,
};
use crate::pin::PrefixPins;
use crate::rewrite::message_text;

fn text_of(content: &Value) -> String {
    message_text(&json!({ "content": content }))
}

/// Deterministic UUID for conversation blobs. Same seed → same id.
#[must_use]
pub fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update([0]);
        }
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

struct Turn {
    user_text: String,
    steps: Vec<Step>,
}

enum Step {
    Assistant(String),
    Tool { result: Option<String> },
}

fn parse_turns(messages: &[Value]) -> (String, Vec<Turn>, String) {
    let mut system = Vec::new();
    let mut turns = Vec::new();
    let mut current: Option<Turn> = None;
    let mut pending_user = String::new();
    for message in messages {
        match message.get("role").and_then(Value::as_str) {
            Some("system" | "developer") => {
                let text = message_text(message);
                if !text.is_empty() {
                    system.push(text);
                }
            }
            Some("user") => {
                if let Some(turn) = current.take() {
                    turns.push(turn);
                }
                pending_user = text_of(message.get("content").unwrap_or(&Value::Null));
                current = Some(Turn {
                    user_text: pending_user.clone(),
                    steps: Vec::new(),
                });
            }
            Some("assistant") => {
                let turn = current.get_or_insert_with(|| Turn {
                    user_text: pending_user.clone(),
                    steps: Vec::new(),
                });
                let text = text_of(message.get("content").unwrap_or(&Value::Null));
                if !text.is_empty() {
                    turn.steps.push(Step::Assistant(text));
                }
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for _ in calls {
                        turn.steps.push(Step::Tool { result: None });
                    }
                }
            }
            Some("tool") => {
                if let Some(turn) = current.as_mut()
                    && let Some(Step::Tool { result, .. }) =
                        turn.steps.iter_mut().rev().find(
                            |step| matches!(step, Step::Tool { result, .. } if result.is_none()),
                        )
                {
                    *result = Some(text_of(message.get("content").unwrap_or(&Value::Null)));
                }
            }
            _ => {}
        }
    }
    if let Some(turn) = current.take() {
        pending_user.clone_from(&turn.user_text);
        turns.push(turn);
    }
    let user_text = turns
        .last()
        .map(|t| t.user_text.clone())
        .unwrap_or_default();
    (system.join("\n\n"), turns, user_text)
}

fn openai_tools(payload: &Value) -> Vec<McpTool> {
    payload
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    let fn_obj = tool.get("function").unwrap_or(tool);
                    let name = fn_obj.get("name").and_then(Value::as_str)?.to_owned();
                    let description = fn_obj
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let schema = fn_obj
                        .get("parameters")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    Some(McpTool {
                        tool_name: name.clone(),
                        name: name.clone(),
                        description,
                        input_schema: encode_json_value_bytes(&schema),
                        provider_identifier: "dsh".into(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sha_blob(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

/// Encode a Completions body as a Connect-framed AgentService/Run payload.
#[must_use]
pub fn openai_to_cursor(
    payload: &Value,
    conversation: Option<&str>,
    model: Option<&str>,
    pins: Option<&mut PrefixPins>,
) -> Option<Vec<u8>> {
    let messages = payload.get("messages").and_then(Value::as_array)?;
    let resolved_id = conversation_id(payload, conversation, model);
    let (system_prompt, turns, user_text) = parse_turns(messages);
    let (pinned, extra) = match pins {
        Some(pins) => {
            let result = pins.pin(
                &resolved_id,
                &system_prompt,
                &[super::CURSOR_STABLE_SESSION],
            );
            (result.pinned, result.extra)
        }
        None => (system_prompt, String::new()),
    };
    let mut root_prompt_blobs = Vec::new();
    if !pinned.is_empty() {
        let blob = serde_json::to_vec(&json!({ "role": "system", "content": pinned })).ok()?;
        root_prompt_blobs.push(sha_blob(&blob));
    }
    if !extra.is_empty() {
        let blob = serde_json::to_vec(&json!({ "role": "system", "content": extra })).ok()?;
        root_prompt_blobs.push(sha_blob(&blob));
    }

    let mut turn_blobs = Vec::new();
    for (index, turn) in turns.iter().enumerate() {
        let steps_key = format!("{}", turn.steps.len());
        let message_id = stable_id(&[
            &resolved_id,
            "turn",
            &index.to_string(),
            &turn.user_text,
            &steps_key,
        ]);
        let user_blob = encode_user_message(&turn.user_text, &message_id, 1);
        let user_id = sha_blob(&user_blob);
        let step_blobs: Vec<Vec<u8>> = turn
            .steps
            .iter()
            .map(|step| match step {
                Step::Assistant(text) => sha_blob(&encode_assistant_step(text)),
                Step::Tool { .. } => sha_blob(&encode_assistant_step("")),
            })
            .collect();
        let request_id = stable_id(&[
            &resolved_id,
            "req",
            &index.to_string(),
            &turn.user_text,
            &steps_key,
        ]);
        turn_blobs.push(sha_blob(&encode_conversation_turn(
            &user_id,
            &step_blobs,
            &request_id,
        )));
    }

    let model_id = super::peel_fast(
        model
            .or_else(|| payload.get("model").and_then(Value::as_str))
            .unwrap_or("composer-2"),
    )
    .to_owned();
    let user_message_id = stable_id(&[&resolved_id, "user", &user_text]);
    let user_message = encode_user_message(&user_text, &user_message_id, 1);
    let conversation_state = encode_conversation_state(&root_prompt_blobs, &turn_blobs, 1, "dsh");
    let tools = openai_tools(payload);
    let requested = encode_requested_model(&model_id, &[]);
    let request = encode_agent_run_request(
        &conversation_state,
        &user_message,
        &requested,
        &resolved_id,
        &tools,
    );
    Some(frame_connect(&encode_agent_client_message(&request), false))
}

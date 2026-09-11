//! Minimal protobuf + Connect-RPC v1 framing for AgentService Run / models / KV.
//! Field numbers from Rahularya01/pi-cursor `proto/agent.proto` (MIT).
//! Do not vendor a protobuf crate.

use serde_json::Value;

#[cfg(test)]
mod tests;

const WIRE_VARINT: u64 = 0;
const WIRE_64: u64 = 1;
const WIRE_LEN: u64 = 2;

pub const CONNECT_FLAG_NONE: u8 = 0;
pub const CONNECT_FLAG_END: u8 = 0x02;

#[derive(Debug, Clone)]
pub struct ProtoField {
    pub field: u64,
    pub wire: u64,
    pub varint: Option<u64>,
    pub bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ConnectFrame {
    pub flags: u8,
    pub end: bool,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelParam {
    pub id: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedTool {
    pub name: Option<String>,
    pub description: Option<String>,
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedClientMessage {
    pub conversation_id: Option<String>,
    pub model_id: Option<String>,
    pub max_mode: bool,
    pub user_text: Option<String>,
    pub tools: Vec<DecodedTool>,
    pub has_conversation_state: bool,
    pub parameters: Vec<ModelParam>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsableModel {
    pub id: String,
    pub display_id: String,
    pub name: String,
    pub max_mode: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AvailableVariant {
    pub display_name: Option<String>,
    pub is_max_mode: bool,
    pub parameters: Vec<ModelParam>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AvailableModel {
    pub name: String,
    pub supports_images: bool,
    pub supports_max_mode: bool,
    pub context_token_limit: Option<u64>,
    pub context_token_limit_for_max_mode: Option<u64>,
    pub client_display_name: Option<String>,
    pub server_model_name: Option<String>,
    pub supports_non_max_mode: bool,
    pub variants: Vec<AvailableVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    Interaction {
        text: Option<String>,
        thinking: Option<String>,
        tokens: Option<u64>,
        tool_call: Option<ToolCall>,
        turn_ended: bool,
        usage: Option<TokenUsage>,
    },
    Exec {
        id: Option<u64>,
        exec_id: Option<String>,
        mcp_name: Option<String>,
        mcp_tool_call_id: Option<String>,
    },
    Kv {
        id: Option<u64>,
        blob_id: Option<Vec<u8>>,
        blob_data: Option<Vec<u8>>,
        set: bool,
    },
    Query {
        id: Option<u64>,
    },
    Other,
}

/// Encode an unsigned varint.
#[must_use]
pub fn encode_varint(value: u64) -> Vec<u8> {
    let mut n = value;
    let mut out = Vec::new();
    while n > 0x7f {
        out.push((n as u8 & 0x7f) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
    out
}

#[must_use]
pub fn encode_key(field: u64, wire: u64) -> Vec<u8> {
    encode_varint((field << 3) | wire)
}

#[must_use]
pub fn encode_bytes(field: u64, value: &[u8]) -> Vec<u8> {
    let mut out = encode_key(field, WIRE_LEN);
    out.extend(encode_varint(value.len() as u64));
    out.extend_from_slice(value);
    out
}

#[must_use]
pub fn encode_string(field: u64, value: &str) -> Vec<u8> {
    encode_bytes(field, value.as_bytes())
}

#[must_use]
pub fn encode_bool(field: u64, value: bool) -> Vec<u8> {
    let mut out = encode_key(field, WIRE_VARINT);
    out.extend(encode_varint(u64::from(value)));
    out
}

#[must_use]
pub fn encode_uint32(field: u64, value: u64) -> Vec<u8> {
    let mut out = encode_key(field, WIRE_VARINT);
    out.extend(encode_varint(value));
    out
}

#[must_use]
pub fn encode_message(field: u64, bytes: &[u8]) -> Vec<u8> {
    encode_bytes(field, bytes)
}

/// Read a varint. Returns `(value, next_offset)`.
#[must_use]
pub fn read_varint(buf: &[u8], offset: usize) -> (u64, usize) {
    let mut n = 0u64;
    let mut shift = 0u32;
    let mut i = offset;
    while i < buf.len() {
        let b = buf[i];
        i += 1;
        n |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return (n, i);
        }
        shift += 7;
        if shift > 63 {
            break;
        }
    }
    (n, i)
}

#[must_use]
pub fn decode_fields(buf: &[u8]) -> Vec<ProtoField> {
    let mut fields = Vec::new();
    let mut offset = 0;
    while offset < buf.len() {
        let (tag, next) = read_varint(buf, offset);
        if next == offset {
            break;
        }
        offset = next;
        let field = tag >> 3;
        let wire = tag & 7;
        if wire == WIRE_VARINT {
            let (value, next) = read_varint(buf, offset);
            fields.push(ProtoField {
                field,
                wire,
                varint: Some(value),
                bytes: None,
            });
            offset = next;
        } else if wire == WIRE_LEN {
            let (len, start) = read_varint(buf, offset);
            let end = (start + len as usize).min(buf.len());
            fields.push(ProtoField {
                field,
                wire,
                varint: None,
                bytes: Some(buf[start.min(buf.len())..end].to_vec()),
            });
            offset = end;
        } else {
            break;
        }
    }
    fields
}

#[must_use]
pub fn field_bytes(fields: &[ProtoField], number: u64) -> Vec<&[u8]> {
    fields
        .iter()
        .filter(|row| row.field == number)
        .filter_map(|row| row.bytes.as_deref())
        .collect()
}

#[must_use]
pub fn field_string(fields: &[ProtoField], number: u64) -> Option<String> {
    fields
        .iter()
        .find(|item| item.field == number && item.bytes.is_some())
        .and_then(|row| row.bytes.as_ref())
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
}

#[must_use]
pub fn field_varint(fields: &[ProtoField], number: u64) -> Option<u64> {
    fields
        .iter()
        .find(|item| item.field == number && item.varint.is_some())
        .and_then(|row| row.varint)
}

#[must_use]
pub fn encode_proto_value(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => encode_uint32(1, 0),
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            let mut out = encode_key(2, WIRE_64);
            out.extend_from_slice(&f.to_le_bytes());
            out
        }
        Value::String(s) => encode_string(3, s),
        Value::Bool(b) => encode_bool(4, *b),
        Value::Array(items) => {
            let mut inner = Vec::new();
            for item in items {
                inner.extend(encode_message(1, &encode_proto_value(item)));
            }
            encode_message(6, &inner)
        }
        Value::Object(map) => {
            let mut entries = Vec::new();
            for (key, inner) in map {
                let mut entry = encode_string(1, key);
                entry.extend(encode_message(2, &encode_proto_value(inner)));
                entries.extend(encode_message(1, &entry));
            }
            encode_message(5, &entries)
        }
    }
}

#[must_use]
pub fn encode_json_value_bytes(value: &Value) -> Vec<u8> {
    encode_proto_value(value)
}

#[must_use]
pub fn frame_connect(payload: &[u8], end: bool) -> Vec<u8> {
    let mut frame = vec![0u8; 5 + payload.len()];
    frame[0] = if end {
        CONNECT_FLAG_END
    } else {
        CONNECT_FLAG_NONE
    };
    let len = payload.len() as u32;
    frame[1..5].copy_from_slice(&len.to_be_bytes());
    frame[5..].copy_from_slice(payload);
    frame
}

#[must_use]
pub fn split_connect_frames(buf: &[u8]) -> (Vec<ConnectFrame>, Vec<u8>) {
    let mut frames = Vec::new();
    let mut offset = 0;
    while offset + 5 <= buf.len() {
        let flags = buf[offset];
        let length = u32::from_be_bytes([
            buf[offset + 1],
            buf[offset + 2],
            buf[offset + 3],
            buf[offset + 4],
        ]) as usize;
        if offset + 5 + length > buf.len() {
            break;
        }
        frames.push(ConnectFrame {
            flags,
            end: flags & CONNECT_FLAG_END != 0,
            payload: buf[offset + 5..offset + 5 + length].to_vec(),
        });
        offset += 5 + length;
    }
    (frames, buf[offset..].to_vec())
}

#[must_use]
pub fn encode_user_message(
    text: &str,
    message_id: &str,
    selected_context_blob: Option<&[u8]>,
) -> Vec<u8> {
    let mut parts = encode_string(1, text);
    parts.extend(encode_string(2, message_id));
    parts.extend(encode_uint32(4, 1));
    if let Some(blob) = selected_context_blob {
        parts.extend(encode_bytes(10, blob));
    }
    if !message_id.is_empty() {
        parts.extend(encode_string(17, message_id));
    }
    parts
}

#[must_use]
pub fn encode_requested_model(
    model_id: &str,
    max_mode: bool,
    parameters: &[ModelParam],
) -> Vec<u8> {
    let mut parts = encode_string(1, model_id);
    if max_mode {
        parts.extend(encode_bool(2, true));
    }
    for parameter in parameters {
        let mut inner = encode_string(1, &parameter.id);
        inner.extend(encode_string(2, &parameter.value));
        parts.extend(encode_message(3, &inner));
    }
    parts
}

#[must_use]
pub fn encode_mcp_tools(tools: &[McpTool]) -> Vec<u8> {
    let mut defs = Vec::new();
    for tool in tools {
        let schema = tool.input_schema.as_deref().map_or_else(
            || encode_json_value_bytes(&Value::Object(serde_json::Map::new())),
            |s| s.to_vec(),
        );
        let mut inner = encode_string(1, &tool.name);
        inner.extend(encode_string(2, &tool.description));
        inner.extend(encode_bytes(3, &schema));
        inner.extend(encode_string(4, &tool.provider_identifier));
        inner.extend(encode_string(5, &tool.tool_name));
        defs.extend(encode_message(1, &inner));
    }
    defs
}

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub provider_identifier: String,
    pub tool_name: String,
    pub input_schema: Option<Vec<u8>>,
}

#[must_use]
pub fn encode_conversation_state(
    root_prompt_blobs: &[Vec<u8>],
    turn_blobs: &[Vec<u8>],
    client_name: &str,
) -> Vec<u8> {
    let mut parts = Vec::new();
    for blob in root_prompt_blobs {
        parts.extend(encode_bytes(1, blob));
    }
    for blob in turn_blobs {
        parts.extend(encode_bytes(8, blob));
    }
    parts.extend(encode_uint32(10, 1));
    if !client_name.is_empty() {
        parts.extend(encode_string(22, client_name));
    }
    parts
}

#[must_use]
pub fn encode_conversation_turn(
    user_message_blob: &[u8],
    step_blobs: &[Vec<u8>],
    request_id: &str,
) -> Vec<u8> {
    let mut agent = encode_bytes(1, user_message_blob);
    for blob in step_blobs {
        agent.extend(encode_bytes(2, blob));
    }
    if !request_id.is_empty() {
        agent.extend(encode_string(3, request_id));
    }
    encode_message(1, &agent)
}

#[must_use]
pub fn encode_assistant_step(text: &str) -> Vec<u8> {
    encode_message(1, &encode_string(1, text))
}

#[must_use]
pub fn encode_thinking_step(text: &str) -> Vec<u8> {
    encode_message(3, &encode_string(1, text))
}

#[must_use]
pub fn encode_mcp_tool_step(
    tool_name: &str,
    tool_call_id: &str,
    args: &Value,
    result: Option<&ToolResult>,
) -> Vec<u8> {
    let mut arg_entries = Vec::new();
    if let Value::Object(map) = args {
        for (key, value) in map {
            let mut entry = encode_string(1, key);
            entry.extend(encode_bytes(2, &encode_json_value_bytes(value)));
            arg_entries.extend(encode_message(2, &entry));
        }
    }
    let mut mcp_args = encode_string(
        1,
        if tool_name.is_empty() {
            "tool"
        } else {
            tool_name
        },
    );
    mcp_args.extend(arg_entries);
    mcp_args.extend(encode_string(3, tool_call_id));
    mcp_args.extend(encode_string(4, "dsh"));
    mcp_args.extend(encode_string(
        5,
        if tool_name.is_empty() {
            "tool"
        } else {
            tool_name
        },
    ));
    let mut mcp = encode_message(1, &mcp_args);
    if let Some(result) = result {
        let text_item = encode_message(1, &encode_message(1, &encode_string(1, &result.content)));
        let mut success = encode_message(1, &text_item);
        success.extend(encode_bool(2, result.is_error));
        let tool_result = if result.is_error {
            encode_message(
                2,
                &encode_string(
                    1,
                    if result.content.is_empty() {
                        "error"
                    } else {
                        &result.content
                    },
                ),
            )
        } else {
            encode_message(1, &success)
        };
        mcp.extend(encode_message(2, &tool_result));
    }
    encode_message(2, &encode_message(15, &mcp))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

#[must_use]
pub fn encode_agent_run_request(
    conversation_state: &[u8],
    user_message: &[u8],
    requested_model: Option<&[u8]>,
    conversation_id: &str,
    mcp_tools: &[McpTool],
) -> Vec<u8> {
    let action = encode_message(1, &encode_message(1, user_message));
    let mut parts = encode_message(1, conversation_state);
    parts.extend(encode_message(2, &action));
    if !mcp_tools.is_empty() {
        parts.extend(encode_message(4, &encode_mcp_tools(mcp_tools)));
    }
    if !conversation_id.is_empty() {
        parts.extend(encode_string(5, conversation_id));
    }
    if let Some(model) = requested_model {
        parts.extend(encode_message(9, model));
    }
    parts
}

#[must_use]
pub fn encode_agent_client_message(run_request: &[u8]) -> Vec<u8> {
    encode_message(1, run_request)
}

#[must_use]
pub fn encode_kv_client_message(id: u64, blob_data: Option<&[u8]>) -> Vec<u8> {
    let mut inner = encode_uint32(1, id);
    let result = blob_data.map_or_else(Vec::new, |data| encode_bytes(1, data));
    inner.extend(encode_message(2, &result));
    encode_message(3, &inner)
}

#[must_use]
pub fn encode_exec_throw(id: u64, error: &str) -> Vec<u8> {
    let mut inner = encode_uint32(1, id);
    inner.extend(encode_string(2, error));
    encode_message(5, &encode_message(2, &inner))
}

#[must_use]
pub fn encode_cancel_action() -> Vec<u8> {
    encode_message(4, &encode_message(3, &[]))
}

#[must_use]
pub fn encode_turn_ended_update(usage: TokenUsage) -> Vec<u8> {
    let mut parts = Vec::new();
    if let Some(n) = usage.prompt_tokens {
        parts.extend(encode_uint32(1, n));
    }
    if let Some(n) = usage.completion_tokens {
        parts.extend(encode_uint32(2, n));
    }
    if let Some(n) = usage.cached_tokens {
        parts.extend(encode_uint32(3, n));
    }
    if let Some(n) = usage.cache_write_tokens {
        parts.extend(encode_uint32(4, n));
    }
    if let Some(n) = usage.reasoning_tokens {
        parts.extend(encode_uint32(5, n));
    }
    parts
}

#[must_use]
pub fn decode_turn_ended_update(buf: &[u8]) -> Option<TokenUsage> {
    let fields = decode_fields(buf);
    let usage = TokenUsage {
        prompt_tokens: field_varint(&fields, 1),
        completion_tokens: field_varint(&fields, 2),
        cached_tokens: field_varint(&fields, 3),
        cache_write_tokens: field_varint(&fields, 4),
        reasoning_tokens: field_varint(&fields, 5),
    };
    let empty = usage.prompt_tokens.is_none()
        && usage.completion_tokens.is_none()
        && usage.cached_tokens.is_none()
        && usage.cache_write_tokens.is_none()
        && usage.reasoning_tokens.is_none();
    if empty { None } else { Some(usage) }
}

#[must_use]
pub fn encode_get_usable_models_request(custom_ids: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for id in custom_ids {
        out.extend(encode_string(1, id));
    }
    out
}

#[must_use]
pub fn unwrap_connect_unary(buf: &[u8]) -> Vec<u8> {
    if buf.len() < 5 {
        return buf.to_vec();
    }
    let (frames, _) = split_connect_frames(buf);
    frames
        .iter()
        .find(|frame| !frame.end && !frame.payload.is_empty())
        .or_else(|| frames.iter().find(|frame| !frame.payload.is_empty()))
        .map(|frame| frame.payload.clone())
        .filter(|payload| !payload.is_empty())
        .unwrap_or_else(|| buf.to_vec())
}

#[must_use]
pub fn encode_get_usable_models_response(models: &[UsableModel]) -> Vec<u8> {
    let mut out = Vec::new();
    for model in models {
        let mut inner = encode_string(1, &model.id);
        if !model.display_id.is_empty() {
            inner.extend(encode_string(3, &model.display_id));
        }
        inner.extend(encode_string(4, &model.name));
        if model.max_mode {
            inner.extend(encode_bool(7, true));
        }
        out.extend(encode_message(1, &inner));
    }
    out
}

#[must_use]
pub fn decode_get_usable_models_response(buf: &[u8]) -> Vec<UsableModel> {
    let body = unwrap_connect_unary(buf);
    let mut models = Vec::new();
    for raw in field_bytes(&decode_fields(&body), 1) {
        let fields = decode_fields(raw);
        if let Some(id) = field_string(&fields, 1).filter(|s| !s.is_empty()) {
            let display_id = field_string(&fields, 3).unwrap_or_else(|| id.clone());
            let name = field_string(&fields, 4)
                .or_else(|| field_string(&fields, 5))
                .unwrap_or_else(|| id.clone());
            models.push(UsableModel {
                id,
                display_id,
                name,
                max_mode: field_varint(&fields, 7) == Some(1),
            });
        }
    }
    models
}

#[must_use]
pub fn encode_available_models_request() -> Vec<u8> {
    let mut out = encode_bool(5, true);
    out.extend(encode_bool(7, true));
    out
}

#[must_use]
pub fn encode_available_models_response(models: &[AvailableModel]) -> Vec<u8> {
    let mut out = Vec::new();
    for model in models {
        let mut inner = encode_string(1, &model.name);
        if model.supports_images {
            inner.extend(encode_bool(10, true));
        }
        if let Some(limit) = model.context_token_limit {
            inner.extend(encode_uint32(15, limit));
        }
        if let Some(name) = &model.client_display_name {
            inner.extend(encode_string(17, name));
        }
        out.extend(encode_message(2, &inner));
    }
    out
}

fn decode_available_variant(buf: &[u8]) -> AvailableVariant {
    let fields = decode_fields(buf);
    AvailableVariant {
        display_name: field_string(&fields, 2),
        is_max_mode: field_varint(&fields, 3) == Some(1),
        parameters: field_bytes(&fields, 1)
            .into_iter()
            .map(|raw| {
                let inner = decode_fields(raw);
                ModelParam {
                    id: field_string(&inner, 1).unwrap_or_default(),
                    value: field_string(&inner, 2).unwrap_or_default(),
                }
            })
            .collect(),
    }
}

fn decode_available_model(buf: &[u8]) -> AvailableModel {
    let fields = decode_fields(buf);
    AvailableModel {
        name: field_string(&fields, 1).unwrap_or_default(),
        supports_images: field_varint(&fields, 10) == Some(1),
        supports_max_mode: field_varint(&fields, 14) == Some(1),
        context_token_limit: field_varint(&fields, 15),
        context_token_limit_for_max_mode: field_varint(&fields, 16),
        client_display_name: field_string(&fields, 17),
        server_model_name: field_string(&fields, 18),
        supports_non_max_mode: field_varint(&fields, 19) == Some(1),
        variants: field_bytes(&fields, 30)
            .into_iter()
            .map(decode_available_variant)
            .collect(),
    }
}

#[must_use]
pub fn decode_available_models_response(buf: &[u8]) -> Vec<AvailableModel> {
    let body = unwrap_connect_unary(buf);
    field_bytes(&decode_fields(&body), 2)
        .into_iter()
        .map(decode_available_model)
        .filter(|model| !model.name.is_empty())
        .collect()
}

fn first_bytes(fields: &[ProtoField], number: u64) -> Vec<u8> {
    field_bytes(fields, number)
        .first()
        .copied()
        .unwrap_or(&[])
        .to_vec()
}

#[must_use]
pub fn decode_agent_client_message(buf: &[u8]) -> DecodedClientMessage {
    let root = decode_fields(buf);
    let run_raw = first_bytes(&root, 1);
    if run_raw.is_empty() {
        return DecodedClientMessage {
            conversation_id: None,
            model_id: None,
            max_mode: false,
            user_text: None,
            tools: Vec::new(),
            has_conversation_state: false,
            parameters: Vec::new(),
        };
    }
    let run = decode_fields(&run_raw);
    let action = decode_fields(&first_bytes(&run, 2));
    let user_action = decode_fields(&first_bytes(&action, 1));
    let user = decode_fields(&first_bytes(&user_action, 1));
    let model = decode_fields(&first_bytes(&run, 9));
    let tools_msg = decode_fields(&first_bytes(&run, 4));
    let tools = field_bytes(&tools_msg, 1)
        .into_iter()
        .map(|raw| {
            let fields = decode_fields(raw);
            DecodedTool {
                name: field_string(&fields, 1),
                description: field_string(&fields, 2),
                tool_name: field_string(&fields, 5),
            }
        })
        .collect();
    DecodedClientMessage {
        conversation_id: field_string(&run, 5),
        model_id: field_string(&model, 1),
        max_mode: field_varint(&model, 2) == Some(1),
        user_text: field_string(&user, 1),
        tools,
        has_conversation_state: !field_bytes(&run, 1).is_empty(),
        parameters: field_bytes(&model, 3)
            .into_iter()
            .map(|raw| {
                let fields = decode_fields(raw);
                ModelParam {
                    id: field_string(&fields, 1).unwrap_or_default(),
                    value: field_string(&fields, 2).unwrap_or_default(),
                }
            })
            .collect(),
    }
}

#[must_use]
pub fn decode_agent_server_message(buf: &[u8]) -> ServerMessage {
    let root = decode_fields(buf);
    let interaction = first_bytes(&root, 1);
    if !interaction.is_empty() {
        let fields = decode_fields(&interaction);
        let text = field_string(&decode_fields(&first_bytes(&fields, 1)), 1);
        let thinking = field_string(&decode_fields(&first_bytes(&fields, 4)), 1);
        let tokens = field_varint(&decode_fields(&first_bytes(&fields, 8)), 1);
        let tool_started = first_bytes(&fields, 2);
        let turn_ended_raw = first_bytes(&fields, 14);
        let turn_ended = !turn_ended_raw.is_empty();
        let usage = if turn_ended {
            decode_turn_ended_update(&turn_ended_raw)
        } else {
            None
        };
        let tool_call = if tool_started.is_empty() {
            None
        } else {
            let started = decode_fields(&tool_started);
            let call_id = field_string(&started, 1).or_else(|| field_string(&started, 3));
            let tool = decode_fields(&first_bytes(&started, 2));
            let mcp = decode_fields(&first_bytes(&tool, 15));
            let args = decode_fields(&first_bytes(&mcp, 1));
            let name = field_string(&args, 5)
                .or_else(|| field_string(&args, 1))
                .unwrap_or_else(|| "tool".to_owned());
            Some(ToolCall {
                id: call_id
                    .or_else(|| field_string(&args, 3))
                    .unwrap_or_default(),
                name,
            })
        };
        return ServerMessage::Interaction {
            text,
            thinking,
            tokens,
            tool_call,
            turn_ended,
            usage,
        };
    }
    let exec = first_bytes(&root, 2);
    if !exec.is_empty() {
        let fields = decode_fields(&exec);
        let mcp = first_bytes(&fields, 11);
        let args = decode_fields(&mcp);
        return ServerMessage::Exec {
            id: field_varint(&fields, 1),
            exec_id: field_string(&fields, 15),
            mcp_name: if mcp.is_empty() {
                None
            } else {
                field_string(&args, 5).or_else(|| field_string(&args, 1))
            },
            mcp_tool_call_id: if mcp.is_empty() {
                None
            } else {
                field_string(&args, 3)
            },
        };
    }
    let kv = first_bytes(&root, 4);
    if !kv.is_empty() {
        let fields = decode_fields(&kv);
        let get_args = decode_fields(&first_bytes(&fields, 2));
        let set_args = decode_fields(&first_bytes(&fields, 3));
        return ServerMessage::Kv {
            id: field_varint(&fields, 1),
            blob_id: field_bytes(&get_args, 1)
                .first()
                .copied()
                .map(<[u8]>::to_vec)
                .or_else(|| {
                    field_bytes(&set_args, 1)
                        .first()
                        .copied()
                        .map(<[u8]>::to_vec)
                }),
            blob_data: field_bytes(&set_args, 2)
                .first()
                .copied()
                .map(<[u8]>::to_vec),
            set: !field_bytes(&fields, 3).is_empty(),
        };
    }
    let query = first_bytes(&root, 7);
    if !query.is_empty() {
        return ServerMessage::Query {
            id: field_varint(&decode_fields(&query), 1),
        };
    }
    ServerMessage::Other
}

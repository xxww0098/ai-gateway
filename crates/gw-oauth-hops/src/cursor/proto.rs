//! Minimal protobuf + Connect-RPC v1 framing for AgentService/Run.
//! Field numbers from Rahularya01/pi-cursor proto/agent.proto (MIT).

const WIRE_VARINT: u32 = 0;
const WIRE_64: u32 = 1;
const WIRE_LEN: u32 = 2;

pub const CONNECT_FLAG_NONE: u8 = 0;
pub const CONNECT_FLAG_END: u8 = 0x02;

fn encode_varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while n > 0x7f {
        out.push(((n as u8) & 0x7f) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
    out
}

fn encode_key(field: u32, wire: u32) -> Vec<u8> {
    encode_varint(u64::from((field << 3) | wire))
}

fn encode_bytes(field: u32, value: &[u8]) -> Vec<u8> {
    let mut out = encode_key(field, WIRE_LEN);
    out.extend(encode_varint(value.len() as u64));
    out.extend_from_slice(value);
    out
}

fn encode_string(field: u32, value: &str) -> Vec<u8> {
    encode_bytes(field, value.as_bytes())
}

fn encode_bool(field: u32, value: bool) -> Vec<u8> {
    let mut out = encode_key(field, WIRE_VARINT);
    out.extend(encode_varint(u64::from(value)));
    out
}

fn encode_uint32(field: u32, value: u32) -> Vec<u8> {
    let mut out = encode_key(field, WIRE_VARINT);
    out.extend(encode_varint(u64::from(value)));
    out
}

fn encode_message(field: u32, bytes: &[u8]) -> Vec<u8> {
    encode_bytes(field, bytes)
}

/// google.protobuf.Value — enough JSON for MCP schemas / tool args.
pub fn encode_proto_value(value: &serde_json::Value) -> Vec<u8> {
    match value {
        serde_json::Value::Null => encode_uint32(1, 0),
        serde_json::Value::Number(n) => {
            let mut buf = encode_key(2, WIRE_64);
            buf.extend_from_slice(&n.as_f64().unwrap_or(0.0).to_le_bytes());
            buf
        }
        serde_json::Value::String(text) => encode_string(3, text),
        serde_json::Value::Bool(flag) => encode_bool(4, *flag),
        serde_json::Value::Array(items) => {
            let nested: Vec<u8> = items
                .iter()
                .flat_map(|item| encode_message(1, &encode_proto_value(item)))
                .collect();
            encode_message(6, &nested)
        }
        serde_json::Value::Object(map) => {
            let nested: Vec<u8> = map
                .iter()
                .flat_map(|(key, inner)| {
                    let mut entry = encode_string(1, key);
                    entry.extend(encode_message(2, &encode_proto_value(inner)));
                    encode_message(1, &entry)
                })
                .collect();
            encode_message(5, &nested)
        }
    }
}

pub fn encode_json_value_bytes(value: &serde_json::Value) -> Vec<u8> {
    encode_proto_value(value)
}

pub fn frame_connect(payload: &[u8], end: bool) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(if end {
        CONNECT_FLAG_END
    } else {
        CONNECT_FLAG_NONE
    });
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

pub fn encode_user_message(text: &str, message_id: &str, mode: u32) -> Vec<u8> {
    let mut parts = encode_string(1, text);
    parts.extend(encode_string(2, message_id));
    parts.extend(encode_uint32(4, mode));
    if !message_id.is_empty() {
        parts.extend(encode_string(17, message_id));
    }
    parts
}

pub fn encode_requested_model(model_id: &str, parameters: &[(String, String)]) -> Vec<u8> {
    let mut parts = encode_string(1, model_id);
    for (id, value) in parameters {
        let mut inner = encode_string(1, id);
        inner.extend(encode_string(2, value));
        parts.extend(encode_message(3, &inner));
    }
    parts
}

pub fn encode_mcp_tools(tools: &[McpTool]) -> Vec<u8> {
    tools
        .iter()
        .flat_map(|tool| {
            let mut def = encode_string(1, &tool.name);
            def.extend(encode_string(2, &tool.description));
            def.extend(encode_bytes(3, &tool.input_schema));
            def.extend(encode_string(4, &tool.provider_identifier));
            def.extend(encode_string(5, &tool.tool_name));
            encode_message(1, &def)
        })
        .collect()
}

pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Vec<u8>,
    pub provider_identifier: String,
    pub tool_name: String,
}

pub fn encode_conversation_state(
    root_prompt_blobs: &[Vec<u8>],
    turn_blobs: &[Vec<u8>],
    mode: u32,
    client_name: &str,
) -> Vec<u8> {
    let mut parts = Vec::new();
    for blob in root_prompt_blobs {
        parts.extend(encode_bytes(1, blob));
    }
    for blob in turn_blobs {
        parts.extend(encode_bytes(8, blob));
    }
    parts.extend(encode_uint32(10, mode));
    if !client_name.is_empty() {
        parts.extend(encode_string(22, client_name));
    }
    parts
}

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

pub fn encode_assistant_step(text: &str) -> Vec<u8> {
    encode_message(1, &encode_string(1, text))
}

pub fn encode_agent_run_request(
    conversation_state: &[u8],
    user_message: &[u8],
    requested_model: &[u8],
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
    parts.extend(encode_message(9, requested_model));
    parts
}

pub fn encode_agent_client_message(run_request: &[u8]) -> Vec<u8> {
    encode_message(1, run_request)
}

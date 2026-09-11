use serde_json::{Value, json};

use super::{
    contents_of, convert, function_response_payload, gemini_requires_thought_signature, isolated,
    json_blob, read_tool, request_of, roles_of, sanitize_tool_call_id, schema_tool,
};

#[test]
fn function_response_is_a_singular_struct() {
    assert_eq!(
        function_response_payload(&json!({"ok": true})),
        json!({"ok": true})
    );
    assert_eq!(
        function_response_payload(&json!([{"type": "text", "text": "a"}])),
        json!({"result": [{"type": "text", "text": "a"}]})
    );
    assert_eq!(
        function_response_payload(&json!("[{\"path\":\"a.ts\"}]")),
        json!({"result": [{"path": "a.ts"}]})
    );
    assert_eq!(
        function_response_payload(&json!("plain tool output")),
        json!({"text": "plain tool output"})
    );
    assert_eq!(function_response_payload(&json!("")), json!({}));
    assert_eq!(function_response_payload(&Value::Null), json!({}));

    let _lock = isolated();
    let body = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "messages": [{"role": "tool", "name": "Read", "content": ["a.ts", "b.ts"]}]
        }),
        "p",
    );
    let blob = json_blob(&request_of(&body)["contents"]);
    assert!(!blob.contains("\"functionResponse\":["));
    assert!(!blob.contains("\"response\":["));
    let response = &contents_of(&body)[0]["parts"][0]["functionResponse"]["response"];
    assert!(response.is_object());
    assert!(!response.is_array());
}

#[test]
fn consecutive_tool_messages_share_one_user_turn() {
    let _lock = isolated();
    let body = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "messages": [
                {"role": "user", "content": "read files"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "Read", "arguments": "{\"path\":\"a.ts\"}"}, "thoughtSignature": "sig-read"},
                        {"id": "call_2", "type": "function", "function": {"name": "Grep", "arguments": "{\"q\":\"src\"}"}, "thoughtSignature": "sig-grep"}
                    ]
                },
                {"role": "tool", "tool_call_id": "call_1", "name": "Read", "content": "file a contents"},
                {"role": "tool", "tool_call_id": "call_2", "name": "Grep", "content": [{"type": "text", "text": "hit 1"}, {"type": "text", "text": "hit 2"}]}
            ]
        }),
        "p",
    );
    assert_eq!(roles_of(&body), vec!["user", "model", "user"]);
    let tools = &contents_of(&body)[2]["parts"];
    assert_eq!(tools.as_array().map(Vec::len), Some(2));
    assert!(
        !contents_of(&body)[1]["parts"][0]["functionCall"]
            .as_object()
            .expect("fc")
            .contains_key("id")
    );
}

#[test]
fn gemini_tools_use_json_schema_claude_uses_allowlisted_parameters() {
    let _lock = isolated();
    let gemini = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "tools": [schema_tool()],
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    let decl = &request_of(&gemini)["tools"][0]["functionDeclarations"][0];
    assert!(decl.get("parameters").is_none());
    assert!(decl.get("parametersJsonSchema").is_some());
    assert!(decl["parametersJsonSchema"].get("$schema").is_none());
    assert!(decl["parametersJsonSchema"].get("$defs").is_none());
    assert_eq!(
        decl["parametersJsonSchema"]["properties"]["path"]["format"],
        "uri-reference"
    );
    assert!(
        decl["parametersJsonSchema"]["properties"]["path"]
            .get("$ref")
            .is_none()
    );

    for model in ["claude-sonnet-4-6", "gpt-oss-120b-medium"] {
        let body = convert(
            json!({
                "model": model,
                "tools": [schema_tool()],
                "messages": [{"role": "user", "content": "hi"}]
            }),
            "p",
        );
        let decl = &request_of(&body)["tools"][0]["functionDeclarations"][0];
        assert!(decl.get("parametersJsonSchema").is_none(), "{model}");
        let params = decl["parameters"].as_object().expect("parameters");
        for key in params.keys() {
            assert!(
                [
                    "type",
                    "description",
                    "properties",
                    "required",
                    "items",
                    "enum"
                ]
                .contains(&key.as_str()),
                "{model} leaked {key}"
            );
        }
        assert!(params.get("additionalProperties").is_none());
        assert_eq!(params["properties"]["mode"]["type"], "string");
        assert!(params["properties"]["flag"].get("enum").is_none());
        assert!(params["properties"]["kind"].get("anyOf").is_none());
        assert!(params["properties"]["path"].get("format").is_none());
    }
}

#[test]
fn claude_keeps_function_call_id_gemini_omits_it() {
    let _lock = isolated();
    let messages = json!([
        {"role": "user", "content": "read"},
        {
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {"id": "call/1 extra!", "type": "function", "function": {"name": "Read", "arguments": "{\"path\":\"a.ts\"}"}}
            ]
        },
        {"role": "tool", "tool_call_id": "call/1 extra!", "name": "Read", "content": "file a"}
    ]);
    let claude = convert(
        json!({"model": "claude-sonnet-4-6", "messages": messages}),
        "p",
    );
    let claude_id = contents_of(&claude)
        .iter()
        .find(|c| c["role"] == "model")
        .expect("model")["parts"][0]["functionCall"]["id"]
        .as_str()
        .expect("id");
    assert_eq!(claude_id, sanitize_tool_call_id("call/1 extra!", "Read"));
    assert!(
        claude_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    );
    assert!(claude_id.len() <= 64);

    let mut gemini_messages = messages;
    gemini_messages[1]["tool_calls"][0]["thoughtSignature"] = json!("sig-keep");
    let gemini = convert(
        json!({"model": "gemini-3.7-flash-high", "messages": gemini_messages}),
        "p",
    );
    let gemini_call = &contents_of(&gemini)
        .iter()
        .find(|c| c["role"] == "model")
        .expect("model")["parts"][0]["functionCall"];
    assert!(gemini_call.get("id").is_none());
}

#[test]
fn gemini_unsigned_function_call_becomes_observation_claude_keeps_it() {
    let _lock = isolated();
    assert!(gemini_requires_thought_signature("gemini-3.7-flash-high"));
    assert!(gemini_requires_thought_signature("gemini-pro-agent"));
    assert!(!gemini_requires_thought_signature("claude-sonnet-4-6"));

    let messages = json!([
        {"role": "user", "content": "read"},
        {
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "Read", "arguments": "{\"path\":\"a.ts\"}"}}
            ]
        },
        {"role": "tool", "tool_call_id": "call_1", "name": "Read", "content": "file a contents"}
    ]);
    let gemini = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-drop-1",
            "messages": messages
        }),
        "p",
    );
    let gemini_blob = json_blob(&request_of(&gemini)["contents"]);
    assert!(!gemini_blob.contains("functionCall"), "{gemini_blob}");
    assert!(!gemini_blob.contains("functionResponse"), "{gemini_blob}");
    assert!(gemini_blob.contains("Observation from `Read`"));

    let claude = convert(
        json!({"model": "claude-sonnet-4-6", "messages": messages}),
        "p",
    );
    let claude_blob = json_blob(&request_of(&claude)["contents"]);
    assert!(claude_blob.contains("functionCall"));
    assert!(claude_blob.contains("functionResponse"));
    assert!(!claude_blob.contains("Observation"));
}

#[test]
fn extra_snapshot_does_not_split_function_call_from_response() {
    let _lock = isolated();
    convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-merge-1",
            "messages": [
                {"role": "system", "content": "You are an AI agent."},
                {"role": "user", "content": "first"}
            ]
        }),
        "p",
    );
    let body = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "session_id": "session-merge-1",
            "messages": [
                {"role": "system", "content": "You are an AI agent."},
                {"role": "system", "content": "Current runtime context. This snapshot supersedes earlier runtime-context snapshots."},
                {"role": "user", "content": "first"},
                {"role": "user", "content": "second"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "Read", "arguments": "{}"}, "thoughtSignature": "sig-keep"}
                    ]
                },
                {"role": "tool", "tool_call_id": "call_1", "name": "Read", "content": "ok"}
            ]
        }),
        "p",
    );
    let contents = contents_of(&body);
    let model_idx = contents
        .iter()
        .position(|c| {
            c["parts"]
                .as_array()
                .is_some_and(|p| p.iter().any(|part| part.get("functionCall").is_some()))
        })
        .expect("model");
    let tool_idx = contents
        .iter()
        .position(|c| {
            c["parts"]
                .as_array()
                .is_some_and(|p| p.iter().any(|part| part.get("functionResponse").is_some()))
        })
        .expect("tool");
    assert_eq!(tool_idx, model_idx + 1);
    assert!(json_blob(&Value::Array(contents.to_vec())).contains("Current runtime context"));
}

#[test]
fn gemini_tool_choice_maps_none_any_auto_claude_stays_validated() {
    let _lock = isolated();
    let none = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "tool_choice": "none",
            "tools": [read_tool()],
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(
        request_of(&none)["toolConfig"]["functionCallingConfig"]["mode"],
        "NONE"
    );
    let required = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "tool_choice": "required",
            "tools": [read_tool()],
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(
        request_of(&required)["toolConfig"]["functionCallingConfig"]["mode"],
        "ANY"
    );
    let auto = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "tools": [read_tool()],
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(
        request_of(&auto)["toolConfig"]["functionCallingConfig"]["mode"],
        "AUTO"
    );
    let claude = convert(
        json!({
            "model": "claude-sonnet-4-6",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert_eq!(
        request_of(&claude)["toolConfig"]["functionCallingConfig"]["mode"],
        "VALIDATED"
    );
    assert!(request_of(&claude).get("tools").is_none());
    let gemini_bare = convert(
        json!({
            "model": "gemini-3.7-flash-high",
            "messages": [{"role": "user", "content": "hi"}]
        }),
        "p",
    );
    assert!(request_of(&gemini_bare).get("toolConfig").is_none());
}

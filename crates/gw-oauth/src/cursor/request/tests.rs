use serde_json::json;

use super::{Consumed, consume_frames, first_connect_frame, map_usage, openai_to_cursor};
use crate::cursor::cache::{reset_system_pins, stable_id};
use crate::cursor::proto::{
    TokenUsage, decode_agent_client_message, encode_message, encode_turn_ended_update,
    frame_connect,
};

#[test]
fn completions_to_agent_keeps_model_user_tools_and_conversation_id() {
    reset_system_pins();
    let built = openai_to_cursor(
        &json!({
            "model": "composer-2",
            "session_id": "sess-hop",
            "messages": [
                {"role": "system", "content": "You are DSH."},
                {"role": "user", "content": "list files"}
            ],
            "tools": [{
                "type": "function",
                "function": {"name": "glob", "description": "find files", "parameters": {"type": "object"}}
            }],
            "reasoning_effort": "high",
        }),
        None,
    );
    let decoded = decode_agent_client_message(&built.request_bytes);
    assert_eq!(built.conversation_id, "sess-hop:composer-2");
    assert_eq!(
        decoded.conversation_id.as_deref(),
        Some("sess-hop:composer-2")
    );
    assert_eq!(decoded.model_id.as_deref(), Some("composer-2"));
    assert_eq!(decoded.user_text.as_deref(), Some("list files"));
    assert!(
        decoded
            .tools
            .iter()
            .any(|tool| tool.name.as_deref() == Some("glob"))
    );
    assert!(decoded.has_conversation_state);
    reset_system_pins();
}

#[test]
fn hop_peels_fast_and_sets_requested_model_fast_param() {
    reset_system_pins();
    let built = openai_to_cursor(
        &json!({
            "model": "gpt-5.5-fast",
            "session_id": "sess-fast",
            "reasoning_effort": "high",
            "service_tier": "priority",
            "messages": [{"role": "user", "content": "hi"}],
        }),
        None,
    );
    let decoded = decode_agent_client_message(&built.request_bytes);
    assert_eq!(built.model_id, "gpt-5.5");
    assert_eq!(built.picker_model, "gpt-5.5-fast");
    assert_eq!(decoded.model_id.as_deref(), Some("gpt-5.5"));
    assert!(!decoded.model_id.as_deref().unwrap_or("").ends_with("-fast"));
    assert!(!decoded.max_mode);
    assert_eq!(decoded.parameters.len(), 2);
    assert_eq!(decoded.parameters[0].id, "reasoning");
    assert_eq!(decoded.parameters[0].value, "high");
    assert_eq!(decoded.parameters[1].id, "fast");
    assert_eq!(decoded.parameters[1].value, "true");
    assert_eq!(built.conversation_id, "sess-fast:gpt-5.5");
    let encoded = serde_json::to_string(&json!({
        "conversationId": decoded.conversation_id,
        "modelId": decoded.model_id,
        "parameters": decoded.parameters.iter().map(|p| json!({"id": p.id, "value": p.value})).collect::<Vec<_>>(),
    }))
    .expect("json");
    assert!(!encoded.contains("service_tier"));
    let plain = openai_to_cursor(
        &json!({
            "model": "gpt-5.5",
            "reasoning_effort": "high",
            "messages": [{"role": "user", "content": "hi"}],
        }),
        None,
    );
    let plain_decoded = decode_agent_client_message(&plain.request_bytes);
    assert_eq!(plain_decoded.parameters.len(), 1);
    assert_eq!(plain_decoded.parameters[0].id, "reasoning");
    reset_system_pins();
}

#[test]
fn identical_turns_encode_the_same_prefix_bytes() {
    reset_system_pins();
    let payload = json!({
        "model": "composer-2",
        "session_id": "sess-stable-cursor",
        "messages": [
            {"role": "system", "content": "You are DSH."},
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "hi"},
            {"role": "user", "content": "again"}
        ],
    });
    let first = openai_to_cursor(&payload, None);
    let second = openai_to_cursor(&payload, None);
    assert_eq!(first.request_bytes, second.request_bytes);
    assert_eq!(stable_id(&["a", "b"]), stable_id(&["a", "b"]));
    assert_ne!(stable_id(&["a", "b"]), stable_id(&["a", "c"]));
    let decoded = decode_agent_client_message(&first.request_bytes);
    assert!(decoded.has_conversation_state);
    reset_system_pins();
}

#[test]
fn turn_ended_cache_read_maps_to_openai_cached_tokens() {
    let payload = encode_message(
        1,
        &encode_message(
            14,
            &encode_turn_ended_update(TokenUsage {
                prompt_tokens: Some(1000),
                completion_tokens: Some(40),
                cached_tokens: Some(800),
                cache_write_tokens: Some(50),
                reasoning_tokens: Some(12),
            }),
        ),
    );
    let usage = match crate::cursor::proto::decode_agent_server_message(&payload) {
        crate::cursor::proto::ServerMessage::Interaction { usage, .. } => usage.expect("usage"),
        other => panic!("unexpected {other:?}"),
    };
    let mapped = map_usage(usage);
    assert_eq!(mapped["prompt_tokens"], 1000);
    assert_eq!(mapped["completion_tokens"], 40);
    assert_eq!(mapped["prompt_tokens_details"]["cached_tokens"], 800);
    assert!(
        map_usage(TokenUsage {
            prompt_tokens: Some(10),
            completion_tokens: Some(2),
            ..TokenUsage::default()
        })
        .get("prompt_tokens_details")
        .is_none()
    );
}

#[test]
fn consume_frames_prefers_connect_error_detail() {
    let end = frame_connect(
        serde_json::to_vec(&json!({
            "error": {
                "code": "invalid_argument",
                "message": "Error",
                "details": [{
                    "type": "aiserver.v1.ErrorDetails",
                    "debug": {
                        "error": "ERROR_MODEL_NO_LONGER_SUPPORTED",
                        "details": {
                            "title": "Composer 2 is retired",
                            "detail": "We're upgrading you to Composer 2.5, our most powerful model yet."
                        }
                    }
                }]
            }
        }))
        .expect("json")
        .as_slice(),
        true,
    );
    let mut messages = Vec::new();
    let leftover = consume_frames(&end, &[], &mut messages);
    assert!(leftover.is_empty());
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        Consumed::Error { message } => {
            assert!(message.contains("Composer 2 is retired"), "{message}");
            assert!(message.contains("Composer 2.5"), "{message}");
            assert_ne!(message, "Error");
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[test]
fn first_frame_is_not_an_end_frame() {
    reset_system_pins();
    let built = openai_to_cursor(
        &json!({
            "model": "composer-2",
            "messages": [{"role": "user", "content": "hi"}],
        }),
        None,
    );
    let frame = first_connect_frame(&built);
    assert_eq!(frame[0], 0);
    assert!(frame.len() > 5);
    reset_system_pins();
}

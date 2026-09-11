use super::{
    AvailableModel, ServerMessage, TokenUsage, UsableModel, decode_agent_server_message,
    decode_available_models_response, decode_get_usable_models_response,
    encode_available_models_request, encode_available_models_response,
    encode_get_usable_models_response, encode_message, encode_turn_ended_update, encode_varint,
    frame_connect, read_varint, split_connect_frames,
};

#[test]
fn varint_round_trips_and_zero_is_a_single_byte() {
    let encoded = encode_varint(0);
    assert_eq!(encoded, [0]);
    let (value, offset) = read_varint(&encoded, 0);
    assert_eq!(value, 0);
    assert_eq!(offset, 1);
    let encoded = encode_varint(300);
    let (value, _) = read_varint(&encoded, 0);
    assert_eq!(value, 300);
}

#[test]
fn connect_frames_split_and_leave_a_remainder() {
    let mut buf = frame_connect(b"abc", false);
    buf.extend_from_slice(&[0, 0, 0]);
    let (frames, rest) = split_connect_frames(&buf);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].payload, b"abc");
    assert!(!frames[0].end);
    assert_eq!(rest, [0, 0, 0]);
}

#[test]
fn usable_models_round_trip_through_a_connect_unary_envelope() {
    let source = [
        UsableModel {
            id: "composer-2".into(),
            display_id: "composer-2".into(),
            name: "Composer 2".into(),
            max_mode: false,
        },
        UsableModel {
            id: "gpt-5.5".into(),
            display_id: "gpt-5.5".into(),
            name: "GPT-5.5".into(),
            max_mode: false,
        },
    ];
    let framed = frame_connect(&encode_get_usable_models_response(&source), false);
    let decoded = decode_get_usable_models_response(&framed);
    assert_eq!(decoded.len(), source.len());
    assert_eq!(decoded[0].id, source[0].id);
    assert_eq!(decoded[1].name, source[1].name);
}

#[test]
fn available_models_request_is_non_empty_and_response_round_trips() {
    assert!(!encode_available_models_request().is_empty());
    let decoded =
        decode_available_models_response(&encode_available_models_response(&[AvailableModel {
            name: "kimi-k2.5".into(),
            supports_images: false,
            supports_max_mode: false,
            context_token_limit: Some(262_000),
            context_token_limit_for_max_mode: None,
            client_display_name: Some("Kimi K2.5".into()),
            server_model_name: None,
            supports_non_max_mode: false,
            variants: Vec::new(),
        }]));
    assert_eq!(decoded[0].name, "kimi-k2.5");
    assert_eq!(decoded[0].context_token_limit, Some(262_000));
}

#[test]
fn turn_ended_update_exposes_cache_read_tokens() {
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
    match decode_agent_server_message(&payload) {
        ServerMessage::Interaction {
            turn_ended,
            usage: Some(usage),
            ..
        } => {
            assert!(turn_ended);
            assert_eq!(usage.prompt_tokens, Some(1000));
            assert_eq!(usage.completion_tokens, Some(40));
            assert_eq!(usage.cached_tokens, Some(800));
            assert_eq!(usage.cache_write_tokens, Some(50));
        }
        other => panic!("expected turn-ended interaction, got {other:?}"),
    }
}

use super::*;

/// HTTP transport chunks are unrelated to SSE event boundaries. Every possible
/// single split — plus the one-byte-at-a-time adversarial framing — must produce
/// the same spoken text and the same billing usage for both target dialects.
#[test]
fn google_stream_translation_is_independent_of_network_chunking() {
    let wire: Vec<u8> = google_sse_frames().into_iter().flatten().collect();

    let translated_text = |frames: &[(Option<String>, Value)]| -> String {
        frames
            .iter()
            .filter_map(|(_, value)| {
                value
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
                    .or_else(|| value.pointer("/delta/text").and_then(Value::as_str))
            })
            .collect()
    };

    for translator in [&OpenAiToGoogle as &dyn Translator, &AnthropicToGoogle] {
        let (_, expected_usage) = run_stream(translator);

        for cut in 1..wire.len() {
            let mut stream = translator.stream_translator();
            let mut output = stream.push(&wire[..cut]).expect("first transport chunk");
            output.extend(stream.push(&wire[cut..]).expect("second transport chunk"));
            output.extend(stream.finish().expect("finish"));
            let decoded = decode(&output);
            assert_eq!(
                translated_text(&decoded),
                SPOKEN,
                "SSE split at byte {cut} changed the response"
            );
            assert_eq!(
                stream.usage(),
                expected_usage,
                "SSE split at byte {cut} changed billing usage"
            );
        }

        let mut stream = translator.stream_translator();
        let mut output = Vec::new();
        for byte in &wire {
            output.extend(
                stream
                    .push(std::slice::from_ref(byte))
                    .expect("one-byte transport chunk"),
            );
        }
        output.extend(stream.finish().expect("finish"));
        assert_eq!(translated_text(&decode(&output)), SPOKEN);
        assert_eq!(stream.usage(), expected_usage);
    }
}

/// A final partial JSON event is an upstream truncation, not a successful empty
/// turn. `finish` must surface it instead of fabricating a normal stop frame.
#[test]
fn truncated_google_event_is_not_reported_as_a_clean_eof() {
    for translator in [&OpenAiToGoogle as &dyn Translator, &AnthropicToGoogle] {
        let mut stream = translator.stream_translator();
        assert!(
            stream
                .push(br#"data: {"candidates":[{"content":{"parts":[{"text":"half"#)
                .expect("the incomplete event is buffered")
                .is_empty()
        );
        assert!(matches!(
            stream.finish().expect_err("truncated JSON must fail"),
            TranslateError::UpstreamShape(_)
        ));
    }
}

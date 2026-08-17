//! Unit tests for [`crate::usage`].
//!
//! There is no property-testing dependency here (rule: no new production deps,
//! and the workspace declares no dev-deps), so the generated cases are replaced
//! by an exhaustive walk of the shape space plus a deterministic pseudo-random
//! sweep.

use super::*;

// --- OpenAI -----------------------------------------------------------------

#[test]
fn openai_full_envelope_populates_every_column() {
    let payload = br#"{
        "id": "chatcmpl-123",
        "usage": {
            "prompt_tokens": 120,
            "completion_tokens": 80,
            "total_tokens": 200,
            "prompt_tokens_details": {"cached_tokens": 40},
            "completion_tokens_details": {"reasoning_tokens": 30}
        }
    }"#;
    let tokens = parse_openai_usage(payload).expect("full envelope must parse");
    assert_eq!(tokens.input, Some(120));
    assert_eq!(tokens.output, Some(80));
    assert_eq!(tokens.cached, Some(40));
    assert_eq!(tokens.reasoning, Some(30));
    assert_eq!(tokens.total(), Some(200));
}

#[test]
fn openai_unusable_payloads_yield_none() {
    let cases: [(&str, &[u8]); 7] = [
        ("empty", b""),
        ("whitespace", b"   \n\t "),
        ("malformed", b"{not-json"),
        ("missing-usage", br#"{"id":"x"}"#),
        (
            "zero-usage",
            br#"{"usage":{"prompt_tokens":0,"completion_tokens":0}}"#,
        ),
        ("null-usage", br#"{"usage":null}"#),
        ("wrong-top-type", b"[]"),
    ];
    for (name, payload) in cases {
        assert_eq!(parse_openai_usage(payload), None, "case {name}");
    }
}

/// 模型回复里写的 `usage` 不能冒充顶层信封。
///
/// 守护的 bug：用字节搜索定位 `"usage"`，计费数字就被模型自己编的覆盖。
#[test]
fn usage_inside_content_is_not_the_envelope() {
    let payload = br#"{
        "choices":[{"message":{"content":"{\"usage\":{\"prompt_tokens\":99}}"}}],
        "usage":{"prompt_tokens":3,"completion_tokens":1}
    }"#;
    let tokens = parse_openai_usage(payload).expect("top-level usage must win");
    assert_eq!(tokens.input, Some(3));
    assert_eq!(tokens.output, Some(1));
}

/// The o1 / o3 reasoning breakdown.
#[test]
fn openai_reasoning_breakdown_is_extracted() {
    let payload = br#"{"usage":{"prompt_tokens":10,"completion_tokens":500,
        "completion_tokens_details":{"reasoning_tokens":480}}}"#;
    let tokens = parse_openai_usage(payload).expect("reasoning envelope must parse");
    assert_eq!(tokens.output, Some(500));
    assert_eq!(tokens.reasoning, Some(480));
}

/// An omitted column is `None`, a reported zero is `Some(0)`.
/// `strict_usage_metadata_mode` reads this.
#[test]
fn openai_absent_column_is_none_while_reported_zero_is_some_zero() {
    let tokens = parse_openai_usage(br#"{"usage":{"prompt_tokens":7}}"#).unwrap();
    assert_eq!(tokens.input, Some(7));
    assert_eq!(tokens.output, None, "an omitted column must not read as 0");
    assert_eq!(tokens.cached, None);
    assert_eq!(tokens.reasoning, None);

    let tokens =
        parse_openai_usage(br#"{"usage":{"prompt_tokens":7,"completion_tokens":0}}"#).unwrap();
    assert_eq!(
        tokens.output,
        Some(0),
        "a reported zero must be distinguishable from an omission"
    );
}

#[test]
fn openai_non_numeric_column_fails_the_whole_parse() {
    // A non-numeric column fails the whole decode; a half-parsed tally would
    // be worse than none.
    assert_eq!(
        parse_openai_usage(br#"{"usage":{"prompt_tokens":"12"}}"#),
        None
    );
}

// --- Claude -----------------------------------------------------------------

#[test]
fn claude_message_delta_merges_both_halves() {
    let payload = br#"{
        "type": "message_delta",
        "message": {"usage": {"input_tokens": 150}},
        "delta": {"usage": {"output_tokens": 75}}
    }"#;
    let tokens = parse_claude_usage(payload).expect("message_delta must parse");
    assert_eq!(tokens.input, Some(150));
    assert_eq!(tokens.output, Some(75));
}

/// Creation + read both land in `cached`.
#[test]
fn claude_cache_creation_and_read_both_count_as_cached() {
    let payload = br#"{
        "type": "message",
        "usage": {
            "input_tokens": 200,
            "output_tokens": 60,
            "cache_creation_input_tokens": 45,
            "cache_read_input_tokens": 15
        }
    }"#;
    let tokens = parse_claude_usage(payload).expect("message must parse");
    assert_eq!(tokens.input, Some(200));
    assert_eq!(tokens.output, Some(60));
    assert_eq!(tokens.cached, Some(60), "creation + read");
}

#[test]
fn claude_unusable_payloads_yield_none() {
    let cases: [(&str, &[u8]); 4] = [
        ("empty", b""),
        ("malformed", b"{{"),
        ("no-usage", br#"{"type":"message_stop"}"#),
        (
            "all-zero",
            br#"{"usage":{"input_tokens":0,"output_tokens":0}}"#,
        ),
    ];
    for (name, payload) in cases {
        assert_eq!(parse_claude_usage(payload), None, "case {name}");
    }
}

#[test]
fn claude_takes_the_running_max_across_the_three_usage_sites() {
    // A message_start's input_tokens must not be clobbered by a later, smaller
    // echo of the same field.
    let payload = br#"{
        "usage": {"input_tokens": 10},
        "message": {"usage": {"input_tokens": 900}},
        "delta": {"usage": {"input_tokens": 5, "output_tokens": 42}}
    }"#;
    let tokens = parse_claude_usage(payload).unwrap();
    assert_eq!(tokens.input, Some(900));
    assert_eq!(tokens.output, Some(42));
}

// --- Gemini / Vertex --------------------------------------------------------

#[test]
fn gemini_usage_metadata_maps_thoughts_to_reasoning() {
    let payload = br#"{
        "usageMetadata": {
            "promptTokenCount": 300,
            "candidatesTokenCount": 90,
            "thoughtsTokenCount": 120,
            "cachedContentTokenCount": 50
        }
    }"#;
    let tokens = parse_gemini_usage(payload).expect("usageMetadata must parse");
    assert_eq!(tokens.input, Some(300));
    assert_eq!(tokens.output, Some(90));
    assert_eq!(tokens.reasoning, Some(120));
    assert_eq!(tokens.cached, Some(50));
}

#[test]
fn gemini_unusable_payloads_yield_none() {
    let cases: [(&str, &[u8]); 5] = [
        ("empty", b""),
        ("malformed", b"garbage"),
        ("no-meta", br#"{"candidates":[]}"#),
        ("empty-meta", br#"{"usageMetadata":{}}"#),
        ("null-meta", br#"{"usageMetadata":null}"#),
    ];
    for (name, payload) in cases {
        assert_eq!(parse_gemini_usage(payload), None, "case {name}");
    }
}

/// Vertex mirrors the Gemini envelope.
#[test]
fn vertex_matches_gemini_on_the_same_body() {
    let payload = br#"{"usageMetadata":{"promptTokenCount":500,
        "candidatesTokenCount":250,"thoughtsTokenCount":80}}"#;
    assert_eq!(parse_vertex_usage(payload), parse_gemini_usage(payload));
    let tokens = parse_vertex_usage(payload).unwrap();
    assert_eq!(tokens.input, Some(500));
    assert_eq!(tokens.output, Some(250));
    assert_eq!(tokens.reasoning, Some(80));

    assert_eq!(parse_vertex_usage(b""), None);
    assert_eq!(parse_vertex_usage(b"not-json"), None);
}

// --- Codex ------------------------------------------------------------------

#[test]
fn codex_delegates_to_the_openai_envelope() {
    let payload = br#"{
        "id": "cx-1",
        "usage": {
            "prompt_tokens": 42,
            "completion_tokens": 128,
            "prompt_tokens_details": {"cached_tokens": 12},
            "completion_tokens_details": {"reasoning_tokens": 64}
        }
    }"#;
    assert_eq!(parse_codex_usage(payload), parse_openai_usage(payload));
    let tokens = parse_codex_usage(payload).unwrap();
    assert_eq!(tokens.input, Some(42));
    assert_eq!(tokens.output, Some(128));
    assert_eq!(tokens.cached, Some(12));
    assert_eq!(tokens.reasoning, Some(64));

    assert_eq!(parse_codex_usage(b""), None);
}

// --- SSE scanners -----------------------------------------------------------

#[test]
fn sse_scanner_keeps_the_last_usage_bearing_event() {
    let body = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
                 data: {\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n\
                 data: {\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":22}}\n\n\
                 data: [DONE]\n\n";
    let tokens = parse_openai_stream_usage(body).expect("terminal usage event must be found");
    assert_eq!(tokens.input, Some(11));
    assert_eq!(tokens.output, Some(22));
}

#[test]
fn sse_scanner_falls_back_to_a_plain_json_body() {
    // The non-stream fast path: no SSE framing at all.
    let body = br#"{"usage":{"prompt_tokens":3,"completion_tokens":4}}"#;
    assert_eq!(parse_openai_stream_usage(body), parse_openai_usage(body));
}

#[test]
fn sse_scanner_ignores_done_and_unframed_lines() {
    let body = b": keep-alive comment\n\
                 event: ping\n\
                 data: [DONE]\n\
                 data:\n\
                 not-a-data-line {\"usage\":{\"prompt_tokens\":9}}\n";
    assert_eq!(parse_openai_stream_usage(body), None);
}

#[test]
fn codex_sse_scanner_matches_the_openai_one() {
    let body = b"data: {\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":6}}\n\ndata: [DONE]\n";
    assert_eq!(
        parse_codex_stream_usage(body),
        parse_openai_stream_usage(body)
    );
}

// --- tally algebra -----------------------------------------------------------

#[test]
fn max_merge_never_lets_none_beat_some() {
    let a = UsageTokens {
        input: Some(10),
        output: None,
        cached: Some(0),
        reasoning: None,
    };
    let b = UsageTokens {
        input: Some(4),
        output: Some(7),
        cached: None,
        reasoning: None,
    };
    let merged = UsageTokens::max_merge(a, b);
    assert_eq!(merged.input, Some(10));
    assert_eq!(merged.output, Some(7));
    assert_eq!(merged.cached, Some(0));
    assert_eq!(merged.reasoning, None);
}

#[test]
fn max_merge_is_commutative_and_idempotent() {
    let samples = [
        UsageTokens::default(),
        UsageTokens {
            input: Some(0),
            output: Some(3),
            cached: None,
            reasoning: Some(1),
        },
        UsageTokens {
            input: Some(9),
            output: None,
            cached: Some(2),
            reasoning: None,
        },
    ];
    for a in samples {
        assert_eq!(UsageTokens::max_merge(a, a), a, "idempotent");
        for b in samples {
            assert_eq!(
                UsageTokens::max_merge(a, b),
                UsageTokens::max_merge(b, a),
                "commutative"
            );
        }
    }
}

#[test]
fn total_is_none_only_when_neither_side_was_reported() {
    assert_eq!(UsageTokens::default().total(), None);
    assert_eq!(
        UsageTokens {
            input: Some(5),
            ..Default::default()
        }
        .total(),
        Some(5)
    );
    assert_eq!(
        UsageTokens {
            cached: Some(5),
            ..Default::default()
        }
        .total(),
        None,
        "cached alone says nothing about input+output"
    );
}

#[test]
fn to_record_carries_every_column_through_unchanged() {
    let tokens = UsageTokens {
        input: Some(1),
        output: None,
        cached: Some(0),
        reasoning: Some(3),
    };
    let record = tokens.to_record("gpt-x", "openai");
    assert_eq!(record.model, "gpt-x");
    assert_eq!(record.provider, "openai");
    assert_eq!(record.input_tokens, tokens.input);
    assert_eq!(record.output_tokens, tokens.output);
    assert_eq!(record.cached_tokens, tokens.cached);
    assert_eq!(record.reasoning_tokens, tokens.reasoning);
}

// --- parser / carrier boundary ----------------------------------------------

/// `None` is the only representation of "not presented".
#[test]
fn usage_detail_present_tracks_the_parse_outcome() {
    assert!(!usage_detail_present(None));
    assert!(usage_detail_present(Some(&UsageTokens {
        input: Some(1),
        ..Default::default()
    })));
}

/// Same invariant, driven by a deterministic sweep over the shape space:
/// present / absent / null / empty envelopes, zero and non-zero counts, and
/// non-JSON bytes.
#[test]
fn every_parser_is_deterministic_and_agrees_with_its_carrier() {
    /// A name + parser pair, one per exported `parse_*_usage` function.
    type NamedParser = (&'static str, fn(&[u8]) -> Option<UsageTokens>);

    let parsers: [NamedParser; 5] = [
        ("openai", parse_openai_usage),
        ("codex", parse_codex_usage),
        ("claude", parse_claude_usage),
        ("gemini", parse_gemini_usage),
        ("vertex", parse_vertex_usage),
    ];

    let mut checked = 0usize;
    for body in generated_bodies() {
        for (name, parse) in parsers {
            let first = parse(&body);
            let second = parse(&body);
            assert_eq!(
                first.is_some(),
                second.is_some(),
                "{name} parser is non-deterministic for {:?}",
                String::from_utf8_lossy(&body)
            );
            assert_eq!(
                usage_detail_present(first.as_ref()),
                first.is_some(),
                "{name} carrier disagrees with the parser"
            );
            // Whenever a tally is produced it must clear the non-zero bar, or
            // the fallback path would never fire on an all-zero envelope.
            if let Some(tokens) = first {
                assert!(
                    tokens.has_values(),
                    "{name} returned an all-zero tally for {:?}",
                    String::from_utf8_lossy(&body)
                );
            }
            checked += 1;
        }
    }
    assert!(
        checked >= 500,
        "sweep covered only {checked} parser invocations"
    );
}

/// Deterministic stand-in for three property generators: the cartesian product
/// of every envelope shape × every token-count pattern, plus the malformed and
/// empty variants mixed in.
fn generated_bodies() -> Vec<Vec<u8>> {
    let counts: [&str; 4] = ["0", "1", "100000", "7"];
    let mut bodies: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"   ".to_vec(),
        b"[1,2,3]".to_vec(),
        b"42".to_vec(),
        b"null".to_vec(),
        b"\"hello\"".to_vec(),
        b"{not valid json".to_vec(),
        vec![0xff, 0x00, 0xab, 0x7f, 0x10],
        b"{}".to_vec(),
    ];

    for a in counts {
        for b in counts {
            // OpenAI shape: full / partial / null / empty envelopes.
            bodies.push(
                format!(
                    r#"{{"id":"x","usage":{{"prompt_tokens":{a},"completion_tokens":{b},
                       "prompt_tokens_details":{{"cached_tokens":{a}}},
                       "completion_tokens_details":{{"reasoning_tokens":{b}}}}}}}"#
                )
                .into_bytes(),
            );
            bodies.push(format!(r#"{{"usage":{{"prompt_tokens":{a}}}}}"#).into_bytes());
            bodies.push(br#"{"usage":null}"#.to_vec());
            bodies.push(br#"{"usage":{}}"#.to_vec());

            // Claude shape: the three usage sites, independently present.
            bodies.push(
                format!(
                    r#"{{"type":"message_delta","usage":{{"input_tokens":{a}}},
                       "delta":{{"usage":{{"output_tokens":{b}}}}},
                       "message":{{"usage":{{"cache_read_input_tokens":{a},
                                             "cache_creation_input_tokens":{b}}}}}}}"#
                )
                .into_bytes(),
            );
            bodies.push(
                format!(r#"{{"type":"message_stop","delta":{{"text":"{a}"}}}}"#).into_bytes(),
            );

            // Gemini / Vertex shape.
            bodies.push(
                format!(
                    r#"{{"candidates":[],"usageMetadata":{{"promptTokenCount":{a},
                       "candidatesTokenCount":{b},"thoughtsTokenCount":{a},
                       "cachedContentTokenCount":{b},"totalTokenCount":{a}}}}}"#
                )
                .into_bytes(),
            );
            bodies.push(br#"{"usageMetadata":null}"#.to_vec());
        }
    }
    bodies
}

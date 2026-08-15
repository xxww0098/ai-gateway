//! Unit tests for [`crate::common`].
//!
//! The include_usage rewrite is covered by an exhaustive matrix over its
//! generator space, the stream-idle watchdog by its three behaviours.

use super::*;
use crate::usage::parse_openai_stream_usage;
use futures_util::StreamExt;
use serde_json::json;

// --- include_usage rewrite ---------------------------------------------------

/// The include_usage generator space, enumerated instead of sampled: 3
/// `stream` variants × 5 `stream_options` variants × 2 sets of unrelated sibling
/// fields.
fn include_usage_matrix() -> Vec<Value> {
    let stream_variants: [Option<Value>; 3] = [Some(json!(true)), Some(json!(false)), None];
    let options_variants: [Option<Value>; 5] = [
        None,
        Some(json!({})),
        Some(json!({"include_usage": true})),
        Some(json!({"include_usage": false})),
        Some(json!({"include_usage": false, "include_input_tokens": true})),
    ];
    let sibling_variants = [
        json!({}),
        json!({
            "model": "gpt-4o-mini",
            "max_tokens": 512,
            "temperature": 0.25,
            "messages": [{"role": "user", "content": "hello"}]
        }),
    ];

    let mut out = Vec::new();
    for stream in &stream_variants {
        for options in &options_variants {
            for siblings in &sibling_variants {
                let mut body = siblings.as_object().cloned().unwrap_or_default();
                if let Some(stream) = stream {
                    body.insert("stream".to_owned(), stream.clone());
                }
                if let Some(options) = options {
                    body.insert("stream_options".to_owned(), options.clone());
                }
                out.push(Value::Object(body));
            }
        }
    }
    out
}

#[test]
fn streaming_payloads_gain_include_usage_and_the_rewrite_is_idempotent() {
    for body in include_usage_matrix() {
        let payload = serde_json::to_vec(&body).expect("fixture must serialise");
        let streaming = body.get("stream") == Some(&Value::Bool(true));

        let first = ensure_include_usage(&payload).into_owned();
        let second = ensure_include_usage(&first).into_owned();
        assert_eq!(first, second, "rewrite is not idempotent for {body}");

        if !streaming {
            assert_eq!(
                first, payload,
                "a non-streaming payload must be returned untouched: {body}"
            );
            continue;
        }

        let rewritten: Value = serde_json::from_slice(&first).expect("rewrite must stay JSON");
        assert_eq!(
            rewritten.pointer("/stream_options/include_usage"),
            Some(&Value::Bool(true)),
            "streaming payload did not gain include_usage: {body}"
        );
    }
}

#[test]
fn rewrite_preserves_every_unrelated_key() {
    let payload = serde_json::to_vec(&json!({
        "stream": true,
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}],
        "stream_options": {"include_input_tokens": true, "include_usage": false}
    }))
    .unwrap();

    let rewritten: Value = serde_json::from_slice(&ensure_include_usage(&payload)).unwrap();
    assert_eq!(rewritten["model"], json!("gpt-4o"));
    assert_eq!(rewritten["messages"][0]["role"], json!("user"));
    assert_eq!(
        rewritten["stream_options"]["include_input_tokens"],
        json!(true),
        "sibling stream_options keys must survive"
    );
    assert_eq!(rewritten["stream_options"]["include_usage"], json!(true));
}

/// The rewrite is advisory — the billing fallback path compensates — so an
/// unparseable body must pass through rather than fail the request.
#[test]
fn unparseable_or_non_object_payloads_pass_through_unchanged() {
    let cases: [(&str, &[u8]); 14] = [
        ("empty", b""),
        ("random bytes", &[0xff, 0x00, 0xab, 0x7f, 0x10]),
        ("plain text", b"hello, not json"),
        ("unopened brace", br#""stream":true}"#),
        ("truncated object", br#"{"stream":true"#),
        ("truncated nested", br#"{"stream":true,"stream_options":"#),
        ("unclosed string", br#"{"stream":"tr"#),
        ("invalid token", b"{not valid json}"),
        ("partial array", b"[1,2,"),
        ("json array top-level", b"[1,2,3]"),
        ("json primitive number", b"42"),
        ("json primitive string", br#""hello""#),
        ("json null", b"null"),
        ("stream as string", br#"{"stream":"true"}"#),
    ];
    for (name, input) in cases {
        assert_eq!(
            ensure_include_usage(input).as_ref(),
            input,
            "case {name} was modified"
        );
    }
}

// --- small helpers -----------------------------------------------------------

#[test]
fn token_estimate_rounds_up_to_the_next_whole_token() {
    assert_eq!(approximate_tokens_from_bytes(0), 0);
    // Monotonic, and never under-counts a partial token.
    let mut previous = 0;
    for size in 1..64usize {
        let got = approximate_tokens_from_bytes(size);
        assert!(got >= previous, "estimate must be monotonic in size");
        assert!(got * 4 >= size as i64, "estimate must not under-count");
        assert!((got - 1) * 4 < size as i64, "estimate must not over-count");
        previous = got;
    }
}

#[test]
fn failure_body_is_clipped_without_splitting_a_code_point() {
    let long = "é".repeat(8 * 1024);
    let clipped = truncate_failure_body(long.as_bytes());
    assert!(clipped.len() <= 4 * 1024);
    assert!(long.starts_with(&clipped) || clipped.ends_with('\u{fffd}'));
    // Short bodies survive intact, including non-UTF-8 ones.
    assert_eq!(truncate_failure_body(b"boom"), "boom");
    assert!(!truncate_failure_body(&[0xff, 0xfe]).is_empty());
}

#[test]
fn requested_model_prefers_the_translated_name_then_the_router_hint() {
    let mut req = ProviderRequest {
        model: "  gpt-4o  ".to_owned(),
        ..Default::default()
    };
    req.metadata
        .insert(REQUESTED_MODEL_METADATA_KEY.to_owned(), "alias".to_owned());
    assert_eq!(requested_model(&req), "gpt-4o");

    req.model = "   ".to_owned();
    assert_eq!(requested_model(&req), "alias");

    req.metadata.clear();
    assert_eq!(requested_model(&req), "");
}

#[test]
fn string_from_map_coerces_scalars_and_treats_null_as_absent() {
    let values = json!({
        "text": "  padded  ",
        "blank": "   ",
        "int": 42,
        "float_integral": 3.0,
        "float": 1.5,
        "yes": true,
        "nothing": null,
        "nested": {"a": 1}
    });
    assert_eq!(string_from_map(&values, "text").as_deref(), Some("padded"));
    assert_eq!(string_from_map(&values, "int").as_deref(), Some("42"));
    assert_eq!(
        string_from_map(&values, "float_integral").as_deref(),
        Some("3")
    );
    assert_eq!(string_from_map(&values, "float").as_deref(), Some("1.5"));
    assert_eq!(string_from_map(&values, "yes").as_deref(), Some("true"));
    assert_eq!(
        string_from_map(&values, "nothing"),
        None,
        "a JSON null is absent, not the literal string \"null\""
    );
    assert_eq!(
        string_from_map(&values, "blank"),
        None,
        "whitespace is not a credential"
    );
    assert_eq!(string_from_map(&values, "missing"), None);
    assert_eq!(
        string_from_map(&values, "nested").as_deref(),
        Some(r#"{"a":1}"#)
    );
    assert_eq!(string_from_map(&json!("not an object"), "k"), None);
}

#[test]
fn nested_string_reads_through_an_object_or_an_embedded_json_document() {
    let nested_obj = json!({"token_data": {"access_token": "  abc  "}});
    assert_eq!(
        nested_string(&nested_obj, "token_data", "access_token").as_deref(),
        Some("abc")
    );

    let embedded = json!({"token_data": " {\"access_token\":\"xyz\"} "});
    assert_eq!(
        nested_string(&embedded, "token_data", "access_token").as_deref(),
        Some("xyz")
    );

    for broken in [
        json!({"token_data": "{not json"}),
        json!({"token_data": 7}),
        json!({"token_data": null}),
        json!({}),
    ] {
        assert_eq!(
            nested_string(&broken, "token_data", "access_token"),
            None,
            "{broken}"
        );
    }
}

// --- endpoint construction ---------------------------------------------------

#[test]
fn every_base_url_shape_converges_on_one_endpoint() {
    let expected = "https://api.example.com/v1/chat/completions";
    for base in [
        "https://api.example.com",
        "https://api.example.com/",
        "https://api.example.com/v1",
        "https://api.example.com/v1/",
        "https://api.example.com/v1/chat/completions",
        "  https://api.example.com/v1/chat/completions/  ",
    ] {
        assert_eq!(
            chat_completions_endpoint(base, &[]).unwrap(),
            expected,
            "base {base}"
        );
    }
}

#[test]
fn inbound_query_parameters_are_appended_including_duplicate_keys() {
    let query = vec![
        ("beta".to_owned(), "1".to_owned()),
        ("tag".to_owned(), "a".to_owned()),
        ("tag".to_owned(), "b".to_owned()),
    ];
    let endpoint = chat_completions_endpoint("https://api.example.com/v1", &query).unwrap();
    let parsed = url::Url::parse(&endpoint).unwrap();
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(pairs, query, "order and duplicate keys must both survive");
}

#[test]
fn a_base_url_without_a_host_is_rejected() {
    for base in ["", "not-a-url", "/v1", "https://"] {
        assert!(
            chat_completions_endpoint(base, &[]).is_err(),
            "base {base:?} should not produce an endpoint"
        );
    }
}

// --- stream idle watchdog ----------------------------------------------------

#[tokio::test]
async fn a_stalled_stream_is_terminated_by_the_watchdog() {
    let stalling = futures_util::stream::once(async {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        1u8
    });
    let mut guarded = Box::pin(with_stream_idle_timeout(
        stalling,
        Duration::from_millis(40),
    ));

    assert_eq!(guarded.next().await, Some(Err(StreamIdleElapsed)));
    assert_eq!(
        guarded.next().await,
        None,
        "the stream must end after the idle error, not keep waiting"
    );
}

/// Every delivered item restarts the window.
#[tokio::test]
async fn steady_traffic_keeps_the_stream_alive() {
    let ticking = futures_util::stream::unfold(0u8, |i| async move {
        if i >= 5 {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
        Some((i, i + 1))
    });
    let guarded = with_stream_idle_timeout(ticking, Duration::from_millis(400));
    let items: Vec<_> = guarded.collect().await;
    assert_eq!(items.len(), 5);
    assert!(
        items.iter().all(Result::is_ok),
        "no gap exceeded the idle window, so nothing may be reported as idle"
    );
}

/// A non-positive idle passes the inner stream through untouched.
#[tokio::test]
async fn a_zero_idle_window_disables_the_watchdog() {
    let inner = futures_util::stream::once(async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        7u8
    });
    let guarded = with_stream_idle_timeout(inner, Duration::ZERO);
    let items: Vec<_> = guarded.collect().await;
    assert_eq!(items, vec![Ok(7)]);
}

// --- streamed usage accumulation ---------------------------------------------

fn sse(chunks: &[&'static str]) -> Vec<reqwest::Result<Bytes>> {
    chunks
        .iter()
        .map(|c| Ok(Bytes::from_static(c.as_bytes())))
        .collect()
}

#[tokio::test]
async fn payloads_are_forwarded_verbatim_and_followed_by_a_usage_chunk() {
    let body = futures_util::stream::iter(sse(&[
        "data: {\"choices\":[{\"delta\":{\"content\":\"he\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"llo\"}}]}\n\n",
        "data: {\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":34}}\n\ndata: [DONE]\n\n",
    ]));
    let chunks: Vec<StreamChunk> = usage_stream(
        body,
        Duration::ZERO,
        "gpt-4o".to_owned(),
        PROVIDER_OPENAI,
        parse_openai_stream_usage,
        200,
    )
    .collect()
    .await;

    let (last, payloads) = chunks.split_last().expect("at least one chunk");
    assert_eq!(payloads.len(), 3, "every upstream chunk must be forwarded");
    let forwarded: Vec<u8> = payloads
        .iter()
        .flat_map(|c| match c {
            StreamChunk::Payload(bytes) => bytes.to_vec(),
            other => panic!("expected a payload chunk, got {other:?}"),
        })
        .collect();
    assert!(forwarded.starts_with(b"data: {\"choices\""));

    match last {
        StreamChunk::Usage(record) => {
            assert_eq!(record.model, "gpt-4o");
            assert_eq!(record.provider, PROVIDER_OPENAI);
            assert_eq!(record.input_tokens, Some(12));
            assert_eq!(record.output_tokens, Some(34));
            assert_eq!(record.cached_tokens, None, "an omitted column stays absent");
        }
        other => panic!("expected a trailing usage chunk, got {other:?}"),
    }
}

#[tokio::test]
async fn a_stream_without_a_usage_envelope_emits_no_usage_chunk() {
    // Absence of the chunk is the signal strict-usage-metadata mode keys off.
    let body = futures_util::stream::iter(sse(&[
        "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
        "data: [DONE]\n\n",
    ]));
    let chunks: Vec<StreamChunk> = usage_stream(
        body,
        Duration::ZERO,
        "gpt-4o".to_owned(),
        PROVIDER_OPENAI,
        parse_openai_stream_usage,
        200,
    )
    .collect()
    .await;

    assert_eq!(chunks.len(), 2);
    assert!(
        !chunks.iter().any(|c| matches!(c, StreamChunk::Usage(_))),
        "no usage envelope upstream means no usage chunk downstream"
    );
}

#[tokio::test]
async fn usage_survives_a_body_longer_than_the_retained_window() {
    let mut chunks: Vec<reqwest::Result<Bytes>> = Vec::new();
    chunks.push(Ok(Bytes::from_static(b"data: {\"choices\":[]}\n\n")));
    for _ in 0..64 {
        chunks.push(Ok(Bytes::from(vec![b'x'; 4096])));
    }
    chunks.push(Ok(Bytes::from_static(
        b"\ndata: {\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":6}}\n\ndata: [DONE]\n\n",
    )));

    let out: Vec<StreamChunk> = usage_stream(
        futures_util::stream::iter(chunks),
        Duration::ZERO,
        "m".to_owned(),
        PROVIDER_OPENAI,
        parse_openai_stream_usage,
        200,
    )
    .collect()
    .await;

    match out.last().expect("chunks") {
        StreamChunk::Usage(record) => {
            assert_eq!(record.input_tokens, Some(5));
            assert_eq!(record.output_tokens, Some(6));
        }
        other => panic!("terminal usage lost past the buffer window: {other:?}"),
    }
}

#[tokio::test]
async fn an_idle_stall_mid_stream_surfaces_an_error_chunk_then_settles_usage() {
    let body = futures_util::stream::unfold(0u8, |i| async move {
        match i {
            0 => Some((
                Ok(Bytes::from_static(
                    b"data: {\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2}}\n\n",
                )),
                1,
            )),
            _ => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                None
            }
        }
    });
    // 206 rather than 200: the relay cannot have guessed it, so an error chunk
    // carrying it proves the status was threaded through from the response
    // rather than assumed.
    let out: Vec<StreamChunk> = usage_stream(
        body,
        Duration::from_millis(40),
        "m".to_owned(),
        PROVIDER_OPENAI,
        parse_openai_stream_usage,
        206,
    )
    .collect()
    .await;

    assert!(matches!(out.first(), Some(StreamChunk::Payload(_))));
    match out.get(1) {
        Some(StreamChunk::Error { status, message }) => {
            assert_eq!(*status, Some(206), "the relayed status must survive");
            assert!(!message.is_empty(), "a stall must say what happened");
        }
        other => panic!("the stall must be reported: {other:?}"),
    }
    // A truncated stream still bills whatever usage did arrive.
    assert!(matches!(out.get(2), Some(StreamChunk::Usage(_))), "{out:?}");
}

// --- upstream status / header pass-through -----------------------------------

/// Builds a `reqwest::Response` without a socket, so the assembly step can be
/// tested for what it forwards rather than for what it can reach.
fn upstream_response(
    status: u16,
    headers: &[(&str, &str)],
    body: &'static str,
) -> reqwest::Response {
    let mut builder = http::Response::builder().status(status);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    reqwest::Response::from(builder.body(body).expect("a well-formed response"))
}

/// The defect this guards: with only a chunk stream to return, a relay has
/// nothing to answer with but a hardcoded `200 text/event-stream`, and every
/// header the upstream set is dropped on the floor.
#[tokio::test]
async fn the_upstream_status_and_headers_reach_the_relay() {
    let response = upstream_response(
        206,
        &[
            ("content-type", "text/event-stream; charset=utf-8"),
            ("x-request-id", "req_abc123"),
            ("anthropic-ratelimit-requests-remaining", "41"),
        ],
        "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
    );

    let assembled = stream_response(response, |response, status| {
        usage_stream(
            response.bytes_stream(),
            Duration::ZERO,
            "m".to_owned(),
            PROVIDER_OPENAI,
            parse_openai_stream_usage,
            status,
        )
    });

    assert_eq!(assembled.status, 206, "a status must never be assumed");
    assert_eq!(
        assembled.headers.get("x-request-id").map(|v| v.as_bytes()),
        Some(&b"req_abc123"[..]),
        "an upstream trace header must survive the relay"
    );
    assert_eq!(
        assembled
            .headers
            .get("anthropic-ratelimit-requests-remaining")
            .map(|v| v.as_bytes()),
        Some(&b"41"[..]),
        "rate-limit headers are the caller's only budget signal"
    );
    assert_eq!(
        assembled
            .headers
            .get(http::header::CONTENT_TYPE)
            .map(|v| v.as_bytes()),
        Some(&b"text/event-stream; charset=utf-8"[..]),
        "the upstream's own framing must not be overwritten"
    );

    let relayed: Vec<u8> = assembled
        .chunks
        .filter_map(|c| async move {
            match c {
                StreamChunk::Payload(bytes) => Some(bytes.to_vec()),
                _ => None,
            }
        })
        .concat()
        .await;
    assert!(
        relayed.starts_with(b"data: {\"choices\""),
        "headers must not come at the cost of the body"
    );
}

/// Multi-valued headers survive as a list rather than collapsing to the last
/// one.
#[tokio::test]
async fn a_repeated_upstream_header_keeps_every_value() {
    let response = upstream_response(
        200,
        &[("set-cookie", "a=1"), ("set-cookie", "b=2")],
        "data: [DONE]\n\n",
    );

    let assembled = stream_response(response, |response, status| {
        usage_stream(
            response.bytes_stream(),
            Duration::ZERO,
            "m".to_owned(),
            PROVIDER_OPENAI,
            parse_openai_stream_usage,
            status,
        )
    });

    let values: Vec<&[u8]> = assembled
        .headers
        .get_all("set-cookie")
        .iter()
        .map(http::HeaderValue::as_bytes)
        .collect();
    assert_eq!(values, vec![&b"a=1"[..], &b"b=2"[..]]);
}

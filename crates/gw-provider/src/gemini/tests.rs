//! Unit tests for the Generative Language API executor.

use gw_authcore::AuthRecord;
use serde_json::json;

use super::*;

fn auth_with(metadata: serde_json::Value) -> AuthRecord {
    AuthRecord {
        metadata,
        ..AuthRecord::new("auth-1", PROVIDER_GEMINI, chrono::Utc::now())
    }
}

fn provider(base_url: &str, api_key: &str) -> GeminiProvider {
    GeminiProvider::new(
        &ProviderConfig {
            base_url: base_url.to_owned(),
            api_key: api_key.to_owned(),
            enabled: true,
        },
        0,
    )
    .expect("provider")
}

fn endpoint(query: &[(String, String)], model: &str, stream: bool) -> Url {
    GeminiProvider::generate_content_endpoint(query, "https://gl.example.com", model, stream)
        .expect("endpoint")
}

fn values_of(url: &Url, key: &str) -> Vec<String> {
    url.query_pairs()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .collect()
}

// --- endpoint ---------------------------------------------------------------

/// Streaming and non-streaming differ only in the action verb; sharing the rest
/// of the path is what keeps a custom `base_url` working for both.
#[test]
fn streaming_changes_only_the_action_verb() {
    let plain = endpoint(&[], "gemini-2.5-pro", false);
    let streamed = endpoint(&[], "gemini-2.5-pro", true);
    assert_ne!(plain.path(), streamed.path());
    assert_eq!(
        plain.path().rsplit_once(':').map(|(head, _)| head),
        streamed.path().rsplit_once(':').map(|(head, _)| head),
        "only the trailing verb may differ"
    );
}

/// The usage relay parses SSE framing, so the caller must not be able to pick a
/// different one.
#[test]
fn a_caller_cannot_downgrade_the_stream_framing() {
    let url = endpoint(
        &[("alt".to_owned(), "json".to_owned())],
        "gemini-2.5-pro",
        true,
    );
    assert_eq!(values_of(&url, "alt"), vec!["sse".to_owned()]);
}

#[test]
fn unrelated_caller_parameters_survive_the_forced_parameter() {
    let url = endpoint(
        &[
            ("tuning".to_owned(), "on".to_owned()),
            ("alt".to_owned(), "json".to_owned()),
        ],
        "gemini-2.5-pro",
        true,
    );
    assert_eq!(values_of(&url, "tuning"), vec!["on".to_owned()]);
}

#[test]
fn a_non_streaming_request_carries_no_framing_parameter() {
    assert!(values_of(&endpoint(&[], "gemini-2.5-pro", false), "alt").is_empty());
}

/// A model name is caller-controlled; a slash inside one must not add a path
/// segment and repoint the request.
#[test]
fn a_slash_in_the_model_name_cannot_add_a_path_segment() {
    let benign = endpoint(&[], "gemini-2.5-pro", false);
    let hostile = endpoint(&[], "../../v1/tunedModels/evil", false);
    assert_eq!(
        benign.path_segments().map(Iterator::count),
        hostile.path_segments().map(Iterator::count),
        "an injected model name must not change the shape of the path"
    );
}

#[test]
fn a_base_url_without_a_host_is_rejected() {
    assert!(GeminiProvider::generate_content_endpoint(&[], "gl.example.com", "m", false).is_err());
    assert!(
        GeminiProvider::generate_content_endpoint(&[], "https://", "m", false).is_err(),
        "a hostless URL must not re-parse with a path segment as the host"
    );
    assert!(
        GeminiProvider::new(
            &ProviderConfig {
                base_url: "gl.example.com".to_owned(),
                api_key: String::new(),
                enabled: true,
            },
            0
        )
        .is_err()
    );
}

// --- credentials ------------------------------------------------------------

#[test]
fn credential_precedence_prefers_the_most_specific_rung() {
    let provider = provider("https://gl.example.com", "from-config");
    let cases = [
        (
            json!({
                "api_key": "top-api-key",
                "access_token": "top-access-token",
                "token_data": {"api_key": "nested-api-key", "access_token": "nested-access-token"},
            }),
            "top-api-key",
        ),
        (
            json!({
                "access_token": "top-access-token",
                "token_data": {"api_key": "nested-api-key", "access_token": "nested-access-token"},
            }),
            "nested-api-key",
        ),
        (
            json!({
                "access_token": "top-access-token",
                "token_data": {"access_token": "nested-access-token"},
            }),
            "top-access-token",
        ),
        (
            json!({"token_data": {"access_token": "nested-access-token"}}),
            "nested-access-token",
        ),
        (json!({}), "from-config"),
    ];
    for (metadata, expected) in cases {
        assert_eq!(
            provider.resolve_credentials(Some(&auth_with(metadata))).0,
            expected
        );
    }
    assert_eq!(provider.resolve_credentials(None).0, "from-config");
}

#[test]
fn a_record_can_override_the_base_url() {
    let provider = provider("https://gl.example.com", "k");
    for key in ["base_url", "base-url"] {
        let mut auth = auth_with(json!({}));
        auth.attributes
            .insert(key.to_owned(), "https://override.example.com/".to_owned());
        assert_eq!(
            provider.resolve_credentials(Some(&auth)).1,
            "https://override.example.com"
        );
    }
}

#[test]
fn the_api_key_travels_as_a_header_when_one_is_configured() {
    let mut headers = HeaderMap::new();
    GeminiProvider::inject_api_key(&mut headers, "  secret  ").expect("inject");
    assert_eq!(headers["x-goog-api-key"], "secret");
}

/// With no configured key the request still goes out unauthenticated: the
/// caller may have supplied `?key=`, and Google is the right place to reject a
/// request that has neither.
#[test]
fn a_missing_api_key_is_not_turned_into_a_header() {
    for key in ["", "   "] {
        let mut headers = HeaderMap::new();
        GeminiProvider::inject_api_key(&mut headers, key).expect("inject");
        assert!(headers.is_empty());
    }
}

#[test]
fn an_api_key_that_cannot_be_a_header_value_is_reported_not_panicked() {
    let mut headers = HeaderMap::new();
    let err = GeminiProvider::inject_api_key(&mut headers, "bad\nkey").expect_err("rejected");
    assert!(matches!(err, ProviderError::Credential(_)));
}

// --- stream usage -----------------------------------------------------------

/// `usageMetadata` is cumulative, so the last frame — not the largest, and not
/// a merge — is authoritative.
#[test]
fn the_last_sse_frame_wins() {
    let body = concat!(
        "data: {\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5}}\n",
        "\n",
        "data: {\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":42}}\n",
        "\n",
        "data: [DONE]\n",
    );
    let tokens = parse_gemini_stream_usage(body.as_bytes()).expect("usage");
    assert_eq!((tokens.input, tokens.output), (Some(10), Some(42)));
}

#[test]
fn blank_line_separated_json_chunks_are_understood_too() {
    let body = concat!(
        "{\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5}}\n",
        "\n",
        "{\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":42}}\n",
    );
    let tokens = parse_gemini_stream_usage(body.as_bytes()).expect("usage");
    assert_eq!((tokens.input, tokens.output), (Some(10), Some(42)));
}

#[test]
fn reasoning_and_cache_columns_survive_the_stream_parse() {
    let body = concat!(
        "data: {\"usageMetadata\":{\"promptTokenCount\":9,\"candidatesTokenCount\":3,",
        "\"thoughtsTokenCount\":88,\"cachedContentTokenCount\":4}}\n",
    );
    let tokens = parse_gemini_stream_usage(body.as_bytes()).expect("usage");
    assert_eq!(tokens.reasoning, Some(88));
    assert_eq!(tokens.cached, Some(4));
}

#[test]
fn a_plain_json_body_takes_the_fast_path() {
    let tokens = parse_gemini_stream_usage(
        br#"{"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":2}}"#,
    )
    .expect("usage");
    assert_eq!((tokens.input, tokens.output), (Some(1), Some(2)));
}

#[test]
fn a_stream_with_no_usage_metadata_yields_no_tally() {
    assert!(parse_gemini_stream_usage(b"data: {\"candidates\":[]}\n\ndata: [DONE]\n").is_none());
    assert!(parse_gemini_stream_usage(b"data: [DONE]\n").is_none());
    assert!(parse_gemini_stream_usage(b"").is_none());
}

// --- provider surface -------------------------------------------------------

/// An API-key provider has nothing to rotate, but refusing the record would
/// fail the whole credential load at startup.
#[tokio::test]
async fn refresh_reactivates_a_record_without_touching_its_metadata() {
    let provider = provider("https://gl.example.com", "k");
    let mut auth = auth_with(json!({"api_key": "unchanged"}));
    auth.status = AuthStatus::Error;

    let refreshed = provider.refresh(&auth).await.expect("refresh");
    assert_eq!(refreshed.status, AuthStatus::Active);
    assert_eq!(refreshed.metadata, auth.metadata);
    assert!(refreshed.updated_at >= auth.updated_at);
}

#[tokio::test]
async fn token_counting_grows_with_the_payload_and_never_goes_negative() {
    let provider = provider("https://gl.example.com", "k");
    let auth = auth_with(json!({}));
    let mut previous = -1;
    for len in [0, 4, 400, 4000] {
        let count = provider
            .count_tokens(
                &auth,
                ProviderRequest {
                    payload: vec![b'x'; len],
                    ..Default::default()
                },
            )
            .await
            .expect("count");
        assert!(count >= 0);
        assert!(count > previous, "{len} bytes should cost more than fewer");
        previous = count;
    }
}

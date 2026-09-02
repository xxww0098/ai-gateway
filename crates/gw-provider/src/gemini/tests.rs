//! Unit tests for the Generative Language API executor.

use gw_authcore::AuthRecord;
use serde_json::json;

use super::*;
use bytes::Bytes;

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
    GeminiProvider::generate_content_endpoint(None, query, "https://gl.example.com", model, stream)
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
    assert!(
        GeminiProvider::generate_content_endpoint(None, &[], "gl.example.com", "m", false).is_err()
    );
    assert!(
        GeminiProvider::generate_content_endpoint(None, &[], "https://", "m", false).is_err(),
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
fn the_api_key_travels_as_a_credential_not_as_a_header() {
    // The relay sets `x-goog-api-key` from `RoutePlan::credential`, after
    // stripping whatever the client sent. A planner that also stamped the
    // header would defeat the strip.
    let provider = provider("https://gl.example.com", "k");
    let plan = provider
        .plan_generate_content(
            &model_request("gemini-2.5-pro"),
            "  secret  ",
            "https://gl.example.com",
        )
        .expect("plans");
    assert!(!plan.headers.contains_key("x-goog-api-key"));
    assert!(matches!(&plan.credential, gw_relay::Credential::GoogleApiKey(k) if k == "secret"));
}

/// A keyless account is refused here rather than sent as an empty
/// `x-goog-api-key`: the relay always sets the credential header, so an empty
/// one would reach Google as a malformed request instead of a missing one —
/// and refusing lets the dispatcher fail over to an account that has a key.
#[test]
fn a_missing_api_key_is_refused_rather_than_sent_empty() {
    let provider = provider("https://gl.example.com", "k");
    for key in ["", "   "] {
        let err = provider
            .plan_generate_content(
                &model_request("gemini-2.5-pro"),
                key,
                "https://gl.example.com",
            )
            .expect_err("a keyless account must not produce a plan");
        assert!(matches!(err, ProviderError::Credential(_)), "{err:?}");
    }
}

#[test]
fn a_request_without_a_model_is_refused() {
    let provider = provider("https://gl.example.com", "k");
    let err = provider
        .plan_generate_content(&ProviderRequest::default(), "k", "https://gl.example.com")
        .expect_err("GenerateContent needs a model in the path");
    assert!(matches!(err, ProviderError::Other(_)), "{err:?}");
}

/// A request whose model the planner needs in the URL path.
fn model_request(model: &str) -> ProviderRequest {
    ProviderRequest {
        model: model.to_owned(),
        ..Default::default()
    }
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

/// 增量探测器与整 body 扫描器必须给出同一个结论 —— 三种 framing 都是。
///
/// 这条把「按行喂」与「一次喂完」对账：Gemini 会对不同调用方分别用 SSE 帧与
/// 空行分隔的 JSON chunk 作答，两种都得走通。
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

/// Google 上游确实有 `:countTokens`，但 `count_tokens` 的唯一入口是 Anthropic
/// 方言的 `POST /v1/messages/count_tokens` —— 方言对不上，所以这里**报错**，
/// 而不是回到 `payload.len() / 4` 的伪造值
/// （`docs/relay-surface-plan.md` §2.1 缺陷 ①）。
#[tokio::test]
async fn token_counting_refuses_rather_than_fabricating_a_number() {
    let provider = provider("https://gl.example.com", "k");
    let auth = auth_with(json!({}));
    for len in [0, 4, 400, 4000] {
        assert!(
            provider
                .plan_count_tokens(
                    &auth,
                    &ProviderRequest {
                        payload: Bytes::from(vec![b'x'; len]),
                        ..Default::default()
                    },
                )
                .await
                .is_err(),
            "{len} bytes produced a fabricated count"
        );
    }
}

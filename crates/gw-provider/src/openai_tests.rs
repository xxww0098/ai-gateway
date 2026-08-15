//! Unit tests for [`crate::openai`].
//!
//! These lock in the executor's wiring: endpoint shape, credential precedence
//! and the outbound header contract.

use super::*;
use crate::types::is_skipped_proxy_header;
use gw_authcore::AuthRecord;
use serde_json::json;
use std::collections::HashMap;

fn config(base_url: &str) -> ProviderConfig {
    ProviderConfig {
        base_url: base_url.to_owned(),
        api_key: "sk-config".to_owned(),
        enabled: true,
    }
}

fn provider() -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(&config("https://api.example.com/v1/"), 0)
        .expect("valid config must build")
}

fn auth(metadata: serde_json::Value, attributes: &[(&str, &str)]) -> AuthRecord {
    AuthRecord {
        id: "auth-1".to_owned(),
        provider: PROVIDER_OPENAI.to_owned(),
        label: "test".to_owned(),
        attributes: attributes
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<HashMap<_, _>>(),
        metadata,
        ..Default::default()
    }
}

fn bare_auth() -> AuthRecord {
    auth(json!({}), &[])
}

// --- construction ------------------------------------------------------------

#[test]
fn construction_requires_an_enabled_provider_with_both_credentials() {
    let cases = [
        (
            "disabled",
            ProviderConfig {
                enabled: false,
                ..config("https://api.example.com")
            },
        ),
        (
            "no base url",
            ProviderConfig {
                base_url: "   ".to_owned(),
                ..config("")
            },
        ),
        (
            "no api key",
            ProviderConfig {
                api_key: String::new(),
                ..config("https://api.example.com")
            },
        ),
        ("hostless base url", config("not-a-url")),
        ("scheme only", config("https://")),
    ];
    for (name, cfg) in cases {
        assert!(
            OpenAiCompatibleProvider::new(&cfg, 0).is_err(),
            "case {name} should not build"
        );
    }
}

#[test]
fn construction_trims_the_base_url_and_reports_the_provider_key() {
    let provider =
        OpenAiCompatibleProvider::new(&config("  https://api.example.com/v1/  "), 30).unwrap();
    assert_eq!(provider.base_url(), "https://api.example.com/v1");
    assert_eq!(provider.name(), PROVIDER_OPENAI);
}

#[test]
fn a_non_positive_timeout_falls_back_to_the_default() {
    let fast = OpenAiCompatibleProvider::new(&config("https://api.example.com"), 5).unwrap();
    let defaulted = OpenAiCompatibleProvider::new(&config("https://api.example.com"), 0).unwrap();
    assert_eq!(fast.timeout, Duration::from_secs(5));
    assert_eq!(defaulted.timeout, crate::common::DEFAULT_TIMEOUT);
}

// --- credential resolution ----------------------------------------------------

#[test]
fn credential_metadata_overrides_the_configured_api_key() {
    let provider = provider();
    let (key, base) = provider.resolve_credentials(&auth(json!({"api_key": " sk-auth "}), &[]));
    assert_eq!(key, "sk-auth");
    assert_eq!(base, provider.base_url());

    let (key, _) = provider.resolve_credentials(&auth(json!({"api_key": ""}), &[]));
    assert_eq!(key, "sk-config", "a blank override must not win");

    let (key, _) = provider.resolve_credentials(&bare_auth());
    assert_eq!(key, "sk-config");
}

#[test]
fn both_base_url_spellings_are_accepted_with_underscore_winning() {
    let provider = provider();
    let (_, base) = provider.resolve_credentials(&auth(
        json!({}),
        &[
            ("base_url", "https://a.example.com/"),
            ("base-url", "https://b.example.com"),
        ],
    ));
    assert_eq!(base, "https://a.example.com");

    let (_, base) =
        provider.resolve_credentials(&auth(json!({}), &[("base-url", "https://b.example.com")]));
    assert_eq!(base, "https://b.example.com");
}

// --- outbound request ---------------------------------------------------------

fn built(
    provider: &OpenAiCompatibleProvider,
    req: &ProviderRequest,
    stream: bool,
) -> reqwest::Request {
    provider
        .build_request(req, stream, "sk-live", provider.base_url())
        .expect("request must build")
        .build()
        .expect("request must be valid")
}

#[test]
fn the_outbound_request_carries_the_bearer_token_and_json_defaults() {
    let provider = provider();
    let req = ProviderRequest {
        payload: br#"{"model":"gpt-4o"}"#.to_vec(),
        ..Default::default()
    };
    let request = built(&provider, &req, false);

    assert_eq!(request.method(), http::Method::POST);
    assert_eq!(
        request.url().as_str(),
        "https://api.example.com/v1/chat/completions"
    );
    assert_eq!(request.headers()[AUTHORIZATION], "Bearer sk-live");
    assert_eq!(request.headers()[CONTENT_TYPE], "application/json");
    assert_eq!(request.headers()[ACCEPT], "application/json");
    assert_eq!(
        request.body().and_then(reqwest::Body::as_bytes),
        Some(req.payload.as_slice()),
        "a non-streaming payload must go out untouched"
    );
}

#[test]
fn streaming_requests_ask_for_sse_and_force_include_usage() {
    let provider = provider();
    let req = ProviderRequest {
        payload: br#"{"model":"gpt-4o","stream":true}"#.to_vec(),
        stream: true,
        ..Default::default()
    };
    let request = built(&provider, &req, true);

    assert_eq!(request.headers()[ACCEPT], "text/event-stream");
    let sent: serde_json::Value =
        serde_json::from_slice(request.body().and_then(reqwest::Body::as_bytes).unwrap()).unwrap();
    assert_eq!(
        sent.pointer("/stream_options/include_usage"),
        Some(&json!(true)),
        "the terminal usage envelope must be requested"
    );
}

#[test]
fn a_body_that_does_not_declare_stream_is_never_rewritten() {
    // Defense in depth: the caller's `stream` flag alone must not mutate a
    // payload whose body says otherwise.
    let provider = provider();
    let req = ProviderRequest {
        payload: br#"{"model":"gpt-4o"}"#.to_vec(),
        stream: true,
        ..Default::default()
    };
    let request = built(&provider, &req, true);
    assert_eq!(
        request.body().and_then(reqwest::Body::as_bytes),
        Some(req.payload.as_slice())
    );
}

#[test]
fn inbound_headers_are_forwarded_except_the_denied_ones() {
    let provider = provider();
    let mut headers = http::HeaderMap::new();
    headers.insert("x-custom", HeaderValue::from_static("keep"));
    headers.insert("openai-organization", HeaderValue::from_static("org-1"));
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer inbound"));
    headers.insert(
        http::header::HOST,
        HeaderValue::from_static("gateway.local"),
    );
    headers.insert(
        http::header::TRANSFER_ENCODING,
        HeaderValue::from_static("chunked"),
    );

    let req = ProviderRequest {
        headers: headers.clone(),
        ..Default::default()
    };
    let request = built(&provider, &req, false);

    assert_eq!(request.headers()["x-custom"], "keep");
    assert_eq!(request.headers()["openai-organization"], "org-1");
    assert_eq!(
        request.headers()[AUTHORIZATION],
        "Bearer sk-live",
        "the inbound Authorization must be replaced, never forwarded"
    );
    for denied in headers
        .keys()
        .filter(|k| is_skipped_proxy_header(k.as_str()))
    {
        if denied == AUTHORIZATION {
            continue;
        }
        assert!(
            !request.headers().contains_key(denied),
            "{denied} must not reach the upstream"
        );
    }
}

#[test]
fn an_inbound_accept_header_survives_on_non_streaming_requests_only() {
    let provider = provider();
    let mut headers = http::HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/x-ndjson"));
    let req = ProviderRequest {
        headers,
        ..Default::default()
    };

    assert_eq!(
        built(&provider, &req, false).headers()[ACCEPT],
        "application/x-ndjson"
    );
    assert_eq!(
        built(&provider, &req, true).headers()[ACCEPT],
        "text/event-stream",
        "a stream must override whatever the client asked to accept"
    );
}

#[test]
fn a_credential_that_cannot_be_encoded_as_a_header_is_rejected() {
    let provider = provider();
    let err = provider
        .build_request(
            &ProviderRequest::default(),
            false,
            "bad\nkey",
            provider.base_url(),
        )
        .expect_err("a newline in a credential must not reach the wire");
    assert!(matches!(err, ProviderError::Credential(_)), "{err:?}");
}

// --- trait surface ------------------------------------------------------------

#[tokio::test]
async fn refresh_only_remarks_a_static_key_as_healthy() {
    let provider = provider();
    let mut stale = bare_auth();
    stale.status = AuthStatus::Error;
    let before = stale.updated_at;

    let refreshed = provider.refresh(&stale).await.unwrap();
    assert_eq!(refreshed.status, AuthStatus::Active);
    assert!(refreshed.updated_at >= before);
    assert_eq!(refreshed.id, stale.id, "identity must be preserved");
}

#[tokio::test]
async fn count_tokens_estimates_from_the_payload_length() {
    let provider = provider();
    let req = ProviderRequest {
        payload: vec![b'x'; 400],
        ..Default::default()
    };
    let counted = provider.count_tokens(&bare_auth(), req).await.unwrap();
    assert_eq!(counted, approximate_tokens_from_bytes(400));
}

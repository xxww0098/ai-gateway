//! Unit tests for [`crate::codex`].
//!
//! With no property-testing dependency available, the generator is replaced by
//! a deterministic sweep over the model-name character class, at a fixed
//! iteration count.

use super::*;
use bytes::Bytes;
use gw_authcore::AuthRecord;
use serde_json::json;
use std::collections::HashMap;

fn config(base_url: &str) -> ProviderConfig {
    ProviderConfig {
        base_url: base_url.to_owned(),
        api_key: "config-token".to_owned(),
        enabled: true,
    }
}

fn provider() -> CodexProvider {
    CodexProvider::new(&config(""), 0).expect("an empty base url must fall back to the default")
}

fn auth(metadata: Value, attributes: &[(&str, &str)]) -> AuthRecord {
    AuthRecord {
        id: "codex-1".to_owned(),
        provider: PROVIDER_CODEX.to_owned(),
        label: "test".to_owned(),
        attributes: attributes
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<HashMap<_, _>>(),
        metadata,
        ..Default::default()
    }
}

// --- codex_model_from_body ---------------------------------------------------

/// Deterministic stand-in for a string-matching property generator. Walks the
/// model-name character class, so the sweep covers the shapes without a
/// property-testing dependency.
fn generated_model_names(count: usize) -> Vec<String> {
    const HEAD: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    const TAIL: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-/";
    // A linear congruential generator: reproducible, no dependency, and its
    // sequence visits the whole class rather than clustering.
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = |modulus: usize| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as usize) % modulus
    };

    (0..count)
        .map(|_| {
            let len = next(64);
            let mut name = String::with_capacity(len + 1);
            name.push(HEAD[next(HEAD.len())] as char);
            for _ in 0..len {
                name.push(TAIL[next(TAIL.len())] as char);
            }
            name
        })
        .collect()
}

/// For any body carrying a non-empty string `model`, the extracted name is
/// exactly that model, so a billed record can never be model-less.
#[test]
fn any_model_field_round_trips_out_of_the_request_body() {
    let names = generated_model_names(250);
    assert!(names.len() >= 200, "the sweep runs ≥200 iterations");

    for model in names {
        let bare = serde_json::to_vec(&json!({ "model": model })).unwrap();
        assert_eq!(codex_model_from_body(&bare), model);

        // The same body as a real request carries: extra fields must not
        // shadow the model.
        let realistic = serde_json::to_vec(&json!({
            "model": model,
            "temperature": 0.7,
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}],
        }))
        .unwrap();
        assert_eq!(codex_model_from_body(&realistic), model);
    }
}

#[test]
fn a_missing_or_non_string_model_reads_as_absent() {
    let cases: [(&str, &[u8]); 8] = [
        ("empty body", b""),
        ("invalid JSON", b"{not valid json"),
        ("missing model field", br#"{"messages":[]}"#),
        ("empty model string", br#"{"model":""}"#),
        ("model with only spaces", br#"{"model":"   "}"#),
        ("model is number", br#"{"model":123}"#),
        ("model is null", br#"{"model":null}"#),
        ("model is array", br#"{"model":["gpt-4"]}"#),
    ];
    for (name, body) in cases {
        assert_eq!(codex_model_from_body(body), "", "case {name}");
    }
}

// --- construction & credentials ----------------------------------------------

#[test]
fn an_empty_base_url_falls_back_to_the_openai_origin() {
    assert_eq!(provider().base_url(), CODEX_DEFAULT_BASE_URL);
    assert_eq!(provider().access_token(), "config-token");
    assert_eq!(provider().name(), PROVIDER_CODEX);
    assert!(CodexProvider::new(&config("not-a-url"), 0).is_err());
}

#[test]
fn the_token_cascade_prefers_the_most_specific_credential() {
    let provider = provider();
    let cases = [
        (
            "metadata access_token",
            json!({
                "access_token": "direct",
                "token_data": {"access_token": "nested", "api_key": "nested-key"},
                "api_key": "flat-key"
            }),
            "direct",
        ),
        (
            "nested access_token",
            json!({"token_data": {"access_token": "nested"}, "api_key": "flat-key"}),
            "nested",
        ),
        (
            "metadata api_key",
            json!({"api_key": "flat-key"}),
            "flat-key",
        ),
        (
            "nested api_key",
            json!({"token_data": {"api_key": "nested-key"}}),
            "nested-key",
        ),
        ("nothing stored", json!({}), "config-token"),
        (
            "token_data as an embedded document",
            json!({"token_data": "{\"access_token\":\"embedded\"}"}),
            "embedded",
        ),
    ];
    for (name, metadata, expected) in cases {
        let (token, _) = provider.resolve_credentials(&auth(metadata, &[]));
        assert_eq!(token, expected, "case {name}");
    }
}

#[test]
fn a_credential_may_repoint_the_base_url() {
    let provider = provider();
    let (_, base) = provider.resolve_credentials(&auth(
        json!({}),
        &[("base_url", "https://proxy.example.com/v1/")],
    ));
    assert_eq!(base, "https://proxy.example.com/v1");

    let (_, base) = provider.resolve_credentials(&auth(json!({}), &[("base_url", "  ")]));
    assert_eq!(
        base, CODEX_DEFAULT_BASE_URL,
        "a blank override must not win"
    );
}

#[test]
fn the_refresh_token_is_read_from_either_nesting_level() {
    assert_eq!(
        CodexProvider::resolve_refresh_token(&auth(json!({"refresh_token": "top"}), &[]))
            .as_deref(),
        Some("top")
    );
    assert_eq!(
        CodexProvider::resolve_refresh_token(&auth(
            json!({"token_data": {"refresh_token": "nested"}}),
            &[]
        ))
        .as_deref(),
        Some("nested")
    );
    assert_eq!(
        CodexProvider::resolve_refresh_token(&auth(json!({}), &[])),
        None
    );
}

// --- route plan ---------------------------------------------------------------

#[test]
fn a_request_without_a_token_is_refused_before_anything_is_planned() {
    let provider = provider();
    let err = provider
        .plan_request(&ProviderRequest::default(), "", CODEX_DEFAULT_BASE_URL)
        .expect_err("an empty access token must not produce a plan");
    assert!(matches!(err, ProviderError::Credential(_)), "{err:?}");
}

#[test]
fn streaming_requests_force_include_usage_like_the_openai_planner() {
    let provider = provider();
    let req = ProviderRequest {
        payload: Bytes::from_static(br#"{"model":"gpt-5-codex","stream":true}"#),
        stream: true,
        ..Default::default()
    };
    let plan = provider
        .plan_request(&req, "tok", CODEX_DEFAULT_BASE_URL)
        .expect("plans");

    assert_eq!(
        plan.endpoint.as_str(),
        "https://api.openai.com/v1/chat/completions"
    );
    assert!(matches!(&plan.credential, gw_relay::Credential::Bearer(t) if t == "tok"));
    assert_eq!(plan.headers[ACCEPT], "text/event-stream");
    // 插入内容本身由 `common_tests` 覆盖；这里只证明 planner 接上了那条路。
    let body = plan
        .body
        .as_ref()
        .expect("a streaming plan rewrites the body");
    assert!(body.len() > req.payload.len());
    assert_eq!(
        body.len(),
        crate::common::ensure_include_usage(&req.payload, Surface::OpenAiCompletions)
            .expect("fixture must be spliceable")
            .len()
    );
}

// --- OAuth token rotation ------------------------------------------------------

#[tokio::test]
async fn a_credential_with_no_refresh_token_is_only_remarked_healthy() {
    // No network call may happen on this path — a config-seeded static token
    // has nothing to rotate.
    let provider = provider();
    let mut stale = auth(json!({"access_token": "static"}), &[]);
    stale.status = AuthStatus::Error;

    let refreshed = provider.refresh(&stale).await.unwrap();
    assert_eq!(refreshed.status, AuthStatus::Active);
    assert_eq!(refreshed.metadata, stale.metadata, "metadata is untouched");
    assert!(refreshed.last_refreshed_at.is_none());
}

#[test]
fn token_data_merges_into_the_previous_blob_without_dropping_unknown_keys() {
    let previous = json!({"account_id": "acct-1", "access_token": "old", "refresh_token": "old-r"});
    let token = CodexRefreshResponse {
        access_token: "new".to_owned(),
        refresh_token: String::new(),
        id_token: "id".to_owned(),
        expires_in: 3600,
    };

    let merged = updated_token_data(Some(&previous), &token, "old-r", "NOW", Some("LATER"));
    assert_eq!(
        merged["account_id"],
        json!("acct-1"),
        "unknown keys survive"
    );
    assert_eq!(merged["access_token"], json!("new"));
    assert_eq!(
        merged["refresh_token"],
        json!("old-r"),
        "an omitted refresh token keeps the previous one"
    );
    assert_eq!(merged["id_token"], json!("id"));
    assert_eq!(merged["expired"], json!("LATER"));
    assert_eq!(merged["last_refresh"], json!("NOW"));
}

#[test]
fn token_data_tolerates_a_previous_blob_of_any_shape() {
    let token = CodexRefreshResponse {
        access_token: "new".to_owned(),
        ..Default::default()
    };
    for previous in [
        None,
        Some(json!(null)),
        Some(json!(7)),
        Some(json!("{not json")),
        Some(json!("{\"account_id\":\"acct-2\"}")),
    ] {
        let merged = updated_token_data(previous.as_ref(), &token, "", "NOW", None);
        assert_eq!(merged["access_token"], json!("new"));
        assert!(
            !merged.contains_key("expired"),
            "no expiry may be invented when the response omitted expires_in"
        );
    }
    let merged = updated_token_data(
        Some(&json!("{\"account_id\":\"acct-2\"}")),
        &token,
        "",
        "NOW",
        None,
    );
    assert_eq!(merged["account_id"], json!("acct-2"));
}

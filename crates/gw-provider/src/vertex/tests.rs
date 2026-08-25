//! Unit tests for the Vertex AI executor.
//!
//! The usage-recovery cases are the regression net for the M2 undercharge,
//! where a stale per-chunk tally beat the real — but split — terminal frame.

use base64::Engine as _;
use gw_authcore::AuthRecord;
use serde_json::json;

use super::*;
use bytes::Bytes;

/// Throwaway keys generated for these tests only; they authenticate nothing.
const PKCS1_KEY: &str = include_str!("testdata/service-account-pkcs1.pem");
const PKCS8_KEY: &str = include_str!("testdata/service-account-pkcs8.pem");

fn auth_with(metadata: serde_json::Value) -> AuthRecord {
    AuthRecord {
        metadata,
        ..AuthRecord::new("auth-1", PROVIDER_VERTEX, Utc::now())
    }
}

fn provider(base_url: &str, service_account: &str) -> VertexProvider {
    VertexProvider::new(
        &ProviderConfig {
            base_url: base_url.to_owned(),
            api_key: service_account.to_owned(),
            enabled: true,
        },
        0,
    )
    .expect("provider")
}

fn service_account_json(private_key: &str) -> String {
    json!({
        "client_email": "robot@project.iam.gserviceaccount.com",
        "private_key": private_key,
        "project_id": "sa-project",
    })
    .to_string()
}

fn expiry(offset: chrono::TimeDelta) -> String {
    shared::rfc3339(Utc::now() + offset)
}

// --- usage recovery ---------------------------------------------------------

/// M2 少收的复现，现在由**跨帧行解析**根治。
///
/// 终局帧被读边界切成两半时，任何「按 chunk」的解析都看不见它 —— 那正是当初要为
/// Vertex 单独写一个累加器（per-chunk latch + 收尾时对整个窗口再解析一遍）的原因。
/// `StreamUsageProbe` 把跨帧的半行接上，这条路变成了普通情况。
/// 合并是**按列**的，不是整体替换：终局帧省略了某一列，不得抹掉更早的帧
/// 为那一列报过的值。「省略」与「零」是两件事。
/// Vertex answers some callers with SSE and others with a chunked JSON array,
/// so both framings have to parse to the same tally.
#[test]
fn both_sse_and_bare_json_framings_are_understood() {
    let framed =
        extract_latest_vertex_usage(b"data: {\"usageMetadata\":{\"promptTokenCount\":3}}\n")
            .expect("sse");
    let bare = extract_latest_vertex_usage(b"{\"usageMetadata\":{\"promptTokenCount\":3}}\n")
        .expect("bare");
    assert_eq!(framed, bare);
}

#[test]
fn empty_and_usage_free_chunks_yield_nothing() {
    assert!(extract_latest_vertex_usage(b"").is_none());
    assert!(extract_latest_vertex_usage(b"   \n\n  ").is_none());
    assert!(extract_latest_vertex_usage(b"data: [DONE]\n").is_none());
}

// --- endpoint ---------------------------------------------------------------

fn target(base_url: &str, location: &str) -> VertexEndpoint {
    VertexEndpoint {
        base_url: base_url.to_owned(),
        project: "p".to_owned(),
        location: location.to_owned(),
    }
}

#[test]
fn streaming_changes_only_the_action_verb() {
    let target = target("https://vx.example.com", "us-central1");
    let plain =
        VertexProvider::generate_content_endpoint(&[], &target, "gemini-2.5-pro", false).unwrap();
    let streamed =
        VertexProvider::generate_content_endpoint(&[], &target, "gemini-2.5-pro", true).unwrap();
    assert_ne!(plain.path(), streamed.path());
    assert_eq!(
        plain.path().rsplit_once(':').map(|(head, _)| head),
        streamed.path().rsplit_once(':').map(|(head, _)| head),
    );
}

/// The router may hand over a `vertex/`-qualified name, but the publisher path
/// already says which publisher this is.
#[test]
fn the_provider_prefix_is_stripped_from_the_model() {
    let target = target("https://vx.example.com", "us-central1");
    let prefixed =
        VertexProvider::generate_content_endpoint(&[], &target, "vertex/gemini-2.5-pro", false)
            .unwrap();
    let bare =
        VertexProvider::generate_content_endpoint(&[], &target, "gemini-2.5-pro", false).unwrap();
    assert_eq!(prefixed, bare);
}

#[test]
fn a_slash_in_the_model_name_cannot_add_a_path_segment() {
    let target = target("https://vx.example.com", "us-central1");
    let benign =
        VertexProvider::generate_content_endpoint(&[], &target, "gemini-2.5-pro", false).unwrap();
    let hostile =
        VertexProvider::generate_content_endpoint(&[], &target, "a/b/c/evil", false).unwrap();
    assert_eq!(
        benign.path_segments().map(Iterator::count),
        hostile.path_segments().map(Iterator::count),
    );
}

/// Vertex is regional: with no explicit base URL the host has to follow the
/// location, or the request lands in the wrong region.
#[test]
fn an_absent_base_url_is_derived_from_the_location() {
    let west =
        VertexProvider::generate_content_endpoint(&[], &target("", "europe-west4"), "m", false)
            .unwrap();
    let central =
        VertexProvider::generate_content_endpoint(&[], &target("", "us-central1"), "m", false)
            .unwrap();
    assert_ne!(west.host_str(), central.host_str());
    assert!(west.host_str().unwrap().starts_with("europe-west4"));
}

#[test]
fn caller_query_parameters_reach_the_endpoint() {
    let url = VertexProvider::generate_content_endpoint(
        &[("trace".to_owned(), "1".to_owned())],
        &target("https://vx.example.com", "us-central1"),
        "m",
        true,
    )
    .unwrap();
    assert!(url.query_pairs().any(|(k, v)| k == "trace" && v == "1"));
}

// --- endpoint settings ------------------------------------------------------

#[test]
fn record_attributes_outrank_the_service_accounts_project() {
    let provider = provider("", "");
    let sa = VertexServiceAccount {
        project_id: "sa-project".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        provider
            .resolve_endpoint_settings(Some(&auth_with(json!({}))), Some(&sa))
            .project,
        "sa-project"
    );

    for key in ["project", "project_id"] {
        let mut auth = auth_with(json!({}));
        auth.attributes
            .insert(key.to_owned(), " attr-project ".to_owned());
        assert_eq!(
            provider
                .resolve_endpoint_settings(Some(&auth), Some(&sa))
                .project,
            "attr-project"
        );
    }
}

#[test]
fn location_accepts_either_spelling_and_drives_the_host() {
    let provider = provider("", "");
    for key in ["location", "region"] {
        let mut auth = auth_with(json!({}));
        auth.attributes
            .insert(key.to_owned(), "asia-east1".to_owned());
        let resolved = provider.resolve_endpoint_settings(Some(&auth), None);
        assert_eq!(resolved.location, "asia-east1");
        assert!(resolved.base_url.contains("asia-east1"));
    }
}

#[test]
fn a_record_can_override_the_base_url() {
    let provider = provider("https://configured.example.com", "");
    for key in ["base_url", "base-url"] {
        let mut auth = auth_with(json!({}));
        auth.attributes
            .insert(key.to_owned(), "https://override.example.com/".to_owned());
        assert_eq!(
            provider
                .resolve_endpoint_settings(Some(&auth), None)
                .base_url,
            "https://override.example.com"
        );
    }
}

/// Minting a token costs a signature and a round trip; a record that can never
/// be routed anywhere should not pay for one.
#[tokio::test]
async fn a_record_with_no_project_is_rejected_before_any_token_is_minted() {
    let projectless = json!({
        "client_email": "robot@example.com",
        "private_key": PKCS1_KEY,
    })
    .to_string();
    let err = provider("", &projectless)
        .credentials_for_request(Some(&auth_with(json!({}))))
        .await
        .expect_err("rejected");
    assert!(matches!(err, ProviderError::Credential(_)));
    assert!(err.to_string().contains("project"));
}

// --- service account --------------------------------------------------------

#[test]
fn the_service_account_is_found_wherever_it_was_persisted() {
    let provider = provider("", "");
    let raw = service_account_json(PKCS1_KEY);
    let as_object: serde_json::Value = serde_json::from_str(&raw).unwrap();

    let shapes = [
        json!({"service_account": raw}),
        json!({"token_data": {"service_account": raw}}),
        json!({"storage": {"service_account": raw}}),
        json!({"service_account": as_object}),
    ];
    for metadata in shapes {
        let sa = provider
            .resolve_service_account(Some(&auth_with(metadata)))
            .expect("service account");
        assert_eq!(sa.client_email, "robot@project.iam.gserviceaccount.com");
    }
}

#[test]
fn the_configured_service_account_is_the_last_resort() {
    let provider = provider("", &service_account_json(PKCS1_KEY));
    assert_eq!(
        provider
            .resolve_service_account(Some(&auth_with(json!({}))))
            .expect("service account")
            .project_id,
        "sa-project"
    );
    assert_eq!(
        provider
            .resolve_service_account(None)
            .expect("service account")
            .project_id,
        "sa-project"
    );
}

/// The token endpoint is rarely configured, and every refresh needs one.
#[test]
fn an_omitted_token_uri_is_defaulted_rather_than_rejected() {
    let defaulted = provider("", &service_account_json(PKCS1_KEY))
        .resolve_service_account(None)
        .unwrap();
    assert!(!defaulted.token_uri.is_empty());

    let explicit = json!({
        "client_email": "robot@example.com",
        "private_key": PKCS1_KEY,
        "token_uri": "https://token.example.com/oauth",
    })
    .to_string();
    assert_eq!(
        provider("", &explicit)
            .resolve_service_account(None)
            .unwrap()
            .token_uri,
        "https://token.example.com/oauth"
    );
}

#[test]
fn an_unusable_service_account_names_what_is_missing() {
    let cases = [
        (String::new(), "required"),
        ("{not json".to_owned(), "valid JSON"),
        (json!({"private_key": "x"}).to_string(), "client_email"),
        (json!({"client_email": "a@b.c"}).to_string(), "private_key"),
    ];
    for (raw, expected) in cases {
        let err = provider("", &raw)
            .resolve_service_account(None)
            .expect_err("rejected");
        assert!(
            err.to_string().contains(expected),
            "{raw:?} should complain about {expected}, said {err}"
        );
    }
}

// --- assertion signing ------------------------------------------------------

fn decode_segment(segment: &str) -> serde_json::Value {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(segment)
        .expect("base64url");
    serde_json::from_slice(&raw).expect("json")
}

/// Google issues keys in both PKCS#1 and PKCS#8; a gateway that accepts only
/// one of them fails for half its tenants.
#[test]
fn both_pem_encodings_produce_a_well_formed_assertion() {
    let now = Utc::now();
    for key in [PKCS1_KEY, PKCS8_KEY] {
        let sa = VertexServiceAccount {
            client_email: "robot@project.iam.gserviceaccount.com".to_owned(),
            private_key: key.to_owned(),
            token_uri: "https://token.example.com/oauth".to_owned(),
            project_id: "p".to_owned(),
        };
        let assertion = VertexProvider::signed_assertion(&sa, now).expect("assertion");
        let parts: Vec<_> = assertion.split('.').collect();
        assert_eq!(parts.len(), 3, "a JWS has three segments");
        assert!(!parts[2].is_empty(), "the signature must not be empty");

        let header = decode_segment(parts[0]);
        assert_eq!(header["alg"], "RS256");

        let claims = decode_segment(parts[1]);
        assert_eq!(claims["iss"], sa.client_email);
        assert_eq!(
            claims["aud"], sa.token_uri,
            "the audience must be the endpoint the assertion is sent to"
        );
        assert_eq!(
            claims["iat"].as_i64().unwrap(),
            now.timestamp(),
            "the assertion is stamped with the caller's clock"
        );
        assert!(
            claims["exp"].as_i64().unwrap() > claims["iat"].as_i64().unwrap(),
            "an assertion must outlive its own issuance"
        );
        assert!(
            claims["scope"].as_str().unwrap().starts_with("https://"),
            "the scope is a Google OAuth scope URL"
        );
    }
}

#[test]
fn signing_the_same_claims_twice_is_stable() {
    let now = Utc::now();
    let sa = VertexServiceAccount {
        client_email: "robot@example.com".to_owned(),
        private_key: PKCS8_KEY.to_owned(),
        token_uri: "https://token.example.com/oauth".to_owned(),
        project_id: "p".to_owned(),
    };
    assert_eq!(
        VertexProvider::signed_assertion(&sa, now).unwrap(),
        VertexProvider::signed_assertion(&sa, now).unwrap(),
        "RS256 is deterministic, so a replayed sign must not produce a new token"
    );
}

#[test]
fn a_key_that_is_not_a_pem_rsa_key_is_reported_not_panicked() {
    let sa = VertexServiceAccount {
        client_email: "robot@example.com".to_owned(),
        private_key: "-----BEGIN RSA PRIVATE KEY-----\nnope\n-----END RSA PRIVATE KEY-----"
            .to_owned(),
        token_uri: "https://token.example.com/oauth".to_owned(),
        project_id: "p".to_owned(),
    };
    let err = VertexProvider::signed_assertion(&sa, Utc::now()).expect_err("rejected");
    assert!(matches!(err, ProviderError::Credential(_)));
}

// --- token caching ----------------------------------------------------------

/// A stored token is reusable only while its own expiry vouches for it, and the
/// skew means "expires in a moment" already counts as spent.
#[test]
fn a_stored_token_is_only_returned_while_it_is_comfortably_valid() {
    let now = Utc::now();

    let fresh = auth_with(json!({
        "access_token": "tok",
        "expires_at": expiry(chrono::TimeDelta::hours(1)),
    }));
    assert_eq!(
        VertexProvider::cached_access_token(Some(&fresh), now).as_deref(),
        Some("tok")
    );

    let expired = auth_with(json!({
        "access_token": "tok",
        "expires_at": expiry(-chrono::TimeDelta::minutes(1)),
    }));
    assert!(VertexProvider::cached_access_token(Some(&expired), now).is_none());

    let about_to_expire = auth_with(json!({
        "access_token": "tok",
        "expires_at": expiry(chrono::TimeDelta::seconds(30)),
    }));
    assert!(
        VertexProvider::cached_access_token(Some(&about_to_expire), now).is_none(),
        "a token expiring inside the refresh skew would die mid-flight"
    );

    assert!(
        VertexProvider::cached_access_token(Some(&auth_with(json!({"access_token": "tok"}))), now)
            .is_none(),
        "an undated token cannot be vouched for"
    );
    assert!(VertexProvider::cached_access_token(None, now).is_none());
}

/// Either expiry key may be the one a record carries; both govern.
#[test]
fn both_expiry_spellings_are_honoured() {
    let now = Utc::now();
    for key in ["expires_at", "expired"] {
        let auth = auth_with(json!({
            "access_token": "tok",
            key: expiry(chrono::TimeDelta::hours(1)),
        }));
        assert_eq!(
            VertexProvider::cached_access_token(Some(&auth), now).as_deref(),
            Some("tok")
        );
    }
}

/// A token nested under `token_data` must be governed by the expiry nested
/// beside it — a top-level stamp may belong to an entirely different token.
#[test]
fn a_nested_token_is_governed_by_its_own_expiry() {
    let now = Utc::now();

    let valid = auth_with(json!({
        "token_data": {"access_token": "nested", "expires_at": expiry(chrono::TimeDelta::hours(1))},
    }));
    assert_eq!(
        VertexProvider::cached_access_token(Some(&valid), now).as_deref(),
        Some("nested")
    );

    let borrowed = auth_with(json!({
        "expires_at": expiry(chrono::TimeDelta::hours(1)),
        "token_data": {"access_token": "nested"},
    }));
    assert!(
        VertexProvider::cached_access_token(Some(&borrowed), now).is_none(),
        "a top-level expiry must not vouch for a nested token"
    );
}

#[test]
fn a_refresh_preserves_unrelated_token_data_keys() {
    let updated = VertexProvider::updated_token_data(
        Some(&json!({"scope": "custom", "access_token": "old"})),
        "new",
        "2030-01-01T00:00:00Z",
        "2026-01-01T00:00:00Z",
    );
    assert_eq!(updated["scope"], "custom");
    assert_eq!(updated["access_token"], "new");
    assert!(updated.contains_key("expires_at"));
    assert!(updated.contains_key("last_refresh"));
}

#[test]
fn token_data_stored_as_a_json_string_is_still_merged() {
    let updated = VertexProvider::updated_token_data(
        Some(&json!(r#"{"scope":"custom"}"#)),
        "new",
        "2030-01-01T00:00:00Z",
        "2026-01-01T00:00:00Z",
    );
    assert_eq!(updated["scope"], "custom");
    assert_eq!(updated["access_token"], "new");
}

/// The executor-side cache exists because the trait hands out `&AuthRecord`:
/// without it every request would sign a fresh assertion.
#[test]
fn the_executor_cache_serves_a_freshly_minted_token_and_expires_it() {
    let provider = provider("", "");
    let now = Utc::now();
    let refreshed = auth_with(json!({
        "access_token": "minted",
        "expires_at": expiry(chrono::TimeDelta::hours(1)),
    }));
    provider.store_executor_token(&refreshed);

    assert_eq!(
        provider
            .cached_executor_token(Some(&refreshed), now)
            .as_deref(),
        Some("minted")
    );
    assert!(
        provider
            .cached_executor_token(Some(&refreshed), now + chrono::TimeDelta::hours(2))
            .is_none()
    );

    let other = AuthRecord {
        ..AuthRecord::new("someone-else", PROVIDER_VERTEX, now)
    };
    assert!(
        provider.cached_executor_token(Some(&other), now).is_none(),
        "one record's token must never serve another"
    );
}

#[test]
fn an_undated_refresh_result_is_not_cached() {
    let provider = provider("", "");
    let refreshed = auth_with(json!({"access_token": "minted"}));
    provider.store_executor_token(&refreshed);
    assert!(
        provider
            .cached_executor_token(Some(&refreshed), Utc::now())
            .is_none(),
        "a token with no expiry would otherwise be served forever"
    );
}

// --- misc -------------------------------------------------------------------

#[test]
fn the_provider_prefix_is_stripped_only_when_it_is_a_whole_segment() {
    assert_eq!(
        strip_vertex_prefix(" vertex/gemini-2.5-pro "),
        "gemini-2.5-pro"
    );
    assert_eq!(strip_vertex_prefix("vertexai-model"), "vertexai-model");
    assert_eq!(strip_vertex_prefix("gemini-2.5-pro"), "gemini-2.5-pro");
}

/// 理由与 gemini 那条逐字相同：入口是 Anthropic 方言，Vertex 的 `:countTokens`
/// 接不上，所以**报错**而不是编数字（`docs/relay-surface-plan.md` §2.1 缺陷 ①）。
#[tokio::test]
async fn token_counting_refuses_rather_than_fabricating_a_number() {
    let provider = provider("", "");
    let auth = auth_with(json!({}));
    for len in [0, 8, 80, 800] {
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

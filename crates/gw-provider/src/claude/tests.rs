//! Unit tests for the Anthropic executor.
//!
//! These cover the parts of the executor that need no socket: the credential
//! precedence ladder, endpoint normalisation, header injection, and SSE usage
//! merging.

use gw_authcore::AuthRecord;
use serde_json::json;

use super::*;
use crate::common::{requested_model, REQUESTED_MODEL_METADATA_KEY};

fn auth_with(metadata: serde_json::Value) -> AuthRecord {
    AuthRecord {
        metadata,
        ..AuthRecord::new("auth-1", PROVIDER_CLAUDE, chrono::Utc::now())
    }
}

fn provider(base_url: &str, api_key: &str) -> ClaudeProvider {
    ClaudeProvider::new(
        &ProviderConfig {
            base_url: base_url.to_owned(),
            api_key: api_key.to_owned(),
            enabled: true,
        },
        0,
    )
    .expect("provider")
}

fn endpoint(base_url: &str) -> Url {
    ClaudeProvider::messages_endpoint(None, &[], base_url).expect("endpoint")
}

// --- count_tokens（根除伪造值，`docs/relay-surface-plan.md` §2.1 缺陷 ①）--------

/// 计数端点是 Messages 端点再挂一段，且**同一套 base 归一化**。
///
/// 测的是「三种 base 写法收敛到同一个计数端点，且它就是 messages 端点的子路径」，
/// 不核对任何硬编码 URL。
#[test]
fn the_count_tokens_endpoint_hangs_off_the_messages_endpoint() {
    for base in [
        "https://relay.example.com",
        "https://relay.example.com/v1",
        "https://relay.example.com/v1/messages",
    ] {
        let messages = ClaudeProvider::messages_endpoint(None, &[], base).expect("messages");
        let counting = ClaudeProvider::count_tokens_endpoint(None, &[], base).expect("count");
        assert_eq!(
            counting.origin(),
            messages.origin(),
            "base {base}: 计数端点换了主机"
        );
        assert!(
            counting.path().starts_with(messages.path()),
            "base {base}: {} 不在 {} 之下",
            counting.path(),
            messages.path()
        );
        assert_ne!(counting.path(), messages.path(), "base {base}: 端点没变");
    }
}

#[test]
fn caller_query_parameters_reach_the_count_tokens_endpoint() {
    let query = vec![("beta".to_owned(), "1".to_owned())];
    let url = ClaudeProvider::count_tokens_endpoint(None, &query, "https://relay.example.com")
        .expect("endpoint");
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(pairs, query);
}

// --- endpoint ---------------------------------------------------------------

/// The three accepted spellings of a base URL converge on one endpoint. That
/// convergence — not any particular path string — is what an operator relies on
/// when pointing the gateway at a relay.
#[test]
fn every_base_url_spelling_resolves_to_the_same_endpoint() {
    let bare = endpoint("https://relay.example.com");
    let versioned = endpoint("https://relay.example.com/v1");
    let complete = endpoint(bare.as_str());

    assert_eq!(bare, versioned);
    assert_eq!(
        bare, complete,
        "an already-complete endpoint must not grow a second path"
    );
}

#[test]
fn trailing_slashes_and_padding_do_not_change_the_endpoint() {
    assert_eq!(
        endpoint("https://relay.example.com"),
        endpoint("  https://relay.example.com///  ")
    );
}

#[test]
fn caller_query_parameters_reach_the_endpoint_in_order() {
    let url = ClaudeProvider::messages_endpoint(
        None,
        &[
            ("beta".to_owned(), "first".to_owned()),
            ("beta".to_owned(), "second".to_owned()),
        ],
        "https://relay.example.com",
    )
    .expect("endpoint");
    let pairs: Vec<_> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("beta".to_owned(), "first".to_owned()),
            ("beta".to_owned(), "second".to_owned())
        ],
        "duplicate keys and their order are both significant"
    );
}

#[test]
fn a_base_url_without_a_host_is_rejected() {
    assert!(ClaudeProvider::messages_endpoint(None, &[], "not-a-url").is_err());
    assert!(
        ClaudeProvider::messages_endpoint(None, &[], "https://").is_err(),
        "a hostless URL must not re-parse with a path segment as the host"
    );
    assert!(ClaudeProvider::new(
        &ProviderConfig {
            base_url: "not-a-url".to_owned(),
            api_key: String::new(),
            enabled: true,
        },
        0
    )
    .is_err());
}

// --- credentials ------------------------------------------------------------

/// The ladder is strictly ordered: each rung is consulted only when every rung
/// above it is absent.
#[test]
fn credential_precedence_prefers_the_most_specific_rung() {
    let provider = provider("https://api.example.com", "from-config");

    let all = auth_with(json!({
        "api_key": "top-api-key",
        "access_token": "top-access-token",
        "token_data": {"api_key": "nested-api-key", "access_token": "nested-access-token"},
        "storage": {"APIKey": "stored-api-key"},
    }));
    assert_eq!(
        provider.resolve_credentials(Some(&all)).0.value,
        "top-api-key"
    );

    let nested_key = auth_with(json!({
        "access_token": "top-access-token",
        "token_data": {"api_key": "nested-api-key"},
    }));
    assert_eq!(
        provider.resolve_credentials(Some(&nested_key)).0.value,
        "nested-api-key",
        "a nested api_key still outranks any access token"
    );

    let oauth = auth_with(json!({
        "access_token": "top-access-token",
        "token_data": {"access_token": "nested-access-token"},
    }));
    let resolved = provider.resolve_credentials(Some(&oauth)).0;
    assert_eq!(resolved.value, "top-access-token");
    assert_eq!(resolved.source, CredentialSource::OauthToken);

    let stored = auth_with(json!({"storage": {"AccessToken": "stored-access-token"}}));
    assert_eq!(
        provider.resolve_credentials(Some(&stored)).0.value,
        "stored-access-token"
    );

    let fallback = provider.resolve_credentials(Some(&auth_with(json!({})))).0;
    assert_eq!(fallback.value, "from-config");
    assert_eq!(fallback.source, CredentialSource::ApiKey);
}

#[test]
fn a_record_without_any_credential_falls_back_to_the_configured_key() {
    let provider = provider("https://api.example.com", "from-config");
    assert_eq!(provider.resolve_credentials(None).0.value, "from-config");
}

/// The persisted blob is written by two different producers, so both the JSON
/// spelling and the struct field name have to resolve.
#[test]
fn stored_credentials_resolve_under_either_spelling() {
    let provider = provider("https://api.example.com", "");
    for (stored, expected) in [
        (json!({"storage": {"api_key": "snake"}}), "snake"),
        (json!({"storage": {"APIKey": "pascal"}}), "pascal"),
    ] {
        assert_eq!(
            provider
                .resolve_credentials(Some(&auth_with(stored)))
                .0
                .value,
            expected
        );
    }
}

#[test]
fn a_record_can_override_the_base_url() {
    let provider = provider("https://api.example.com", "k");
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
fn refresh_token_precedence_matches_the_credential_ladder() {
    let cases = [
        (
            json!({
                "refresh_token": "top",
                "token_data": {"refresh_token": "nested"},
                "storage": {"RefreshToken": "stored"},
            }),
            "top",
        ),
        (
            json!({
                "token_data": {"refresh_token": "nested"},
                "storage": {"RefreshToken": "stored"},
            }),
            "nested",
        ),
        (json!({"storage": {"refresh_token": "stored"}}), "stored"),
    ];
    for (metadata, expected) in cases {
        assert_eq!(
            ClaudeProvider::resolve_refresh_token(Some(&auth_with(metadata))).as_deref(),
            Some(expected)
        );
    }
    assert!(ClaudeProvider::resolve_refresh_token(Some(&auth_with(json!({})))).is_none());
    assert!(ClaudeProvider::resolve_refresh_token(None).is_none());
}

/// Some tooling persists `token_data` as a JSON *string*. The resolver has to
/// see through that, or a perfectly good record silently loses its credential.
#[test]
fn token_data_encoded_as_a_json_string_is_still_readable() {
    let provider = provider("https://api.example.com", "");
    let auth = auth_with(json!({"token_data": r#"{"api_key":"inside-a-string"}"#}));
    assert_eq!(
        provider.resolve_credentials(Some(&auth)).0.value,
        "inside-a-string"
    );
}

// --- headers ----------------------------------------------------------------

/// An inbound `x-api-key` belongs to the client leg and must never reach
/// Anthropic. The planner does not stamp one at all — the credential rides on
/// [`RoutePlan::credential`], and `gw-relay` strips the client's carrier
/// before setting it.
#[test]
fn the_plan_carries_the_credential_as_a_credential_not_as_a_header() {
    let provider = provider("https://api.anthropic.com", "sk-config");
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", HeaderValue::from_static("caller-key"));
    let req = ProviderRequest {
        headers,
        ..Default::default()
    };
    let plan = provider
        .plan_messages(
            &req,
            &ClaudeCredential {
                value: "real-key".to_owned(),
                source: CredentialSource::ApiKey,
            },
            "https://api.anthropic.com",
        )
        .expect("plans");

    assert!(
        !plan.headers.contains_key("x-api-key"),
        "the credential must not be planned as a plain header"
    );
    assert!(matches!(&plan.credential, gw_relay::Credential::XApiKey(k) if k == "real-key"));
}

#[test]
fn a_caller_supplied_api_version_survives_but_a_missing_one_is_filled_in() {
    let provider = provider("https://api.anthropic.com", "sk-config");
    let credential = ClaudeCredential {
        value: "k".to_owned(),
        source: CredentialSource::ApiKey,
    };

    let mut pinned = HeaderMap::new();
    pinned.insert("anthropic-version", HeaderValue::from_static("1999-01-01"));
    let plan = provider
        .plan_messages(
            &ProviderRequest {
                headers: pinned,
                ..Default::default()
            },
            &credential,
            "https://api.anthropic.com",
        )
        .expect("plans");
    assert!(
        !plan.headers.contains_key("anthropic-version"),
        "a pinned version is left alone so the relay forwards the client's own"
    );

    let plan = provider
        .plan_messages(
            &ProviderRequest::default(),
            &credential,
            "https://api.anthropic.com",
        )
        .expect("plans");
    assert!(plan.headers.contains_key("anthropic-version"));
}

#[test]
fn oauth_tokens_travel_as_bearer_and_api_keys_as_x_api_key() {
    let provider = provider("https://api.anthropic.com", "sk-config");
    let oauth = provider
        .plan_messages(
            &ProviderRequest::default(),
            &ClaudeCredential {
                value: "oat".to_owned(),
                source: CredentialSource::OauthToken,
            },
            "https://api.anthropic.com",
        )
        .expect("plans");
    assert!(matches!(&oauth.credential, gw_relay::Credential::Bearer(k) if k == "oat"));

    let key = provider
        .plan_messages(
            &ProviderRequest::default(),
            &ClaudeCredential {
                value: "sk".to_owned(),
                source: CredentialSource::ApiKey,
            },
            "https://api.anthropic.com",
        )
        .expect("plans");
    assert!(matches!(&key.credential, gw_relay::Credential::XApiKey(k) if k == "sk"));
}

/// Prompt-cache and OAuth are separate Anthropic betas. An API-key plan must
/// not claim the OAuth beta (that header is rejected on console keys).
#[test]
fn beta_features_follow_the_credential_source() {
    let provider = provider("https://api.anthropic.com", "sk-config");
    let oauth = provider
        .plan_messages(
            &ProviderRequest::default(),
            &ClaudeCredential {
                value: "oat".to_owned(),
                source: CredentialSource::OauthToken,
            },
            "https://api.anthropic.com",
        )
        .expect("plans");
    let oauth_beta = oauth
        .headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(oauth_beta.contains("oauth"), "{oauth_beta}");
    assert!(oauth_beta.contains("prompt-caching"), "{oauth_beta}");

    let key = provider
        .plan_messages(
            &ProviderRequest::default(),
            &ClaudeCredential {
                value: "sk".to_owned(),
                source: CredentialSource::ApiKey,
            },
            "https://api.anthropic.com",
        )
        .expect("plans");
    let key_beta = key
        .headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(key_beta.contains("prompt-caching"), "{key_beta}");
    assert!(!key_beta.contains("oauth"), "{key_beta}");
}

#[test]
fn a_caller_supplied_beta_list_is_left_alone() {
    let provider = provider("https://api.anthropic.com", "sk-config");
    let mut headers = HeaderMap::new();
    headers.insert("anthropic-beta", HeaderValue::from_static("custom-beta"));
    let plan = provider
        .plan_messages(
            &ProviderRequest {
                headers,
                ..Default::default()
            },
            &ClaudeCredential {
                value: "sk".to_owned(),
                source: CredentialSource::ApiKey,
            },
            "https://api.anthropic.com",
        )
        .expect("plans");
    assert!(!plan.headers.contains_key("anthropic-beta"));
}

/// A body that already opted into cache_control is forwarded untouched.
#[test]
fn existing_cache_control_is_not_rewritten() {
    let original =
        br#"{"system":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}"#;
    assert!(inject_prompt_cache_breakpoints(original).is_none());
}

/// A string system prompt becomes a breakpointed block so subsequent turns
/// share a prefix. The property is "a cache_control object appears", not a
/// particular Anthropic date string.
#[test]
fn a_string_system_prompt_gains_a_cache_breakpoint() {
    let rewritten =
        inject_prompt_cache_breakpoints(br#"{"system":"stable prefix","tools":[{"name":"x"}]}"#)
            .expect("rewritten");
    let value: serde_json::Value = serde_json::from_slice(&rewritten).expect("json");
    assert!(json_contains_key(&value, "cache_control"));
    assert_eq!(value["system"][0]["text"], "stable prefix");
}

#[test]
fn a_blank_credential_is_refused_before_anything_is_planned() {
    let provider = provider("https://api.anthropic.com", "sk-config");
    let err = provider
        .plan_messages(
            &ProviderRequest::default(),
            &ClaudeCredential {
                value: String::new(),
                source: CredentialSource::ApiKey,
            },
            "https://api.anthropic.com",
        )
        .expect_err("a keyless account must not produce a plan");
    assert!(matches!(err, ProviderError::Credential(_)), "{err:?}");
}

// --- stream usage -----------------------------------------------------------

/// Claude splits the tally across frames: `message_start` knows the input
/// count, the last `message_delta` knows the final output count. Neither frame
/// alone settles correctly.
#[test]
fn usage_is_merged_across_message_start_and_message_delta() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1200,\"output_tokens\":1}}}\n",
        "\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":37}}\n",
        "\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n",
    );
    let tokens = parse_claude_stream_usage(body.as_bytes()).expect("usage");
    assert_eq!(tokens.input, Some(1200));
    assert_eq!(tokens.output, Some(37));
}

#[test]
fn a_stream_with_no_usage_frames_yields_no_tally() {
    assert!(parse_claude_stream_usage(b"event: ping\ndata: {\"type\":\"ping\"}\n\n").is_none());
    assert!(parse_claude_stream_usage(b"data: [DONE]\n").is_none());
    assert!(parse_claude_stream_usage(b"").is_none());
}

/// The same helper backs both paths, so an upstream that answers a stream
/// request with a whole JSON envelope must still bill.
#[test]
fn a_plain_json_body_takes_the_fast_path() {
    let tokens = parse_claude_stream_usage(
        br#"{"type":"message","usage":{"input_tokens":7,"output_tokens":11}}"#,
    )
    .expect("usage");
    assert_eq!((tokens.input, tokens.output), (Some(7), Some(11)));
}

/// A frame cut in half by a TCP boundary is unparseable; it must not drag the
/// tally down below what a complete frame already reported.
#[test]
fn a_truncated_frame_cannot_defeat_a_complete_one() {
    let body = concat!(
        "data: {\"usage\":{\"input_tokens\":10,\"output_tokens\":900}}\n",
        "data: {\"usage\":{\"input_tokens\":10,\"output_to\n",
    );
    let tokens = parse_claude_stream_usage(body.as_bytes()).expect("usage");
    assert_eq!(tokens.output, Some(900));
}

// --- shared helpers ---------------------------------------------------------

#[test]
fn the_router_hint_supplies_the_model_when_the_request_has_none() {
    let mut req = ProviderRequest {
        model: "  ".to_owned(),
        ..Default::default()
    };
    req.metadata.insert(
        REQUESTED_MODEL_METADATA_KEY.to_owned(),
        " claude-opus-4 ".to_owned(),
    );
    assert_eq!(requested_model(&req), "claude-opus-4");

    let explicit = ProviderRequest {
        model: " claude-sonnet-4 ".to_owned(),
        ..req
    };
    assert_eq!(requested_model(&explicit), "claude-sonnet-4");
}

#[test]
fn metadata_that_is_not_an_object_is_replaced_rather_than_lost() {
    let mut metadata = serde_json::Value::String("junk".to_owned());
    shared::metadata_object_mut(&mut metadata).insert("k".to_owned(), json!("v"));
    assert_eq!(metadata, json!({"k": "v"}));
}

/// A model name is caller-controlled, so a path separator inside one must not
/// survive into the URL.
#[test]
fn a_path_segment_cannot_climb_out_of_its_segment() {
    assert!(!shared::path_escape("../../admin").contains('/'));
    assert_eq!(shared::path_escape("gemini-2.5-pro"), "gemini-2.5-pro");
}

#[test]
fn round_tripping_a_timestamp_through_metadata_preserves_the_instant() {
    let now = chrono::Utc::now();
    let parsed = shared::parse_rfc3339(&shared::rfc3339(now)).expect("parsed");
    assert_eq!(parsed.timestamp(), now.timestamp());
    assert!(shared::parse_rfc3339("not a timestamp").is_none());
}

/// A provider-owned parameter must win over a caller's, and must not be
/// duplicated.
#[test]
fn setting_a_query_key_drops_every_earlier_value_for_it() {
    let mut query = vec![
        ("alt".to_owned(), "json".to_owned()),
        ("keep".to_owned(), "me".to_owned()),
        ("alt".to_owned(), "proto".to_owned()),
    ];
    shared::set_query(&mut query, "alt", "sse");
    assert_eq!(
        query,
        vec![
            ("keep".to_owned(), "me".to_owned()),
            ("alt".to_owned(), "sse".to_owned())
        ]
    );
}

/// [`ClaudeCredential`] 的 `Debug` 不许带出解析出来的活密钥。
///
/// 这个类型存在的理由就是「密钥 + 它来自哪一级」，于是任何一句
/// `tracing::debug!(?cred)` 或 `assert_eq!` 失败信息都会把密钥原样打出来 ——
/// 而 `source` 才是排错要看的那一半，它留着。
///
/// 密文是这条测试自己造的，生产源码里没有它（规范 2.11）。
#[test]
fn claude_credential_debug_never_carries_the_live_secret() {
    const LIVE: &str = "sk-ant-UNIQUE-KNIFE3-claude-7c2f10";

    for source in [CredentialSource::ApiKey, CredentialSource::OauthToken] {
        let cred = ClaudeCredential {
            value: LIVE.to_owned(),
            source,
        };
        let dump = format!("{cred:?}");
        assert!(!dump.contains(LIVE), "凭证的 Debug 打出了活密钥：{dump}");
        assert!(
            dump.contains(source.as_str()) || dump.contains(&format!("{source:?}")),
            "来源是排错要看的那一半，不该跟着被抹掉：{dump}"
        );
    }
}

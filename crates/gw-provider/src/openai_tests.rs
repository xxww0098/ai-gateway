//! Unit tests for [`crate::openai`].
//!
//! These lock in the planner's wiring: endpoint shape, credential precedence
//! and the provider-owned headers. What is *not* here any more is header
//! forwarding — the relay owns that denylist now, and asserting it twice is
//! how two denylists drift apart.

use super::*;
use bytes::Bytes;
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
    let overridden = auth(json!({"api_key": " sk-auth "}), &[]);
    let (key, base) = provider.resolve_credentials(&overridden);
    assert_eq!(key, "sk-auth");
    assert_eq!(base, provider.base_url());

    let blank = auth(json!({"api_key": ""}), &[]);
    let (key, _) = provider.resolve_credentials(&blank);
    assert_eq!(key, "sk-config", "a blank override must not win");

    let bare = bare_auth();
    let (key, _) = provider.resolve_credentials(&bare);
    assert_eq!(key, "sk-config");
}

#[test]
fn both_base_url_spellings_are_accepted_with_underscore_winning() {
    let provider = provider();
    let both = auth(
        json!({}),
        &[
            ("base_url", "https://a.example.com/"),
            ("base-url", "https://b.example.com"),
        ],
    );
    let (_, base) = provider.resolve_credentials(&both);
    assert_eq!(base, "https://a.example.com");

    let dashed = auth(json!({}), &[("base-url", "https://b.example.com")]);
    let (_, base) = provider.resolve_credentials(&dashed);
    assert_eq!(base, "https://b.example.com");
}

// --- route plan ---------------------------------------------------------------

fn planned(provider: &OpenAiCompatibleProvider, req: &ProviderRequest) -> RoutePlan {
    provider
        .plan_request(req, "sk-live", provider.base_url())
        .expect("the plan must build")
}

#[test]
fn the_outbound_request_carries_the_bearer_token_and_json_defaults() {
    let provider = provider();
    let req = ProviderRequest {
        payload: Bytes::from_static(br#"{"model":"gpt-4o"}"#),
        ..Default::default()
    };
    let plan = planned(&provider, &req);

    assert_eq!(
        plan.endpoint.as_str(),
        "https://api.example.com/v1/chat/completions"
    );
    assert!(matches!(&plan.credential, gw_relay::Credential::Bearer(t) if t == "sk-live"));
    assert_eq!(plan.headers[CONTENT_TYPE], "application/json");
    assert_eq!(plan.headers[ACCEPT], "application/json");
    assert!(
        plan.body.is_none(),
        "a non-streaming payload must go out untouched"
    );
}

#[test]
fn streaming_requests_ask_for_sse_and_force_include_usage() {
    let provider = provider();
    let req = ProviderRequest {
        payload: Bytes::from_static(br#"{"model":"gpt-4o","stream":true}"#),
        stream: true,
        ..Default::default()
    };
    let plan = planned(&provider, &req);

    assert_eq!(plan.headers[ACCEPT], "text/event-stream");
    assert_carries_the_spliced_body(&plan, &req.payload);
}

/// 插入段的内容与幂等性由 `common_tests` 的全量矩阵覆盖，这里只证明
/// **planner 确实接上了那条路，并且把两段都带上了**。
fn assert_carries_the_spliced_body(plan: &RoutePlan, payload: &Bytes) {
    let body = plan.body.as_ref().expect("a streaming plan must rewrite");
    assert!(body.len() > payload.len(), "插入的那一段没算进来");
    assert_eq!(
        body.len(),
        crate::common::ensure_include_usage(payload, Surface::OpenAiCompletions)
            .expect("fixture must be spliceable")
            .len(),
        "长度必须等于两段之和，否则上游会读短或读挂"
    );
}

#[test]
fn a_body_that_does_not_declare_stream_is_never_rewritten() {
    // Defense in depth: the caller's `stream` flag alone must not mutate a
    // payload whose body says otherwise.
    let provider = provider();
    let req = ProviderRequest {
        payload: Bytes::from_static(br#"{"model":"gpt-4o"}"#),
        stream: true,
        ..Default::default()
    };
    assert!(
        planned(&provider, &req).body.is_none(),
        "a body that does not declare `stream` must not be rewritten"
    );
}

#[test]
fn the_plan_carries_no_credential_header_of_its_own() {
    // The credential travels as `RoutePlan::credential` so the relay can strip
    // the client's carrier and mark the replacement sensitive. A provider that
    // also stamped a header would defeat both.
    let provider = provider();
    let mut headers = http::HeaderMap::new();
    headers.insert("authorization", HeaderValue::from_static("Bearer inbound"));
    let req = ProviderRequest {
        headers,
        ..Default::default()
    };
    let plan = planned(&provider, &req);

    for carrier in ["authorization", "x-api-key", "x-goog-api-key"] {
        assert!(
            !plan.headers.contains_key(carrier),
            "{carrier} must not be planned as a plain header"
        );
    }
    assert!(matches!(&plan.credential, gw_relay::Credential::Bearer(t) if t == "sk-live"));
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

    assert!(
        !planned(&provider, &req).headers.contains_key(ACCEPT),
        "an inbound Accept is left alone, so the relay forwards it verbatim"
    );

    let streaming = ProviderRequest {
        stream: true,
        ..req
    };
    assert_eq!(
        planned(&provider, &streaming).headers[ACCEPT],
        "text/event-stream",
        "a stream must override whatever the client asked to accept"
    );
}

#[test]
fn a_blank_credential_is_refused_before_anything_is_planned() {
    let provider = provider();
    for blank in ["", "   "] {
        let err = provider
            .plan_request(&ProviderRequest::default(), blank, provider.base_url())
            .expect_err("a keyless account must not produce a plan");
        assert!(matches!(err, ProviderError::Credential(_)), "{err:?}");
    }
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

/// 上游没有计数端点，所以这里必须**报错**，不能编一个数字
/// （`docs/relay-surface-plan.md` §2.1 缺陷 ①）。
///
/// 测的性质是「任何输入都拿不到数字」，而不是核对某一句错误文案 ——
/// 后者会把实现抄进断言。
#[tokio::test]
async fn count_tokens_refuses_rather_than_fabricating_a_number() {
    let provider = provider();
    for len in [0, 4, 400, 4000] {
        let req = ProviderRequest {
            payload: Bytes::from(vec![b'x'; len]),
            ..Default::default()
        };
        assert!(
            provider
                .plan_count_tokens(&bare_auth(), &req)
                .await
                .is_err(),
            "{len} bytes produced a fabricated count"
        );
    }
}

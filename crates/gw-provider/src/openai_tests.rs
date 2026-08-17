//! Unit tests for [`crate::openai`].
//!
//! These lock in the executor's wiring: endpoint shape, credential precedence
//! and the outbound header contract.

use super::*;
use crate::types::is_skipped_proxy_header;
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
        payload: Bytes::from_static(br#"{"model":"gpt-4o"}"#),
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
        Some(req.payload.as_ref()),
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
    let request = built(&provider, &req, true);

    assert_eq!(request.headers()[ACCEPT], "text/event-stream");
    assert_declares_the_spliced_length(&request, &req.payload);
}

/// 定点插入之后 body 是**两帧零拷贝流**（前缀 + 原 body 的切片），不再是一块能
/// 直接读回来的缓冲区 —— 那正是省掉那次全量拷贝的代价。上游真正看到的长度契约
/// 是网关显式声明的 `content-length`，所以 executor 这一层就断言它。
///
/// 插入段的内容与幂等性由 `common_tests` 的全量矩阵覆盖，这里只证明
/// **executor 确实接上了那条路，并且把长度算对了**。
fn assert_declares_the_spliced_length(request: &reqwest::Request, payload: &Bytes) {
    let declared: usize = request.headers()[http::header::CONTENT_LENGTH]
        .to_str()
        .expect("content-length must be ASCII")
        .parse()
        .expect("content-length must be a number");
    assert!(
        declared > payload.len(),
        "content-length 没有把插入的那一段算进去"
    );
    assert_eq!(
        declared,
        crate::common::ensure_include_usage(payload, Surface::OpenAiCompletions)
            .expect("fixture must be spliceable")
            .len(),
        "声明的长度必须等于两段之和，否则上游会读短或读挂"
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
    let request = built(&provider, &req, true);
    assert_eq!(
        request.body().and_then(reqwest::Body::as_bytes),
        Some(req.payload.as_ref())
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
            provider.count_tokens(&bare_auth(), req).await.is_err(),
            "{len} bytes produced a fabricated count"
        );
    }
}


/// 大小写与两侧空白都不该改变「这个头该不该转发」的判定。
///
/// 守护的 bug：改回 `to_ascii_lowercase()`（每头一次分配），或者改成
/// 只认小写、把 `Authorization` 漏出去打到上游。
#[test]
fn hop_by_hop_header_names_match_without_regard_to_case() {
    for name in ["Authorization", "AUTHORIZATION", " authorization ", "Host", "content-length"] {
        assert!(
            is_skipped_proxy_header(name),
            "{name} must stay on the denylist"
        );
    }
    assert!(!is_skipped_proxy_header("x-custom"));
    assert!(!is_skipped_proxy_header("openai-organization"));
}

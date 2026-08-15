//! Tenant authentication, including the two second-stage gates documented at
//! length: the user-status recheck and the entitlement filter.

use std::sync::Arc;

use super::*;
use crate::ports::{ApiKeyRow, SubscriptionQuota};
use crate::testsupport::{FakeCrypto, FakeDirectory};

const KEY: &str = "cpa-abc";

struct Fixture {
    provider: AccessProvider,
    directory: Arc<FakeDirectory>,
    crypto: Arc<FakeCrypto>,
}

fn fixture() -> Fixture {
    let directory = FakeDirectory::shared();
    let crypto = FakeCrypto::shared();
    Fixture {
        provider: AccessProvider::new(directory.clone(), crypto.clone()),
        directory,
        crypto,
    }
}

impl Fixture {
    fn key_hash(&self) -> String {
        self.crypto.hash_api_key(KEY)
    }

    fn register_key(&self, row: ApiKeyRow) {
        self.directory.with_active_key(&self.key_hash(), row);
    }

    async fn auth(&self, header: &str) -> Result<crate::ports::AccessMetadata, AuthError> {
        self.provider.authenticate(Some(header)).await
    }
}

fn active_key(user_id: crate::ports::Id, group_id: Option<crate::ports::Id>) -> ApiKeyRow {
    ApiKeyRow {
        id: 11,
        user_id,
        group_id,
        status: "active".to_owned(),
    }
}

// ---------------------------------------------------------------- header parsing

#[test]
fn a_bearer_header_yields_its_token_whatever_the_scheme_casing() {
    assert_eq!(bearer_token("Bearer abc"), Some("abc"));
    assert_eq!(bearer_token("bearer abc"), Some("abc"));
    assert_eq!(bearer_token("BEARER abc"), Some("abc"));
    assert_eq!(bearer_token("  Bearer   abc  "), Some("abc"));
}

#[test]
fn malformed_authorization_headers_are_rejected_outright() {
    for header in ["", "abc", "Bearer", "Bearer ", "Basic abc", "Bearer a b"] {
        assert_eq!(bearer_token(header), None, "accepted {header:?}");
    }
}

// ---------------------------------------------------------------- surface gate

#[test]
fn one_prefix_covers_the_whole_metered_surface() {
    // 收敛后只剩一个前缀。`/v1beta` 曾经是 `/v1` 的**兄弟**而不是子路径，
    // 所以它需要自己的一条判据；那个面删掉之后判据也回到一条。
    for path in ["/v1/chat/completions", "/v1/messages", "/v1/models"] {
        assert!(is_proxy_path(path), "{path} escaped the gate");
    }
    for path in ["/v1beta/models", "/v1beta/models/gemini-2.5-pro"] {
        assert!(!is_proxy_path(path), "{path} 属于已被硬删的 Gemini 原生面");
    }
}

#[test]
fn everything_billed_is_authenticated_first() {
    // 单向蕴含，不是等价：计费面**必须**是鉴权面的子集，否则会出现
    // 「计费但匿名」的路由。反向不成立是本轮有意为之 ——
    // `GET /v1/models` 与 `count_tokens` 鉴权但不计费。
    for path in [
        "/v1/messages",
        "/v1/messages/count_tokens",
        "/v1/models",
        "/api/panel/user/profile",
        "/healthz",
        "/v1betaX/models",
        "/v1",
    ] {
        for method in [axum::http::Method::POST, axum::http::Method::GET] {
            if crate::hold::is_billable(&method, path) {
                assert!(
                    is_proxy_path(path),
                    "{method} {path} 会被计费，却不在鉴权面上",
                );
            }
        }
    }
}

#[test]
fn the_zero_cost_endpoints_are_authenticated_but_not_billed() {
    // 移出计费范围的三条：两条 catalogue 读 + count_tokens。
    for (method, path) in [
        (axum::http::Method::GET, "/v1/models"),
        (axum::http::Method::GET, "/v1/models/gpt-4o"),
        (axum::http::Method::POST, "/v1/messages/count_tokens"),
    ] {
        assert!(is_proxy_path(path), "{path} 必须仍然要鉴权");
        assert!(
            !crate::hold::is_billable(&method, path),
            "{method} {path} 仍然在按 LLM 价格收钱",
        );
    }
}

#[test]
fn the_panel_and_health_surfaces_keep_their_own_auth() {
    assert!(!is_proxy_path("/api/panel/admin/users"));
    assert!(!is_proxy_path("/healthz"));
    assert!(
        !is_proxy_path("/v1betamodels"),
        "the prefix must end at a path separator, not swallow arbitrary suffixes",
    );
}

#[tokio::test]
async fn a_request_without_credentials_is_distinguished_from_a_bad_one() {
    let fixture = fixture();
    assert_eq!(
        fixture.provider.authenticate(None).await.unwrap_err(),
        AuthError::NoCredentials
    );
    assert_eq!(
        fixture.auth("Bearer cpa-").await.unwrap_err(),
        AuthError::InvalidCredential
    );
}

// ---------------------------------------------------------------- api key path

#[tokio::test]
async fn an_active_key_resolves_to_its_tenant() {
    let fixture = fixture();
    fixture.register_key(active_key(42, None));

    let meta = fixture
        .auth(&format!("Bearer {KEY}"))
        .await
        .expect("authenticates");
    assert_eq!(meta.user_id, 42);
    assert_eq!(meta.api_key_id, 11);
    assert_eq!(meta.rate_mult, 1.0);
    assert_eq!(
        fixture.directory.touched.lock().as_slice(),
        [11],
        "last_used_at must be bumped for the key that was used",
    );
}

#[tokio::test]
async fn an_unknown_key_is_rejected() {
    let fixture = fixture();
    assert_eq!(
        fixture.auth(&format!("Bearer {KEY}")).await.unwrap_err(),
        AuthError::InvalidCredential
    );
}

#[tokio::test]
async fn a_deactivated_key_is_rejected_even_though_the_row_exists() {
    // A cache entry can outlive a DB-side deactivation, so the status gate has
    // to fire on the lookup result rather than only in the SQL predicate.
    let fixture = fixture();
    fixture
        .directory
        .users
        .lock()
        .insert(9, "active".to_owned());
    fixture.directory.api_keys.lock().insert(
        fixture.key_hash(),
        ApiKeyRow {
            id: 11,
            user_id: 9,
            group_id: None,
            status: "revoked".to_owned(),
        },
    );
    assert_eq!(
        fixture.auth(&format!("Bearer {KEY}")).await.unwrap_err(),
        AuthError::InvalidCredential
    );
}

#[tokio::test]
async fn a_suspended_owner_invalidates_an_otherwise_active_key() {
    let fixture = fixture();
    fixture.register_key(active_key(42, None));
    fixture
        .directory
        .users
        .lock()
        .insert(42, "banned".to_owned());
    assert_eq!(
        fixture.auth(&format!("Bearer {KEY}")).await.unwrap_err(),
        AuthError::InvalidCredential
    );
}

#[tokio::test]
async fn a_deleted_owner_invalidates_an_otherwise_active_key() {
    let fixture = fixture();
    fixture.register_key(active_key(42, None));
    fixture.directory.users.lock().remove(&42);
    assert_eq!(
        fixture.auth(&format!("Bearer {KEY}")).await.unwrap_err(),
        AuthError::InvalidCredential
    );
}

#[tokio::test]
async fn a_status_lookup_outage_denies_rather_than_admits() {
    // Fail closed: the alternative is letting a suspended tenant spend for the
    // duration of a database blip.
    let fixture = fixture();
    fixture.register_key(active_key(42, None));
    *fixture.directory.user_status_errors.lock() = true;
    assert_eq!(
        fixture.auth(&format!("Bearer {KEY}")).await.unwrap_err(),
        AuthError::InvalidCredential
    );
}

// ---------------------------------------------------------------- entitlement

#[tokio::test]
async fn a_discounted_group_needs_a_live_subscription() {
    let fixture = fixture();
    fixture.register_key(active_key(42, Some(5)));
    fixture.directory.groups.lock().insert(5, 0.5);
    fixture.directory.entitlements.lock().push((42, 5));

    let meta = fixture
        .auth(&format!("Bearer {KEY}"))
        .await
        .expect("authenticates");
    assert_eq!(meta.group_id, Some(5));
    assert_eq!(meta.rate_mult, 0.5);
}

#[tokio::test]
async fn a_lapsed_subscription_collapses_the_principal_to_baseline() {
    // The key still points at the discounted group in the DB; only the
    // entitlement recheck is what stops the discount from outliving the
    // subscription (otherwise it would survive until the cache TTL).
    let fixture = fixture();
    fixture.register_key(active_key(42, Some(5)));
    fixture.directory.groups.lock().insert(5, 0.5);
    // no entitlement row registered

    let meta = fixture
        .auth(&format!("Bearer {KEY}"))
        .await
        .expect("authenticates");
    assert_eq!(meta.group_id, None, "a revoked group must not stay bound");
    assert_eq!(
        meta.rate_mult, 1.0,
        "a revoked group must not keep its discount",
    );
}

#[tokio::test]
async fn the_baseline_group_needs_no_subscription() {
    let fixture = fixture();
    fixture.register_key(active_key(42, Some(5)));
    fixture.directory.groups.lock().insert(5, 1.0);

    let meta = fixture
        .auth(&format!("Bearer {KEY}"))
        .await
        .expect("authenticates");
    assert_eq!(meta.group_id, Some(5));
    assert_eq!(meta.rate_mult, 1.0);
}

#[tokio::test]
async fn a_missing_group_row_denies_the_multiplier() {
    let fixture = fixture();
    fixture.register_key(active_key(42, Some(5)));

    let meta = fixture
        .auth(&format!("Bearer {KEY}"))
        .await
        .expect("authenticates");
    assert_eq!(meta.group_id, None);
    assert_eq!(meta.rate_mult, 1.0);
}

// ---------------------------------------------------------------- jwt path

#[tokio::test]
async fn a_valid_jwt_resolves_to_its_tenant_without_an_api_key() {
    let fixture = fixture();
    fixture.crypto.with_jwt("token-1", 42);
    fixture
        .directory
        .users
        .lock()
        .insert(42, "active".to_owned());

    let meta = fixture.auth("Bearer token-1").await.expect("authenticates");
    assert_eq!(meta.user_id, 42);
    assert_eq!(meta.api_key_id, 0, "the JWT path has no api key to bill to");
    assert_eq!(meta.group_id, None);
}

#[tokio::test]
async fn an_unverifiable_jwt_is_rejected() {
    let fixture = fixture();
    assert_eq!(
        fixture.auth("Bearer token-1").await.unwrap_err(),
        AuthError::InvalidCredential
    );
}

#[tokio::test]
async fn a_jwt_for_a_suspended_user_leaks_no_subscription_state() {
    // The rejection must be indistinguishable from any other invalid
    // credential, so the endpoint cannot be used as a user-status oracle.
    let fixture = fixture();
    fixture.crypto.with_jwt("token-1", 42);
    fixture
        .directory
        .users
        .lock()
        .insert(42, "banned".to_owned());
    fixture.directory.subscriptions.lock().insert(
        42,
        SubscriptionQuota {
            id: 77,
            ..SubscriptionQuota::default()
        },
    );
    assert_eq!(
        fixture.auth("Bearer token-1").await.unwrap_err(),
        AuthError::InvalidCredential
    );
}

// ---------------------------------------------------------------- metadata

#[tokio::test]
async fn an_active_subscription_travels_with_the_principal() {
    let fixture = fixture();
    fixture.register_key(active_key(42, None));
    fixture.directory.subscriptions.lock().insert(
        42,
        SubscriptionQuota {
            id: 77,
            group_id: 5,
            daily_usage_usd: 1.5,
            daily_limit_usd: Some(10.0),
            ..SubscriptionQuota::default()
        },
    );

    let meta = fixture
        .auth(&format!("Bearer {KEY}"))
        .await
        .expect("authenticates");
    let sub = meta.subscription.clone().expect("subscription attached");
    assert_eq!(sub.id, 77);
    assert_eq!(sub.daily_limit_usd, Some(10.0));

    // The map is stringified for the wire; the shape is preserved for logs
    // and for parity with the legacy implementation.
    let wire = meta.to_map();
    assert_eq!(wire.get("user_id").map(String::as_str), Some("42"));
    assert_eq!(wire.get("subscription_id").map(String::as_str), Some("77"));
    assert_eq!(wire.get("daily_limit").map(String::as_str), Some("10"));
    assert!(
        !wire.contains_key("weekly_limit"),
        "an unset limit must be absent, not zero — zero means 'no spend allowed'",
    );
}

#[tokio::test]
async fn a_missing_subscription_is_not_an_error() {
    let fixture = fixture();
    fixture.register_key(active_key(42, None));
    let meta = fixture
        .auth(&format!("Bearer {KEY}"))
        .await
        .expect("authenticates");
    assert!(meta.subscription.is_none());
}

// ---------------------------------------------------------------- 凭证载体

fn headers_with(pairs: &[(&str, &str)]) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).expect("header name"),
            axum::http::HeaderValue::from_str(value).expect("header value"),
        );
    }
    headers
}

#[test]
fn a_credential_is_read_from_authorization_and_from_nowhere_else() {
    // 三面收敛之前，`/v1beta` 还接受 `x-goog-api-key` / `x-api-key` / `?key=`
    // 三种载体。那个面已经不存在，三种载体一并下线：带着它们来的请求
    // 就是「没有凭据」。
    let carriers = headers_with(&[("x-goog-api-key", KEY), ("x-api-key", KEY)]);
    assert_eq!(credential_from(&carriers), None);

    let authed = headers_with(&[("authorization", &format!("Bearer {KEY}"))]);
    assert_eq!(credential_from(&authed), Some(KEY));
}

#[test]
fn the_anthropic_key_header_is_not_a_tenant_credential() {
    // 这条不是「载体少了一个」，而是 `x-api-key` 回到了它在 `/v1` 上一直以来的
    // 身份：**Anthropic 自己的上游头**。把它当租户凭据读，等于让任何一个照着
    // Anthropic 文档配置的客户端用它自己的上游 key 冒充租户。
    let headers = headers_with(&[("x-api-key", KEY), ("authorization", "Bearer other-token")]);
    assert_eq!(
        credential_from(&headers),
        Some("other-token"),
        "Authorization 是唯一被读的载体",
    );
}

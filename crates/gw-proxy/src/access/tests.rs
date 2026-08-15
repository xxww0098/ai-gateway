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
fn both_dialect_prefixes_are_on_the_metered_surface() {
    // `/v1beta` is a sibling of `/v1`, not a child: a `starts_with("/v1/")`
    // gate lets the entire Gemini surface through unauthenticated and unbilled.
    for path in [
        "/v1/chat/completions",
        "/v1/models",
        "/v1beta/models",
        "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
    ] {
        assert!(is_proxy_path(path), "{path} escaped the gate");
    }
}

#[test]
fn the_gate_matches_what_the_billing_layer_reserves_for() {
    // Authentication and reservation must cover the same set, or a route ends
    // up billed but anonymous — or authenticated but free.
    for path in [
        "/v1/messages",
        "/v1beta/models/gemini-2.5-pro:generateContent",
        "/api/panel/user/profile",
        "/healthz",
        "/v1betaX/models",
        "/v1",
    ] {
        assert_eq!(
            is_proxy_path(path),
            crate::hold::is_billable(&axum::http::Method::POST, path),
            "the two gates disagree about {path}",
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

// ---------------------------------------------------------------- credential carriers

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
fn the_v1_surface_reads_a_credential_from_authorization_alone() {
    // Exactly one header is read. `/v1` is implemented by this repo's own
    // code, so it stays that way.
    let headers = headers_with(&[("x-goog-api-key", KEY), ("x-api-key", KEY)]);
    assert_eq!(
        credential_from("/v1/chat/completions", &headers, Some("key=leaked")),
        None,
    );
    let authed = headers_with(&[("authorization", &format!("Bearer {KEY}"))]);
    assert_eq!(
        credential_from("/v1/chat/completions", &authed, None),
        Some(KEY)
    );
}

#[test]
fn the_gemini_surface_reads_the_carriers_google_sdks_actually_use() {
    for carrier in ["x-goog-api-key", "x-api-key"] {
        let headers = headers_with(&[(carrier, KEY)]);
        assert_eq!(
            credential_from("/v1beta/models/m:generateContent", &headers, None),
            Some(KEY),
            "{carrier} was not read",
        );
    }
    assert_eq!(
        credential_from(
            "/v1beta/models/m:generateContent",
            &axum::http::HeaderMap::new(),
            Some(&format!("alt=sse&key={KEY}")),
        ),
        Some(KEY),
    );
}

#[test]
fn the_carrier_priority_is_fixed_so_a_request_with_several_has_one_outcome() {
    let headers = headers_with(&[
        ("authorization", "Bearer from-authorization"),
        ("x-goog-api-key", "from-goog"),
        ("x-api-key", "from-api-key"),
    ]);
    let query = Some("key=from-query");
    let path = "/v1beta/models/m:generateContent";

    assert_eq!(
        credential_from(path, &headers, query),
        Some("from-authorization")
    );

    let mut headers = headers;
    headers.remove("authorization");
    assert_eq!(credential_from(path, &headers, query), Some("from-goog"));
    headers.remove("x-goog-api-key");
    assert_eq!(credential_from(path, &headers, query), Some("from-api-key"));
    headers.remove("x-api-key");
    assert_eq!(credential_from(path, &headers, query), Some("from-query"));
}

#[test]
fn a_blank_carrier_is_no_credential_at_all() {
    let path = "/v1beta/models/m:generateContent";
    let headers = headers_with(&[("x-goog-api-key", "   "), ("x-api-key", "")]);
    assert_eq!(credential_from(path, &headers, Some("key=")), None);
    assert_eq!(credential_from(path, &headers, Some("alt=sse")), None);
}

#[test]
fn a_consumed_credential_is_removed_from_what_gets_relayed() {
    let mut headers = headers_with(&[
        ("x-goog-api-key", KEY),
        ("x-api-key", KEY),
        ("accept", "*/*"),
    ]);
    let mut query = vec![
        ("alt".to_owned(), "sse".to_owned()),
        ("key".to_owned(), KEY.to_owned()),
    ];

    strip_consumed_credentials("/v1beta/models/m:generateContent", &mut headers, &mut query);

    assert!(headers.get("x-goog-api-key").is_none());
    assert!(headers.get("x-api-key").is_none());
    assert!(
        headers.get("accept").is_some(),
        "unrelated headers must survive"
    );
    assert_eq!(query, vec![("alt".to_owned(), "sse".to_owned())]);
}

#[test]
fn the_v1_surface_relays_its_headers_untouched() {
    // `x-api-key` there is Anthropic's own credential header, and `/v1` never
    // read a tenant credential from it — stripping it would break the executor.
    let mut headers = headers_with(&[("x-api-key", "anthropic-key")]);
    let mut query = vec![("key".to_owned(), "caller-supplied".to_owned())];

    strip_consumed_credentials("/v1/messages", &mut headers, &mut query);

    assert!(headers.get("x-api-key").is_some());
    assert_eq!(query.len(), 1);
}

#[test]
fn a_credential_in_a_query_string_is_masked_before_anything_can_log_it() {
    // A query string is the one part of a URI that access logs and tracing
    // spans record by default, and on this surface it is credential material.
    let redacted = redact_query(&format!("alt=sse&key={KEY}&pageSize=5"));
    assert!(
        !redacted.contains(KEY),
        "the credential survived: {redacted}"
    );
    assert!(redacted.contains("alt=sse") && redacted.contains("pageSize=5"));
}

#[test]
fn redaction_leaves_a_query_without_a_credential_alone() {
    // So a caller can apply it unconditionally to every URI it renders.
    assert_eq!(redact_query("alt=sse&pageSize=5"), "alt=sse&pageSize=5");
    assert_eq!(redact_query(""), "");
}

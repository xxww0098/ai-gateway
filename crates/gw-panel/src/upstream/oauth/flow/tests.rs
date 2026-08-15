//! Unit tests for the OAuth start.
//!
//! This is the security-relevant half: which provider a key names, where the
//! operator is sent back to, and the PKCE pair. All pure, so all covered here.

use super::*;
use axum::http::{HeaderName, HeaderValue};

fn config() -> SessionConfig {
    SessionConfig {
        redirect_uri:
            "https://gw.example.test/api/panel/admin/sdk-management/oauth-callback/claude"
                .to_owned(),
        state: "state-1".to_owned(),
        ..SessionConfig::default()
    }
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::try_from(*name).expect("valid header name"),
            HeaderValue::from_str(value).expect("ascii"),
        );
    }
    headers
}

// ---------------------------------------------------------------- providers

#[test]
fn the_three_providers_round_trip() {
    for provider in [Provider::Gemini, Provider::Claude, Provider::Codex] {
        assert_eq!(Provider::parse(provider.as_str()), Some(provider));
    }
}

#[test]
fn provider_parsing_is_case_and_space_insensitive() {
    assert_eq!(Provider::parse(" Claude "), Some(Provider::Claude));
}

#[test]
fn anything_else_is_not_an_oauth_provider() {
    // `openai` and `vertex` are API-key pools; accepting them here would mint a
    // session no callback could ever complete.
    for unknown in ["", "openai", "vertex", "anthropic", "antigravity", "kimi"] {
        assert_eq!(Provider::parse(unknown), None);
    }
}

#[test]
fn the_anthropic_auth_url_key_maps_to_claude() {
    // The endpoint key and the stored provider deliberately differ.
    assert_eq!(
        Provider::from_auth_url_key("anthropic-auth-url"),
        Some(Provider::Claude)
    );
}

#[test]
fn the_delegated_auth_url_keys_are_not_recognised() {
    // `antigravity`/`kimi` were forwarded to the SDK, which is gone. They must
    // fall through to the 404 the gateway serves when it is not wired.
    for key in ["antigravity-auth-url", "kimi-auth-url", "openai-auth-url"] {
        assert_eq!(Provider::from_auth_url_key(key), None);
    }
}

// ---------------------------------------------------------------- redirect

#[test]
fn the_redirect_uri_follows_the_forwarded_headers() {
    let uri = redirect_uri(
        &headers(&[
            ("host", "internal:8080"),
            ("x-forwarded-host", "gw.example.test"),
            ("x-forwarded-proto", "https"),
        ]),
        Provider::Claude,
    );
    assert!(uri.starts_with("https://gw.example.test/"));
    assert!(!uri.contains("internal:8080"));
}

#[test]
fn the_redirect_uri_falls_back_to_the_host_header() {
    let uri = redirect_uri(&headers(&[("host", "gw.example.test")]), Provider::Codex);
    assert!(uri.starts_with("http://gw.example.test/"));
}

#[test]
fn the_redirect_uri_names_the_provider_it_will_come_back_to() {
    // The callback route parses the provider out of the path; a mismatch here
    // makes every flow fail with "provider mismatch".
    for provider in [Provider::Gemini, Provider::Claude, Provider::Codex] {
        let uri = redirect_uri(&headers(&[("host", "h")]), provider);
        assert!(uri.ends_with(provider.as_str()), "{uri}");
    }
}

// ---------------------------------------------------------------- pkce

#[test]
fn the_two_pkce_providers_store_a_verifier() {
    for provider in [Provider::Claude, Provider::Codex] {
        let mut config = config();
        let url = build_authorize_url(provider, "state-1", &mut config).expect("entropy");
        assert!(!config.code_verifier.is_empty());
        assert_eq!(config.code_challenge_method, "S256");
        assert!(url.contains("code_challenge="));
        // The verifier is the secret half and must never be in the URL.
        assert!(!url.contains(&config.code_verifier));
    }
}

#[test]
fn gemini_uses_offline_consent_rather_than_pkce() {
    let mut config = config();
    let url = build_authorize_url(Provider::Gemini, "state-1", &mut config).expect("entropy");
    assert!(config.code_verifier.is_empty());
    assert!(url.contains("access_type=offline"));
    assert!(url.contains("prompt=consent"));
}

#[test]
fn every_authorize_url_carries_the_state_we_minted() {
    // The state is the only thing binding the browser round-trip; a URL without
    // it produces a callback that can never be matched.
    for provider in [Provider::Gemini, Provider::Claude, Provider::Codex] {
        let mut config = config();
        let url = build_authorize_url(provider, "state-xyz", &mut config).expect("entropy");
        assert!(url.contains("state=state-xyz"), "{url}");
    }
}

#[test]
fn every_authorize_url_carries_the_redirect_we_will_accept() {
    for provider in [Provider::Gemini, Provider::Claude, Provider::Codex] {
        let mut config = config();
        let url = build_authorize_url(provider, "s", &mut config).expect("entropy");
        assert!(url.contains("redirect_uri="), "{url}");
    }
}

#[test]
fn two_flows_never_share_a_verifier() {
    let mut first = config();
    let mut second = config();
    build_authorize_url(Provider::Claude, "s", &mut first).expect("entropy");
    build_authorize_url(Provider::Claude, "s", &mut second).expect("entropy");
    assert_ne!(first.code_verifier, second.code_verifier);
}

#[test]
fn the_verifier_is_long_enough_to_be_a_secret() {
    // RFC 7636 puts the floor at 43 characters; 96 random bytes base64url'd is
    // well past it. A short verifier would be brute-forceable.
    let mut config = config();
    build_authorize_url(Provider::Codex, "s", &mut config).expect("entropy");
    assert!(config.code_verifier.len() >= 43);
    assert!(
        config
            .code_verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    );
}

#[test]
fn the_challenge_is_derived_from_the_verifier() {
    // If it were independent, the provider's PKCE check would fail every time.
    let mut config = config();
    let url = build_authorize_url(Provider::Claude, "s", &mut config).expect("entropy");
    let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(config.code_verifier.as_bytes()));
    assert!(url.contains(&expected), "challenge is not S256(verifier)");
}

// ---------------------------------------------------------------- encoding

#[test]
fn the_authorize_url_percent_encodes_its_parameters() {
    // The redirect URI contains `://` and `/`; an unencoded one truncates the
    // query and the provider rejects the request.
    let mut config = config();
    let url = build_authorize_url(Provider::Gemini, "s", &mut config).expect("entropy");
    assert!(url.contains("redirect_uri=https%3A%2F%2F"), "{url}");
}

#[test]
fn spaces_in_a_scope_list_become_plus_signs() {
    // A raw space would end the URL at the first scope.
    let mut config = config();
    let url = build_authorize_url(Provider::Codex, "s", &mut config).expect("entropy");
    assert!(!url.contains(' '), "{url}");
    assert!(url.contains("scope=openid+email"), "{url}");
}

#[test]
fn form_encoding_is_stable() {
    // Sorted output keeps an authorize URL diffable in a log.
    let params = [
        ("z", "1".to_owned()),
        ("a", "2".to_owned()),
        ("m", "3".to_owned()),
    ];
    assert_eq!(form_encode(&params), "a=2&m=3&z=1");
}

#[test]
fn unreserved_characters_survive_encoding() {
    let params = [("k", "aZ0-_.~".to_owned())];
    assert_eq!(form_encode(&params), "k=aZ0-_.~");
}

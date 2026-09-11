use url::Url;

use super::{
    canonicalize_method, refresh_expires_at_ms, social_redirect_uri, social_token_redirect_uri,
    validate_api_key, validate_idp_endpoint, validate_refresh_token,
};

#[test]
fn method_aliases_collapse_to_the_four_stored_kinds() {
    assert_eq!(canonicalize_method("builder-id", None), "idc");
    assert_eq!(canonicalize_method("enterprise", None), "idc");
    assert_eq!(canonicalize_method("github", None), "social");
    assert_eq!(canonicalize_method("gmail", None), "social");
    assert_eq!(canonicalize_method("oauth", None), "social");
    assert_eq!(canonicalize_method("ksk", None), "api_key");
    assert_eq!(canonicalize_method("entra-id", None), "external_idp");
    assert_eq!(
        canonicalize_method(
            "",
            Some("https://login.microsoftonline.com/t/oauth2/v2.0/token")
        ),
        "external_idp"
    );
}

#[test]
fn entra_endpoint_allow_list_is_https_microsoftonline() {
    assert!(
        validate_idp_endpoint("https://login.microsoftonline.com/contoso/oauth2/v2.0/token")
            .is_ok()
    );
    assert!(validate_idp_endpoint("http://login.microsoftonline.com/t/token").is_err());
    assert!(validate_idp_endpoint("https://evil.example/token").is_err());
}

#[test]
fn truncated_refresh_and_non_ksk_keys_are_rejected() {
    let long = format!("rt_{}", "x".repeat(120));
    assert_eq!(validate_refresh_token(&long).expect("rt"), long);
    assert!(validate_refresh_token("too-short").is_err());
    assert!(validate_refresh_token("shortshortshortshortshort...").is_err());
    assert!(validate_api_key("ksk_live_example1").is_ok());
    assert!(validate_api_key("sk-not-kiro").is_err());
}

#[test]
fn social_authorize_drops_path_and_loopback_becomes_localhost() {
    let callback = "http://127.0.0.1:3128/oauth/callback";
    let origin = social_redirect_uri(callback).expect("origin");
    assert!(origin.contains("://"), "{origin}");
    assert!(Url::parse(&origin).is_ok(), "{origin}");
    assert!(!origin.contains("127.0.0.1"));
    assert!(!origin.contains("/oauth/callback"));
    assert_ne!(origin, callback);
    let token = social_token_redirect_uri(callback, Some("/signin/callback"), Some("Google"))
        .expect("token");
    assert!(token.contains("/signin/callback"));
    assert!(token.contains("login_option=google"));
    assert_ne!(token, origin);
    assert!(!token.contains("127.0.0.1"));
}

#[test]
fn refresh_ttl_comes_from_the_response_not_the_stored_stamp() {
    let stale = 1_756_634_673_000i64;
    let now = 1_800_000_000_000i64;
    let body = serde_json::json!({"expiresIn": 1234, "accessToken": "next"});
    let next = refresh_expires_at_ms(&body, now);
    assert_ne!(next, stale);
    assert_eq!(next, now + 1_234_000);
    let kept = refresh_expires_at_ms(&serde_json::json!({"expiresAt": stale}), now);
    assert_eq!(kept, stale);
}

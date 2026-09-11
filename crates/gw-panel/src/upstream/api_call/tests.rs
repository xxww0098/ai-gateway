use super::{fill_token, parse_allowed_url};
use std::collections::HashMap;

#[test]
fn https_allow_listed_hosts_are_accepted() {
    for url in [
        "https://api.anthropic.com/api/oauth/usage",
        "https://chatgpt.com/backend-api/wham/usage",
        "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota",
        "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
    ] {
        assert!(parse_allowed_url(url).is_ok(), "{url}");
    }
}

#[test]
fn http_unknown_hosts_and_userinfo_are_rejected() {
    for url in [
        "http://api.anthropic.com/api/oauth/usage",
        "https://evil.example/steal",
        "https://api.anthropic.com.evil.example/",
        "https://user:pass@api.anthropic.com/",
        "not-a-url",
    ] {
        assert!(parse_allowed_url(url).is_err(), "{url}");
    }
}

#[test]
fn token_placeholders_are_substituted_in_every_header() {
    let mut headers = HashMap::from([
        ("Authorization".to_owned(), "Bearer $TOKEN$".to_owned()),
        ("X-Other".to_owned(), "keep".to_owned()),
    ]);
    fill_token(&mut headers, "secret-token");
    assert_eq!(headers["Authorization"], "Bearer secret-token");
    assert_eq!(headers["X-Other"], "keep");
}

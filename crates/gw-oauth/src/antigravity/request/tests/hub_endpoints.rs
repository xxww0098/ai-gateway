use crate::antigravity::{DAILY_API_URL, PROD_API_URL};

use super::{
    chat_headers, cloud_code_fallbacks, cloud_code_fallbacks_between, should_retry_on_prod,
};

#[test]
fn chat_headers_are_user_agent_only() {
    let headers = chat_headers("tok").expect("headers");
    let names: Vec<String> = headers
        .keys()
        .map(|n| n.as_str().to_ascii_lowercase())
        .collect();
    assert_eq!(names.len(), 4);
    for required in ["authorization", "accept", "content-type", "user-agent"] {
        assert!(
            names.contains(&required.to_owned()),
            "{names:?} missing {required}"
        );
    }
    for forbidden in ["x-goog-api-client", "client-metadata", "anthropic-beta"] {
        assert!(
            !names.iter().any(|n| n == forbidden),
            "chat leaked {forbidden}: {names:?}"
        );
    }
    let ua = headers
        .get(http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .expect("ua");
    assert!(
        ua.starts_with("antigravity/hub/"),
        "hub fingerprint missing: {ua}"
    );
    assert!(
        ua.split_whitespace()
            .nth(1)
            .is_some_and(|p| p.contains('/')),
        "os/arch missing: {ua}"
    );
}

#[test]
fn daily_generate_falls_back_to_prod_only_on_5xx() {
    let daily = format!("{DAILY_API_URL}/v1internal:generateContent");
    let urls = cloud_code_fallbacks(&daily);
    assert_eq!(urls.len(), 2);
    assert_eq!(urls[0], daily);
    assert!(urls[1].starts_with(PROD_API_URL));
    assert!(!urls[1].starts_with(DAILY_API_URL));

    let already_prod = format!("{PROD_API_URL}/v1internal:generateContent");
    assert_eq!(cloud_code_fallbacks(&already_prod), vec![already_prod]);

    assert!(should_retry_on_prod(500));
    assert!(should_retry_on_prod(503));
    assert!(!should_retry_on_prod(400));
    assert!(!should_retry_on_prod(403));
    assert!(!should_retry_on_prod(404));
    assert!(!should_retry_on_prod(429));
}

#[test]
fn fallback_rewrite_is_host_substitution_not_a_second_rpc() {
    let daily = "http://127.0.0.1:9";
    let prod = "http://127.0.0.1:8";
    let url = format!("{daily}/v1internal:loadCodeAssist");
    let urls = cloud_code_fallbacks_between(&url, daily, prod);
    assert_eq!(urls[0], url);
    assert_eq!(urls[1], format!("{prod}/v1internal:loadCodeAssist"));
    assert_eq!(
        cloud_code_fallbacks_between(prod, daily, prod),
        vec![prod.to_owned()]
    );
}

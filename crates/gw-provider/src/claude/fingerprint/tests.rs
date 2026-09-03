use http::{HeaderMap, HeaderValue};
use serde_json::json;

use super::*;
use crate::types::ProviderRequest;

/// Published NTT123 gist vector. The salt and truncation widths live in
/// the implementation; this only checks the publicly documented output.
#[test]
fn published_hey_vector_matches_community_capture() {
    let header = billing_header("hey", "2.1.37");
    assert!(header.contains("cc_version=2.1.37.0d9"), "{header}");
    assert!(header.contains("cch=fa690"), "{header}");
    assert!(header.contains("cc_entrypoint=cli"), "{header}");
}

#[test]
fn empty_message_cch_is_sha256_of_empty() {
    let header = billing_header("", "2.1.37");
    assert!(header.contains("cch=e3b0c"), "{header}");
}

#[test]
fn a_longer_message_changes_both_hashes() {
    let short = billing_header("hey", "2.1.37");
    let long = billing_header("hello from a much longer user turn", "2.1.37");
    assert_ne!(short, long);
}

#[test]
fn version_text_picks_the_highest_semver() {
    let parsed = parse_version_text("claude 2.1.10 (Claude Code) extra 2.1.233").expect("semver");
    assert_eq!(parsed, "2.1.233");
}

#[test]
fn oauth_cloak_puts_billing_first_without_cache_control() {
    let req = ProviderRequest {
        payload: Bytes::from(
            json!({
                "system": "stable prefix",
                "messages": [{"role": "user", "content": "hey"}],
                "tools": [{"name": "Read"}]
            })
            .to_string(),
        ),
        ..Default::default()
    };
    let mut headers = HeaderMap::new();
    let body = cloak(&req, &mut headers).expect("rewritten");
    let value: Value = serde_json::from_slice(&body).expect("json");
    let first = value["system"][0]["text"].as_str().unwrap_or("");
    assert!(first.starts_with("x-anthropic-billing-header:"), "{first}");
    assert!(value["system"][0].get("cache_control").is_none());
    assert!(
        value["system"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|block| block.get("cache_control").is_some()),
        "{value}"
    );
    assert!(
        headers
            .get(http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ua| ua.starts_with("claude-cli/") && ua.contains("external, cli")),
        "{headers:?}"
    );
    assert_eq!(
        headers.get("x-app").and_then(|v| v.to_str().ok()),
        Some("cli")
    );
    assert!(headers.contains_key("x-stainless-runtime"));
    assert!(!headers.contains_key("x-stainless-helper-method"));
}

#[test]
fn an_already_cloaked_body_is_not_given_a_second_billing_block() {
    let billing = billing_header("hey", "2.1.37");
    let req = ProviderRequest {
        payload: Bytes::from(
            json!({
                "system": [{"type": "text", "text": billing}],
                "messages": [{"role": "user", "content": "hey"}]
            })
            .to_string(),
        ),
        ..Default::default()
    };
    let mut headers = HeaderMap::new();
    let body = cloak(&req, &mut headers).expect("json");
    let value: Value = serde_json::from_slice(&body).expect("json");
    let billing_blocks = value["system"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| {
            block
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.starts_with("x-anthropic-billing-header:"))
        })
        .count();
    assert_eq!(billing_blocks, 1, "{value}");
}

#[test]
fn a_caller_user_agent_is_not_replaced() {
    let mut inbound = HeaderMap::new();
    inbound.insert(
        http::header::USER_AGENT,
        HeaderValue::from_static("claude-cli/2.1.233 (external, cli)"),
    );
    let req = ProviderRequest {
        headers: inbound,
        payload: Bytes::from(json!({"messages":[]}).to_string()),
        ..Default::default()
    };
    let mut headers = HeaderMap::new();
    cloak(&req, &mut headers);
    assert!(!headers.contains_key(http::header::USER_AGENT));
}

#[test]
fn metadata_user_id_is_stable_across_cloaks() {
    let req = ProviderRequest {
        payload: Bytes::from(json!({"messages":[]}).to_string()),
        ..Default::default()
    };
    let mut a = HeaderMap::new();
    let mut b = HeaderMap::new();
    let left: Value = serde_json::from_slice(&cloak(&req, &mut a).expect("a")).expect("json");
    let right: Value = serde_json::from_slice(&cloak(&req, &mut b).expect("b")).expect("json");
    let user_a = left["metadata"]["user_id"].as_str().unwrap_or("");
    let user_b = right["metadata"]["user_id"].as_str().unwrap_or("");
    assert!(!user_a.is_empty());
    assert_eq!(user_a, user_b);
    assert!(user_a.contains("_account_"));
    assert!(user_a.contains("_session_"));
}

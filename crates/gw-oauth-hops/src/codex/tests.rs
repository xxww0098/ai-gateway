use super::plan;
use crate::rewrite::{HopInput, HopRewrite};
use serde_json::json;

fn header_str<'a>(headers: &'a http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn rewrite(body: serde_json::Value) -> (HopRewrite, serde_json::Value) {
    let bytes = serde_json::to_vec(&body).unwrap();
    let hop = plan(&HopInput {
        body: &bytes,
        ..HopInput::default()
    });
    let rewritten = hop.body.as_deref().unwrap_or(&bytes);
    let value: serde_json::Value = serde_json::from_slice(rewritten).unwrap();
    (hop, value)
}

/// Codex sticky routing is one id in three headers. Splitting them is an
/// affinity miss on chatgpt.com.
#[test]
fn cache_headers_are_the_same_id() {
    let (hop, _) = rewrite(json!({
        "model": "gpt-5.5",
        "prompt_cache_key": "conv-1",
        "session_id": "should-be-stripped",
    }));
    let session = header_str(&hop.headers, "session-id").unwrap();
    assert_eq!(header_str(&hop.headers, "thread-id"), Some(session));
    assert_eq!(
        header_str(&hop.headers, "x-client-request-id"),
        Some(session)
    );
    assert_eq!(session, hop.cache_session_id.as_deref().unwrap());
}

/// DSH `session_id` must not go upstream. chatgpt.com 400s on it.
#[test]
fn session_id_is_stripped_from_the_body() {
    let (_, value) = rewrite(json!({
        "session_id": "dsh-sess",
        "prompt_cache_key": "pin-a",
        "instructions": "stay",
        "store": false,
        "include": ["reasoning.encrypted_content"],
    }));
    assert!(value.get("session_id").is_none());
    assert_eq!(value["prompt_cache_key"], json!("pin-a"));
}

/// A body already in Codex shape must leave `HopRewrite.body` as None so
/// the relay can forward the inbound `Bytes` by refcount.
#[test]
fn unchanged_json_does_not_allocate_a_body() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.5",
        "instructions": "stay",
        "store": false,
        "include": ["reasoning.encrypted_content"],
    }))
    .unwrap();
    let hop = plan(&HopInput {
        body: &body,
        ..HopInput::default()
    });
    assert!(hop.body.is_none());
}

/// Credential belongs on `RoutePlan.credential`, never on hop headers.
#[test]
fn hop_headers_do_not_carry_authorization() {
    let hop = plan(&HopInput::default());
    assert!(!hop.headers.contains_key(http::header::AUTHORIZATION));
    assert!(hop.headers.contains_key("originator"));
    assert!(hop.headers.contains_key("user-agent"));
    assert!(!hop.headers.contains_key("x-grok-conv-id"));
}

/// An account id becomes `chatgpt-account-id`; missing stays missing.
#[test]
fn account_header_is_opt_in() {
    let with = plan(&HopInput {
        account_id: Some("acct_123"),
        ..HopInput::default()
    });
    assert_eq!(
        header_str(&with.headers, "chatgpt-account-id"),
        Some("acct_123")
    );
    let without = plan(&HopInput::default());
    assert!(!without.headers.contains_key("chatgpt-account-id"));
}

/// Leading system/developer in `input` must not stay at the front: Codex
/// caches `instructions` then `input`, and a DSH snapshot at index 0 is a
/// prefix miss every turn.
#[test]
fn leading_system_is_lifted_out_of_input() {
    let (_, value) = rewrite(json!({
        "input": [
            { "role": "system", "content": "be brief" },
            { "role": "user", "content": "hi" }
        ]
    }));
    assert_eq!(value["instructions"], json!("be brief"));
    let input = value["input"].as_array().expect("input array");
    assert_eq!(input[0]["role"], json!("user"));
    assert_eq!(value["store"], json!(false));
}

/// Extra leading text that is not the existing `instructions` parks at the
/// suffix so the conversation prefix can still hit.
#[test]
fn extra_leading_developer_parks_at_the_suffix() {
    let (_, value) = rewrite(json!({
        "instructions": "be brief",
        "input": [
            { "role": "developer", "content": "be brief\n\nthis snapshot supersedes" },
            { "role": "user", "content": "hi" }
        ]
    }));
    assert_eq!(value["instructions"], json!("be brief"));
    let input = value["input"].as_array().expect("input array");
    assert_eq!(input[0]["role"], json!("user"));
    assert_eq!(input.last().unwrap()["role"], json!("developer"));
}

/// gpt-5.6 400s on public-API cache options / `session_id` / store=true.
#[test]
fn chatgpt_rejected_fields_leave_the_body() {
    let (_, value) = rewrite(json!({
        "store": true,
        "session_id": "dsh-sess",
        "prompt_cache_retention": "24h",
        "prompt_cache_options": { "max": 1 },
        "safety_identifier": "x",
        "max_output_tokens": 99,
        "service_tier": "fast",
        "instructions": "stay",
        "include": ["reasoning.encrypted_content"],
    }));
    assert!(value.get("session_id").is_none());
    assert!(value.get("prompt_cache_retention").is_none());
    assert!(value.get("prompt_cache_options").is_none());
    assert!(value.get("safety_identifier").is_none());
    assert!(value.get("max_output_tokens").is_none());
    assert_eq!(value["store"], json!(false));
    assert_eq!(value["service_tier"], json!("priority"));
}

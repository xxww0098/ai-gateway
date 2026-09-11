use serde_json::json;

use super::{
    API_URL, parse_api_key, parse_me, refresh, session, start, upstream_headers, default_account,
    is_cloud_chat_url, is_opaque_account, is_permanent_refresh_error,
};
use crate::login::StartInput;
use crate::{Error, FlowKind, StartOutcome};

fn paste_input(key: &str) -> StartInput {
    StartInput {
        method: "paste".to_owned(),
        body: json!({"api_key": key}),
        ..StartInput::default()
    }
}

#[test]
fn registry_public_key_is_not_a_cloud_bearer() {
    let pem = "-----BEGIN PUBLIC KEY-----\nabc\n-----END PUBLIC KEY-----";
    let err = parse_api_key(pem).expect_err("pubkey");
    match err {
        Error::InvalidImport(message) => {
            assert!(message.contains("public key"), "{message}");
            assert!(!message.to_ascii_lowercase().contains("localhost"));
        }
        other => panic!("expected invalid import, got {other:?}"),
    }
    assert!(parse_api_key("short").is_err());
    assert!(parse_api_key("   ").is_err());
}

#[test]
fn pasted_key_is_trimmed_and_never_printed_as_account() {
    let key = "  sk-ollama-cloud-key  ";
    let parsed = parse_api_key(key).expect("key");
    assert_eq!(parsed, parsed.trim());
    assert!(parsed.len() >= 8);
    let stored = session(&parsed, "paste", None).expect("session");
    assert_eq!(stored.access_token, parsed);
    assert_eq!(stored.refresh_token, parsed);
    assert_eq!(stored.source, "paste");
    assert_ne!(stored.account, parsed);
    assert!(stored.account.starts_with("ollama-"));
    assert!(is_opaque_account(&stored.account));
    assert_eq!(stored.account, default_account(&parsed));
}

#[tokio::test]
async fn start_paste_and_env_are_ready_sessions() {
    let paste = start(&paste_input("sk-ollama-paste-key")).await.expect("paste");
    match paste {
        StartOutcome::Ready(session) => {
            assert_eq!(session.source, "paste");
            assert_eq!(session.family, super::ID);
            assert!(session.expires_at_ms > 0);
        }
        other => panic!("expected ready, got {other:?}"),
    }

    let env = start(&StartInput {
        method: "env".to_owned(),
        body: json!({"OLLAMA_API_KEY": "sk-ollama-env-key"}),
        ..StartInput::default()
    })
    .await
    .expect("env");
    match env {
        StartOutcome::Ready(session) => assert_eq!(session.source, "env"),
        other => panic!("expected ready, got {other:?}"),
    }
}

#[tokio::test]
async fn device_and_pkce_are_not_cloud_login() {
    for method in ["device", "pkce", "authcode"] {
        let err = start(&StartInput {
            method: method.to_owned(),
            body: json!({"api_key": "sk-ollama-unused"}),
            ..StartInput::default()
        })
        .await
        .expect_err(method);
        assert!(matches!(err, Error::UnsupportedFlow), "{method}: {err:?}");
    }
}

#[test]
fn refresh_is_a_no_op_and_never_permanent() {
    let stored = session("sk-ollama-refresh-key", "paste", None).expect("session");
    let again = refresh(stored.clone()).expect("refresh");
    assert_eq!(again.access_token, stored.access_token);
    assert_eq!(again.expires_at_ms, stored.expires_at_ms);
    assert!(!is_permanent_refresh_error(&Error::Permanent));
    assert!(refresh(crate::Session::new(crate::Family::Ollama, "", "")).is_err());
}

#[test]
fn hop_is_ollama_cloud_not_local_daemon() {
    assert!(is_cloud_chat_url(API_URL));
    assert!(!API_URL.contains("11434"));
    assert!(!API_URL.contains("127.0.0.1"));
    assert!(!is_cloud_chat_url("http://127.0.0.1:11434/v1/chat/completions"));
    assert!(!is_cloud_chat_url("http://localhost:11434/api/chat"));
    let headers = upstream_headers(&session("sk-ollama-header-key", "paste", None).expect("s"));
    let auth = headers.get(http::header::AUTHORIZATION).unwrap().to_str().unwrap();
    assert!(auth.starts_with("Bearer "));
    assert!(!auth.contains("11434"));
}

#[test]
fn me_prefers_pascal_case_email_and_plan() {
    let identity = parse_me(&json!({
        "Email": "cloud@ollama.local",
        "Name": "Cloud",
        "Plan": "pro",
        "email": "ignored@example"
    }));
    assert_eq!(identity["account"], "cloud@ollama.local");
    assert_eq!(identity["plan_type"], "pro");
    assert!(parse_me(&json!([])).is_empty());
}

#[test]
fn api_key_flow_kind_is_not_browser_oauth() {
    assert_ne!(super::flow_kind(), FlowKind::AuthorizationCode);
    assert_ne!(super::flow_kind(), FlowKind::Device);
}

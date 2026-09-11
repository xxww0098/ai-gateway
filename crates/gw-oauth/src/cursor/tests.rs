use http::HeaderMap;
use serde_json::json;

use super::{
    CLIENT_TYPE, LOGIN_URL, POLL_URL, REFRESH_URL, accept_refresh_body, access_still_valid,
    account_from_token, apply_poll_response, chat_headers, complete_login, display_account,
    is_opaque_account, login_params, parse_token_response, poll_auth, poll_query_url,
    refresh_guard, refresh_tokens, start, usage_headers, wire_model_id,
};
use crate::login::StartInput;
use crate::{Error, FlowKind, StartOutcome};

fn jwt(payload: &str) -> String {
    use base64::Engine as _;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.as_bytes());
    format!("{header}.{body}.x")
}

fn valid_access(email: &str) -> String {
    let exp = chrono::Utc::now().timestamp() + 3600;
    jwt(&format!(r#"{{"email":"{email}","exp":{exp}}}"#))
}

#[test]
fn login_deep_control_is_cli_poll_without_loopback() {
    let started = login_params().expect("entropy");
    assert!(
        started.login_url.starts_with(&format!("{LOGIN_URL}?")),
        "{}",
        started.login_url
    );
    let url = url::Url::parse(&started.login_url).expect("url");
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("challenge"), Some(started.challenge.as_str()));
    assert_eq!(get("uuid"), Some(started.uuid.as_str()));
    assert_eq!(get("mode"), Some("login"));
    assert_eq!(get("redirectTarget"), Some("cli"));
    assert_ne!(started.verifier, started.challenge);
}

#[tokio::test]
async fn start_pkce_returns_browser_cli_poll_and_empty_redirect() {
    let out = start(&StartInput {
        state: "st-1".into(),
        ..StartInput::default()
    })
    .await
    .expect("start");
    match out {
        StartOutcome::Browser {
            authorize_url,
            redirect_uri,
            flow,
            extra,
            verifier,
            ..
        } => {
            assert!(authorize_url.contains("loginDeepControl"));
            assert!(authorize_url.contains("redirectTarget=cli"));
            assert!(redirect_uri.is_empty(), "no loopback callback");
            assert_eq!(flow, FlowKind::CliPoll);
            assert!(!verifier.is_empty());
            assert!(
                extra
                    .get("uuid")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            );
            assert_eq!(
                extra.get("mode").and_then(serde_json::Value::as_str),
                Some("cli")
            );
        }
        other => panic!("expected Browser, got {other:?}"),
    }
}

#[tokio::test]
async fn start_import_parses_token_json() {
    let access = valid_access("imp@x");
    let out = start(&StartInput {
        method: "import".into(),
        body: json!({
            "accessToken": access,
            "refreshToken": "rt-imp",
            "source": "import",
        }),
        ..StartInput::default()
    })
    .await
    .expect("import");
    match out {
        StartOutcome::Ready(session) => {
            assert_eq!(session.refresh_token, "rt-imp");
            assert_eq!(session.access_token, access);
            assert_eq!(session.family, "cursor");
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[tokio::test]
async fn start_rejects_unknown_method() {
    let err = start(&StartInput {
        method: "device".into(),
        ..StartInput::default()
    })
    .await
    .expect_err("device is not a cursor flow");
    assert!(matches!(err, Error::UnsupportedFlow));
}

#[test]
fn apply_poll_404_is_waiting() {
    assert_eq!(apply_poll_response(404, b"").expect("404"), None);
}

#[tokio::test]
async fn poll_waits_on_404_then_stores_tokens() {
    let mut calls = 0u32;
    let tokens = poll_auth(
        "u1",
        "verifier",
        |url| {
            calls += 1;
            assert!(url.starts_with(POLL_URL), "{url}");
            assert!(url.contains("uuid=u1"));
            assert!(url.contains("verifier=verifier"));
            let n = calls;
            async move {
                if n == 1 {
                    Ok((404, Vec::new()))
                } else {
                    Ok((
                        200,
                        serde_json::to_vec(&json!({
                            "accessToken": "at-poll",
                            "refreshToken": "rt-poll",
                        }))
                        .expect("json"),
                    ))
                }
            }
        },
        |_| async {},
        8,
    )
    .await
    .expect("poll");
    assert_eq!(calls, 2);
    assert_eq!(tokens.refresh_token, "rt-poll");
}

#[test]
fn poll_query_url_encodes_both_params() {
    let url = poll_query_url("a b", "x&y");
    assert!(url.starts_with(POLL_URL));
    assert!(url.contains("uuid=a+b") || url.contains("uuid=a%20b"));
    assert!(url.contains("verifier="));
}

#[tokio::test]
async fn refresh_posts_bearer_and_empty_json() {
    refresh_guard::reset();
    let next = valid_access("refresh@x");
    let tokens = refresh_tokens(
        "rt-old",
        |url, headers, body| async move {
            assert_eq!(url, REFRESH_URL);
            assert_eq!(body, b"{}");
            let auth = headers
                .get("authorization")
                .map(|v| v.as_bytes())
                .expect("auth");
            assert_eq!(auth, b"Bearer rt-old");
            Ok((
                200,
                serde_json::to_vec(&json!({
                    "accessToken": next,
                    "refreshToken": "rt-new",
                }))
                .expect("json"),
            ))
        },
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .expect("refresh");
    assert_eq!(tokens.refresh_token, "rt-new");
}

#[test]
fn failed_refresh_marks_the_token_known_bad() {
    refresh_guard::reset();
    let err = accept_refresh_body("rt-bad", 401, b"nope", 1_000).expect_err("401");
    assert!(err.is_permanent());
    assert!(refresh_guard::is_known_bad("rt-bad", 1_000));
    assert!(!refresh_guard::is_known_bad("rt-bad", i64::MAX));
    refresh_guard::mark_succeeded("rt-bad");
    assert!(!refresh_guard::is_known_bad("rt-bad", 1_000));
}

#[tokio::test]
async fn refresh_skips_known_bad_token() {
    refresh_guard::reset();
    refresh_guard::mark_failed("rt-stale", 10);
    let err = refresh_tokens(
        "rt-stale",
        |_url, _headers, _body| async { panic!("must not hit the network") },
        10,
    )
    .await
    .expect_err("known-bad");
    assert!(err.is_permanent());
}

#[test]
fn chat_headers_are_cli_not_sdk_and_not_codex() {
    assert_ne!(CLIENT_TYPE, "sdk");
    let headers = chat_headers("tok", Some("req-cursor-1"), false);
    assert_eq!(
        headers
            .get("x-cursor-client-type")
            .map(http::HeaderValue::as_bytes),
        Some(CLIENT_TYPE.as_bytes())
    );
    assert_ne!(
        headers
            .get("x-cursor-client-type")
            .map(http::HeaderValue::as_bytes),
        Some(b"sdk".as_slice())
    );
    assert_eq!(
        headers.get("x-request-id").map(http::HeaderValue::as_bytes),
        Some(b"req-cursor-1".as_slice())
    );
    assert_eq!(
        headers
            .get("x-original-request-id")
            .map(http::HeaderValue::as_bytes),
        Some(b"req-cursor-1".as_slice())
    );
    assert!(headers.get("x-parent-request-id").is_none());
    assert!(headers.get("x-root-parent-request-id").is_none());
    assert!(headers.get("session-id").is_none());
    assert!(headers.get("x-grok-conv-id").is_none());
    assert_eq!(
        headers.get("content-type").map(http::HeaderValue::as_bytes),
        Some(b"application/connect+proto".as_slice())
    );
    let unary = chat_headers("tok", Some("req-cursor-1"), true);
    assert_eq!(
        unary.get("content-type").map(http::HeaderValue::as_bytes),
        Some(b"application/proto".as_slice())
    );
}

#[test]
fn usage_headers_stay_cli() {
    let headers: HeaderMap = usage_headers("tok");
    assert_eq!(
        headers
            .get("x-cursor-client-type")
            .map(http::HeaderValue::as_bytes),
        Some(b"cli".as_slice())
    );
}

#[test]
fn opaque_jwt_sub_is_not_a_display_account() {
    let opaque = "grok|user_01TESTOPAQUEID0001";
    let token = jwt(&format!(
        r#"{{"sub":"{opaque}","exp":{}}}"#,
        chrono::Utc::now().timestamp() + 3600
    ));
    assert!(account_from_token(&token).is_none());
    let session = super::cursor_session(super::SessionBuild {
        access_token: token,
        refresh_token: Some("rt-opaque".into()),
        source: "pkce".into(),
        account: Some(opaque.into()),
        ..super::SessionBuild::default()
    })
    .expect("session");
    assert!(display_account(&session).is_none());
    assert_eq!(session.account, opaque);
}

#[test]
fn jwt_email_wins_over_sub() {
    let token = jwt(r#"{"email":"named@x","sub":"auth0|abc"}"#);
    assert_eq!(account_from_token(&token).as_deref(), Some("named@x"));
}

#[test]
fn parse_and_complete_login_tag_pkce() {
    let parsed =
        parse_token_response(&json!({"accessToken":"a","refreshToken":"r"}), "t").expect("parse");
    let session = complete_login(parsed, "pkce").expect("complete");
    assert_eq!(session.source, "pkce");
    assert_eq!(session.refresh_token, "r");
}

#[test]
fn wire_model_id_peels_fast() {
    assert_eq!(wire_model_id("gpt-5.5-fast"), "gpt-5.5");
    assert!(!wire_model_id("gpt-5.5-fast").ends_with("-fast"));
    assert_eq!(wire_model_id("  "), "composer-2");
}

#[test]
fn access_token_with_future_exp_is_still_valid() {
    let token = valid_access("cli@cursor.local");
    assert!(access_still_valid(
        &token,
        chrono::Utc::now().timestamp_millis()
    ));
}

#[test]
fn is_opaque_account_rejects_workos_and_literal_cursor() {
    assert!(is_opaque_account("cursor"));
    assert!(is_opaque_account("auth0|abc"));
    assert!(!is_opaque_account("named@x"));
}

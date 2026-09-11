//! Properties of Grok login, discovery, session, refresh, and hop headers.
//!
//! Rule 2.11: fixtures live in the test. Do not echo `CLIENT_ID` / `SCOPE`
//! constants back as expected values.

use serde_json::{Value, json};

use super::device::interpret_poll;
use super::test_support::{self, Reply};
use super::{
    Endpoints, authorize_browser, credential_headers, credits_headers, exchange_at,
    parse_discovery, refresh_at, session_from_tokens, start_with, tier_from_value, token_error,
    upstream_headers, validate_xai_endpoint,
};
use crate::{Error, FlowKind, Session, StartInput, StartOutcome};

fn jwt(payload: &str) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
    let body = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    format!("{header}.{body}.x")
}

fn xai_endpoints() -> Endpoints {
    Endpoints {
        authorization: "https://auth.x.ai/oauth2/auth".to_owned(),
        token: "https://auth.x.ai/oauth2/token".to_owned(),
        device_authorization: "https://auth.x.ai/oauth2/device/code".to_owned(),
    }
}

#[test]
fn discovery_rejects_an_off_issuer_authorization_host() {
    let doc = json!({
        "authorization_endpoint": "https://evil.example/authorize",
        "token_endpoint": "https://auth.x.ai/oauth2/token",
    });
    let err = parse_discovery(&doc).expect_err("off-issuer");
    match err {
        Error::Payload(msg) => assert!(msg.contains("non-x.ai"), "{msg}"),
        other => panic!("expected payload, got {other}"),
    }
}

#[test]
fn discovery_rejects_http_even_on_xai() {
    assert!(validate_xai_endpoint("http://auth.x.ai/oauth2/token").is_err());
    assert!(validate_xai_endpoint("https://auth.x.ai.evil.test/token").is_err());
}

#[test]
fn discovery_accepts_xai_hosts_and_fills_a_missing_device_endpoint_on_issuer() {
    let doc = json!({
        "authorization_endpoint": "https://auth.x.ai/oauth2/auth",
        "token_endpoint": "https://auth.x.ai/oauth2/token",
    });
    let parsed = parse_discovery(&doc).expect("x.ai document");
    assert!(validate_xai_endpoint(&parsed.authorization).is_ok());
    assert!(validate_xai_endpoint(&parsed.token).is_ok());
    assert!(validate_xai_endpoint(&parsed.device_authorization).is_ok());
}

#[test]
fn discovery_does_not_fall_back_when_device_endpoint_is_off_issuer() {
    let doc = json!({
        "authorization_endpoint": "https://auth.x.ai/oauth2/auth",
        "token_endpoint": "https://auth.x.ai/oauth2/token",
        "device_authorization_endpoint": "https://evil.example/device",
    });
    assert!(parse_discovery(&doc).is_err());
}

#[test]
fn unknown_start_method_is_unsupported() {
    let input = StartInput {
        method: "sms".to_owned(),
        ..StartInput::default()
    };
    let err = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt")
        .block_on(super::start(&input))
        .expect_err("sms");
    assert!(matches!(err, Error::UnsupportedFlow));
}

#[test]
fn pkce_authorize_url_is_s256_against_the_caller_redirect() {
    let redirect = "http://127.0.0.1:56121/callback";
    let input = StartInput {
        redirect_uri: redirect.to_owned(),
        state: "st-9".to_owned(),
        method: "pkce".to_owned(),
        body: Value::Null,
    };
    let endpoints = xai_endpoints();
    let StartOutcome::Browser {
        authorize_url,
        state,
        verifier,
        redirect_uri,
        flow,
        extra,
    } = authorize_browser(&input, &endpoints).expect("pkce")
    else {
        panic!("expected browser");
    };
    assert_eq!(flow, FlowKind::AuthorizationCode);
    assert_eq!(state, "st-9");
    assert_eq!(redirect_uri, redirect);
    assert!(authorize_url.starts_with(&endpoints.authorization));
    let url = url::Url::parse(&authorize_url).expect("url");
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("response_type"), Some("code"));
    assert_eq!(get("code_challenge_method"), Some("S256"));
    assert_eq!(get("redirect_uri"), Some(redirect));
    assert_eq!(get("state"), Some("st-9"));
    let client_id = get("client_id").expect("client_id");
    assert!(!client_id.is_empty());
    assert_eq!(extra["client_id"], client_id);
    let challenge = get("code_challenge").expect("challenge");
    assert!(!challenge.is_empty());
    assert_ne!(challenge, verifier);
    assert!(!verifier.is_empty());
    assert_eq!(extra["token_endpoint"], endpoints.token);
    assert!(get("nonce").is_some_and(|n| n.chars().all(|c| c.is_ascii_hexdigit())));
}

#[test]
fn session_reads_account_from_id_token_and_tier_from_access_token() {
    let exp = chrono::Utc::now().timestamp() + 3600;
    let access = jwt(&format!(r#"{{"tier":4,"exp":{exp},"sub":"user-1"}}"#));
    let id = jwt(r#"{"email":"g@x.ai"}"#);
    let session = session_from_tokens(
        &json!({
            "access_token": access,
            "refresh_token": "rt",
            "expires_in": 3600,
            "id_token": id,
        }),
        "https://auth.x.ai/oauth2/token",
        None,
    )
    .expect("session");
    assert_eq!(session.account, "g@x.ai");
    assert_eq!(session.plan_type, "X Premium+");
    assert!(session.expires_at_ms > chrono::Utc::now().timestamp_millis());
    assert_eq!(
        session.extra["token_endpoint"],
        "https://auth.x.ai/oauth2/token"
    );
}

#[test]
fn session_requires_a_refresh_token_and_a_usable_expiry() {
    let access = jwt(r#"{"sub":"u"}"#);
    let missing_refresh = session_from_tokens(
        &json!({"access_token": access, "expires_in": 60}),
        "https://auth.x.ai/oauth2/token",
        None,
    );
    assert!(missing_refresh.is_err());
    let missing_expiry = session_from_tokens(
        &json!({"access_token": access, "refresh_token": "rt"}),
        "https://auth.x.ai/oauth2/token",
        None,
    );
    assert!(missing_expiry.is_err());
}

#[test]
fn numeric_and_named_tiers_round_trip_through_plan_labels() {
    assert_eq!(tier_from_value(&json!(0)).as_deref(), Some("Free"));
    assert_eq!(tier_from_value(&json!(4)).as_deref(), Some("X Premium+"));
    assert_eq!(
        tier_from_value(&json!("5")).as_deref(),
        Some("SuperGrok Heavy")
    );
    assert_eq!(
        crate::format_plan_label("SuperGrokPro", crate::Family::Grok),
        "SuperGrok Heavy"
    );
}

#[test]
fn refresh_invalid_grant_is_permanent_exchange_is_not() {
    match token_error(400, r#"{"error":"invalid_grant"}"#, true) {
        Error::Permanent => {}
        other => panic!("refresh must be permanent, got {other}"),
    }
    match token_error(400, r#"{"error":"invalid_grant"}"#, false) {
        Error::TokenEndpoint { status: 400, .. } => {}
        other => panic!("exchange invalid_grant is not permanent, got {other}"),
    }
}

#[test]
fn hop_headers_share_cli_identity_and_never_copy_codex_session_id() {
    let session = Session::new(crate::Family::Grok, "tok", "rt");
    let cred = credential_headers();
    let up = upstream_headers(&session);
    assert_eq!(cred.get("user-agent"), up.get("user-agent"));
    assert!(up.get("x-xai-token-auth").is_some());
    assert!(up.get("authorization").is_some());
    assert!(up.get("session-id").is_none());
    assert!(up.get("x-client-request-id").is_none());
    let credits = credits_headers(&session);
    assert!(credits.get("session-id").is_none());
    assert_eq!(
        credits.get("content-type").map(|v| v.as_bytes()),
        Some(&b"application/grpc-web+proto"[..])
    );
}

#[test]
fn jwt_sub_becomes_x_userid_on_the_hop() {
    let access = jwt(r#"{"sub":"user-1","exp":9999999999}"#);
    let session = Session::new(crate::Family::Grok, access, "rt");
    let up = upstream_headers(&session);
    assert_eq!(
        up.get("x-userid").map(|v| v.as_bytes()),
        Some(&b"user-1"[..])
    );
}

#[test]
fn pending_poll_does_not_mint_a_session() {
    let step = interpret_poll(400, r#"{"error":"authorization_pending"}"#, 5).expect("pending");
    assert!(matches!(
        step,
        super::device::PollStep::Pending { interval_secs: 5 }
    ));
}

#[test]
fn slow_down_raises_the_interval() {
    let step = interpret_poll(400, r#"{"error":"slow_down"}"#, 5).expect("slow");
    match step {
        super::device::PollStep::SlowDown { interval_secs } => {
            assert!(interval_secs > 5, "{interval_secs}");
        }
        other => panic!("expected slow_down, got {other:?}"),
    }
}

#[test]
fn denied_or_expired_device_codes_fail() {
    assert!(interpret_poll(400, r#"{"error":"access_denied"}"#, 5).is_err());
    assert!(interpret_poll(400, r#"{"error":"expired_token"}"#, 5).is_err());
}

#[tokio::test]
async fn device_start_posts_form_scope_and_returns_user_code() {
    let server = test_support::spawn(1, |req| {
        assert_eq!(req.method, "POST");
        let body = String::from_utf8_lossy(&req.body);
        assert!(body.contains("client_id="), "{body}");
        assert!(body.contains("scope="), "{body}");
        assert!(
            !body.contains("\"client_id\""),
            "must be form not JSON: {body}"
        );
        Reply::json(
            200,
            r#"{"device_code":"dev","user_code":"WDJB-MJHT","verification_uri":"https://auth.x.ai/device","interval":5,"expires_in":30}"#,
        )
    });
    let client = test_support::client();
    let input = StartInput {
        state: "st".to_owned(),
        method: "device".to_owned(),
        ..StartInput::default()
    };
    let outcome = super::device::request_device(
        &client,
        &format!("{}/device", server.base),
        &format!("{}/token", server.base),
        &input.state,
    )
    .await
    .expect("device start");
    match outcome {
        StartOutcome::Device {
            user_code,
            verification_uri,
            extra,
            interval_secs,
            ..
        } => {
            assert_eq!(user_code, "WDJB-MJHT");
            assert!(!verification_uri.is_empty());
            assert!(interval_secs > 0);
            assert_eq!(extra["device_code"], "dev");
            assert!(
                extra
                    .get("token_endpoint")
                    .and_then(Value::as_str)
                    .is_some()
            );
        }
        other => panic!("expected device, got {other:?}"),
    }
}

#[tokio::test]
async fn poll_pending_then_ready_mints_a_session() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hits = std::sync::Arc::new(AtomicUsize::new(0));
    let hits_h = std::sync::Arc::clone(&hits);
    let server = test_support::spawn(2, move |_req| {
        let n = hits_h.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Reply::json(400, r#"{"error":"authorization_pending"}"#)
        } else {
            let exp = chrono::Utc::now().timestamp() + 3600;
            let access = {
                use base64::Engine as _;
                use base64::engine::general_purpose::URL_SAFE_NO_PAD;
                let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
                let body =
                    URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp},"tier":1}}"#).as_bytes());
                format!("{header}.{body}.x")
            };
            Reply::json(
                200,
                format!(r#"{{"access_token":"{access}","refresh_token":"rt","expires_in":3600}}"#),
            )
        }
    });
    let client = test_support::client();
    let token_url = format!("{}/token", server.base);
    let first = super::device::poll_at(&client, &token_url, "dev", 5)
        .await
        .expect("pending");
    assert!(matches!(first, super::PollOutcome::Pending { .. }));
    let second = super::device::poll_at(&client, &token_url, "dev", 5)
        .await
        .expect("ready");
    match second {
        super::PollOutcome::Ready(session) => {
            assert!(!session.access_token.is_empty());
            assert_eq!(session.refresh_token, "rt");
        }
        other => panic!("expected ready, got {other:?}"),
    }
}

#[tokio::test]
async fn refresh_posts_form_and_invalid_grant_is_permanent() {
    let server = test_support::spawn(1, |req| {
        let body = String::from_utf8_lossy(&req.body);
        assert!(body.contains("grant_type=refresh_token"), "{body}");
        assert!(body.contains("refresh_token="), "{body}");
        Reply::json(400, r#"{"error":"invalid_grant"}"#)
    });
    let client = test_support::client();
    let mut session = Session::new(crate::Family::Grok, "old", "rt");
    session.extra.insert(
        "token_endpoint".to_owned(),
        json!(format!("{}/token", server.base)),
    );
    let err = refresh_at(&client, &session).await.expect_err("permanent");
    assert!(err.is_permanent(), "{err}");
}

#[tokio::test]
async fn exchange_403_does_not_look_like_a_retryable_transport_error() {
    let server = test_support::spawn(1, |_req| Reply::json(403, r#"{"error":"access_denied"}"#));
    let client = test_support::client();
    let err = exchange_at(
        &client,
        &format!("{}/token", server.base),
        "code",
        "verifier",
        "http://127.0.0.1/cb",
        "challenge",
    )
    .await
    .expect_err("403");
    match err {
        Error::TokenEndpoint { status: 403, body } => {
            assert!(body.contains("API"), "{body}");
        }
        other => panic!("expected 403 token error, got {other}"),
    }
}

#[tokio::test]
async fn start_pkce_uses_discovered_authorize_host() {
    let server = test_support::spawn(1, |_req| {
        Reply::json(
            200,
            r#"{"authorization_endpoint":"https://auth.x.ai/oauth2/auth","token_endpoint":"https://auth.x.ai/oauth2/token","device_authorization_endpoint":"https://auth.x.ai/oauth2/device/code"}"#,
        )
    });
    let client = test_support::client();
    let input = StartInput {
        redirect_uri: "http://127.0.0.1:9/callback".to_owned(),
        state: "st".to_owned(),
        method: "pkce".to_owned(),
        body: Value::Null,
    };
    let outcome = start_with(
        &client,
        &input,
        &format!("{}/.well-known/openid-configuration", server.base),
    )
    .await
    .expect("pkce start");
    match outcome {
        StartOutcome::Browser {
            authorize_url,
            flow,
            ..
        } => {
            assert_eq!(flow, FlowKind::AuthorizationCode);
            assert!(authorize_url.starts_with("https://auth.x.ai/"));
        }
        other => panic!("expected browser, got {other:?}"),
    }
}

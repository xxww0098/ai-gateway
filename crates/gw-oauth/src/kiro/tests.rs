use serde_json::json;
use url::Url;

use super::net::{Body, Posted, Scripted};
use super::session::{build_session, extra_str};
use super::{
    PORTAL_URL, exchange_social_code_with, refresh_with, social_redirect_uri, start, start_with,
};
use crate::login::StartInput;
use crate::{Error, FlowKind, StartOutcome};

fn long_rt() -> String {
    format!("rt_{}", "x".repeat(120))
}

fn input(method: &str, body: serde_json::Value) -> StartInput {
    StartInput {
        redirect_uri: "http://127.0.0.1:3128/oauth/callback".into(),
        state: "st".into(),
        method: method.into(),
        body,
    }
}

#[tokio::test]
async fn social_authorize_is_origin_only_and_not_the_callback_path() {
    let started = start(&input("authcode", json!({}))).await.expect("start");
    let StartOutcome::Browser {
        authorize_url,
        redirect_uri,
        flow,
        extra,
        ..
    } = started
    else {
        panic!("expected browser flow");
    };
    assert_eq!(flow, FlowKind::AuthorizationCode);
    let url = Url::parse(&authorize_url).expect("url");
    assert_eq!(
        format!("{}{}", url.origin().ascii_serialization(), url.path()),
        format!("{PORTAL_URL}/signin")
    );
    let redirect = url
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .map(|(_, v)| v.into_owned())
        .expect("redirect");
    assert_eq!(
        redirect,
        social_redirect_uri("http://127.0.0.1:3128/oauth/callback").expect("origin")
    );
    assert_eq!(redirect, redirect_uri);
    assert!(!redirect.contains("/oauth/callback"));
    assert!(!redirect.contains("127.0.0.1"));
    assert_ne!(redirect, "http://127.0.0.1:3128/oauth/callback");
    assert_eq!(
        url.query_pairs()
            .find(|(k, _)| k == "code_challenge_method")
            .map(|(_, v)| v.into_owned())
            .as_deref(),
        Some("S256")
    );
    assert_eq!(
        url.query_pairs()
            .find(|(k, _)| k == "redirect_from")
            .map(|(_, v)| v.into_owned())
            .as_deref(),
        Some("KiroIDE")
    );
    assert!(
        extra
            .get("machine_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|m| m.len() == 64)
    );
}

#[tokio::test]
async fn social_token_posts_landed_path_and_login_option() {
    let poster = Scripted::new(vec![Posted::ok(json!({
        "accessToken": "at",
        "refreshToken": long_rt(),
        "expiresIn": 1234,
    }))]);
    let session = exchange_social_code_with(
        "code-1",
        "verifier",
        "http://localhost:3128/oauth/callback",
        Some("/signin/callback"),
        Some("Google"),
        None,
        |u, h, b| poster.post(u, h, b),
    )
    .await
    .expect("exchange");
    assert_eq!(session.access_token, "at");
    let calls = poster.calls.lock().unwrap_or_else(|e| e.into_inner());
    let Body::Json(body) = &calls[0].1 else {
        panic!("json");
    };
    assert_eq!(
        body["redirect_uri"],
        "http://localhost:3128/signin/callback?login_option=google"
    );
    assert!(body.get("code").is_some());
    assert!(body.get("code_verifier").is_some());
    assert_eq!(extra_str(&session, "auth_method"), "social");
}

#[tokio::test]
async fn device_start_registers_then_requests_device_authorization() {
    let poster = Scripted::new(vec![
        Posted::ok(json!({"clientId": "cid", "clientSecret": "sec"})),
        Posted::ok(json!({
            "deviceCode": "dc",
            "userCode": "ABCD-EFGH",
            "verificationUri": "https://view.awsapps.com/start/#/device",
            "interval": 7,
        })),
    ]);
    let outcome = start_with(
        &input(
            "idc",
            json!({"start_url": "https://d-123.awsapps.com/start"}),
        ),
        |u, h, b| poster.post(u, h, b),
    )
    .await
    .expect("device");
    let StartOutcome::Device {
        user_code,
        extra,
        interval_secs,
        ..
    } = outcome
    else {
        panic!("expected device");
    };
    assert_eq!(user_code, "ABCD-EFGH");
    assert_eq!(interval_secs, 7);
    assert_eq!(extra["kind"], "idc");
    assert_eq!(extra["client_id"], "cid");
    let calls = poster.calls.lock().unwrap_or_else(|e| e.into_inner());
    assert!(calls[0].0.ends_with("/client/register"));
    assert!(calls[1].0.ends_with("/device_authorization"));
    let Body::Json(register) = &calls[0].1 else {
        panic!("json")
    };
    assert_eq!(register["clientType"], "public");
    assert_eq!(register["issuerUrl"], "https://d-123.awsapps.com/start");
}

#[tokio::test]
async fn import_start_requires_access_and_accepts_ksk() {
    let missing = start(&StartInput {
        method: "import".into(),
        body: json!({"token": {"refresh_token": "rt"}}),
        ..StartInput::default()
    })
    .await
    .expect_err("missing");
    assert!(matches!(missing, Error::InvalidImport(_)));
    let ready = start(&StartInput {
        method: "import".into(),
        body: json!({"token": "ksk_live_example1"}),
        ..StartInput::default()
    })
    .await
    .expect("key");
    let StartOutcome::Ready(session) = ready else {
        panic!("ready");
    };
    assert_eq!(extra_str(&session, "auth_method"), "api_key");
    assert!(session.access_token.starts_with("ksk_"));
}

#[tokio::test]
async fn refresh_rewrites_stale_expires_at_from_expires_in() {
    let stale = 1_756_634_673_000i64;
    let now = 1_800_000_000_000i64;
    let mut session = build_session(
        Some("old".into()),
        Some(long_rt()),
        "social",
        &json!({"expiresAt": stale, "authMethod": "social"}),
        stale,
    )
    .expect("session");
    session.expires_at_ms = stale;
    let poster = Scripted::new(vec![Posted::ok(json!({
        "accessToken": "next",
        "refreshToken": long_rt(),
        "expiresIn": 1234,
    }))]);
    let next = refresh_with(&session, now, |u, h, b| poster.post(u, h, b))
        .await
        .expect("refresh");
    assert_eq!(next.access_token, "next");
    assert_ne!(next.expires_at_ms, stale);
    assert_eq!(next.expires_at_ms, now + 1_234_000);
    let calls = poster.calls.lock().unwrap_or_else(|e| e.into_inner());
    assert!(calls[0].0.contains("auth.desktop.kiro.dev/refreshToken"));
}

#[tokio::test]
async fn refresh_routes_idc_and_entra_to_matching_endpoints() {
    let rt = long_rt();
    let idc = build_session(
        Some("old".into()),
        Some(rt.clone()),
        "idc",
        &json!({"clientId": "cid", "clientSecret": "sec", "authMethod": "idc"}),
        1,
    )
    .expect("idc");
    let poster = Scripted::new(vec![Posted::ok(
        json!({"accessToken": "next", "expiresIn": 50}),
    )]);
    let _ = refresh_with(&idc, 10, |u, h, b| poster.post(u, h, b))
        .await
        .expect("idc refresh");
    {
        let calls = poster.calls.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(calls[0].0, "https://oidc.us-east-1.amazonaws.com/token");
        let Body::Json(body) = &calls[0].1 else {
            panic!("json")
        };
        assert_eq!(body["grantType"], "refresh_token");
    }

    let entra = build_session(
        Some("old".into()),
        Some(rt),
        "external_idp",
        &json!({
            "clientId": "cid",
            "tokenEndpoint": "https://login.microsoftonline.com/t/oauth2/v2.0/token",
            "authMethod": "external_idp",
        }),
        1,
    )
    .expect("entra");
    let poster = Scripted::new(vec![Posted::ok(
        json!({"access_token": "next", "expires_in": 50}),
    )]);
    let _ = refresh_with(&entra, 10, |u, h, b| poster.post(u, h, b))
        .await
        .expect("entra refresh");
    let calls = poster.calls.lock().unwrap_or_else(|e| e.into_inner());
    assert!(calls[0].0.contains("microsoftonline"));
    assert!(matches!(calls[0].1, Body::Form(_)));
}

#[tokio::test]
async fn unknown_start_method_is_unsupported() {
    let err = start(&input("not-a-method", json!({})))
        .await
        .expect_err("unsupported");
    assert!(matches!(err, Error::UnsupportedFlow));
}

//! Unit tests for the device-code state machine and import parser.
//!
//! Network calls are not exercised here. Token-poll transitions go through
//! [`super::interpret_token_http`] with canned status + body pairs.

use super::*;
use serde_json::json;

fn interval() -> i64 {
    5
}

#[test]
fn a_pending_token_body_keeps_waiting() {
    let outcome = interpret_token_http(400, r#"{"error":"authorization_pending"}"#, interval());
    match outcome {
        DevicePollOutcome::Pending { interval } => assert_eq!(interval, 5),
        other => panic!("expected pending, got {other:?}"),
    }
}

#[test]
fn slow_down_raises_the_interval_by_five_seconds() {
    let outcome = interpret_token_http(400, r#"{"error":"slow_down"}"#, 5);
    match outcome {
        DevicePollOutcome::SlowDown { interval } => assert_eq!(interval, 10),
        other => panic!("expected slow_down, got {other:?}"),
    }
}

#[test]
fn an_expired_device_code_fails_the_session() {
    let outcome = interpret_token_http(
        400,
        r#"{"error":"expired_token","error_description":"expired"}"#,
        5,
    );
    match outcome {
        DevicePollOutcome::Failed { error, .. } => assert_eq!(error, "expired_token"),
        other => panic!("expected failed, got {other:?}"),
    }
}

#[test]
fn access_denied_fails_the_session() {
    let outcome = interpret_token_http(400, r#"{"error":"access_denied"}"#, 5);
    match outcome {
        DevicePollOutcome::Failed { error, .. } => assert_eq!(error, "access_denied"),
        other => panic!("expected failed, got {other:?}"),
    }
}

#[test]
fn a_successful_token_body_completes() {
    let outcome = interpret_token_http(
        200,
        r#"{"access_token":"at-1","refresh_token":"rt-1","expires_in":3600}"#,
        5,
    );
    match outcome {
        DevicePollOutcome::Completed(tokens) => {
            assert_eq!(tokens.access_token, "at-1");
            assert_eq!(tokens.refresh_token, "rt-1");
            assert_eq!(tokens.expires_in, 3600);
        }
        other => panic!("expected completed, got {other:?}"),
    }
}

#[test]
fn a_camel_case_token_body_completes() {
    let outcome = interpret_token_http(
        200,
        r#"{"accessToken":"at-2","refreshToken":"rt-2","expiresIn":1800}"#,
        5,
    );
    match outcome {
        DevicePollOutcome::Completed(tokens) => {
            assert_eq!(tokens.access_token, "at-2");
            assert_eq!(tokens.refresh_token, "rt-2");
            assert_eq!(tokens.expires_in, 1800);
        }
        other => panic!("expected completed, got {other:?}"),
    }
}

#[test]
fn a_200_without_an_access_token_is_a_failure() {
    let outcome = interpret_token_http(200, r#"{"token_type":"Bearer"}"#, 5);
    assert!(matches!(outcome, DevicePollOutcome::Failed { .. }));
}

#[test]
fn an_unparseable_body_is_a_failure() {
    let outcome = interpret_token_http(200, "not-json", 5);
    assert!(matches!(outcome, DevicePollOutcome::Failed { .. }));
}

// ---------------------------------------------------------------- xAI host

#[test]
fn xai_endpoints_must_be_https_on_xai() {
    assert!(validate_xai_endpoint("https://auth.x.ai/oauth/token"));
    assert!(validate_xai_endpoint("https://api.x.ai/v1"));
}

#[test]
fn a_non_xai_host_is_rejected() {
    for bad in [
        "http://auth.x.ai/oauth/token",
        "https://evil.example/token",
        "https://x.ai.evil.example/token",
        "https://notx.ai/token",
        "not-a-url",
        "",
    ] {
        assert!(!validate_xai_endpoint(bad), "{bad}");
    }
}

// ---------------------------------------------------------------- ttl / interval

#[test]
fn a_missing_expires_in_uses_the_pkce_ttl() {
    assert_eq!(device_session_ttl(0), SESSION_TTL);
}

#[test]
fn a_short_device_lifetime_is_raised_to_the_pkce_ttl() {
    assert_eq!(device_session_ttl(30), SESSION_TTL);
}

#[test]
fn a_long_device_lifetime_is_capped_at_thirty_minutes() {
    assert_eq!(device_session_ttl(86_400), Duration::minutes(30));
}

#[test]
fn an_empty_last_poll_is_always_due() {
    let config = SessionConfig::default();
    assert!(interval_elapsed(&config, Utc::now()));
}

#[test]
fn a_recent_poll_is_not_due_until_the_interval() {
    let mut config = SessionConfig {
        interval: 30,
        ..SessionConfig::default()
    };
    let now = Utc::now();
    mark_polled(&mut config, now, 30);
    assert!(!interval_elapsed(&config, now + Duration::seconds(5)));
    assert!(interval_elapsed(&config, now + Duration::seconds(31)));
}

// ---------------------------------------------------------------- import

#[test]
fn a_snake_case_kiro_cache_imports() {
    let tokens = parse_kiro_import(&json!({
        "access_token": "at-import",
        "refresh_token": "rt-import",
        "client_id": "cid",
        "client_secret": "csec",
        "region": "us-west-2",
        "start_url": "https://view.awsapps.com/start",
        "auth_method": "builder-id",
    }))
    .expect("imports");
    assert_eq!(tokens.access_token, "at-import");
    assert_eq!(tokens.refresh_token, "rt-import");
    assert_eq!(tokens.extra["client_id"], json!("cid"));
    assert_eq!(tokens.extra["region"], json!("us-west-2"));
}

#[test]
fn a_camel_case_kiro_cache_imports() {
    let tokens = parse_kiro_import(&json!({
        "accessToken": "at-2",
        "refreshToken": "rt-2",
        "clientId": "cid-2",
        "clientSecret": "csec-2",
        "startUrl": "https://d-example.awsapps.com/start",
        "authMethod": "idc",
    }))
    .expect("imports");
    assert_eq!(tokens.access_token, "at-2");
    assert_eq!(tokens.extra["client_id"], json!("cid-2"));
    assert_eq!(tokens.extra["auth_method"], json!("idc"));
}

#[test]
fn an_import_without_an_access_token_is_rejected() {
    assert!(parse_kiro_import(&json!({"refresh_token": "rt"})).is_err());
}

#[test]
fn a_non_object_import_is_rejected() {
    assert!(parse_kiro_import(&json!(["not", "an", "object"])).is_err());
}

#[test]
fn kiro_start_body_defaults_to_device() {
    let body = KiroStartBody::from_value(None);
    assert_eq!(body.method_key(), "device");
    let body = KiroStartBody::from_value(Some(&json!({"method": "AUTHCODE"})));
    assert_eq!(body.method_key(), "authcode");
    let body = KiroStartBody::from_value(Some(&json!({"method": "idc"})));
    assert_eq!(body.method_key(), "idc");
}

#[test]
fn a_device_session_is_recognised_without_a_redirect() {
    let config = SessionConfig {
        flow: "device".to_owned(),
        device_code: "dc".to_owned(),
        ..SessionConfig::default()
    };
    assert!(is_device_flow(&config));
    assert!(!is_device_flow(&SessionConfig::default()));
}

#[test]
fn every_kiro_method_alias_routes_somewhere() {
    // 四个 method 各自走不同的后端流程，认错一个就会静默启动另一种登录。
    for (raw, expected) in [
        ("", "device"),
        ("builder-id", "device"),
        ("builder_id", "device"),
        ("  Device  ", "device"),
        ("auth-code", "authcode"),
        ("authorization_code", "authcode"),
        ("authorization-code", "authcode"),
        ("IAM", "idc"),
        ("sso", "idc"),
        ("Import", "import"),
        ("没听过的方式", "device"),
    ] {
        let body = KiroStartBody::from_value(Some(&json!({ "method": raw })));
        assert_eq!(body.method_key(), expected, "method={raw:?}");
    }
}

#[test]
fn idc_and_import_sessions_need_no_redirect_either() {
    for flow in ["idc", "import"] {
        let config = SessionConfig {
            flow: flow.to_owned(),
            ..SessionConfig::default()
        };
        assert!(is_device_flow(&config), "flow={flow}");
    }
    // authorization_code 是带 redirect 的，不能被当成设备码会话。
    let config = SessionConfig {
        flow: "authorization_code".to_owned(),
        ..SessionConfig::default()
    };
    assert!(!is_device_flow(&config));
}

#[test]
fn the_device_start_payload_opens_the_prefilled_page() {
    // 控制台直接 window.open 这个 URL：带 user_code 的那个能省掉操作者手输。
    let started = DeviceCodeResponse {
        device_code: "dc-1".to_owned(),
        user_code: "WDJB-MJHT".to_owned(),
        verification_uri: "https://auth.x.ai/device".to_owned(),
        verification_uri_complete: "https://auth.x.ai/device?user_code=WDJB-MJHT".to_owned(),
        expires_in: 900,
        interval: 0,
    };
    let payload = device_start_payload("st-1", &started);
    assert_eq!(
        payload["auth_url"],
        json!(started.verification_uri_complete)
    );
    assert_eq!(payload["url"], json!(started.verification_uri_complete));
    assert_eq!(payload["state"], json!("st-1"));
    assert_eq!(payload["user_code"], json!("WDJB-MJHT"));
    assert_eq!(payload["flow"], json!("device"));
    // interval 为 0 时必须补默认值，否则前端会拿到 0 并疯狂轮询。
    assert_eq!(payload["interval"], json!(5));
}

#[test]
fn a_start_without_a_complete_uri_falls_back_to_the_plain_one() {
    let started = DeviceCodeResponse {
        device_code: "dc-1".to_owned(),
        user_code: "ABCD-EFGH".to_owned(),
        verification_uri: "https://oidc.us-east-1.amazonaws.com/device".to_owned(),
        verification_uri_complete: String::new(),
        expires_in: 600,
        interval: 5,
    };
    let payload = device_start_payload("st-2", &started);
    assert_eq!(payload["auth_url"], json!(started.verification_uri));
}

#[test]
fn a_poll_records_the_interval_the_provider_asked_for() {
    let mut config = SessionConfig {
        interval: 5,
        ..SessionConfig::default()
    };
    let now = Utc::now();
    mark_polled(&mut config, now, 10);
    assert_eq!(config.interval, 10);
    assert!(!interval_elapsed(&config, now));
    assert!(interval_elapsed(&config, now + Duration::seconds(10)));

    // interval <= 0 不能把已经知道的节奏清成 0。
    mark_polled(&mut config, now, 0);
    assert_eq!(config.interval, 10);
}

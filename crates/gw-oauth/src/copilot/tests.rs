use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;

use super::{
    CLIENT_ID, DEVICE_URL, EDITOR_VERSION, EXCHANGE_URL, INTEGRATION_ID, PLUGIN_VERSION, QUOTA_URL,
    SCOPE, USER_AGENT, Endpoints, complete_device_at, device_authorization_json, exchange_at,
    hop_headers, identity_headers, is_github_user_token, is_session_token, parse_exchange_payload,
    parse_user, quota_authorization, start, start_at,
};
use crate::login::StartInput;
use crate::{Error, Session, StartOutcome};

struct Captured {
    method: String,
    content_type: String,
    authorization: String,
    body: String,
}

fn spawn_json(status: u16, response_body: &str) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    let payload = response_body.to_owned();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else { return };
        let captured = read_request(&mut stream).unwrap_or(Captured {
            method: String::new(),
            content_type: String::new(),
            authorization: String::new(),
            body: String::new(),
        });
        let _ = tx.send(captured);
        let resp = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        );
        let _ = stream.write_all(resp.as_bytes());
    });
    (format!("http://{addr}/"), rx)
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Captured> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut data = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                data.extend_from_slice(&buf[..n]);
                if let Some(captured) = parse_http(&data) {
                    return Some(captured);
                }
            }
            Err(_) => break,
        }
        if data.len() > 32_768 {
            break;
        }
    }
    parse_http(&data)
}

fn parse_http(data: &[u8]) -> Option<Captured> {
    let text = std::str::from_utf8(data).ok()?;
    let (head, rest) = text.split_once("\r\n\r\n")?;
    let mut lines = head.lines();
    let method = lines.next()?.split_whitespace().next()?.to_owned();
    let mut content_length = 0usize;
    let mut content_type = String::new();
    let mut authorization = String::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.trim().parse().unwrap_or(0);
        }
        if name.eq_ignore_ascii_case("content-type") {
            content_type = value.trim().to_owned();
        }
        if name.eq_ignore_ascii_case("authorization") {
            authorization = value.trim().to_owned();
        }
    }
    if rest.len() < content_length {
        return None;
    }
    Some(Captured {
        method,
        content_type,
        authorization,
        body: rest[..content_length].to_owned(),
    })
}

#[test]
fn device_json_is_github_app_not_opencode() {
    let body = device_authorization_json();
    assert_eq!(body["scope"], SCOPE);
    assert_eq!(body["client_id"], CLIENT_ID);
    assert_eq!(body.as_object().map(serde_json::Map::len), Some(2));
    assert!(CLIENT_ID.starts_with("Iv1."));
    assert!(!CLIENT_ID.starts_with("Ov23"));
    assert!(!DEVICE_URL.contains("ghe.com"));
    assert!(DEVICE_URL.starts_with("https://github.com/"));
}

#[tokio::test]
async fn device_start_posts_json_client_id_and_scope() {
    let (url, rx) = spawn_json(
        200,
        r#"{"device_code":"dev","user_code":"WDJB-MJHT","verification_uri":"https://github.com/login/device","interval":5,"expires_in":900}"#,
    );
    let outcome = start_at(
        &StartInput {
            state: "st".to_owned(),
            method: "device".to_owned(),
            ..StartInput::default()
        },
        &Endpoints {
            device_url: url,
            token_url: super::TOKEN_URL.to_owned(),
            exchange_url: EXCHANGE_URL.to_owned(),
        },
    )
    .await
    .expect("device");
    let captured = rx.recv_timeout(Duration::from_secs(3)).expect("captured");
    assert!(captured.content_type.contains("application/json"));
    let posted: serde_json::Value = serde_json::from_str(&captured.body).expect("json");
    assert_eq!(posted["client_id"], CLIENT_ID);
    assert_eq!(posted["scope"], SCOPE);
    assert!(posted.get("code_challenge").is_none());
    match outcome {
        StartOutcome::Device {
            user_code, extra, ..
        } => {
            assert_eq!(user_code, "WDJB-MJHT");
            assert_eq!(extra["json_body"], true);
            assert!(!extra["client_id"].as_str().unwrap_or("").starts_with("Ov23"));
        }
        other => panic!("expected device, got {other:?}"),
    }
}

#[tokio::test]
async fn paste_exchanges_ghu_for_tid_before_ready() {
    let (url, rx) = spawn_json(
        200,
        r#"{"token":"tid=session-token","expires_at":2000000000,"endpoints":{"api":"https://api.githubcopilot.com"}}"#,
    );
    let outcome = start_at(
        &StartInput {
            method: "paste".to_owned(),
            body: json!({"github_token": "ghu_paste_token"}),
            ..StartInput::default()
        },
        &Endpoints {
            device_url: super::DEVICE_URL.to_owned(),
            token_url: super::TOKEN_URL.to_owned(),
            exchange_url: url,
        },
    )
    .await
    .expect("paste");
    let captured = rx.recv_timeout(Duration::from_secs(3)).expect("captured");
    assert_eq!(captured.method, "GET");
    assert!(captured.authorization.starts_with("token ghu_"));
    assert!(!captured.authorization.contains("tid="));
    match outcome {
        StartOutcome::Ready(session) => {
            assert!(is_session_token(&session.access_token));
            assert!(!is_github_user_token(&session.access_token));
            let github = session.extra["githubToken"].as_str().unwrap();
            assert!(is_github_user_token(github));
            let quota = quota_authorization(&session).expect("quota");
            assert!(quota.starts_with("token "));
            assert!(quota.contains(github));
            assert!(!quota.contains("tid="));
            let hop = hop_headers(&session, Some("sess-1"), false, "user");
            let auth = hop.get(http::header::AUTHORIZATION).unwrap().to_str().unwrap();
            assert!(auth.starts_with("Bearer tid="));
            assert_ne!(auth, quota);
        }
        other => panic!("expected ready, got {other:?}"),
    }
}

#[tokio::test]
async fn gho_404_falls_back_to_raw_bearer() {
    let (url, rx) = spawn_json(404, r#"{"message":"Not Found"}"#);
    let exchanged = exchange_at("gho_opencode_token", &url).await.expect("fallback");
    let captured = rx.recv_timeout(Duration::from_secs(3)).expect("captured");
    assert!(captured.authorization.starts_with("token gho_"));
    assert_eq!(exchanged.token, "gho_opencode_token");
}

#[tokio::test]
async fn ghu_404_does_not_pretend_to_be_a_session() {
    let (url, _) = spawn_json(404, r#"{"message":"Not Found"}"#);
    let err = exchange_at("ghu_missing", &url).await.expect_err("ghu 404");
    assert!(matches!(err, Error::TokenEndpoint { status: 404, .. }));
}

#[tokio::test]
async fn complete_device_stores_github_token_separately() {
    let (url, _) = spawn_json(
        200,
        r#"{"token":"tid=from-device","expires_at":2000000000}"#,
    );
    let session = complete_device_at(
        &json!({"access_token": "ghu_device", "refresh_token": "ghu_refresh"}),
        &url,
    )
    .await
    .expect("complete");
    assert_eq!(session.access_token, "tid=from-device");
    assert_eq!(session.extra["githubToken"], "ghu_device");
    assert_eq!(session.source, "oauth");
}

#[tokio::test]
async fn pkce_is_rejected() {
    let err = start(&StartInput {
        method: "pkce".to_owned(),
        ..StartInput::default()
    })
    .await
    .expect_err("pkce");
    assert!(matches!(err, Error::UnsupportedFlow));
}

#[test]
fn identity_headers_are_vscode_chat() {
    let headers = identity_headers();
    assert_eq!(headers.get("user-agent").unwrap(), USER_AGENT);
    assert_eq!(headers.get("editor-version").unwrap(), EDITOR_VERSION);
    assert_eq!(headers.get("editor-plugin-version").unwrap(), PLUGIN_VERSION);
    assert_eq!(headers.get("copilot-integration-id").unwrap(), INTEGRATION_ID);
    assert!(USER_AGENT.starts_with("GitHubCopilotChat/"));
    assert!(EDITOR_VERSION.starts_with("vscode/"));
    assert!(!INTEGRATION_ID.is_empty());
    assert!(!INTEGRATION_ID.contains("opencode"));
}

#[test]
fn quota_url_is_github_not_copilot_com() {
    assert!(QUOTA_URL.starts_with("https://api.github.com/"));
    assert!(!QUOTA_URL.contains("githubcopilot.com"));
    let mut session = Session::new(crate::Family::Copilot, "tid=hop", "ghu_quota");
    session
        .extra
        .insert("githubToken".to_owned(), json!("ghu_quota"));
    let quota = quota_authorization(&session).expect("quota");
    assert!(quota.starts_with("token ghu_"));
    assert!(!quota.contains("tid="));
}

#[test]
fn exchange_payload_reads_seconds_or_ms() {
    let parsed = parse_exchange_payload(&json!({
        "token": "tid=x",
        "expires_at": 1_700_000_000,
        "endpoints": {"api": "https://api.githubcopilot.com/"}
    }))
    .expect("parsed");
    assert_eq!(parsed.token, "tid=x");
    assert_eq!(parsed.expires_at_ms, 1_700_000_000_000);
    assert!(!parsed.api_endpoint.ends_with('/'));
}

#[test]
fn user_login_is_not_the_token() {
    assert_eq!(parse_user(&json!({"login": "octocat", "name": "X"})).as_deref(), Some("octocat"));
}

#[test]
fn hop_headers_do_not_copy_codex_or_grok_names() {
    let mut session = Session::new(crate::Family::Copilot, "tid=hop", "ghu_x");
    session
        .extra
        .insert("githubToken".to_owned(), json!("ghu_x"));
    let headers = hop_headers(&session, Some("sess-1"), true, "agent");
    assert!(headers.get("session-id").is_none());
    assert!(headers.get("x-grok-conv-id").is_none());
    assert_eq!(headers.get("x-interaction-id").unwrap(), "sess-1");
    assert_eq!(headers.get("x-initiator").unwrap(), "agent");
    assert_eq!(headers.get("copilot-vision-request").unwrap(), "true");
    assert!(headers.get("x-interaction-type").is_none());
}

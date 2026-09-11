use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;

use super::{
    CLIENT_ID, DEVICE_GRANT, DEVICE_URL, TOKEN_URL, complete_device, credential_headers,
    device_authorization_body, parse_api_key, parse_user_info, refresh, session_from_cli_file, start,
    start_at, upstream_headers,
};
use crate::login::StartInput;
use crate::{Error, StartOutcome};

struct Captured {
    content_type: String,
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
            content_type: String::new(),
            body: String::new(),
        });
        let _ = tx.send(captured);
        let resp = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        );
        let _ = stream.write_all(resp.as_bytes());
    });
    (format!("http://{addr}/device"), rx)
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
    let mut content_length = 0usize;
    let mut content_type = String::new();
    for line in head.lines().skip(1) {
        let (name, value) = line.split_once(':')?;
        let name = name.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.trim().parse().ok()?;
        }
        if name.eq_ignore_ascii_case("content-type") {
            content_type = value.trim().to_owned();
        }
    }
    if rest.len() < content_length {
        return None;
    }
    Some(Captured {
        content_type,
        body: rest[..content_length].to_owned(),
    })
}

#[test]
fn device_body_is_client_id_only() {
    let body = device_authorization_body();
    assert!(body.starts_with("client_id=") || body.contains("client_id="));
    assert!(!body.contains("scope"));
    assert!(!body.contains("code_challenge"));
    assert!(!body.contains("redirect_uri"));
    assert_eq!(body.matches('=').count(), 1);
    assert_eq!(
        body,
        crate::form::encode(&[("client_id", CLIENT_ID.to_owned())])
    );
    assert!(DEVICE_GRANT.contains("device_code"));
    assert!(DEVICE_URL.contains("device_authorization"));
    assert!(TOKEN_URL.ends_with("/token") || TOKEN_URL.contains("/token"));
}

#[tokio::test]
async fn device_start_posts_form_without_scope() {
    let (url, rx) = spawn_json(
        200,
        r#"{"device_code":"dev","user_code":"KIMI-CODE","verification_uri":"https://auth.kimi.com/device","interval":5,"expires_in":900}"#,
    );
    let outcome = start_at(
        &StartInput {
            state: "st".to_owned(),
            method: "device".to_owned(),
            ..StartInput::default()
        },
        &url,
    )
    .await
    .expect("device");
    let captured = rx.recv_timeout(Duration::from_secs(3)).expect("captured");
    assert!(captured.content_type.contains("application/x-www-form-urlencoded"));
    assert!(!captured.body.contains("scope"));
    assert!(captured.body.contains("client_id="));
    match outcome {
        StartOutcome::Device {
            user_code,
            extra,
            interval_secs,
            ..
        } => {
            assert_eq!(user_code, "KIMI-CODE");
            assert_eq!(interval_secs, 5);
            assert_eq!(extra["json_body"], false);
            assert_eq!(extra["restart_on_expired"], true);
            assert_eq!(extra["client_id"], CLIENT_ID);
        }
        other => panic!("expected device, got {other:?}"),
    }
}

#[tokio::test]
async fn import_kimi_code_json_is_ready() {
    let outcome = start(&StartInput {
        method: "import".to_owned(),
        body: json!({
            "access_token": "cli-access-token",
            "refresh_token": "cli-refresh-token",
            "expires_at": 1_700_000_000
        }),
        ..StartInput::default()
    })
    .await
    .expect("import");
    match outcome {
        StartOutcome::Ready(session) => {
            assert_eq!(session.source, "cli");
            assert_eq!(session.access_token, "cli-access-token");
            assert_eq!(session.refresh_token, "cli-refresh-token");
            assert_eq!(session.expires_at_ms, 1_700_000_000_000);
            assert_ne!(session.account, session.access_token);
        }
        other => panic!("expected ready, got {other:?}"),
    }
}

#[tokio::test]
async fn pasted_api_key_does_not_refresh() {
    let outcome = start(&StartInput {
        method: "paste".to_owned(),
        body: json!({"api_key": "sk-kimi-paste-key"}),
        ..StartInput::default()
    })
    .await
    .expect("paste");
    match outcome {
        StartOutcome::Ready(session) => {
            assert_eq!(session.source, "paste");
            assert_eq!(session.access_token, session.refresh_token);
            let again = refresh(session.clone()).await.expect("refresh");
            assert_eq!(again.access_token, session.access_token);
        }
        other => panic!("expected ready, got {other:?}"),
    }
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
fn cli_file_requires_both_tokens() {
    assert!(session_from_cli_file(&json!({"access_token": "only"})).is_err());
    let parsed = session_from_cli_file(&json!({
        "access_token": "a-token-xx",
        "refresh_token": "r-token-xx",
        "expires_in": 60
    }))
    .expect("cli");
    assert!(parsed.expires_at_ms > 0);
    assert!(parse_api_key("short").is_err());
}

#[test]
fn complete_device_maps_tokens() {
    let session = complete_device(&json!({
        "access_token": "tok",
        "refresh_token": "ref",
        "expires_in": 3600
    }))
    .expect("complete");
    assert_eq!(session.source, "oauth");
    assert_eq!(session.access_token, "tok");
}

#[test]
fn identity_headers_are_not_pi() {
    let headers = credential_headers();
    let ua = headers.get("user-agent").unwrap().to_str().unwrap();
    assert!(!ua.to_ascii_lowercase().contains("pi-provider"));
    assert!(headers.get("x-msh-platform").is_some());
    let hop = upstream_headers(&complete_device(&json!({
        "access_token": "tok",
        "refresh_token": "ref",
        "expires_in": 1
    }))
    .expect("s"));
    assert!(hop.get(http::header::AUTHORIZATION).unwrap().to_str().unwrap().starts_with("Bearer "));
}

#[test]
fn me_uses_email_not_the_token() {
    let info = parse_user_info(&json!({
        "email": "user@kimi.com",
        "user_level_name": "Pro"
    }));
    assert_eq!(info["account"], "user@kimi.com");
    assert_eq!(info["plan_type"], "Pro");
}

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

use crate::codex::start;
use crate::login::StartInput;
use crate::{Error, Family, FlowKind, Session, StartOutcome};

use super::{
    TokenSet, credential_headers, exchange_code_at, refresh_at, routing_hint, session_from_tokens,
    token_set_from_value, upstream_headers,
};

fn unsigned_jwt(payload: &Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    format!("{header}.{body}.x")
}

fn id_token(account: &str, email: &str, plan: &str) -> String {
    unsigned_jwt(&json!({
        "email": email,
        "https://api.openai.com/auth": {
            "chatgpt_account_id": account,
            "chatgpt_plan_type": plan,
        },
    }))
}

struct Capture {
    content_type: String,
    originator: String,
    user_agent: String,
    body: String,
}

fn spawn_token_server(status: u16, response_body: &str) -> (String, mpsc::Receiver<Capture>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let body = response_body.to_owned();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let captured = read_http(&mut stream);
        let _ = tx.send(captured);
        let resp = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
    });
    (format!("http://{addr}/token"), rx)
}

fn read_http(stream: &mut std::net::TcpStream) -> Capture {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let header_end = loop {
        match stream.read(&mut tmp) {
            Ok(0) => break None,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(end) = find_header_end(&buf) {
                    break Some(end);
                }
            }
            Err(_) => break None,
        }
    };
    let Some(header_end) = header_end else {
        return Capture {
            content_type: String::new(),
            originator: String::new(),
            user_agent: String::new(),
            body: String::new(),
        };
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let needed = header_end + content_length(&headers);
    while buf.len() < needed {
        match stream.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let body = String::from_utf8_lossy(buf.get(header_end..).unwrap_or(&[])).into_owned();
    Capture {
        content_type: header_value(&headers, "content-type"),
        originator: header_value(&headers, "originator"),
        user_agent: header_value(&headers, "user-agent"),
        body,
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn content_length(headers: &str) -> usize {
    header_value(headers, "content-length").parse().unwrap_or(0)
}

fn header_value(headers: &str, name: &str) -> String {
    for line in headers.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.eq_ignore_ascii_case(name) {
            return value.trim().to_owned();
        }
    }
    String::new()
}

#[tokio::test]
async fn start_builds_https_pkce_authorize_url() {
    let input = StartInput {
        redirect_uri: "http://localhost:1455/auth/callback".to_owned(),
        state: "st".to_owned(),
        ..StartInput::default()
    };
    let StartOutcome::Browser {
        authorize_url,
        verifier,
        flow,
        state,
        redirect_uri,
        ..
    } = start(&input).await.expect("start")
    else {
        panic!("expected browser flow");
    };
    let url = url::Url::parse(&authorize_url).expect("url");
    assert_eq!(url.scheme(), "https");
    assert!(url.path().contains("authorize"));
    assert!(
        url.host_str()
            .is_some_and(|host| host.ends_with("openai.com"))
    );
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["response_type"], "code");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["prompt"], "login");
    assert_eq!(query["id_token_add_organizations"], "true");
    assert_eq!(query["codex_cli_simplified_flow"], "true");
    assert!(
        query
            .get("client_id")
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        query
            .get("originator")
            .is_some_and(|value| !value.is_empty())
    );
    assert!(query.get("scope").is_some_and(|value| !value.is_empty()));
    assert!(!verifier.is_empty());
    assert_eq!(query["code_challenge"].len(), 43);
    assert_eq!(flow, FlowKind::AuthorizationCode);
    assert_eq!(state, input.state);
    assert_eq!(redirect_uri, input.redirect_uri);
}

#[test]
fn session_reads_cli_token_claims() {
    let email = "plus@example.com";
    let account = "org-1";
    let plan = "plus";
    let tokens = token_set_from_value(&json!({
        "access_token": unsigned_jwt(&json!({"exp": chrono::Utc::now().timestamp() + 3600})),
        "refresh_token": "refresh",
        "expires_in": 3600,
        "id_token": id_token(account, email, plan),
    }))
    .expect("tokens");
    let session = session_from_tokens(tokens, None).expect("session");
    assert_eq!(session.family, Family::Codex.as_str());
    assert_eq!(session.account, email);
    assert_eq!(session.plan_type, plan);
    assert_eq!(session.extra["account_id"], account);
    assert!(session.expires_at_ms > chrono::Utc::now().timestamp_millis());
}

#[test]
fn routing_hint_omits_empty_model_and_empty_tier() {
    let model = "gpt-5.6-luna";
    assert_eq!(
        routing_hint(model, Some("priority")).as_deref(),
        Some("model=gpt-5.6-luna;tier=priority")
    );
    assert_eq!(
        routing_hint("gpt-5.5", None).as_deref(),
        Some("model=gpt-5.5")
    );
    assert_eq!(
        routing_hint("gpt-5.5", Some("")).as_deref(),
        Some("model=gpt-5.5")
    );
    assert!(routing_hint("", Some("priority")).is_none());
}

#[test]
fn credential_and_upstream_headers_share_the_cli_pair() {
    let cred = credential_headers();
    let originator = cred.get("originator").expect("originator");
    let user_agent = cred.get("user-agent").expect("user-agent");
    assert!(!originator.is_empty());
    assert!(user_agent.as_bytes().starts_with(originator.as_bytes()));
    let mut session = Session::new(Family::Codex, "tok", "rt");
    session.extra.insert("account_id".to_owned(), json!("acct"));
    let upstream = upstream_headers(&session);
    assert_eq!(upstream.get("authorization").unwrap(), "Bearer tok");
    assert_eq!(upstream.get("chatgpt-account-id").unwrap(), "acct");
    assert_eq!(upstream.get("originator"), cred.get("originator"));
    assert_eq!(upstream.get("user-agent"), cred.get("user-agent"));
}

#[tokio::test]
async fn exchange_is_form_encoded_and_refresh_is_json() {
    let account = "acct-1";
    let ok = serde_json::to_string(&json!({
        "access_token": unsigned_jwt(&json!({"exp": chrono::Utc::now().timestamp() + 3600})),
        "refresh_token": "r",
        "expires_in": 3600,
        "id_token": id_token(account, "a@b.test", "plus"),
    }))
    .expect("json");

    let (url, rx) = spawn_token_server(200, &ok);
    let session = exchange_code_at(&url, "http://localhost/cb", "code", "ver")
        .await
        .expect("exchange");
    let captured = rx.recv_timeout(Duration::from_secs(2)).expect("capture");
    assert!(
        captured
            .content_type
            .contains("application/x-www-form-urlencoded")
    );
    assert!(!captured.originator.is_empty());
    assert!(captured.user_agent.starts_with(&captured.originator));
    assert!(captured.body.contains("grant_type=authorization_code"));
    assert_eq!(session.extra["account_id"], account);

    let (url, rx) = spawn_token_server(200, &ok);
    refresh_at(&url, &session).await.expect("refresh");
    let captured = rx.recv_timeout(Duration::from_secs(2)).expect("capture");
    assert!(captured.content_type.contains("json"));
    let body: Value = serde_json::from_str(&captured.body).expect("refresh json");
    assert_eq!(body["grant_type"], "refresh_token");
    assert!(
        body.get("refresh_token")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        body.get("client_id")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    );
}

#[tokio::test]
async fn refresh_permanent_codes_are_not_retryable() {
    let mut session = Session::new(Family::Codex, "tok", "rt");
    session.extra.insert("account_id".to_owned(), json!("acct"));
    for code in [
        "refresh_token_expired",
        "refresh_token_reused",
        "refresh_token_invalidated",
        "invalid_grant",
    ] {
        let (url, _rx) = spawn_token_server(400, &format!(r#"{{"error":"{code}"}}"#));
        let err = refresh_at(&url, &session).await.expect_err(code);
        assert!(err.is_permanent(), "{code} should be permanent");
    }
}

#[tokio::test]
async fn refresh_other_errors_are_not_permanent() {
    let mut session = Session::new(Family::Codex, "tok", "rt");
    session.extra.insert("account_id".to_owned(), json!("acct"));
    let (url, _rx) = spawn_token_server(503, r#"{"error":"temporarily_unavailable"}"#);
    let err = refresh_at(&url, &session).await.expect_err("503");
    assert!(!err.is_permanent());
    match err {
        Error::TokenEndpoint { status, .. } => assert_eq!(status, 503),
        other => panic!("expected token endpoint, got {other}"),
    }
}

#[tokio::test]
async fn empty_verifier_is_rejected_without_network() {
    let err = exchange_code_at("http://127.0.0.1:1/", "http://localhost/cb", "code", "")
        .await
        .expect_err("verifier");
    assert!(matches!(err, Error::MissingVerifier));
}

#[test]
fn refresh_without_new_id_token_keeps_account() {
    let account = "keep-me";
    let mut previous = Session::new(Family::Codex, "old", "rt");
    previous.account = "old@example.com".to_owned();
    previous.plan_type = "plus".to_owned();
    previous
        .extra
        .insert("account_id".to_owned(), json!(account));
    previous
        .extra
        .insert("id_token".to_owned(), json!("prev-id"));
    let tokens = TokenSet {
        access_token: unsigned_jwt(&json!({"exp": chrono::Utc::now().timestamp() + 60})),
        refresh_token: None,
        id_token: None,
        expires_in: Some(60),
    };
    let session = session_from_tokens(tokens, Some(&previous)).expect("session");
    assert_eq!(session.extra["account_id"], account);
    assert_eq!(session.refresh_token, "rt");
    assert_eq!(session.account, "old@example.com");
}

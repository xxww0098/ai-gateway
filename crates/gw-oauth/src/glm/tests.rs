use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::{
    CLI_USER_AGENT, Hosts, PollInput, PollStatus, Region, StartInput, USER_AGENT, anthropic_origin,
    anthropic_url, coding_url, desktop_headers, exchange_with, is_opaque_account,
    is_permanent_refresh_error, normalize_region, pick_human_account, poll_with, refresh, start,
    start_with,
};
use crate::{Error, Family, FlowKind, Session, StartOutcome};

fn origin_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_default()
}

#[test]
fn china_aliases_share_a_host_distinct_from_global() {
    let china = [
        normalize_region("bigmodel"),
        normalize_region("cn"),
        normalize_region("zcode"),
        normalize_region("china"),
        normalize_region("BIGMODEL"),
    ];
    assert!(china.iter().all(|region| *region == Region::BigModel));
    assert_eq!(normalize_region("zai"), Region::Zai);
    assert_eq!(normalize_region(""), Region::Zai);
    assert_ne!(
        origin_host(&anthropic_url(Region::Zai)),
        origin_host(&anthropic_url(Region::BigModel))
    );
    assert_ne!(
        origin_host(&coding_url(Region::Zai)),
        origin_host(&coding_url(Region::BigModel))
    );
    assert_eq!(
        origin_host(anthropic_origin(Region::BigModel)),
        origin_host(&anthropic_url(Region::BigModel))
    );
    assert_ne!(Region::BigModel.cli_provider(), "zcode");
    assert_eq!(Region::BigModel.cli_provider(), Region::BigModel.as_str());
    assert_eq!(Region::Zai.cli_provider(), Region::Zai.as_str());
}

#[test]
fn start_plan_and_import_are_not_supported() {
    for method in ["import", "start-plan", "zcode-plan", "captcha"] {
        let input = StartInput {
            method: method.to_owned(),
            ..StartInput::default()
        };
        let err = futures_block_on(start(&input)).expect_err(method);
        assert!(matches!(err, Error::UnsupportedFlow), "{method}");
    }
    let body = StartInput {
        body: json!({"baseURL": "https://zcode.z.ai/api/v1/zcode-plan/anthropic"}),
        ..StartInput::default()
    };
    assert!(matches!(
        futures_block_on(start(&body)),
        Err(Error::UnsupportedFlow)
    ));
}

#[test]
fn refresh_is_a_no_op_that_keeps_expiry_far_in_the_future() {
    let mut session = Session::new(Family::Glm, "id.secret", "");
    session.expires_at_ms = 1;
    let next = futures_block_on(refresh(&session)).expect("refresh");
    let now = chrono::Utc::now().timestamp_millis();
    assert_eq!(next.access_token, session.access_token);
    assert_eq!(next.refresh_token, session.access_token);
    assert!(next.expires_at_ms > now + 86_400_000_i64 * 365);
    assert!(!is_permanent_refresh_error(&Error::Permanent));
}

#[test]
fn cli_init_and_poll_parse_vendor_envelopes() {
    let started = super::parse_cli_init(json!({
        "code": 0,
        "data": {
            "flow_id": "flow-1",
            "authorize_url": "https://chat.z.ai/api/oauth/authorize",
            "poll_interval_sec": 2,
            "expires_at": 9_000_000_000_000i64
        }
    }))
    .expect("init");
    assert!(!started.flow_id.is_empty());
    assert!(!started.authorize_url.is_empty());
    assert!(started.interval_secs > 0);

    let pending = super::parse_cli_poll(
        json!({"code": 0, "data": {"status": "pending"}}),
        Region::Zai,
    )
    .expect("pending");
    assert!(matches!(pending, PollStatus::Pending { .. }));

    let ready = super::parse_cli_poll(
        json!({
            "code": 0,
            "data": {
                "status": "ready",
                "token": "zcode-jwt",
                "zai": {"access_token": "oauth-access"},
                "user": {"email": "dev@z.ai", "id": "u1"}
            }
        }),
        Region::Zai,
    )
    .expect("ready");
    let super::PollStatus::Ready(tokens) = ready else {
        panic!("expected ready");
    };
    assert_eq!(tokens.oauth_access, "oauth-access");
    assert_eq!(tokens.email.as_deref(), Some("dev@z.ai"));
    assert!(matches!(
        super::unwrap_envelope(json!({"code": 401, "msg": "denied"}), "login"),
        Err(Error::Payload(msg)) if msg.contains("denied")
    ));
}

#[test]
fn opaque_vendor_ids_are_not_card_titles() {
    assert!(is_opaque_account(Region::Zai.as_str()));
    assert!(is_opaque_account(Region::BigModel.as_str()));
    assert!(is_opaque_account(super::ID));
    assert!(is_opaque_account("12345"));
    assert!(!is_opaque_account("live@bigmodel.cn"));
    assert!(!is_opaque_account("张三"));
    let human = pick_human_account(&[Region::BigModel.as_str(), "12345", "live@z.ai"]);
    assert_eq!(human.as_deref(), Some("live@z.ai"));
}

#[tokio::test]
async fn start_posts_cli_provider_for_each_region_and_never_zcode() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock({
        let log = Arc::clone(&log);
        move |req| {
            log.lock().expect("log").push(req.clone());
            json_ok(json!({
                "code": 0,
                "data": {
                    "flow_id": "flow-1",
                    "authorize_url": "https://bigmodel.cn/login?app_id=zcode",
                    "poll_interval_sec": 2,
                    "expires_at": 9_000_000_000_000i64
                }
            }))
        }
    });
    let client = test_client();
    let hosts = Hosts::local(&base);

    for (method, body, expected) in [
        ("zcode", Value::Null, Region::BigModel),
        ("bigmodel", Value::Null, Region::BigModel),
        ("zai", Value::Null, Region::Zai),
        ("", json!({"mode": "cn"}), Region::BigModel),
        ("", json!({"region": "zai"}), Region::Zai),
    ] {
        log.lock().expect("log").clear();
        let input = StartInput {
            method: method.to_owned(),
            state: "st".to_owned(),
            body,
            ..StartInput::default()
        };
        let outcome = start_with(&input, &client, &hosts).await.expect(method);
        let recovered = PollInput::from_start(&outcome).expect("poll input");
        assert_eq!(recovered.region, expected);
        let StartOutcome::Browser {
            authorize_url,
            verifier,
            flow,
            extra,
            ..
        } = outcome
        else {
            panic!("expected browser CLI poll for {method}");
        };
        assert_eq!(flow, FlowKind::CliPoll);
        assert!(!authorize_url.is_empty());
        assert!(!verifier.is_empty());
        assert_eq!(extra["region"], expected.as_str());
        assert_eq!(recovered.poll_token, verifier);
        assert!(
            extra
                .get("flow_id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
        );
        let captured = log.lock().expect("log").last().cloned().expect("request");
        assert!(
            captured.path.contains("/oauth/cli/init"),
            "{}",
            captured.path
        );
        assert_eq!(captured.method, "POST");
        let posted: Value = serde_json::from_str(&captured.body).expect("json");
        assert_eq!(posted["provider"], expected.cli_provider());
        assert_ne!(posted["provider"], "zcode");
        let ua = header_value(&captured.headers, "user-agent");
        assert_eq!(ua, CLI_USER_AGENT);
        assert!(!ua.contains("ai-sdk"));
        assert!(!captured.headers.to_ascii_lowercase().contains("dsh-plugin"));
    }
}

#[tokio::test]
async fn poll_stays_pending_until_ready_then_exchange_splits_regions() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock({
        let log = Arc::clone(&log);
        move |req| {
            log.lock().expect("log").push(req.clone());
            if req.path.contains("/oauth/cli/poll/") {
                if req.path.contains("pending") {
                    return json_ok(json!({"code": 0, "data": {"status": "pending"}}));
                }
                if req.path.contains("bm") {
                    return json_ok(json!({
                        "code": 0,
                        "data": {
                            "status": "ready",
                            "token": "bm-jwt",
                            "zcode": {"access_token": "bm-oauth"},
                            "user": {"email": "cn@bigmodel.cn"}
                        }
                    }));
                }
                return json_ok(json!({
                    "code": 0,
                    "data": {
                        "status": "ready",
                        "token": "zcode-jwt",
                        "zai": {"access_token": "oauth-access"},
                        "user": {"email": "dev@z.ai"}
                    }
                }));
            }
            if req.path.contains("/api/auth/z/login") {
                return json_ok(
                    json!({"code": 200, "success": true, "data": {"access_token": "biz"}}),
                );
            }
            if req.path.contains("/getCustomerInfo") {
                return json_ok(json!({
                    "code": 200,
                    "data": {
                        "email": "from-userinfo@z.ai",
                        "organizations": [{
                            "organizationId": "org-1",
                            "isDefault": true,
                            "projects": [{"projectId": "proj-1", "isDefault": true}]
                        }]
                    }
                }));
            }
            if req.path.ends_with("/api_keys") && req.method == "GET" {
                return json_ok(json!({"code": 200, "data": []}));
            }
            if req.path.ends_with("/api_keys") && req.method == "POST" {
                return json_ok(
                    json!({"code": 200, "data": {"apiKey": "aaaa1111", "name": "ai-gateway"}}),
                );
            }
            if req.path.contains("/copy/") {
                return json_ok(json!({"code": 200, "data": {"secretKey": "bbbb222233334444"}}));
            }
            json_ok(json!({"code": 0, "data": {}}))
        }
    });
    let client = test_client();
    let hosts = Hosts::local(&base);

    let pending = poll_with(
        &PollInput {
            flow_id: "pending-1".to_owned(),
            poll_token: "poll".to_owned(),
            region: Region::Zai,
        },
        &client,
        &hosts,
    )
    .await
    .expect("pending");
    assert!(matches!(pending, PollStatus::Pending { .. }));

    let ready_cn = poll_with(
        &PollInput {
            flow_id: "bm-1".to_owned(),
            poll_token: "poll".to_owned(),
            region: Region::BigModel,
        },
        &client,
        &hosts,
    )
    .await
    .expect("bm ready");
    let PollStatus::Ready(tokens) = ready_cn else {
        panic!("bigmodel should be ready");
    };
    let before_exchange = log.lock().expect("log").len();
    let session = exchange_with(&tokens, &client, &hosts)
        .await
        .expect("bm exchange");
    assert_eq!(session.access_token, "bm-jwt");
    assert_eq!(session.extra["region"], "bigmodel");
    assert_eq!(session.account, "cn@bigmodel.cn");
    assert!(session.expires_at_ms > chrono::Utc::now().timestamp_millis());
    assert_eq!(log.lock().expect("log").len(), before_exchange);

    let ready_zai = poll_with(
        &PollInput {
            flow_id: "zai-1".to_owned(),
            poll_token: "poll".to_owned(),
            region: Region::Zai,
        },
        &client,
        &hosts,
    )
    .await
    .expect("zai ready");
    let PollStatus::Ready(tokens) = ready_zai else {
        panic!("zai should be ready");
    };
    let session = exchange_with(&tokens, &client, &hosts)
        .await
        .expect("zai exchange");
    assert_eq!(session.access_token, "aaaa1111.bbbb222233334444");
    assert_eq!(session.extra["region"], "zai");
    assert_eq!(session.account, "dev@z.ai");
    let mint_calls = log.lock().expect("log");
    assert!(
        mint_calls
            .iter()
            .any(|c| c.path.contains("/api/auth/z/login"))
    );
    assert!(mint_calls.iter().any(|c| c.path.contains("/copy/")));
}

#[test]
fn desktop_ua_is_cli_ua_plus_anthropic_sdk_and_leaks_no_plugin_name() {
    assert!(USER_AGENT.starts_with(CLI_USER_AGENT));
    assert!(USER_AGENT.contains("ai-sdk/anthropic"));
    assert!(!USER_AGENT.contains("dsh-plugin"));
    assert!(!USER_AGENT.contains("ai-gateway"));
    let headers = desktop_headers(Some("sess-1"));
    let ua = headers
        .get("user-agent")
        .expect("ua")
        .to_str()
        .expect("utf8");
    assert_eq!(ua, USER_AGENT);
    assert_eq!(
        headers
            .get("x-session-id")
            .expect("session")
            .to_str()
            .expect("utf8"),
        "sess-1"
    );
    assert!(headers.get("session-id").is_none());
    assert!(headers.get("x-grok-conv-id").is_none());
}

fn futures_block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt")
        .block_on(fut)
}

fn test_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .build()
        .expect("client")
}

#[derive(Clone, Debug)]
struct Captured {
    method: String,
    path: String,
    headers: String,
    body: String,
}

fn spawn_mock(handler: impl Fn(&Captured) -> (u16, String) + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let Some(captured) = read_request(&mut stream) else {
                continue;
            };
            let (status, body) = handler(&captured);
            let _ = write_response(&mut stream, status, &body);
        }
    });
    format!("http://{addr}")
}

fn json_ok(body: Value) -> (u16, String) {
    (200, body.to_string())
}

fn read_request(stream: &mut TcpStream) -> Option<Captured> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    loop {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(header_end) = find_header_end(&buf) {
            let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
            let content_length = content_length_of(&headers);
            while buf.len() < header_end + content_length {
                let n = stream.read(&mut tmp).ok()?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let body = String::from_utf8_lossy(&buf[header_end.min(buf.len())..]).into_owned();
            let first = headers.lines().next().unwrap_or_default();
            let mut parts = first.split_whitespace();
            let method = parts.next().unwrap_or_default().to_owned();
            let path = parts.next().unwrap_or_default().to_owned();
            return Some(Captured {
                method,
                path,
                headers,
                body,
            });
        }
        if buf.len() > 64 * 1024 {
            break;
        }
    }
    None
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn content_length_of(headers: &str) -> usize {
    for line in headers.lines() {
        let (name, value) = match line.split_once(':') {
            Some(pair) => pair,
            None => continue,
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().unwrap_or(0);
        }
    }
    0
}

fn write_response(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = if status == 200 { "OK" } else { "Error" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
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

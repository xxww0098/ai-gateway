use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

use super::{
    CLIENT_ID, DAILY_API_URL, Endpoints, MAC_APP_PLIST, PROD_API_URL, VERIFY_CODE,
    apply_validation, exchange_with, is_permanent_refresh_error, is_permanent_refresh_error_text,
    normalize_version, parse_plist_version, parse_validation, parse_version_text, platform,
    refresh_with, request_user_agent, start, version,
};
use crate::antigravity::request::{chat_headers, fetch_cloud_code_at};
use crate::{Error, FlowKind, Session, StartInput, StartOutcome};

#[derive(Clone, Debug)]
struct Captured {
    method_path: String,
    headers: String,
    body: String,
}

fn spawn_scripted(replies: Vec<(u16, String)>) -> (String, Arc<Mutex<Vec<Captured>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let hits = Arc::new(Mutex::new(Vec::new()));
    let hits2 = hits.clone();
    thread::spawn(move || {
        for (status, body) in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let captured = read_http(&mut stream);
            hits2
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(captured);
            let reason = if (200..300).contains(&status) {
                "OK"
            } else {
                "ERR"
            };
            let resp = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (format!("http://{addr}"), hits)
}

fn read_http(stream: &mut TcpStream) -> Captured {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
        if let Some(at) = find_headers_end(&buf) {
            let headers = String::from_utf8_lossy(&buf[..at]).into_owned();
            let content_len = content_length(&headers);
            let body_at = at + 4;
            while buf.len() < body_at + content_len {
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    Err(_) => break,
                }
            }
            let body = String::from_utf8_lossy(buf.get(body_at..).unwrap_or(&[])).into_owned();
            let method_path = headers.lines().next().unwrap_or("").to_owned();
            return Captured {
                method_path,
                headers,
                body,
            };
        }
        if buf.len() > 64 * 1024 {
            break;
        }
    }
    Captured {
        method_path: String::new(),
        headers: String::from_utf8_lossy(&buf).into_owned(),
        body: String::new(),
    }
}

fn find_headers_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(headers: &str) -> usize {
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

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .build()
        .expect("client")
}

fn endpoints_for(base: &str) -> Endpoints {
    Endpoints {
        token: format!("{base}/token"),
        userinfo: format!("{base}/userinfo"),
        daily: base.to_owned(),
        prod: format!("{base}/prod"),
    }
}

fn query_map(url: &str) -> std::collections::HashMap<String, String> {
    Url::parse(url)
        .expect("url")
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn google_validation_denied() -> Value {
    json!({
        "error": {
            "code": 403,
            "message": "Verify your account to continue.",
            "status": "PERMISSION_DENIED",
            "details": [{
                "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                "reason": "VALIDATION_REQUIRED",
                "domain": "cloudcode-pa.googleapis.com",
                "metadata": {
                    "validation_url": "https://accounts.google.com/signin/continue?continue=https://developers.google.com/gemini-code-assist/auth/auth_success_gemini&plt=one-time-test"
                }
            }]
        }
    })
}

#[tokio::test]
async fn start_is_google_installed_app_without_pkce() {
    let input = StartInput {
        redirect_uri: "http://localhost:51121/oauth-callback".into(),
        state: "st".into(),
        method: String::new(),
        body: json!({}),
    };
    let StartOutcome::Browser {
        authorize_url,
        verifier,
        redirect_uri,
        flow,
        ..
    } = start(&input).await.expect("start")
    else {
        panic!("expected browser flow");
    };
    assert_eq!(flow, FlowKind::AuthorizationCode);
    assert!(verifier.is_empty(), "installed-app must not mint PKCE");
    assert_eq!(redirect_uri, input.redirect_uri);

    let parsed = Url::parse(&authorize_url).expect("authorize");
    assert_eq!(parsed.scheme(), "https");
    assert_eq!(parsed.host_str(), Some("accounts.google.com"));
    let q = query_map(&authorize_url);
    assert_eq!(q.get("access_type").map(String::as_str), Some("offline"));
    assert_eq!(q.get("prompt").map(String::as_str), Some("consent"));
    assert_eq!(q.get("response_type").map(String::as_str), Some("code"));
    assert_eq!(q.get("redirect_uri"), Some(&input.redirect_uri));
    assert_eq!(q.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert!(q.get("scope").is_some_and(|s| s.contains("cloud-platform")));
    assert!(!q.contains_key("code_challenge"));
    assert!(!q.contains_key("code_challenge_method"));
    assert!(!q.contains_key("code_verifier"));
}

#[test]
fn request_user_agent_is_hub_os_arch() {
    let ua = request_user_agent();
    let plat = platform();
    assert!(ua.starts_with("antigravity/hub/"), "{ua}");
    assert!(ua.ends_with(&plat), "{ua} / {plat}");
    assert_eq!(ua.split_whitespace().count(), 2, "{ua}");
    assert!(!ua.contains("Client-Metadata"));
    assert!(!ua.contains("x-goog-api-client"));
    assert!(!ua.contains("2.5.5"), "must not fingerprint IDE.app: {ua}");
    let ver = version();
    assert!(ua.contains(ver), "{ua} vs {ver}");
}

#[test]
fn plist_helper_reads_short_version_and_ignores_ide_app() {
    let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.google.antigravity</string>
	<key>CFBundleShortVersionString</key>
	<string>2.11.0</string>
	<key>CFBundleVersion</key>
	<string>99.0.0</string>
</dict>
</plist>"#;
    assert_eq!(parse_plist_version(plist).as_deref(), Some("2.11.0"));
    assert_eq!(
        parse_version_text("Antigravity 9.8.7\n").as_deref(),
        Some("9.8.7")
    );
    assert_eq!(normalize_version("9.8.7.0").as_deref(), Some("9.8.7"));
    assert_eq!(normalize_version("9.8.7.1").as_deref(), Some("9.8.7.1"));
    assert!(MAC_APP_PLIST.contains("Antigravity.app"));
    assert!(!MAC_APP_PLIST.contains("IDE.app"));
}

#[test]
fn validation_required_is_not_permanent() {
    let denied = google_validation_denied();
    let info = parse_validation(&denied).expect("required");
    assert!(info.required);
    assert_eq!(info.code, VERIFY_CODE);
    assert!(
        info.validation_url
            .as_deref()
            .is_some_and(|u| u.starts_with("https://accounts.google.com/signin/continue"))
    );
    assert!(parse_validation(&json!({"error": {"message": "quota exceeded"}})).is_none());
    assert!(!is_permanent_refresh_error(&denied));
    assert!(!is_permanent_refresh_error(&json!({"code": VERIFY_CODE})));
    assert!(is_permanent_refresh_error(
        &json!({"error": "invalid_grant"})
    ));
    assert!(is_permanent_refresh_error(
        &json!({"error": "invalid_client"})
    ));
    assert!(is_permanent_refresh_error(
        &json!({"error": "unauthorized_client"})
    ));
    assert!(is_permanent_refresh_error_text(
        r#"{"error":"invalid_grant"}"#
    ));
    assert!(!is_permanent_refresh_error_text(&denied.to_string()));
}

#[tokio::test]
async fn exchange_discovers_project_via_daily_load_code_assist() {
    let project = "cogent-snow-4mnnp";
    let (base, hits) = spawn_scripted(vec![
        (
            200,
            json!({"access_token": "acc", "refresh_token": "ref", "expires_in": 3600}).to_string(),
        ),
        (200, json!({"email": "dev@gmail.com"}).to_string()),
        (
            200,
            json!({"cloudaicompanionProject": project, "currentTier": {"id": "free-tier"}})
                .to_string(),
        ),
    ]);
    let session = exchange_with(
        &http_client(),
        &endpoints_for(&base),
        "code",
        "http://localhost/cb",
    )
    .await
    .expect("exchange");
    assert_eq!(session.account, "dev@gmail.com");
    assert_eq!(session.extra["project_id"].as_str(), Some(project));
    assert!(!session.access_token.is_empty());
    assert!(!session.refresh_token.is_empty());
    assert!(session.expires_at_ms > 0);

    let captured = hits.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(captured.len(), 3);
    assert!(captured[0].method_path.contains("/token"));
    assert!(
        captured[0]
            .headers
            .to_ascii_lowercase()
            .contains("user-agent")
    );
    assert!(captured[0].body.contains("grant_type=authorization_code"));
    assert!(!captured[0].body.contains("code_verifier"));
    assert!(captured[1].method_path.contains("userinfo"));
    assert!(captured[2].method_path.contains("loadCodeAssist"));
    assert!(captured[2].body.contains("ANTIGRAVITY"));
    assert!(!captured[2].body.contains("IDE_UNSPECIFIED"));
    let load_headers = captured[2].headers.to_ascii_lowercase();
    assert!(load_headers.contains("user-agent"));
    assert!(!load_headers.contains("x-goog-api-client"));
    assert!(!load_headers.contains("client-metadata"));
}

#[tokio::test]
async fn empty_load_falls_back_to_daily_onboard_user() {
    let (base, hits) = spawn_scripted(vec![
        (
            200,
            json!({"access_token": "acc", "refresh_token": "ref", "expires_in": 120}).to_string(),
        ),
        (200, json!({"email": "keep@x"}).to_string()),
        (
            200,
            json!({"allowedTiers": [{"id": "free-tier", "isDefault": true}]}).to_string(),
        ),
        (
            200,
            json!({"done": true, "response": {"cloudaicompanionProject": {"id": "from-onboard"}}})
                .to_string(),
        ),
    ]);
    let session = exchange_with(
        &http_client(),
        &endpoints_for(&base),
        "code",
        "http://localhost/cb",
    )
    .await
    .expect("exchange");
    assert_eq!(session.extra["project_id"], "from-onboard");
    let captured = hits.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert!(captured[2].method_path.contains("loadCodeAssist"));
    assert!(captured[3].method_path.contains("onboardUser"));
    let onboard_headers = captured[3].headers.to_ascii_lowercase();
    assert!(onboard_headers.contains("x-goog-api-client"));
    assert!(captured[3].body.contains("ide_type"));
}

#[tokio::test]
async fn five_xx_on_daily_hits_prod_four_xx_does_not() {
    let (daily, daily_hits) = spawn_scripted(vec![(503, "busy".into())]);
    let (prod, prod_hits) = spawn_scripted(vec![(200, json!({"ok": true}).to_string())]);
    let url = format!("{daily}/v1internal:loadCodeAssist");
    let ok = fetch_cloud_code_at(
        &http_client(),
        &url,
        &daily,
        &prod,
        chat_headers("tok").expect("h"),
        json!({"metadata":{"ideType":"ANTIGRAVITY"}})
            .to_string()
            .into_bytes(),
    )
    .await
    .expect("fallback");
    assert!(ok.status().is_success());
    assert_eq!(
        daily_hits.lock().unwrap_or_else(|e| e.into_inner()).len(),
        1
    );
    assert_eq!(prod_hits.lock().unwrap_or_else(|e| e.into_inner()).len(), 1);

    let (denied, denied_hits) = spawn_scripted(vec![(403, "no".into())]);
    let (unused_prod, unused_hits) =
        spawn_scripted(vec![(200, json!({"should":"not"}).to_string())]);
    let denied_url = format!("{denied}/v1internal:loadCodeAssist");
    let response = fetch_cloud_code_at(
        &http_client(),
        &denied_url,
        &denied,
        &unused_prod,
        chat_headers("tok").expect("h"),
        b"{}".to_vec(),
    )
    .await
    .expect("4xx is returned, not retried");
    assert_eq!(response.status().as_u16(), 403);
    assert_eq!(
        denied_hits.lock().unwrap_or_else(|e| e.into_inner()).len(),
        1
    );
    assert_eq!(
        unused_hits.lock().unwrap_or_else(|e| e.into_inner()).len(),
        0,
        "4xx must not fall back to prod"
    );
}

#[tokio::test]
async fn refresh_invalid_grant_is_permanent_validation_is_not() {
    let (base, _) = spawn_scripted(vec![(400, json!({"error": "invalid_grant"}).to_string())]);
    let mut session = Session::new(crate::Family::Antigravity, "acc", "ref");
    session.extra.insert("project_id".into(), json!("proj-1"));
    let err = refresh_with(&http_client(), &endpoints_for(&base), &session)
        .await
        .expect_err("invalid_grant");
    assert!(err.is_permanent(), "{err}");
    assert!(matches!(err, Error::Permanent));

    let (base, _) = spawn_scripted(vec![(403, google_validation_denied().to_string())]);
    let err = refresh_with(&http_client(), &endpoints_for(&base), &session)
        .await
        .expect_err("validation");
    assert!(!err.is_permanent(), "{err}");
    assert!(!matches!(err, Error::Permanent));
}

#[tokio::test]
async fn refresh_keeps_project_and_rotates_access_token() {
    let (base, hits) = spawn_scripted(vec![(
        200,
        json!({"access_token": "acc-2", "expires_in": 3600}).to_string(),
    )]);
    let mut session = Session::new(crate::Family::Antigravity, "acc-1", "ref-1");
    session.account = "dev@x".into();
    session.plan_type = "g1-pro-tier".into();
    session
        .extra
        .insert("project_id".into(), json!("proj-keep"));
    let next = refresh_with(&http_client(), &endpoints_for(&base), &session)
        .await
        .expect("refresh");
    assert_eq!(next.access_token, "acc-2");
    assert_eq!(next.refresh_token, "ref-1");
    assert_eq!(next.extra["project_id"], "proj-keep");
    assert_eq!(next.account, "dev@x");
    let captured = hits.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert!(captured[0].body.contains("grant_type=refresh_token"));
    assert!(!captured[0].body.contains("redirect_uri"));
}

#[test]
fn apply_validation_sets_and_clears_the_flag() {
    let mut session = Session::new(crate::Family::Antigravity, "a", "r");
    session.extra.insert("project_id".into(), json!("p"));
    let info = parse_validation(&google_validation_denied()).expect("info");
    let marked = apply_validation(session.clone(), Some(&info));
    assert_eq!(marked.extra["needs_validation"], true);
    assert!(
        marked.extra["validation_url"]
            .as_str()
            .is_some_and(|u| u.contains("accounts.google.com"))
    );
    let cleared = apply_validation(marked, None);
    assert!(cleared.extra.get("needs_validation").is_none());
    assert!(cleared.extra.get("validation_url").is_none());
}

#[test]
fn hub_cloud_code_urls_are_daily_not_ide_prod() {
    use crate::antigravity::request::{generate_url, stream_url};
    assert_ne!(DAILY_API_URL, PROD_API_URL);
    assert!(DAILY_API_URL.starts_with("https://"));
    assert!(PROD_API_URL.starts_with("https://"));
    let generate = generate_url();
    let stream = stream_url();
    assert!(generate.starts_with(DAILY_API_URL), "{generate}");
    assert!(!generate.starts_with(PROD_API_URL), "{generate}");
    assert!(stream.starts_with(DAILY_API_URL), "{stream}");
    assert!(stream.contains("streamGenerateContent"), "{stream}");
    assert!(stream.contains("alt=sse"), "{stream}");
}

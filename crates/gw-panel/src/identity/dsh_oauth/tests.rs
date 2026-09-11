use axum::http::{HeaderMap, HeaderValue, StatusCode};
use chrono::Duration;

use super::session::{self, TransitionError};
use super::{public_origin, transition_failure};

#[test]
fn public_origin_prefers_forwarded_proto_and_host() {
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
    headers.insert("x-forwarded-host", HeaderValue::from_static("gw.example"));
    assert_eq!(
        public_origin(&headers, "127.0.0.1", 8888),
        "https://gw.example"
    );
}

#[test]
fn public_origin_falls_back_to_host_header_then_bind_address() {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::HOST,
        HeaderValue::from_static("panel.local:9"),
    );
    assert_eq!(
        public_origin(&headers, "127.0.0.1", 8888),
        "http://panel.local:9"
    );
    assert_eq!(
        public_origin(&HeaderMap::new(), "127.0.0.1", 8888),
        "http://127.0.0.1:8888"
    );
}

// ── approve 预检：注定失败的审批不该走到发 key ────────────────────────────────
//
// `approve_device` 在调用 `generate_api_key` 之前用 `session::require_pending`
// 把关；这里锁死那道闸门本身——过期/已处理的会话必须被拒，且响应语义与
// `session::approve` 的失败一致（400 / 409）。闸门失守 = 孤儿 key 复活。

#[test]
fn an_expired_session_is_refused_before_any_key_is_minted() {
    let now = chrono::Utc::now();
    let session = session::start(now, Duration::seconds(10), "https://gw.example".to_owned());
    let late = now + Duration::seconds(10);
    assert_eq!(
        session::require_pending(&session, late),
        Err(TransitionError::Expired)
    );
    assert_eq!(
        transition_failure(TransitionError::Expired).status(),
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn a_resolved_session_is_refused_before_any_key_is_minted() {
    let now = chrono::Utc::now();
    let session = session::start(now, Duration::seconds(60), "https://gw.example".to_owned());
    let approved = session::approve(&session, now, 1, "agw-first".to_owned()).unwrap();
    assert_eq!(
        session::require_pending(&approved, now + Duration::seconds(1)),
        Err(TransitionError::AlreadyResolved)
    );
    assert_eq!(
        transition_failure(TransitionError::AlreadyResolved).status(),
        StatusCode::CONFLICT
    );
}

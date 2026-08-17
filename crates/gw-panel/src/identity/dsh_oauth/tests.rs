use axum::http::{HeaderMap, HeaderValue};

use super::public_origin;

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
    headers.insert(axum::http::header::HOST, HeaderValue::from_static("panel.local:9"));
    assert_eq!(
        public_origin(&headers, "127.0.0.1", 8888),
        "http://panel.local:9"
    );
    assert_eq!(
        public_origin(&HeaderMap::new(), "127.0.0.1", 8888),
        "http://127.0.0.1:8888"
    );
}

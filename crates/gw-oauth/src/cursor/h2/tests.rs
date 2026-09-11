use super::describe_h2_transport_error;
use crate::cursor::{CLIENT_TYPE, chat_headers};

#[test]
fn h2_not_supported_mentions_alpn_proxy() {
    let message = describe_h2_transport_error(
        "ERR_HTTP2_ERROR: h2 is not supported",
        "https://agentn.us.api5.cursor.sh",
    );
    assert!(message.contains("HTTP/2"), "{message}");
    assert!(message.contains("ALPN"), "{message}");
    assert!(message.contains("agentn.us.api5.cursor.sh"), "{message}");
}

#[test]
fn unrelated_transport_error_is_left_alone() {
    let raw = "connection reset by peer";
    assert_eq!(describe_h2_transport_error(raw, "https://example"), raw);
}

#[test]
fn unary_headers_are_cli_and_proto() {
    let headers = chat_headers("tok", Some("id-1"), true);
    assert_eq!(
        headers
            .get("x-cursor-client-type")
            .map(http::HeaderValue::as_bytes),
        Some(CLIENT_TYPE.as_bytes())
    );
    assert_ne!(CLIENT_TYPE, "sdk");
    assert_eq!(
        headers.get("content-type").map(http::HeaderValue::as_bytes),
        Some(b"application/proto".as_slice())
    );
}

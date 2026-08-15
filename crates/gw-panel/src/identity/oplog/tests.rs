use super::*;
use axum::http::Request;

fn parts_with(headers: &[(&str, &str)]) -> Parts {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/panel/auth/login");
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    builder.body(()).expect("build request").into_parts().0
}

#[test]
fn client_ip_prefers_the_first_forwarded_hop() {
    // X-Forwarded-For 是 "client, proxy1, proxy2"；只有最左边那个是调用方。
    let parts = parts_with(&[("x-forwarded-for", "203.0.113.7, 10.0.0.1, 10.0.0.2")]);
    assert_eq!(client_ip(&parts), "203.0.113.7");
}

#[test]
fn client_ip_falls_back_to_real_ip_then_to_empty() {
    let parts = parts_with(&[("x-real-ip", "198.51.100.4")]);
    assert_eq!(client_ip(&parts), "198.51.100.4");

    // 什么都没有时旧实现的 ClientIP 给空串，不是某个占位地址。
    assert_eq!(client_ip(&parts_with(&[])), "");
}

#[test]
fn client_ip_ignores_blank_forwarded_headers() {
    // 空的转发头必须继续往下找，否则代理配错就把所有请求归成同一个空 IP。
    let parts = parts_with(&[("x-forwarded-for", "  ,  "), ("x-real-ip", "198.51.100.4")]);
    assert_eq!(client_ip(&parts), "198.51.100.4");
}

#[test]
fn trace_id_passes_the_inbound_header_through_unchanged() {
    let incoming = "0b9c1d2e-3f40-5162-8394-a5b6c7d8e9f0";
    let parts = parts_with(&[(TRACE_ID_HEADER, incoming)]);
    assert_eq!(trace_id(&parts), incoming);
}

#[test]
fn trace_id_is_generated_and_unique_when_absent() {
    // 性质：缺头时每次都得到一个新的非空 id —— 否则整条链路的日志会串在一起。
    let a = trace_id(&parts_with(&[]));
    let b = trace_id(&parts_with(&[]));
    assert!(!a.is_empty());
    assert_ne!(a, b);
}

#[test]
fn trace_id_treats_a_whitespace_header_as_absent() {
    let generated = trace_id(&parts_with(&[(TRACE_ID_HEADER, "   ")]));
    assert!(!generated.trim().is_empty());
}

#[tokio::test]
async fn req_meta_records_method_and_raw_path_without_a_matched_route() {
    // 没有 MatchedPath 扩展时（例如 404 落到兜底 handler），退回原始 path。
    let mut parts = parts_with(&[("x-real-ip", "192.0.2.9")]);
    let ReqMeta(meta) = ReqMeta::from_request_parts(&mut parts, &())
        .await
        .expect("infallible");
    assert_eq!(meta.method, "POST");
    assert_eq!(meta.path, "/api/panel/auth/login");
    assert_eq!(meta.ip_address, "192.0.2.9");
    assert!(!meta.request_id.is_empty());
}

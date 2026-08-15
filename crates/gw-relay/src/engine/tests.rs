//! OWNER: worker `relay-core`。
//!
//! 规范 2.11：断言的期望值全部来自测试自己造的输入（入站 path、入站 query、
//! 上游造的 header/body），或是「个数」「同一块内存」「Ok 而不是 Err」这类性质。
//!
//! **一个字节都不走真实网络** —— 传输是注入的（[`Transport`]）。

use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http::uri::PathAndQuery;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use http_body_util::BodyExt;
use url::Url;

use super::{RelayEngine, RelayOptions, Transport, UpstreamHead, UpstreamRequest};
use crate::body::fixtures::{Hanging, drain, fallible, frames, payloads};
use crate::contract::{
    Credential, Relay, RelayBody, RelayError, RelayRequest, RelayResponse, RelayResponseBody,
    RelayTimeouts, RelayTransportError, RelayUsage, UpstreamDialect, UpstreamTarget, UsageProbe,
};

// ------------------------------------------------------------------ 假传输

struct Fake {
    seen: Arc<Mutex<Option<UpstreamRequest>>>,
    reply: Mutex<Option<UpstreamHead>>,
}

impl Fake {
    fn new(reply: UpstreamHead) -> (Arc<Self>, Arc<Mutex<Option<UpstreamRequest>>>) {
        let seen = Arc::new(Mutex::new(None));
        let fake = Arc::new(Self {
            seen: Arc::clone(&seen),
            reply: Mutex::new(Some(reply)),
        });
        (fake, seen)
    }
}

#[async_trait]
impl Transport for Arc<Fake> {
    async fn send(&self, req: UpstreamRequest) -> Result<UpstreamHead, RelayTransportError> {
        *self.seen.lock().expect("测试锁") = Some(req);
        Ok(self
            .reply
            .lock()
            .expect("测试锁")
            .take()
            .expect("假传输只能用一次"))
    }
}

fn head(
    status: u16,
    headers: &[(&str, &str)],
    body: http_body_util::combinators::BoxBody<Bytes, RelayError>,
) -> UpstreamHead {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).expect("测试 header 名"),
            HeaderValue::from_str(value).expect("测试 header 值"),
        );
    }
    UpstreamHead {
        status: StatusCode::from_u16(status).expect("测试 status"),
        headers: map,
        body,
    }
}

fn sse(status: u16, body: http_body_util::combinators::BoxBody<Bytes, RelayError>) -> UpstreamHead {
    head(status, &[("content-type", "text/event-stream")], body)
}

fn request(target: &str) -> RelayRequest {
    RelayRequest {
        method: Method::POST,
        target: PathAndQuery::from_str(target).expect("测试 target"),
        headers: HeaderMap::new(),
        body: RelayBody::Buffered(Bytes::new()),
    }
}

fn upstream(origin: &str, timeouts: RelayTimeouts) -> UpstreamTarget {
    UpstreamTarget {
        origin: Url::parse(origin).expect("测试 origin"),
        credential: Credential::Bearer("upstream-token".to_owned()),
        timeouts,
        dialect: UpstreamDialect::OpenAiChat,
    }
}

fn quick() -> RelayTimeouts {
    RelayTimeouts {
        connect: Duration::from_millis(50),
        request: Duration::from_secs(5),
        stream_idle: Duration::from_millis(50),
    }
}

async fn relay_once(
    reply: UpstreamHead,
    target: &str,
    origin: &str,
    timeouts: RelayTimeouts,
    probe: Option<Box<dyn UsageProbe>>,
) -> (RelayResponse, Arc<Mutex<Option<UpstreamRequest>>>) {
    let (fake, seen) = Fake::new(reply);
    let engine = RelayEngine::with_transport(fake, RelayOptions::default());
    let response = engine
        .relay(request(target), &upstream(origin, timeouts), probe)
        .await
        .expect("上游给了响应就不该是 Err");
    (response, seen)
}

// ------------------------------------------------------------------ 探针

#[derive(Clone, Default)]
struct ProbeLog {
    frames: Arc<Mutex<Vec<Bytes>>>,
    finishes: Arc<AtomicUsize>,
}

struct Recorder(ProbeLog);

impl UsageProbe for Recorder {
    fn observe(&mut self, frame: &Bytes) {
        self.0.frames.lock().expect("测试锁").push(frame.clone());
    }

    fn finish(self: Box<Self>) -> Option<RelayUsage> {
        self.0.finishes.fetch_add(1, Ordering::SeqCst);
        None
    }
}

// ------------------------------------------------------------------ 用例

/// 缺陷 #1（S1）的守护测试：上游端点 = origin + **入站 path**，不许由 provider 猜。
///
/// 把 URL 构造改成「按 family 拼 /v1/chat/completions」这条就红。
#[tokio::test]
async fn the_upstream_endpoint_is_the_inbound_path() {
    for target in ["/v1/chat/completions", "/v1/responses", "/v1/messages"] {
        let (_, seen) = relay_once(
            head(200, &[], frames(Vec::new())),
            target,
            "https://api.example.test",
            RelayTimeouts::default(),
            None,
        )
        .await;
        let sent = seen.lock().expect("测试锁");
        let sent = sent.as_ref().expect("传输被调用过");
        assert_eq!(
            sent.url.path(),
            target,
            "入站 path 必须原样出现在上游 URL 上"
        );
    }
}

/// origin 自带路径前缀时，前缀与入站 path 拼接，两边都不丢。
#[tokio::test]
async fn an_origin_path_prefix_is_prepended_not_dropped() {
    let target = "/v1/messages";
    for origin in [
        "https://gw.example.test/proxy",
        "https://gw.example.test/proxy/",
    ] {
        let (_, seen) = relay_once(
            head(200, &[], frames(Vec::new())),
            target,
            origin,
            RelayTimeouts::default(),
            None,
        )
        .await;
        let sent = seen.lock().expect("测试锁");
        let path = sent.as_ref().expect("传输被调用过").url.path();
        let prefix = Url::parse(origin).expect("测试 origin");
        let prefix = prefix.path().trim_end_matches('/');
        assert_eq!(path, format!("{prefix}{target}"), "origin={origin}");
    }
}

/// 缺陷 #9 的守护测试：query 是**字节**，不解码、不重编码、不拆键值对。
///
/// 把它改成 `url.query_pairs_mut().append_pair(..)` 这条就红 ——
/// `%20` 会变成 `%2520`，`+` 会变成 `%2B`。
#[tokio::test]
async fn raw_query_bytes_are_forwarded_verbatim() {
    let query = "tag=a%20b&plus=a+b&pct=%25&empty=&flag&nested=%7B%22a%22%3A1%7D";
    let target = format!("/v1/messages?{query}");

    let (_, seen) = relay_once(
        head(200, &[], frames(Vec::new())),
        &target,
        "https://api.example.test",
        RelayTimeouts::default(),
        None,
    )
    .await;

    let sent = seen.lock().expect("测试锁");
    assert_eq!(
        sent.as_ref().expect("传输被调用过").url.query(),
        Some(query)
    );
}

/// 缺陷 #3（S1）+ #12 的守护测试：上游的 4xx/5xx 是 `Ok(RelayResponse)`，
/// 限流相关的 header 一条不丢，错误体逐字节原样（含非 UTF-8）。
///
/// 把它改回「4xx/5xx → Err(status + String)」这条就红。
#[tokio::test]
async fn an_upstream_error_status_is_a_response_with_all_its_headers() {
    let rate_limit_headers = [
        ("retry-after", "12"),
        ("x-ratelimit-reset-requests", "60s"),
        ("request-id", "req_abc123"),
        ("cf-ray", "8a1b2c3d4e5f"),
        ("content-type", "text/html; charset=utf-8"),
    ];
    // Cloudflare 的错误页可能不是 UTF-8。
    let error_body = Bytes::from_static(&[0x3c, 0x68, 0x74, 0x6d, 0x6c, 0xff, 0xfe, 0x3e]);

    let (response, _) = relay_once(
        head(429, &rate_limit_headers, frames(vec![error_body.clone()])),
        "/v1/messages",
        "https://api.example.test",
        RelayTimeouts::default(),
        None,
    )
    .await;

    assert_eq!(response.status.as_u16(), 429);
    for (name, value) in rate_limit_headers {
        assert_eq!(
            response.headers.get(name).map(HeaderValue::as_bytes),
            Some(value.as_bytes()),
            "{name} 丢了"
        );
    }
    let RelayResponseBody::Buffered(got) = response.body else {
        panic!("非流式响应必须是 Buffered");
    };
    assert_eq!(got, error_body, "错误体必须逐字节原样，不经过 String");
}

/// SSE 帧边界不许被重新分帧：上游读到什么就转发什么。
///
/// 故意造出「多行 data」「注释行」「一个帧里塞两个事件」「一个事件被切成两帧」
/// 四种形状 —— 中继层不做 SSE 解析，所以这些全部原样过。
#[tokio::test]
async fn sse_frames_are_forwarded_without_reframing() {
    let upstream_frames = vec![
        Bytes::from_static(b": ping\n\n"),
        Bytes::from_static(b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n"),
        Bytes::from_static(b"data: line one\ndata: line two\n\ndata: {\"a\":1}\n\n"),
        Bytes::from_static(b"data: {\"partial\""),
        Bytes::from_static(b":true}\n\n"),
    ];

    let (response, _) = relay_once(
        sse(200, frames(upstream_frames.clone())),
        "/v1/messages",
        "https://api.example.test",
        quick(),
        None,
    )
    .await;

    let RelayResponseBody::Stream(body) = response.body else {
        panic!("event-stream 必须走 Stream");
    };
    let got = drain(body).await;
    assert_eq!(payloads(&got), upstream_frames, "帧边界与字节都必须原样");
}

/// 缺陷 #6 的守护测试：流式中途失败必须以 `Err` 抵达 hyper。
///
/// 把 item 类型改回 `Infallible` + 一行 `warn!` 这条就红 ——
/// 客户端只会看到一次干净的 EOF。
#[tokio::test]
async fn a_mid_stream_upstream_failure_is_visible_to_the_client() {
    let good = Bytes::from_static(b"data: {\"delta\":\"hi\"}\n\n");
    let (response, _) = relay_once(
        sse(
            200,
            fallible(vec![
                Ok(good.clone()),
                Err(RelayError::Upstream("connection reset".to_owned())),
            ]),
        ),
        "/v1/messages",
        "https://api.example.test",
        quick(),
        None,
    )
    .await;

    let RelayResponseBody::Stream(body) = response.body else {
        panic!("event-stream 必须走 Stream");
    };
    let got = drain(body).await;
    assert_eq!(payloads(&got), vec![good]);
    assert!(
        got.last().expect("至少两项").is_err(),
        "中途失败不能表现为正常结束"
    );
}

/// 帧间空闲看门狗：超时也必须表现为 `Err`，而不是流正常结束。
#[tokio::test]
async fn a_stalled_stream_times_out_between_frames() {
    let first = Bytes::from_static(b"data: {}\n\n");
    let (source, _) = Hanging::new(Some(first.clone()));

    let (response, _) = relay_once(
        sse(200, source.boxed()),
        "/v1/messages",
        "https://api.example.test",
        quick(),
        None,
    )
    .await;

    let RelayResponseBody::Stream(body) = response.body else {
        panic!("event-stream 必须走 Stream");
    };
    let got = drain(body).await;
    assert_eq!(payloads(&got), vec![first]);
    assert!(matches!(got.last(), Some(Err(RelayError::Idle(_)))));
}

/// 审计 §2.4：客户端断开必须把取消传播到上游连接。
///
/// 上游 body 就住在流的 state 里（**没有 spawn**），所以丢弃响应体
/// 立刻就 drop 掉它。上游不会继续跑完，token 不会白烧。
#[tokio::test]
async fn dropping_the_response_body_drops_the_upstream_connection() {
    let (source, dropped) = Hanging::new(Some(Bytes::from_static(b"data: {}\n\n")));

    let (response, _) = relay_once(
        sse(200, source.boxed()),
        "/v1/messages",
        "https://api.example.test",
        quick(),
        None,
    )
    .await;

    let RelayResponseBody::Stream(mut body) = response.body else {
        panic!("event-stream 必须走 Stream");
    };
    drop(body.frame().await.expect("第一帧").expect("第一帧不是错误"));
    assert!(!dropped.load(Ordering::SeqCst), "还在读的时候不该被 drop");

    drop(body);
    assert!(dropped.load(Ordering::SeqCst), "客户端断开必须传播到上游");
}

/// 旁路 probe 看到的必须是**转发出去的**每一帧，顺序一致，
/// 且 `finish()` 恰好被调一次。
#[tokio::test]
async fn the_probe_observes_every_forwarded_frame_exactly_once() {
    let upstream_frames = vec![
        Bytes::from_static(b"data: a\n\n"),
        Bytes::from_static(b"data: b\n\n"),
        Bytes::from_static(b"data: c\n\n"),
    ];
    let log = ProbeLog::default();

    let (response, _) = relay_once(
        sse(200, frames(upstream_frames.clone())),
        "/v1/messages",
        "https://api.example.test",
        quick(),
        Some(Box::new(Recorder(log.clone()))),
    )
    .await;

    let RelayResponseBody::Stream(body) = response.body else {
        panic!("event-stream 必须走 Stream");
    };
    let got = drain(body).await;

    assert_eq!(payloads(&got), upstream_frames);
    assert_eq!(&*log.frames.lock().expect("测试锁"), &upstream_frames);
    assert_eq!(log.finishes.load(Ordering::SeqCst), 1);
}

/// 客户端中途断开时，`finish()` 仍然必须被调到 —— 否则计费拿不到已产出的
/// token，结算只能靠估算。
#[tokio::test]
async fn the_probe_finishes_even_when_the_client_disconnects() {
    let (source, _) = Hanging::new(Some(Bytes::from_static(b"data: a\n\n")));
    let log = ProbeLog::default();

    let (response, _) = relay_once(
        sse(200, source.boxed()),
        "/v1/messages",
        "https://api.example.test",
        quick(),
        Some(Box::new(Recorder(log.clone()))),
    )
    .await;

    let RelayResponseBody::Stream(mut body) = response.body else {
        panic!("event-stream 必须走 Stream");
    };
    drop(body.frame().await.expect("第一帧").expect("第一帧不是错误"));
    assert_eq!(log.finishes.load(Ordering::SeqCst), 0);

    drop(body);
    assert_eq!(log.finishes.load(Ordering::SeqCst), 1);
}

/// 非流式响应：probe 拿到的是**完整 body**（同一套解析器吃整块）。
#[tokio::test]
async fn a_non_streaming_response_hands_the_probe_the_whole_body() {
    let chunks = vec![
        Bytes::from_static(b"{\"usage\":"),
        Bytes::from_static(b"{\"prompt_tokens\":3}}"),
    ];
    let whole: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
    let log = ProbeLog::default();

    let (response, _) = relay_once(
        head(200, &[("content-type", "application/json")], frames(chunks)),
        "/v1/chat/completions",
        "https://api.example.test",
        RelayTimeouts::default(),
        Some(Box::new(Recorder(log.clone()))),
    )
    .await;

    let RelayResponseBody::Buffered(got) = response.body else {
        panic!("非 event-stream 必须走 Buffered");
    };
    assert_eq!(got.as_ref(), whole.as_slice());

    let seen = log.frames.lock().expect("测试锁");
    assert_eq!(seen.len(), 1, "非流式必须整块喂给 probe，不是逐帧");
    assert_eq!(seen[0].as_ref(), whole.as_slice());
    assert_eq!(log.finishes.load(Ordering::SeqCst), 1);
}

/// 出站 header 走的是同一套策略：入站凭证被剥掉，上游凭证被装上。
#[tokio::test]
async fn the_outbound_request_carries_the_upstream_credential_only() {
    let (fake, seen) = Fake::new(head(200, &[], frames(Vec::new())));
    let engine = RelayEngine::with_transport(fake, RelayOptions::default());

    let mut req = request("/v1/messages");
    req.headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_static("tenant-key"),
    );

    drop(
        engine
            .relay(
                req,
                &upstream("https://api.example.test", RelayTimeouts::default()),
                None,
            )
            .await
            .expect("上游给了响应"),
    );

    let sent = seen.lock().expect("测试锁");
    let sent = sent.as_ref().expect("传输被调用过");
    assert!(!sent.headers.contains_key("x-api-key"), "租户 key 跟出去了");
    assert!(sent.headers.contains_key(http::header::AUTHORIZATION));
}

/// 拼不出合法 URL 时是 `BadTarget`，且错误信息里**不含**拼接结果 ——
/// query 里可能有客户端凭证。
#[test]
fn a_broken_origin_is_reported_without_leaking_the_query() {
    let secret = "super-secret-key";
    let origin = Url::parse("data:text/plain,x").expect("合法的 data URL");
    let target =
        PathAndQuery::from_str(&format!("/v1/messages?key={secret}")).expect("测试 target");

    let err = super::upstream_url(&origin, &target).expect_err("data: 上拼不出 http URL");
    let RelayTransportError::BadTarget(message) = err else {
        panic!("必须是 BadTarget");
    };
    assert!(!message.contains(secret), "错误信息泄漏了客户端凭证");
}

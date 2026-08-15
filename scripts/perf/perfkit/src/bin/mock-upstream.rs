//! 独立 mock 上游。
//!
//! # 为什么不用 `crates/gw-proxy/src/testsupport/upstream.rs`
//!
//! 那里的 `FakeProvider` 是 **`Provider` trait 的进程内替身**：它直接返回
//! `ProviderResponse`，整条路径上没有 reqwest、没有 socket、没有 HTTP 编解码，
//! 也没有 SSE 的时间轴。用它测出来的"转发开销"会漏掉本任务最关心的三样：
//! reqwest 客户端与连接池、body 在 `Bytes`/`Vec<u8>` 之间的真实搬运、以及
//! 流式中继的首字节与 chunk 间抖动。另外它 `#[cfg(test)] pub(crate)`，
//! crate 外根本引用不到。
//!
//! 所以这里起一个真 HTTP 上游，被测网关通过 reqwest 真的连它。
//!
//! # 用法
//!
//! ```text
//! mock-upstream --port 18081
//! POST /v1/chat/completions?resp_bytes=2048
//! POST /v1/chat/completions?stream=1&chunks=500&chunk_bytes=1024&interval_us=1000
//! ```
//!
//! 场景参数走 query 而不是 body：query 会被网关原样透传给上游
//! （`ProviderRequest.query` → `chat_completions_endpoint`），而 body 在流式
//! 路径上会被 `ensure_include_usage` 重写，拿它当控制通道不可靠。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::Query;
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct MockParams {
    /// 非流式响应体的目标字节数。
    #[serde(default = "default_resp_bytes")]
    resp_bytes: usize,
    /// 置 1 走 SSE。
    #[serde(default)]
    stream: u8,
    #[serde(default = "default_chunks")]
    chunks: usize,
    #[serde(default = "default_chunk_bytes")]
    chunk_bytes: usize,
    #[serde(default = "default_interval_us")]
    interval_us: u64,
    /// 跨账号 failover 档：前 `fail_first` 次尝试一律回 429，之后才回 200。
    ///
    /// 判据是被测端每次尝试自己打上的 `x-perf-attempt` 头，**不是进程内计数器** ——
    /// 计数器会让并发和预热互相污染，而且重跑不可复现。0 = 从不失败（默认）。
    #[serde(default)]
    fail_first: u32,
}

fn default_resp_bytes() -> usize {
    2048
}
fn default_chunks() -> usize {
    500
}
fn default_chunk_bytes() -> usize {
    1024
}
fn default_interval_us() -> u64 {
    1000
}

/// 造一个指定大小、且带真 `usage` 信封的 OpenAI 风格响应体。
///
/// 必须带 usage：否则 `parse_openai_usage` 返回 `None`，网关会走 fallback
/// 结算路径，量到的就不是正常路径的开销。
fn unary_body(target: usize) -> Vec<u8> {
    const HEAD: &str = r#"{"id":"perf","object":"chat.completion","created":0,"model":"gpt-4o","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":""#;
    const TAIL: &str = r#""}}],"usage":{"prompt_tokens":256,"completion_tokens":512,"total_tokens":768}}"#;
    let overhead = HEAD.len() + TAIL.len();
    let pad = target.saturating_sub(overhead);
    let mut out = Vec::with_capacity(overhead + pad);
    out.extend_from_slice(HEAD.as_bytes());
    out.extend(std::iter::repeat_n(b'x', pad));
    out.extend_from_slice(TAIL.as_bytes());
    out
}

/// 一个 SSE data 事件，负载凑到 `target` 字节。
fn sse_chunk(index: usize, target: usize) -> Bytes {
    let head = format!(
        r#"data: {{"id":"perf","object":"chat.completion.chunk","model":"gpt-4o","choices":[{{"index":{index},"delta":{{"content":""#
    );
    let tail = "\"}}]}\n\n";
    let pad = target.saturating_sub(head.len() + tail.len());
    let mut out = Vec::with_capacity(head.len() + pad + tail.len());
    out.extend_from_slice(head.as_bytes());
    out.extend(std::iter::repeat_n(b'x', pad));
    out.extend_from_slice(tail.as_bytes());
    Bytes::from(out)
}

/// 终局 usage 事件 + `[DONE]`，喂 `parse_openai_stream_usage`。
fn sse_tail() -> Bytes {
    Bytes::from_static(
        b"data: {\"id\":\"perf\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[],\"usage\":{\"prompt_tokens\":256,\"completion_tokens\":512,\"total_tokens\":768}}\n\ndata: [DONE]\n\n",
    )
}

/// 上游 429 的错误体：带 `retry-after` 与 `x-ratelimit-*`，因为审计缺陷 #3 的
/// 判据就是"这些头有没有活着到客户端手里"。
fn rate_limited() -> Response {
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::RETRY_AFTER, "12")
        .header("x-ratelimit-remaining-requests", "0")
        .body(Body::from(
            br#"{"error":{"message":"perf: simulated upstream rate limit","type":"rate_limit_error"}}"#
                .as_slice(),
        ))
        .expect("429 response builds")
}

/// 第一发请求到达时把 HTTP 版本打出来，**只打一次**。
///
/// TLS + h2 档唯一的自检：`PERF_TLS_CERT` 设了、证书也装上了、连接也建起来了，
/// 都**不**保证 ALPN 真的协商到了 h2 —— 任何一环退化成 http/1.1，这一档量的就是
/// "TLS over h1"，而结论会被写成 "h2 下差值不失真"。那是编数字。
/// 看 `/tmp/perf-mock-tls.log` 的这一行，写着 `HTTP/2.0` 才算数。
static VERSION_LOGGED: AtomicBool = AtomicBool::new(false);

async fn chat_completions(
    Query(p): Query<MockParams>,
    version: axum::http::Version,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    if !VERSION_LOGGED.swap(true, Ordering::Relaxed) {
        eprintln!("mock-upstream: first request arrived over {version:?}");
    }
    // 真的把请求体读完（`body: Bytes` 已经做到了），这是上游必须付的成本，
    // 让 floor 与 gateway 两侧对称。
    let _ = body.len();

    if p.fail_first > 0 {
        let attempt: u32 = headers
            .get("x-perf-attempt")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if attempt < p.fail_first {
            return rate_limited();
        }
    }

    if p.stream == 1 {
        let chunk_bytes = p.chunk_bytes;
        let total = p.chunks;
        let interval = Duration::from_micros(p.interval_us);
        let stream = futures_util::stream::unfold(0usize, move |i| async move {
            if i > total {
                return None;
            }
            // 第一个 chunk 不等待：TTFB 要量的是网关的开销，不是 mock 的定时器。
            if i > 0 && !interval.is_zero() {
                tokio::time::sleep(interval).await;
            }
            let item = if i == total {
                sse_tail()
            } else {
                sse_chunk(i, chunk_bytes)
            };
            Some((Ok::<Bytes, std::io::Error>(item), i + 1))
        });
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from_stream(stream))
            .expect("sse response builds");
    }

    let payload = unary_body(p.resp_bytes);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload))
        .expect("unary response builds")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(18081);
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();

    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/health", axum::routing::get(|| async { "ok" }));

    // TLS + h2 档：`PERF_TLS_CERT` / `PERF_TLS_KEY` 都设了才走 https。
    // 基线全程明文 HTTP/1.1（§1.4），生产上游走 h2 —— 这条分支就是
    // §5.2「上线前必须补一档 TLS+h2 的对照」的落点。ALPN 由 axum-server
    // 设成 `["h2", "http/1.1"]`，所以 reqwest 会协商到 h2。
    if let (Ok(cert), Ok(key)) = (
        std::env::var("PERF_TLS_CERT"),
        std::env::var("PERF_TLS_KEY"),
    ) {
        // rustls 0.23 要求进程显式选一个 crypto provider（reqwest 平时自己装，
        // 这里是我们自己起服务端，得自己来）。
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert, &key).await?;
        eprintln!("mock-upstream listening on https://{addr} (ALPN h2/http1.1)");
        axum_server::bind_rustls(addr, config)
            .serve(app.into_make_service())
            .await?;
        return Ok(());
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("mock-upstream listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

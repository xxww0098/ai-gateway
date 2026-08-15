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

async fn chat_completions(Query(p): Query<MockParams>, body: Bytes) -> Response {
    // 真的把请求体读完（`body: Bytes` 已经做到了），这是上游必须付的成本，
    // 让 floor 与 gateway 两侧对称。
    let _ = body.len();

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

    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("mock-upstream listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

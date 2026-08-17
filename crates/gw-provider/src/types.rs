//! Shared provider contract. COORDINATOR-OWNED: both provider workers compile
//! against this, so propose changes via `orca orchestration ask` instead of
//! editing it.

use gw_authcore::AuthRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A normalised upstream request. `payload` is the raw JSON body in the
/// *caller's* dialect; each provider translates it to its own wire format.
#[derive(Debug, Clone, Default)]
pub struct ProviderRequest {
    pub model: String,
    /// WAVE 3 接缝：`Vec<u8>` → [`bytes::Bytes`]。
    ///
    /// `Bytes::clone` 是 refcount，所以 failover 重试不再重复 memcpy 整个请求体。
    /// 实测代价见 `docs/relay-perf-baseline.md` §3 第 1 条：分配字节达载荷的
    /// **4.20 倍**（下界 1.40 倍），一个 900 KB 的请求在 3 次 failover 下要
    /// memcpy 约 5.4 MB。
    pub payload: bytes::Bytes,
    pub stream: bool,
    pub metadata: HashMap<String, String>,

    /// Inbound headers to forward upstream. Pass through
    /// [`copy_outbound_headers`] — never copy this map verbatim.
    pub headers: http::HeaderMap,

    /// Inbound query parameters, **appended** to the provider's endpoint. A
    /// `Vec` of pairs rather than a map because order and duplicate keys are
    /// both significant.
    /// A provider may still `set` a key it owns afterwards — Gemini and Vertex
    /// force `alt=sse` on streaming that way.
    pub query: Vec<(String, String)>,
}

/// Headers that must never be forwarded to an upstream: the hop-by-hop set plus
/// `authorization` (each provider supplies its own credential). Comparison is
/// case-insensitive on the trimmed name.
pub fn is_skipped_proxy_header(name: &str) -> bool {
    // HeaderName 已经是小写，但本函数吃 &str，大小写都要认。
    // 以前 to_ascii_lowercase() 每个头分配一次 String，热路径上每个
    // 入站头都走这里。
    let name = name.trim();
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("content-length")
        || name.eq_ignore_ascii_case("host")
        || name.eq_ignore_ascii_case("keep-alive")
        || name.eq_ignore_ascii_case("proxy-authenticate")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("te")
        || name.eq_ignore_ascii_case("trailer")
        || name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("upgrade")
}

/// Copies forwardable inbound headers onto an outbound request.
///
/// Appends rather than replaces, so multi-valued headers survive intact.
///
/// Lives here, next to the trait, because all five providers call it — a second
/// copy in a provider module would be a second place for the denylist to drift.
pub fn copy_outbound_headers(dst: &mut http::HeaderMap, src: &http::HeaderMap) {
    for name in src.keys() {
        if is_skipped_proxy_header(name.as_str()) {
            continue;
        }
        for value in src.get_all(name) {
            dst.append(name.clone(), value.clone());
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub status: u16,
    pub headers: http::HeaderMap,
    /// WAVE 3 接缝：`Vec<u8>` → [`bytes::Bytes`]。
    ///
    /// 回程唯一那次拷贝就是被这个字段类型逼出来的：五个 executor 都写
    /// `response.bytes().await?.to_vec()`，把 reqwest 已经持有的 `Bytes`
    /// 复制进 `Vec`。末端 `Body::from(Vec<u8>)` 反而是零拷贝的。
    pub body: bytes::Bytes,
    pub usage: Option<UsageRecord>,
}

/// One server-sent event / chunk of a streamed response.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    Payload(bytes::Bytes),
    Usage(UsageRecord),
    /// The relay failed part-way through a response that had already started.
    /// Folding the upstream status into the chunk keeps the two halves of one
    /// failure together.
    ///
    /// `status` is the upstream response's own status, the same value
    /// [`ProviderError::Upstream`] carries. Every error this crate emits is
    /// raised while relaying a response whose headers were already read, so it
    /// is always `Some`; `None` is reserved for a failure with no upstream
    /// status behind it at all.
    ///
    /// Note this is *not* the channel-health signal for a rejected request: an
    /// upstream that answers 4xx/5xx never reaches the streaming path, it comes
    /// back as [`ProviderError::Upstream`].
    Error {
        status: Option<u16>,
        message: String,
    },
}

/// A streamed upstream response: the status line and headers alongside the
/// chunks themselves.
///
/// `status` is carried explicitly because our callers never see the underlying
/// response — without it a relay can only guess, and guessing means hardcoding
/// `200`.
pub struct StreamResponse {
    pub status: u16,
    pub headers: http::HeaderMap,
    pub chunks: std::pin::Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send>>,
}

/// Prints everything but the chunks — a stream has no observable state to show,
/// and the status/header pair is what a relay traces.
impl std::fmt::Debug for StreamResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

/// Token accounting extracted from an upstream response. `None` fields mean the
/// upstream omitted the value — the billing pipeline distinguishes "absent"
/// from "zero" (strict-usage-metadata mode depends on it).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageRecord {
    pub model: String,
    pub provider: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("upstream {status}: {body}")]
    Upstream { status: u16, body: String },
    #[error("credential unusable: {0}")]
    Credential(String),
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Every upstream implements this.
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// Stable provider key: "openai", "claude", "gemini", "vertex", "codex".
    fn name(&self) -> &'static str;

    async fn execute(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError>;

    /// Returns the upstream's headers *and* its chunks. A non-2xx never
    /// becomes a stream: it is [`ProviderError::Upstream`] instead.
    async fn execute_stream(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<StreamResponse, ProviderError>;

    /// Refresh an expiring OAuth credential; returns the updated record.
    async fn refresh(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError>;

    async fn count_tokens(
        &self,
        _auth: &AuthRecord,
        _req: ProviderRequest,
    ) -> Result<i64, ProviderError> {
        Ok(0)
    }
}

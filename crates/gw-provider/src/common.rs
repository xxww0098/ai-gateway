//! Plumbing shared by every provider executor.
//!
//! OWNER: worker `provider-openai` (but written to be called by all five
//! executors — `claude.rs` / `gemini.rs` / `vertex.rs` are welcome here).
//!
//! Header forwarding lives in [`crate::types`] (`copy_outbound_headers` /
//! `is_skipped_proxy_header`), next to the trait it serves.

use crate::streambuf::StreamUsageProbe;
use crate::types::{ProviderError, ProviderRequest, StreamChunk, StreamResponse};
use crate::usage::UsageTokens;
use bytes::Bytes;
use futures::Stream;
use gw_relay::endpoint::include_usage::Spliced;
use gw_relay::endpoint::{IncludeUsagePolicy, RequestSpec, splice_include_usage};
use gw_relay::{Surface, UpstreamDialect};
use serde_json::Value;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll, ready};
use std::time::Duration;

/// Shared provider identifiers.
pub const PROVIDER_OPENAI: &str = "openai";
/// See [`PROVIDER_OPENAI`].
pub const PROVIDER_CLAUDE: &str = "claude";
/// See [`PROVIDER_OPENAI`].
pub const PROVIDER_GEMINI: &str = "gemini";
/// See [`PROVIDER_OPENAI`].
pub const PROVIDER_CODEX: &str = "codex";
/// See [`PROVIDER_OPENAI`].
pub const PROVIDER_VERTEX: &str = "vertex";
/// xAI Grok OAuth executor. Tokens are also usable via the OpenAI-compatible
/// executor when the record carries `base_url=https://api.x.ai/v1`.
pub const PROVIDER_XAI: &str = "xai";
/// Kiro / AWS Builder ID. The 15-cell relay matrix has no Kiro cell, so this
/// executor is registered for refresh and for an operator who routes to it
/// explicitly; `/v1` candidates never include `"kiro"` today.
pub const PROVIDER_KIRO: &str = "kiro";

/// Whole-request cap for non-streaming calls.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Bounds how long a streaming read may stall — receiving no bytes from
/// upstream — before the request is aborted.
pub const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// [`ProviderRequest::metadata`] key holding the client-facing model name when
/// the router already rewrote [`ProviderRequest::model`] to an upstream alias.
///
/// `gw-proxy` is the only writer.
pub const REQUESTED_MODEL_METADATA_KEY: &str = "requested_model";

/// [`ProviderRequest::metadata`] key holding the **inbound path** this request
/// arrived on（`/v1/chat/completions` / `/v1/responses` / `/v1/messages`）。
///
/// # 为什么需要它：根除缺陷 #1（S1）的另一半
///
/// `docs/relay-passthrough-audit.md` 的缺陷 #1 是「端点由 provider 猜，而不是由
/// 入口决定」。`gw-proxy` 侧已经改成按入口派发，但 executor 这一侧此前**只会**
/// 构造 `{base}/v1/chat/completions` —— 于是 `POST /v1/responses` 的 Responses
/// 形状 body 被发到 Chat Completions 端点，上游必 400，三个保留入口之一 100% 不可用。
///
/// 端点**不许由 provider 名或 model 名猜**（那正是缺陷 #1 的成因），所以入口必须
/// 从上面带下来。这里沿用既有约定：`gw-proxy` 是唯一写入方，与
/// [`REQUESTED_MODEL_METADATA_KEY`] 同一条通路。
///
/// 存的是**路径本身**而不是一个新造的枚举字符串：路径 → 入口的映射
/// [`Surface::from_path`] 已经在 `gw-relay` 里声明过一次了，再造一套词汇就是
/// 第二处声明（规范：一个概念只声明一处）。
pub const SURFACE_PATH_METADATA_KEY: &str = "surface_path";

/// 这个请求走的是哪个入口。
///
/// 键缺失或路径不认识时回落到 [`Surface::OpenAiCompletions`] —— 那正是本键存在
/// 之前的既有行为，所以对还没开始写这个键的调用方，本函数是**严格加性**的、
/// 不改变任何现有行为。
#[must_use]
pub fn request_surface(req: &ProviderRequest) -> Surface {
    req.metadata
        .get(SURFACE_PATH_METADATA_KEY)
        .map(|path| path.trim())
        .and_then(Surface::from_path)
        .unwrap_or(Surface::OpenAiCompletions)
}

/// Provider-specific upstream settings.
///
/// Deliberately local rather than a `gw_config` re-export: the executors need
/// exactly these three fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderConfig {
    pub base_url: String,
    pub api_key: String,
    pub enabled: bool,
}

/// Connection-pool settings shared by every executor client.
///
/// Idle conns are capped at 100/host because the default of 2 forces repeated
/// TCP+TLS handshakes on a relay's hot path. `reqwest` reads `HTTP_PROXY` /
/// `HTTPS_PROXY` / `NO_PROXY` from the environment by default.
fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .pool_max_idle_per_host(100)
        .pool_idle_timeout(Duration::from_secs(90))
}

fn build_or_default(builder: reqwest::ClientBuilder) -> reqwest::Client {
    builder.build().unwrap_or_else(|err| {
        tracing::warn!(error = %err, "falling back to the default reqwest client");
        reqwest::Client::new()
    })
}

/// Process-wide HTTP client with **no** whole-request timeout, and therefore a
/// process-wide connection pool.
///
/// A whole-request timeout also bounds *reading the response body*, so any
/// non-zero value silently truncates long-but-healthy streams (an extended o1 /
/// Claude answer). Stall protection comes from [`with_stream_idle_timeout`]
/// instead.
///
/// Callers that want a bounded non-streaming request can either use
/// [`new_http_client`] or scope the cap per request with
/// `RequestBuilder::timeout` — the latter keeps everything on this one pool
/// (`reqwest` has no API for sharing a pool between two `Client`s).
pub fn shared_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| build_or_default(client_builder()))
        .clone()
}

/// Alias for [`shared_client`]: a client with no whole-request timeout.
pub fn streaming_client() -> reqwest::Client {
    shared_client()
}

/// An executor HTTP client whose `timeout` caps the whole request, including
/// reading the response body. For non-streaming calls only.
///
/// This cannot reuse [`shared_client`]'s pool — `reqwest` binds the pool to the
/// `Client` — so prefer `shared_client().post(..).timeout(..)` when the timeout
/// is the only reason you would build a second client.
pub fn new_http_client(timeout: Duration) -> reqwest::Client {
    build_or_default(client_builder().timeout(timeout))
}

/// Turns a configured timeout in seconds into a [`Duration`], falling back to
/// [`DEFAULT_TIMEOUT`] for non-positive values.
#[must_use]
pub fn resolve_timeout(timeout_seconds: i64) -> Duration {
    if timeout_seconds > 0 {
        Duration::from_secs(timeout_seconds as u64)
    } else {
        DEFAULT_TIMEOUT
    }
}

/// Estimates token count from a payload byte size.
#[must_use]
pub fn approximate_tokens_from_bytes(size: usize) -> i64 {
    if size == 0 {
        return 0;
    }
    size.div_ceil(4) as i64
}

/// Clips a failure payload so a persisted error body stays bounded. 4 KiB is
/// more than enough for provider error envelopes.
#[must_use]
pub fn truncate_failure_body(payload: &[u8]) -> String {
    const MAX: usize = 4 * 1024;
    let end = payload.len().min(MAX);
    // Never split a UTF-8 code point; `from_utf8_lossy` also tolerates the
    // non-UTF-8 bodies some upstreams return on infrastructure errors.
    String::from_utf8_lossy(&payload[..end]).into_owned()
}

/// Resolves the upstream-facing model name for a request.
///
/// Prefers the translated [`ProviderRequest::model`] and falls back to the
/// [`REQUESTED_MODEL_METADATA_KEY`] hint the router stored.
#[must_use]
pub fn requested_model(req: &ProviderRequest) -> &str {
    let model = req.model.trim();
    if !model.is_empty() {
        return model;
    }
    req.metadata
        .get(REQUESTED_MODEL_METADATA_KEY)
        .map(|v| v.trim())
        .unwrap_or("")
}

// --- credential lookup helpers ----------------------------------------------

/// Reads `values[key]` as a string, coercing scalar JSON values. Two
/// deliberate deviations, both narrowing what counts as a credential:
///
/// - Absent is `None` (rather than an empty string that every caller had to
///   compare against), so the check cannot be forgotten.
/// - A JSON `null` is absent (rather than the literal string `"null"`), so it
///   cannot be accepted as a usable credential.
#[must_use]
pub fn string_from_map(values: &Value, key: &str) -> Option<String> {
    let rendered = match values.get(key)? {
        Value::Null => return None,
        Value::String(s) => s.trim().to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            // An integral float is rendered as an integer.
            Some(f) if n.as_i64().is_none() && n.as_u64().is_none() && f.fract() == 0.0 => {
                format!("{}", f as i64)
            }
            _ => n.to_string(),
        },
        other => serde_json::to_string(other).ok()?,
    };
    (!rendered.is_empty()).then_some(rendered)
}

/// Reads `values[parent][key]` as a string, tolerating a `parent` that is a
/// nested object *or* a JSON document embedded in a string.
#[must_use]
pub fn nested_string(values: &Value, parent: &str, key: &str) -> Option<String> {
    match values.get(parent)? {
        v @ Value::Object(_) => string_from_map(v, key),
        Value::String(raw) => {
            let parsed: Value = serde_json::from_str(raw.trim()).ok()?;
            string_from_map(&parsed, key)
        }
        _ => None,
    }
}

// --- include_usage rewrite ---------------------------------------------------

/// 让 OpenAI 兼容的流式请求带上终局 usage 信封 —— **定点字节插入**，
/// 根除审计缺陷 #4（`docs/relay-passthrough-audit.md`）。
///
/// # 这里原来错在哪
///
/// 老实现把整个 body `from_slice::<Value>` 再 `to_vec` 回去，三个后果都是实测的：
///
/// | 后果 | 成因 |
/// | --- | --- |
/// | **递归重排所有键序** | `serde_json` 未开 `preserve_order`，`Map` 就是 `BTreeMap` |
/// | 客户端显式写的 `include_usage: false` 被**静默翻成 `true`** | 无条件 `insert` |
/// | `"seed": 12345678901234567890` → `1.2345678901234568e+22` | 大整数落进 `f64`，可复现性没了 |
/// | 256 KiB 流式请求 **+104.8 µs**（0.409 µs/KiB） | 建整棵 `Value` 树再重新序列化 |
///
/// # 现在怎么做
///
/// 直接调 [`gw_relay::endpoint::splice_include_usage`]（wave 2 已交付、自带测试）。
/// 它在最外层 `{` 之后插入一小段字节，其余字节 **100% 原样**，返回的第二段是原
/// [`Bytes`] 的零拷贝切片。**不反序列化、不重序列化、不重排键、不动数字格式。**
///
/// 返回 `None` = **一个字节都不动**。四个条件任一不满足即返回 `None`：
/// 顶层不是 JSON 对象、body 没写 `stream: true`、客户端已经写了 `stream_options`
/// （写了就尊重它 —— 上面那条 `false` 被翻成 `true` 的缺陷在这里自然消失）、
/// 或者 body 空。
///
/// 纵深防御保留：即使调用方的 [`ProviderRequest::stream`] 为真，仍然要求 body 自己
/// 写了 `stream: true`，所以一个设错的标志位不可能把 `include_usage` 塞进非流式请求。
///
/// # 策略
///
/// 这里钉死 [`IncludeUsagePolicy::Force`]，即今天的行为。
/// [`IncludeUsagePolicy::RespectClient`]（一个字节都不碰，接受 usage 缺失并落
/// fallback 计费）是部署方的选择，开关该挂在 `gw-proxy` 的配置上而不是这里 ——
/// executor 拿不到部署策略。
#[must_use]
pub fn ensure_include_usage(payload: &Bytes, surface: Surface) -> Option<Spliced> {
    let spec = RequestSpec::parse(surface, Some(payload));
    splice_include_usage(
        payload,
        &spec,
        upstream_dialect(surface),
        IncludeUsagePolicy::Force,
    )
}

/// 入口 → 上游方言。**入口 B 绝不能被插 `stream_options`**：Responses API 不认识
/// 这个键，塞进去上游直接 400。今天正是这么坏的 —— 缺陷 #1（打错端点）叠加缺陷 #4
/// （还塞 `stream_options`），入口 B 双重不可用。
///
/// 这里只需要区分本 crate 的 OpenAI 兼容两支；Anthropic 入口不走这条路
/// （`claude.rs` 从不调 [`ensure_include_usage`]），映射到它自己的方言即可。
fn upstream_dialect(surface: Surface) -> UpstreamDialect {
    match surface {
        Surface::OpenAiCompletions => UpstreamDialect::OpenAiChat,
        Surface::OpenAiResponses => UpstreamDialect::OpenAiResponses,
        Surface::AnthropicMessages => UpstreamDialect::AnthropicMessages,
    }
}

/// 把请求体挂到 reqwest 请求上，**零全量拷贝**。
///
/// 两条路都不复制 body：
///
/// - 没有插入（`spliced` 为 `None`）：`reqwest::Body::from(Bytes)` 是接管，不是复制。
/// - 有插入：两段 [`Bytes`] 作为一个两帧的流交出去，并**显式写死
///   `content-length`**。hyper 对用户自己设过的 `content-length` 是尊重的
///   （`proto/h1/role.rs` 的 `Client::set_length`：`existing_con_len` 优先），
///   所以框架仍然是 `Content-Length`，**不会退化成 `Transfer-Encoding: chunked`** ——
///   透传优先，不该因为要计费就改掉发给上游的报文框架。
///
/// `content-length` 由网关重算是既定事实：入站的那一个在
/// [`crate::types::is_skipped_proxy_header`] 的黑名单里，本来就不会被转发
/// （`docs/relay-passthrough-audit.md` §4.1 条 6）。
pub fn attach_body(
    builder: reqwest::RequestBuilder,
    payload: &Bytes,
    spliced: Option<Spliced>,
) -> reqwest::RequestBuilder {
    let Some(spliced) = spliced else {
        return builder.body(payload.clone());
    };
    let length = spliced.len();
    let frames = futures_util::stream::iter([
        Ok::<Bytes, std::io::Error>(spliced.prefix),
        Ok(spliced.rest),
    ]);
    builder
        .header(http::header::CONTENT_LENGTH, length)
        .body(reqwest::Body::wrap_stream(frames))
}

// --- endpoint construction ---------------------------------------------------

/// The two endpoint leaves an OpenAI-compatible upstream exposes, in the order
/// [`openai_compatible_endpoint`] strips them off a configured base URL.
const OPENAI_LEAVES: [&str; 2] = [OPENAI_CHAT_LEAF, OPENAI_RESPONSES_LEAF];
const OPENAI_CHAT_LEAF: &str = "chat/completions";
const OPENAI_RESPONSES_LEAF: &str = "responses";

/// Builds the `chat/completions` endpoint for an OpenAI-compatible base URL and
/// appends the inbound query parameters.
///
/// The base URL may already point at the full path, at `/v1`, or at the bare
/// origin; all three converge on the same endpoint.
pub fn chat_completions_endpoint(
    base_url: &str,
    query: &[(String, String)],
) -> Result<String, ProviderError> {
    openai_compatible_endpoint(base_url, OPENAI_CHAT_LEAF, query)
}

/// Builds the `responses` endpoint for an OpenAI-compatible base URL.
///
/// # 根除缺陷 #1（S1）的另一半
///
/// `docs/relay-passthrough-audit.md` 的缺陷 #1 是「上游端点由 provider 猜」。
/// 在此之前 executor **只会**构造 `chat/completions`，于是入口 B
/// （`POST /v1/responses`）的 Responses 形状 body 被发到 Chat Completions 端点，
/// 上游必 400 —— 三个保留入口之一 100% 不可用。
///
/// `docs/relay-surface-plan.md` §3.6 的 B×openai / B×codex 两格是**直通**格：
/// Responses 是 Codex 的原生协议，不需要任何转义，缺的只是把端点拼对。
///
/// 打哪个端点由**入口**决定（[`request_surface`]），不由 provider 名或 model 名猜。
pub fn responses_endpoint(
    base_url: &str,
    query: &[(String, String)],
) -> Result<String, ProviderError> {
    openai_compatible_endpoint(base_url, OPENAI_RESPONSES_LEAF, query)
}

/// Shared body of the two endpoint builders.
///
/// 一份 base 归一化规则，两个端点共用 —— 分成两份抄的话，下次改 base 容错
/// （比如再多认一种形态）就只会改到其中一份。
///
/// 归一化分两步：先把 base 已经带上的端点尾巴剥掉，再按 `/v1` 边界拼。
/// 剥这一步是新加的：一个配置成 `…/v1/chat/completions` 的 base 此前只能服务
/// chat，现在同一个 base 也能拼出 `…/v1/responses`，而 chat 的结果逐字不变。
fn openai_compatible_endpoint(
    base_url: &str,
    leaf: &str,
    query: &[(String, String)],
) -> Result<String, ProviderError> {
    let base = base_url.trim().trim_end_matches('/');
    // Validate the BASE, not the assembled endpoint. `url` is lenient about
    // slashes for special schemes, so a hostless `https://` would otherwise
    // become `https:/v1/chat/completions` and re-parse with `v1` as the host.
    let base_parsed = url::Url::parse(base)
        .map_err(|err| ProviderError::Other(anyhow::anyhow!("invalid base_url: {err}")))?;
    if base_parsed.host_str().unwrap_or_default().is_empty() {
        return Err(ProviderError::Other(anyhow::anyhow!(
            "invalid base_url: missing host"
        )));
    }

    // 只剥 `/v1/`-锚定的形态：一个恰好叫 `…/responses` 的 origin 不该被误伤。
    let base = OPENAI_LEAVES
        .iter()
        .find_map(|known| base.strip_suffix(&format!("/v1/{known}")))
        .map_or(base, |origin| origin.trim_end_matches('/'));

    let endpoint = if base.ends_with("/v1") {
        format!("{base}/{leaf}")
    } else {
        format!("{base}/v1/{leaf}")
    };
    let mut parsed = url::Url::parse(&endpoint)
        .map_err(|err| ProviderError::Other(anyhow::anyhow!("invalid base_url: {err}")))?;
    if !query.is_empty() {
        let mut pairs = parsed.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
        drop(pairs);
    }
    Ok(parsed.to_string())
}

// --- stream idle watchdog ----------------------------------------------------

/// The upstream stalled for longer than the idle window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("upstream stream idle timeout")]
pub struct StreamIdleElapsed;

/// Wraps a stream so that a gap longer than `idle` between items terminates it
/// with [`StreamIdleElapsed`].
///
/// A stream *is* the sequence of reads, so the reset is implicit — every
/// yielded item restarts the window, and a zero-length item cannot occur
/// (empty reads are not progress).
///
/// A zero `idle` disables the watchdog and passes the inner stream through
/// untouched.
///
/// # Why this is a `Stream` and not `unfold` + `timeout`
///
/// The previous implementation was `unfold` around `tokio::time::timeout(idle,
/// inner.next())`. That builds a new `Sleep` future on **every poll**, including
/// the case where the next chunk is already ready. On a burst SSE stream that
/// is one timer per chunk for a 300 s window that never fires.
///
/// This type polls the inner stream first and only arms a reusable [`Sleep`]
/// when that poll returns `Pending`. Ready chunks therefore do not touch the
/// timer at all.
pub fn with_stream_idle_timeout<S>(
    inner: S,
    idle: Duration,
) -> impl Stream<Item = Result<S::Item, StreamIdleElapsed>> + Send
where
    S: Stream + Send + 'static,
    S::Item: Send,
{
    IdleTimeout::new(inner, idle)
}

/// See [`with_stream_idle_timeout`].
struct IdleTimeout<S> {
    inner: Pin<Box<S>>,
    /// Reused across gaps so a long stream pays one timer allocation, not one
    /// per chunk. `None` until the first `Pending`.
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
    idle: Duration,
    expired: bool,
    /// `true` while `sleep` is counting the current gap. Cleared when an item
    /// arrives so the next `Pending` restarts the window from now.
    armed: bool,
}

impl<S> IdleTimeout<S>
where
    S: Stream,
{
    fn new(inner: S, idle: Duration) -> Self {
        Self {
            inner: Box::pin(inner),
            sleep: None,
            idle,
            expired: false,
            armed: false,
        }
    }

    fn poll_idle(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let deadline = tokio::time::Instant::now() + self.idle;
        if let Some(sleep) = self.sleep.as_mut() {
            if !self.armed {
                sleep.as_mut().reset(deadline);
                self.armed = true;
            }
            return sleep.as_mut().poll(cx);
        }
        let mut sleep = Box::pin(tokio::time::sleep_until(deadline));
        let poll = sleep.as_mut().poll(cx);
        self.sleep = Some(sleep);
        self.armed = true;
        poll
    }
}

impl<S> Stream for IdleTimeout<S>
where
    S: Stream,
{
    type Item = Result<S::Item, StreamIdleElapsed>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.expired {
            return Poll::Ready(None);
        }
        if this.idle.is_zero() {
            return this.inner.as_mut().poll_next(cx).map(|item| item.map(Ok));
        }
        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(item)) => {
                this.armed = false;
                Poll::Ready(Some(Ok(item)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => match this.poll_idle(cx) {
                Poll::Ready(()) => {
                    this.expired = true;
                    this.armed = false;
                    Poll::Ready(Some(Err(StreamIdleElapsed)))
                }
                Poll::Pending => Poll::Pending,
            },
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

// --- streamed usage accumulation ---------------------------------------------

/// Assembles a [`StreamResponse`] from an accepted upstream response.
///
/// The single reason this is a function rather than three lines inlined five
/// times: the header clone **must** happen before the body is consumed, and
/// `bytes_stream()` takes the response by value. Doing it here makes that
/// ordering unrepeatable-by-accident, and gives the five executors one tested
/// seam. `build` receives the response and the status it reported, so error
/// chunks can carry the status the relay actually ran under.
///
/// Callers must have rejected a failing status already — see
/// [`crate::types::Provider::execute_stream`].
pub fn stream_response<F>(response: reqwest::Response, build: F) -> StreamResponse
where
    F: FnOnce(reqwest::Response, u16) -> Pin<Box<dyn Stream<Item = StreamChunk> + Send>>,
{
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    StreamResponse {
        status,
        headers,
        chunks: build(response, status),
    }
}

enum UsagePhase {
    Streaming,
    Trailing,
    Done,
}

/// The per-stream constants [`usage_stream`] threads through every chunk.
struct UsageStreamMeta {
    model: String,
    provider: &'static str,
    /// Status of the response being relayed, stamped onto any error chunk.
    upstream_status: u16,
}

/// One stream: idle watchdog + line probe + [`StreamChunk`] mapping.
///
/// Replaces two stacked `unfold` state machines. State stays in place across
/// polls — the probe's `pending` line buffer is not moved on every chunk —
/// and the idle timer is the [`IdleTimeout`] above, so a ready chunk does not
/// allocate a `Sleep`.
struct UsageRelay<S> {
    inner: IdleTimeout<S>,
    probe: StreamUsageProbe,
    meta: UsageStreamMeta,
    phase: UsagePhase,
}

impl<S> Stream for UsageRelay<S>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
{
    type Item = StreamChunk;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match this.phase {
                UsagePhase::Done => return Poll::Ready(None),
                UsagePhase::Trailing => {
                    this.phase = UsagePhase::Done;
                    return Poll::Ready(
                        this.probe
                            .finish()
                            .map(|tokens| tokens.to_record(&this.meta.model, this.meta.provider))
                            .map(StreamChunk::Usage),
                    );
                }
                UsagePhase::Streaming => match ready!(Pin::new(&mut this.inner).poll_next(cx)) {
                    Some(Ok(Ok(chunk))) => {
                        this.probe.observe(&chunk);
                        return Poll::Ready(Some(StreamChunk::Payload(chunk)));
                    }
                    // Transport error mid-stream: surface it, then still try
                    // to bill whatever usage the truncated body carried.
                    Some(Ok(Err(err))) => {
                        this.phase = UsagePhase::Trailing;
                        return Poll::Ready(Some(StreamChunk::Error {
                            status: Some(this.meta.upstream_status),
                            message: err.to_string(),
                        }));
                    }
                    Some(Err(elapsed)) => {
                        this.phase = UsagePhase::Trailing;
                        return Poll::Ready(Some(StreamChunk::Error {
                            status: Some(this.meta.upstream_status),
                            message: elapsed.to_string(),
                        }));
                    }
                    None => {
                        this.phase = UsagePhase::Trailing;
                    }
                },
            }
        }
    }
}

/// Turns a raw upstream byte stream into the [`StreamChunk`] sequence the
/// [`crate::types::Provider`] trait promises: every payload is forwarded
/// verbatim, and a terminal [`StreamChunk::Usage`] is emitted iff `parse` found
/// a usage envelope in the accumulated body.
///
/// Two behavioural notes:
///
/// - The *absence* of a `StreamChunk::Usage` carries the "nothing parsed"
///   signal, so `strict_usage_metadata_mode` still has its "absent, not zero"
///   input.
/// - Memory stays at `O(one line)` regardless of body length, and no stream
///   byte is copied — see [`StreamUsageProbe`] (perf hotspot #2).
///
/// `parse` is handed **one line at a time**, not the whole body. The four
/// provider scanners behave identically on a single line, so none of them
/// changed when the head/tail window was replaced.
pub fn usage_stream<S>(
    body: S,
    idle: Duration,
    model: String,
    provider: &'static str,
    parse: fn(&[u8]) -> Option<UsageTokens>,
    upstream_status: u16,
) -> Pin<Box<dyn Stream<Item = StreamChunk> + Send>>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
{
    Box::pin(UsageRelay {
        inner: IdleTimeout::new(body, idle),
        probe: StreamUsageProbe::new(parse),
        meta: UsageStreamMeta {
            model,
            provider,
            upstream_status,
        },
        phase: UsagePhase::Streaming,
    })
}

#[cfg(test)]
#[path = "common_tests.rs"]
mod tests;

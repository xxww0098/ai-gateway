//! 字节契约 —— wave 2 的四个 worker 全部编译against 这个文件。
//!
//! **COORDINATOR-OWNED**：要改这里的任何签名，走
//! `orca orchestration ask --question "..." --json`，不要自己动手。
//! 四个 worker 同时依赖它，任何一处静默修改都会让其他三个的编译期假设失效。
//!
//! # 核心原则
//!
//! > **中继层不认识 JSON。** 它只认识 [`Bytes`]、[`HeaderMap`]、[`StatusCode`]
//! > 和帧序列。任何需要看懂内容的东西（计费、协议转义）都挂在**旁路**，
//! > 从中继拿只读视图，永远不往回写路径塞东西。
//!
//! 推论三条，每一条都对应 `docs/relay-passthrough-audit.md` 里一个实锤缺陷：
//!
//! 1. **上游的 4xx/5xx 不是错误，它是一个响应。**[`Result::Err`] 只留给"连不上"。
//!    （缺陷 #3：`ProviderError::Upstream` 只带 `status` + `String`，把上游的
//!    `retry-after` / `x-ratelimit-*` / `request-id` 全丢在地上，Anthropic SDK
//!    读不到 `retry-after` 就退回 `0.5 * 2^n`，在上游明确要求等 12 秒时 0.5 秒
//!    就重试，把限流放大成雪崩。）
//! 2. **没有 `String`。** 请求体、响应体、错误体全是 [`Bytes`]。
//!    （缺陷 #12：`from_utf8_lossy` 把非 UTF-8 错误体有损替换成 U+FFFD。
//!    只要类型是 `String`，就不存在保真度可言。）
//! 3. **没有 `Vec<u8>`。** [`Bytes::clone`] 是 refcount，failover 重试零拷贝。
//!    （缺陷 #13/#14：一个 900 KB 的请求在 3 次 failover 下要 memcpy 约 5.4 MB，
//!    纯粹是被 `payload: Vec<u8>` 这个字段类型逼出来的。）

use std::time::Duration;

use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode, uri::PathAndQuery};
use http_body_util::combinators::BoxBody;

// ===================================================================== 入口面

/// 网关对外只认这三个入口。第四个入口不存在 —— 收敛结论见
/// `docs/relay-surface-plan.md` §2（12 条路由删 6 留 6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surface {
    /// `POST /v1/chat/completions`
    OpenAiCompletions,
    /// `POST /v1/responses`
    ///
    /// 注意它**今天是坏的**：`routes.rs:684-688` 把它归进 `ApiFamily::OpenAi`，
    /// 而 `openai.rs:103` 只会构造 chat/completions 端点，于是 Responses 形状的
    /// body 被发到 chat/completions，上游必 400（缺陷 #1，S1）。
    /// gw-relay 里端点**不许由 provider 猜**：上游 URL = origin + 入站 path。
    OpenAiResponses,
    /// `POST /v1/messages`
    AnthropicMessages,
}

impl Surface {
    /// 入站 path → 入口。不匹配返回 `None`（调用方回 404，不要猜）。
    #[must_use]
    pub fn from_path(path: &str) -> Option<Self> {
        match path {
            "/v1/chat/completions" => Some(Self::OpenAiCompletions),
            "/v1/responses" => Some(Self::OpenAiResponses),
            "/v1/messages" => Some(Self::AnthropicMessages),
            _ => None,
        }
    }

    /// 入口自身的方言。P3 的 400 必须用**入口方言**的错误信封回，
    /// 因为客户端 SDK 只会解析它自己那套结构；回一个陌生结构，
    /// 客户端会把它渲染成一个无字的红叉。
    #[must_use]
    pub fn dialect(self) -> Dialect {
        match self {
            Self::OpenAiCompletions | Self::OpenAiResponses => Dialect::OpenAi,
            Self::AnthropicMessages => Dialect::Anthropic,
        }
    }
}

/// 客户端方言 —— 决定错误信封与 SSE 事件形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    OpenAi,
    Anthropic,
}

/// 上游 wire 协议。**不是**账号 provider 名 —— `gemini` 与 `vertex` 是两套鉴权与
/// 端点前缀，但 wire 协议是同一个 GenerateContent（对比 `gemini.rs:105-133` 与
/// `vertex.rs:489-509`），所以转义器按这个枚举分，2 个转义器覆盖 4 格。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpstreamDialect {
    /// OpenAI Chat Completions（openai、codex 共用）
    OpenAiChat,
    /// OpenAI Responses（openai、codex 共用）
    OpenAiResponses,
    /// Anthropic Messages
    AnthropicMessages,
    /// Google GenerateContent（gemini、vertex 共用）
    GoogleGenerateContent,
}

// ===================================================================== 请求

/// 客户端请求的字节视图。gw-relay 只读，不解析。
#[derive(Debug)]
pub struct RelayRequest {
    pub method: Method,
    /// 入站的 path + query，**原始字节**。不解码、不重编码、不拆键值对。
    ///
    /// 缺陷 #9：现在入站不解码、出站用 `append_pair` 重新编码，于是客户端发的
    /// `?tag=a%20b` 到上游变成 `?tag=a%2520b`。原始 query 是字节，直接拼。
    pub target: PathAndQuery,
    /// 入站 header，原样。凭证剥离与 hop-by-hop 过滤发生在 [`RelayBody::into_upstream`]
    /// 的同伴路径上（见 `headers.rs`），不在这里。
    pub headers: HeaderMap,
    pub body: RelayBody,
}

/// 请求体的两态。缺陷 #2（1 MiB 硬上限打满即 413，Claude Code 长会话的必然终点）
/// 的解药就是这个 enum：**计费降级，转发不降级**。
pub enum RelayBody {
    /// 已缓冲：体积在 peek 阈值内，计费 peek 与转发**共用同一块内存**。
    /// `clone()` 是 refcount，failover 重试零拷贝。
    Buffered(Bytes),
    /// 未缓冲：超过阈值，边收边转，**无上限**。
    /// 计费只能降级到保守估算 —— 但请求转发得出去。
    Streaming(BoxBody<Bytes, RelayError>),
}

impl RelayBody {
    /// 计费 peek 唯一被允许的入口：只读，只在 [`Self::Buffered`] 上生效。
    ///
    /// [`Self::Streaming`] 返回 `None` —— 调用方**必须显式处理**"我看不到 body"
    /// 这件事，而不是拿到一个空切片当成"body 是空的"。
    #[must_use]
    pub fn peek(&self) -> Option<&[u8]> {
        match self {
            Self::Buffered(b) => Some(b.as_ref()),
            Self::Streaming(_) => None,
        }
    }

    /// 这个请求体能不能被重放（failover 需要）。
    /// 流式体只能发一次 —— 第二次尝试必须换成"不重试"，而不是发一个空 body。
    #[must_use]
    pub fn is_replayable(&self) -> bool {
        matches!(self, Self::Buffered(_))
    }
}

impl std::fmt::Debug for RelayBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buffered(b) => f.debug_tuple("Buffered").field(&b.len()).finish(),
            Self::Streaming(_) => f.write_str("Streaming(..)"),
        }
    }
}

// ===================================================================== 响应

/// 上游响应的字节视图。**4xx / 5xx 也是这个类型**，没有第二条回写路径。
pub struct RelayResponse {
    pub status: StatusCode,
    /// 上游 header，原样。hop-by-hop 过滤在写出时做，且必须用 `append` 保多值。
    ///
    /// 缺陷 #7：现在回写用 `insert`，把同名多值折叠成最后一个。OpenAI 经
    /// Cloudflare 返两条 `set-cookie`，客户端只收到一条，缺了 `__cf_bm` 之后
    /// 每个请求都被当新会话，表现为**间歇性 403**。
    pub headers: HeaderMap,
    pub body: RelayResponseBody,
}

pub enum RelayResponseBody {
    Buffered(Bytes),
    /// 帧流。item 是 `Result` 而不是 `Infallible` —— 缺陷 #6 的解药。
    ///
    /// 现在 `Body::from_stream` 的错误类型是 `Infallible`，`StreamChunk::Error`
    /// 只写一行 `warn!` 就被吞掉，于是流式中途失败对客户端表现为**一次干净的
    /// EOF**：Claude Code 拿到截断的回答，不报错、不重试，用户以为模型就是这么
    /// 说的。中途失败必须把 `Err` 交给 hyper —— h2 发 `RST_STREAM`，h1 直接掐
    /// 连接，这是 SSE 唯一能让客户端察觉截断的手段。
    Stream(BoxBody<Bytes, RelayError>),
}

impl std::fmt::Debug for RelayResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

// ===================================================================== 上游

/// 上游端点 + 凭证 + 超时。**不含协议知识** —— 端点由 origin 与入站 path 拼出来，
/// 不由 provider 猜（缺陷 #1）。
#[derive(Debug, Clone)]
pub struct UpstreamTarget {
    /// scheme + host + 可选路径前缀。
    pub origin: url::Url,
    pub credential: Credential,
    pub timeouts: RelayTimeouts,
    /// 上游说哪种 wire 协议。与入口方言不同时才需要转义。
    pub dialect: UpstreamDialect,
}

/// 换掉入站凭证用的出站凭证。**本层读过的凭证载体，本层负责剥掉**
/// （观察 #17：`/v1/*` 上客户端的 `x-api-key` 今天会被原样转给 OpenAI）。
#[derive(Debug, Clone)]
pub enum Credential {
    /// `authorization: Bearer <token>`
    Bearer(String),
    /// `x-api-key: <key>`（Anthropic）
    XApiKey(String),
    /// `x-goog-api-key: <key>`（Google）
    GoogleApiKey(String),
}

#[derive(Debug, Clone, Copy)]
pub struct RelayTimeouts {
    pub connect: Duration,
    /// 非流式：整个请求的墙钟上限。
    pub request: Duration,
    /// 流式：**两帧之间**的空闲上限，不是整流上限。
    pub stream_idle: Duration,
}

impl Default for RelayTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            request: Duration::from_secs(600),
            stream_idle: Duration::from_secs(300),
        }
    }
}

/// 中继的唯一出口。
#[async_trait::async_trait]
pub trait Relay: Send + Sync {
    /// `Err` **只**表示"没能拿到上游响应"（DNS / TCP / TLS / connect 超时）。
    /// 上游返回的任何 status —— 400、429、500、529 —— 都是 `Ok`，
    /// 走与 200 完全相同的回写路径。
    async fn relay(
        &self,
        req: RelayRequest,
        to: &UpstreamTarget,
        probe: Option<Box<dyn UsageProbe>>,
    ) -> Result<RelayResponse, RelayTransportError>;
}

/// 中继只认识"连不上"这一种失败。
#[derive(Debug, thiserror::Error)]
pub enum RelayTransportError {
    #[error("connect failed: {0}")]
    Connect(#[source] reqwest::Error),
    #[error("upstream idle for {0:?}")]
    Idle(Duration),
    #[error("credential is not a valid header value")]
    BadCredential,
    #[error("upstream url is not constructible: {0}")]
    BadTarget(String),
}

/// 帧流中途失败。出现在 [`RelayResponseBody::Stream`] 的 item 里，
/// 交给 hyper 就变成 `RST_STREAM` / 掐连接。
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("upstream stream failed: {0}")]
    Upstream(String),
    #[error("upstream idle for {0:?}")]
    Idle(Duration),
    #[error("translation failed mid-stream: {0}")]
    Translate(String),
}

// ===================================================================== 旁路

/// 旁路取数：计费如何在**不改字节**的前提下拿到 usage。
///
/// 中继把每一帧的**只读视图**喂给它，它自己决定要不要留。实现必须满足：
/// 拿到的 `&[u8]` 只在调用期间有效，要留就自己 [`Bytes::clone`]（refcount，
/// 零字节拷贝）—— **不要** `to_vec()`（缺陷 #16：现在 `StreamUsageBuffer`
/// 对每个 chunk 全量 memcpy，约 2× 全流量的额外内存带宽）。
pub trait UsageProbe: Send {
    /// Claude 的 `input_tokens` 在**首帧** `message_start` 里（`claude.rs:325-342`），
    /// OpenAI / Google 的 usage 在**末帧**。只有需要头窗口的实现返回 `true`，
    /// 其余实现不必为此付出 32 KiB 的常驻拷贝。
    fn needs_head(&self) -> bool {
        false
    }

    /// 中继流过的每一帧。**不许阻塞**，不许在这里做 I/O。
    fn observe(&mut self, frame: &Bytes);

    /// 流结束（正常或失败）后取结果。`None` 表示上游没给 usage ——
    /// 调用方据此走 fallback 结算，**"缺失"与"零"必须能分开**。
    fn finish(self: Box<Self>) -> Option<RelayUsage>;
}

/// 旁路解析出的 token 计数。`None` 字段表示上游**没给**这个值，不是给了 0。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

impl RelayUsage {
    /// 有没有拿到任何一个计数。全 `None` 时计费必须走 fallback。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cached_tokens.is_none()
            && self.reasoning_tokens.is_none()
    }
}

// ===================================================================== 转义

/// 协议转义器。**转义可以丢字段，绝不能丢 usage** —— 丢了就落 fallback 结算，
/// 直接违反「计费语义不变」这条硬约束。
///
/// 15 格矩阵（`docs/relay-surface-plan.md` §3.6）里需要转义的 7 格由 4 个实现覆盖：
/// OpenAI↔Google、Anthropic↔Google（P1 四格，`translate/google.rs`），
/// OpenAI↔Anthropic 双向（P2 三格，`translate/anthropic.rs`）。
pub trait Translator: Send + Sync {
    fn surface(&self) -> Surface;
    fn to_dialect(&self) -> UpstreamDialect;

    /// 入口方言的请求体 → 上游方言的请求体。
    fn translate_request(&self, model: &str, body: &[u8]) -> Result<Bytes, TranslateError>;

    /// 上游方言的非流式响应体 → 入口方言的响应体。
    fn translate_response(&self, body: &[u8]) -> Result<Bytes, TranslateError>;

    /// 流式转义是**有状态**的：Anthropic 方向要合成 `message_start` /
    /// `content_block_start` / `message_stop` 这些 Google 侧没有对应物的框架事件，
    /// 所以每个请求要一个新的状态机。
    fn stream_translator(&self) -> Box<dyn StreamTranslator>;
}

/// 一个请求的流式转义状态机。
pub trait StreamTranslator: Send {
    /// 喂入上游的一个 SSE 帧，产出零或多个入口方言的帧。
    ///
    /// 零帧是合法的（上游的心跳/注释帧在目标方言里没有对应物）；
    /// 多帧也是合法的（一个 Google chunk 可能要展开成 Anthropic 的
    /// `content_block_delta` + `message_delta` 两帧）。
    fn push(&mut self, upstream_frame: &[u8]) -> Result<Vec<Bytes>, TranslateError>;

    /// 上游流**正常结束**时调用，产出收尾帧
    /// （Anthropic 的 `message_stop`、OpenAI 的 `data: [DONE]`）。
    fn finish(&mut self) -> Result<Vec<Bytes>, TranslateError>;

    /// 转义过程中看到的 usage。走转义路径时它**取代** [`UsageProbe`] ——
    /// 因为上游帧已经被解析过一次，再解析一遍是纯浪费。
    fn usage(&self) -> Option<RelayUsage>;
}

#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    /// 入口方言的请求体在目标方言里没有对应表达。**回 400，不要勉强翻译。**
    #[error("{0}")]
    Unsupported(String),
    /// 请求体不是合法 JSON，或缺少必需字段。
    #[error("malformed payload: {0}")]
    Malformed(String),
    /// 上游响应不是预期形状 —— 这是上游或转义器的 bug，不是客户端的错。
    #[error("unexpected upstream shape: {0}")]
    UpstreamShape(String),
}

#[cfg(test)]
mod tests;

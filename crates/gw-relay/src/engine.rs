//! [`crate::Relay`] 的实现 —— 纯字节中继引擎。
//!
//! OWNER: worker `relay-core`。
//!
//! 它不认识 JSON、不认识方言、不认识计费。它只做四件事：
//! 拼上游 URL（origin + 入站 path + 原始 query 字节）、换凭证、
//! 把 body 交出去、把响应原样接回来。
//!
//! **上游返回的任何 status 都是 `Ok`。**`Err` 只留给"没能拿到上游响应"。
//!
//! # 这里根除的缺陷
//!
//! | 编号 | 内容 |
//! | --- | --- |
//! | #1 (S1) | `/v1/responses` 被 provider 猜成 chat/completions → 端点 = origin + **入站 path** |
//! | #3 (S1) | 上游 4xx/5xx 被转成 `Err` 丢掉全部 header → 它就是一个 [`RelayResponse`] |
//! | #6 | 流式中途失败表现为干净 EOF → 帧流 item 是 `Result`，失败交给 hyper |
//! | #9 | query 双重百分号编码 → 原始字节直接拼，不解码不重编码 |
//! | #12 | 错误体经 `String` + `from_utf8_lossy` 有损 → 全程 `Bytes` |
//! | #13 #14 | failover 每次重拷 900 KB → 全程 `Bytes`，`clone` 是 refcount |
//!
//! # 可测性
//!
//! 传输是一个 trait（[`Transport`]），[`ReqwestTransport`] 只是它的生产实现。
//! 单元测试注入假传输，**一个字节都不走真实网络**。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{TryStreamExt, stream};
use http::header::CONTENT_TYPE;
use http::uri::PathAndQuery;
use http::{HeaderMap, Method, StatusCode};
use http_body::Frame;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};
use tokio::time::{Instant, timeout, timeout_at};
use url::Url;

use crate::body::join;
use crate::contract::{
    Relay, RelayBody, RelayError, RelayRequest, RelayResponse, RelayResponseBody, RelayTimeouts,
    RelayTransportError, UpstreamTarget, UsageProbe,
};
use crate::headers;

// ===================================================================== 配置

/// 中继的两个旋钮。**只有这两个** —— 中继不认识别的东西。
#[derive(Debug, Clone, Copy)]
pub struct RelayOptions {
    /// 入站 body 的缓冲阈值，见 [`RelayBody::DEFAULT_BUFFER_LIMIT`]。
    ///
    /// 它**只决定计费能不能看到 body**，不决定请求能不能转发（缺陷 #2）。
    /// 中继自己不用它 —— 调用方拿它去调 [`RelayBody::from_body`]。
    pub request_buffer_limit: usize,
    /// `false`（默认）：请求方向把 `accept-encoding` 收敛成 `identity`，
    /// 旁路 usage 读明文，计费精确。
    ///
    /// `true`：原样透传客户端的 `accept-encoding`，上游按需 gzip，
    /// **显式接受 usage 落 fallback 结算**。见缺陷 #5。
    pub preserve_client_encoding: bool,
}

impl Default for RelayOptions {
    fn default() -> Self {
        Self {
            request_buffer_limit: RelayBody::DEFAULT_BUFFER_LIMIT,
            preserve_client_encoding: false,
        }
    }
}

// ===================================================================== 传输

/// 一次上游往返所需的全部东西。URL 已经拼好，header 已经换过凭证。
pub struct UpstreamRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: RelayBody,
    pub timeouts: RelayTimeouts,
}

/// 上游响应的头 + 未消费的帧流。
///
/// **4xx / 5xx 也走这里** —— 传输层不认识 status 的含义（缺陷 #3）。
pub struct UpstreamHead {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: BoxBody<Bytes, RelayError>,
}

/// 字节出网的那一层。抽出来只为一件事：**测试不打真实网络**。
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// # Errors
    ///
    /// 只在**没能拿到响应头**时返回 `Err`（DNS / TCP / TLS / connect 超时）。
    /// 拿到响应头之后，任何 status 都是 `Ok`。
    async fn send(&self, req: UpstreamRequest) -> Result<UpstreamHead, RelayTransportError>;
}

/// 生产实现。client 是**进程级复用**的，见 [`shared_client`]。
#[derive(Debug, Clone, Copy, Default)]
pub struct ReqwestTransport;

/// 按 connect 超时取进程级共享的 client。
///
/// connect 超时是**连接池的属性**（reqwest 只能在 builder 上设），而
/// [`RelayTimeouts::connect`] 是 per-target 的，所以按这个值分桶：
/// 同一个值共用同一个池。实际部署里不同值只有个位数个，池数量不会失控。
///
/// 显式配置的池参数（今天 `common.rs:67-71` 缺 connect / keepalive / h2 心跳）：
/// 每 host 100 条空闲连接、空闲 90 秒回收、TCP keepalive 60 秒、
/// HTTP/2 心跳 30 秒（NAT / LB 静默断连不再只能等 idle 看门狗兜）。
///
/// **不设整请求超时** —— 那会掐死流式。三档超时全部在
/// [`RelayEngine::relay`] 与 [`ReqwestTransport::send`] 里按 [`RelayTimeouts`] 施加。
fn shared_client(connect: Duration) -> Result<reqwest::Client, reqwest::Error> {
    static CLIENTS: OnceLock<Mutex<HashMap<Duration, reqwest::Client>>> = OnceLock::new();
    let mut pool = CLIENTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(client) = pool.get(&connect) {
        return Ok(client.clone());
    }
    let client = reqwest::Client::builder()
        .connect_timeout(connect)
        .pool_max_idle_per_host(100)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .http2_keep_alive_interval(Duration::from_secs(30))
        .http2_keep_alive_timeout(Duration::from_secs(20))
        .http2_keep_alive_while_idle(true)
        .build()?;
    drop(pool.insert(connect, client.clone()));
    Ok(client)
}

#[async_trait]
impl Transport for ReqwestTransport {
    async fn send(&self, req: UpstreamRequest) -> Result<UpstreamHead, RelayTransportError> {
        let client = shared_client(req.timeouts.connect).map_err(RelayTransportError::Connect)?;
        let built = client
            .request(req.method, req.url)
            .headers(req.headers)
            .body(req.body.into_upstream());

        let response = match timeout(req.timeouts.request, built.send()).await {
            Err(_) => return Err(RelayTransportError::Idle(req.timeouts.request)),
            Ok(Err(err)) => return Err(RelayTransportError::Connect(err)),
            Ok(Ok(response)) => response,
        };

        let status = response.status();
        let headers = response.headers().clone();
        // 上游读到什么就转发什么：**不做 SSE 解析，不重新分帧**。
        // 这是当前实现唯一做对的一条，必须原样继承。
        let body = StreamBody::new(
            response
                .bytes_stream()
                .map_ok(Frame::data)
                .map_err(|err| RelayError::Upstream(err.to_string())),
        )
        .boxed();

        Ok(UpstreamHead {
            status,
            headers,
            body,
        })
    }
}

// ===================================================================== 引擎

/// 纯字节中继引擎。
///
/// 默认走 [`ReqwestTransport`]；测试用 [`RelayEngine::with_transport`] 注入假传输。
#[derive(Debug, Clone, Copy)]
pub struct RelayEngine<T = ReqwestTransport> {
    transport: T,
    options: RelayOptions,
}

impl RelayEngine<ReqwestTransport> {
    #[must_use]
    pub fn new(options: RelayOptions) -> Self {
        Self {
            transport: ReqwestTransport,
            options,
        }
    }
}

impl<T: Transport> RelayEngine<T> {
    #[must_use]
    pub fn with_transport(transport: T, options: RelayOptions) -> Self {
        Self { transport, options }
    }

    /// 调用方需要 [`RelayOptions::request_buffer_limit`] 去构造 [`RelayBody`]。
    #[must_use]
    pub fn options(&self) -> RelayOptions {
        self.options
    }
}

#[async_trait]
impl<T: Transport> Relay for RelayEngine<T> {
    async fn relay(
        &self,
        req: RelayRequest,
        to: &UpstreamTarget,
        probe: Option<Box<dyn UsageProbe>>,
    ) -> Result<RelayResponse, RelayTransportError> {
        let url = upstream_url(&to.origin, &req.target)?;
        let headers = headers::upstream_headers(
            &req.headers,
            &to.credential,
            self.options.preserve_client_encoding,
        )?;

        let started = Instant::now();
        let head = self
            .transport
            .send(UpstreamRequest {
                method: req.method,
                url,
                headers,
                body: req.body,
                timeouts: to.timeouts,
            })
            .await?;

        // 上游给了 status，就再也没有 `Err` 了 —— 429 的 `retry-after`、
        // `x-ratelimit-*`、`request-id` 全部跟着 header 原样回去（缺陷 #3）。
        let body = if is_event_stream(&head.headers) {
            RelayResponseBody::Stream(watch_frames(head.body, probe, to.timeouts.stream_idle))
        } else {
            let budget = to.timeouts.request.saturating_sub(started.elapsed());
            collect_frames(head.body, probe, budget).await
        };

        Ok(RelayResponse {
            status: head.status,
            headers: head.headers,
            body,
        })
    }
}

/// 上游 URL = `origin` + **入站 path** + **原始 query 字节**。
///
/// # 根除缺陷 #1（S1）
///
/// 端点不许由 provider 猜。今天 `/v1/responses` 被归进 `ApiFamily::OpenAi`，
/// 而 `openai.rs:103` 只会构造 chat/completions 端点，于是 Responses 形状的 body
/// 被发到 chat/completions，上游必 400 —— 三个保留入口之一 100% 不可用。
///
/// # 根除缺陷 #9
///
/// query 是**字节**：不解码、不重编码、不拆键值对。今天入站不解码 + 出站
/// `append_pair` 重编码 = 客户端发 `?tag=a%20b`，上游收到 `?tag=a%2520b`。
///
/// # Errors
///
/// 拼出来的字符串不是合法 URL 时返回 [`RelayTransportError::BadTarget`]。
/// 错误信息**不含拼接结果** —— query 里可能有客户端凭证（审计 §4.1 #12）。
fn upstream_url(origin: &Url, target: &PathAndQuery) -> Result<Url, RelayTransportError> {
    // `data:` / `mailto:` 这类非层级 URL 拼上 path 之后仍然「parse 得通」，
    // 但拼出来的东西毫无意义。在这里挡下来，别让它变成一个看不懂的 connect 错误。
    if origin.cannot_be_a_base() {
        return Err(RelayTransportError::BadTarget(format!(
            "upstream origin scheme `{}` is not hierarchical",
            origin.scheme()
        )));
    }
    let base = origin.as_str().trim_end_matches('/');
    let mut raw = String::with_capacity(base.len() + target.as_str().len());
    raw.push_str(base);
    raw.push_str(target.as_str());
    Url::parse(&raw).map_err(|err| RelayTransportError::BadTarget(err.to_string()))
}

fn is_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .trim_start()
                .get(.."text/event-stream".len())
                .is_some_and(|head| head.eq_ignore_ascii_case("text/event-stream"))
        })
}

/// 流式：逐帧转发 + 帧间空闲看门狗 + 旁路 probe。
///
/// # 根除缺陷 #6
///
/// item 类型是 `Result<Frame<Bytes>, RelayError>`，**不是** `Infallible`。
/// 中途失败（上游 transport error 或帧间空闲超时）把 `Err` 交给 hyper：
/// h2 发 `RST_STREAM`，h1 直接掐连接。这是 SSE 唯一能让客户端察觉截断的手段 ——
/// 今天客户端拿到的是一次**干净的 EOF**，Claude Code 不报错不重试。
///
/// # 取消传播
///
/// 上游 body 就住在 unfold 的 state 里，**没有 spawn**。客户端断开 → hyper 丢弃
/// 响应体 → state 被 drop → 上游 body 被 drop → h2 发 `RST_STREAM` / h1 关连接。
/// 上游不会继续跑完，token 不会白烧。
fn watch_frames(
    body: BoxBody<Bytes, RelayError>,
    probe: Option<Box<dyn UsageProbe>>,
    idle: Duration,
) -> BoxBody<Bytes, RelayError> {
    struct State {
        body: BoxBody<Bytes, RelayError>,
        probe: ProbeGuard,
        idle: Duration,
    }

    let state = State {
        body,
        probe: ProbeGuard::new(probe),
        idle,
    };

    StreamBody::new(stream::unfold(Some(state), |state| async move {
        let mut state = state?;
        match timeout(state.idle, state.body.frame()).await {
            // 空闲超时：把失败交出去，state 随之 drop → probe.finish()。
            Err(_) => Some((Err(RelayError::Idle(state.idle)), None)),
            Ok(None) => None,
            Ok(Some(Err(err))) => Some((Err(err), None)),
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    state.probe.observe(data);
                }
                // 原样转发，不合并不拆分 —— SSE 帧边界是上游给的。
                Some((Ok(frame), Some(state)))
            }
        }
    }))
    .boxed()
}

/// 非流式：整体收进 [`RelayResponseBody::Buffered`]，probe 吃完整 body。
///
/// 中途失败时**不丢已经收到的字节** —— 降级成一个帧流：前缀原样交出去，
/// 末尾接一个 `Err`，客户端照样能察觉截断（缺陷 #6 的非流式那一半）。
///
/// 错误体全程 [`Bytes`]，**不经过 `String`**（缺陷 #12）。
async fn collect_frames(
    mut body: BoxBody<Bytes, RelayError>,
    probe: Option<Box<dyn UsageProbe>>,
    budget: Duration,
) -> RelayResponseBody {
    let deadline = Instant::now() + budget;
    let mut parts: Vec<Bytes> = Vec::new();

    let failure = loop {
        let Ok(next) = timeout_at(deadline, body.frame()).await else {
            break Some(RelayError::Idle(budget));
        };
        match next {
            None => break None,
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    parts.push(data);
                }
            }
            Some(Err(err)) => break Some(err),
        }
    };

    let mut guard = ProbeGuard::new(probe);
    match failure {
        None => {
            let whole = join(parts);
            // 非流式也要能取 usage：同一套解析器吃完整 body。
            guard.observe(&whole);
            drop(guard);
            RelayResponseBody::Buffered(whole)
        }
        Some(err) => {
            for part in &parts {
                guard.observe(part);
            }
            drop(guard);
            let items = parts
                .into_iter()
                .map(|part| Ok(Frame::data(part)))
                .chain(std::iter::once(Err(err)));
            RelayResponseBody::Stream(StreamBody::new(stream::iter(items)).boxed())
        }
    }
}

/// 保证 [`UsageProbe::finish`] **恰好被调一次**，包括客户端中途断开那一次。
///
/// `finish` 吃 `Box<Self>`，`Drop` 只有 `&mut self`，所以用 `Option` + `take`。
/// 外面套 `Mutex` 是为了 `Sync`：`dyn UsageProbe` 只要求 `Send`，而契约里的
/// `BoxBody` 要求 `Send + Sync`。取用一律走 `get_mut()`，**不加锁、零开销**。
struct ProbeGuard(Mutex<Option<Box<dyn UsageProbe>>>);

impl ProbeGuard {
    fn new(probe: Option<Box<dyn UsageProbe>>) -> Self {
        Self(Mutex::new(probe))
    }

    fn observe(&mut self, frame: &Bytes) {
        if let Some(probe) = self.0.get_mut().unwrap_or_else(PoisonError::into_inner) {
            probe.observe(frame);
        }
    }
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        if let Some(probe) = self
            .0
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let usage = probe.finish();
            tracing::debug!(?usage, "relay: usage probe finished");
        }
    }
}

#[cfg(test)]
mod tests;

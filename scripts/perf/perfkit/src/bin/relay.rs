//! 第四个被测端：**`gw-relay` 的纯字节中继内核**（`docs/relay-perf-baseline.md` §5.1）。
//!
//! `gw-relay` 是库不是 router，所以这里把 [`RelayEngine`] 包成一个 axum handler
//! 挂在三个入口上，与 `floor` / `full` / `nomw` 同轮交错跑，出 T1–T13 的验收数字。
//!
//! # 它做了什么、没做什么（读数字之前必须知道）
//!
//! 它跑的是 `gw-relay` 在生产里**会跑的那条链**，一条不多一条不少：
//!
//! | 步 | 调用 | 对应 `full` 里的什么 |
//! | --- | --- | --- |
//! | 1 | [`endpoint::validate`] | 路由匹配 + content-type 判定 |
//! | 2 | [`RelayBody::from_body`] | `hold::peek_request_body` / `routes.rs` 读 body |
//! | 3 | [`RequestSpec::parse`] | hold 对 `model`/`stream`/`max_tokens` 的 peek |
//! | 4 | [`splice_include_usage`] | `common.rs::ensure_include_usage`（T10 就量这一步） |
//! | 5 | [`RelayEngine::relay`] + [`SseUsageProbe`] | `Dispatcher` 转发 + `StreamUsageBuffer` 旁路 usage |
//! | 6 | [`copy_preserving_multivalue`] | 响应 header 回写 |
//!
//! **没有**的东西：access 中间件、hold/settle 预扣结算、幂等、限流、熔断、
//! 凭证池与 failover 选择。所以结构上它可比的是 `nomw`，不是 `full`；
//! T1–T13 的目标是照着 floor 定的，本进程也照着 floor 报，但读数时
//! 要记得 `full − nomw` 那 +4.5 µs 的中间件成本本进程一分钱都没付。
//!
//! # 凭证
//!
//! 固定一条 `Bearer perf-upstream-key`，与 `gateway` 的 `OneAuthStore` 同一个值 ——
//! 凭证选择（`Dispatcher::auths_for` 每请求克隆整份凭证表）不在中继内核里，
//! 那是 `gw-proxy` 的开销，不该记在这个被测端头上。
//!
//! # 环境变量
//!
//! | 变量 | 默认 | 含义 |
//! | --- | --- | --- |
//! | `PERF_PORT` | 18088 | 被测端口 |
//! | `PERF_ADMIN_PORT` | 18098 | 分配计数端口 |
//! | `PERF_UPSTREAM` | `http://127.0.0.1:18081` | mock 上游 |
//! | `PERF_WORKERS` | 4 | tokio worker 线程数，钉死以便复现 |
//! | `PERF_COUNT_ALLOC` | 0 | 1 = 打开分配计数 |
//! | `PERF_RELAY_INCLUDE_USAGE` | 1 | 0 = 走 [`IncludeUsagePolicy::Respect`]，用来单独称 T10 |

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use bytes::Bytes;
use futures_util::{TryStreamExt, stream};
use gw_relay::endpoint::{IncludeUsagePolicy, RequestSpec, splice_include_usage, validate};
use gw_relay::engine::{
    RelayEngine, RelayOptions, Transport, UpstreamHead, UpstreamRequest,
};
use gw_relay::probe::{SseUsageProbe, UsageShape};
use gw_relay::{
    Credential, Relay, RelayBody, RelayError, RelayRequest, RelayTimeouts, RelayTransportError,
    RelayUsage, UpstreamDialect, UpstreamTarget, UsageProbe, copy_preserving_multivalue,
};
use http::uri::PathAndQuery;
use http::{HeaderName, HeaderValue, StatusCode};
use http_body::Frame;
use http_body_util::{BodyExt, StreamBody};
use perfkit::counting_alloc::{self, Counting};

#[global_allocator]
static ALLOC: Counting = Counting;

struct Kernel {
    /// `Box<dyn Relay>` 而不是 `RelayEngine`：TLS 档要换一个 [`Transport`]，
    /// 而 `RelayEngine<T>` 是按 transport 参数化的两个不同类型。
    engine: Box<dyn Relay>,
    options: RelayOptions,
    target: UpstreamTarget,
    policy: IncludeUsagePolicy,
    /// failover 档的最多尝试次数。`<= 1` 时挂的是 [`handle`]，这个字段用不到。
    attempts: u32,
    /// failover 档：重试时是 `Bytes::clone` 还是全量拷贝。
    copy_on_retry: bool,
}

/// 包一层 probe，只为一件事：**probe 一次都没解析到 usage 时吼一声**。
///
/// 这是本装置最容易产出的假数据 —— probe 提前 bail（上游压缩了、形状对不上、
/// 行超长被丢弃），于是流式路径看起来"很便宜"，而便宜的原因是它什么都没干。
/// `results/loadgen.err` 与 `/tmp/perf-relay.log` 干净 = 每一发都真的解析了。
/// 只吼第一次：热路径上这是一次 relaxed 原子读。
struct LoudProbe(SseUsageProbe);

static PROBE_WARNED: AtomicBool = AtomicBool::new(false);

impl UsageProbe for LoudProbe {
    fn needs_head(&self) -> bool {
        self.0.needs_head()
    }

    fn observe(&mut self, frame: &Bytes) {
        self.0.observe(frame);
    }

    fn finish(self: Box<Self>) -> Option<RelayUsage> {
        let usage = Box::new(self.0).finish();
        if usage.is_none() && !PROBE_WARNED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "relay: usage probe 一个字段都没解析到 —— 本被测端的数字与 full 不可比，先修这个"
            );
        }
        usage
    }
}

/// `splice_include_usage` 产出的两段字节 → 一个两帧的请求体。
///
/// **一次全量拷贝都没有**：`prefix` 是新分配的几十字节，`rest` 是原 `Bytes` 的
/// 零拷贝切片。代价是这个 body 没有确定长度，于是上游走 chunked ——
/// `RelayBody::Buffered` 那条路会带 `content-length`。这个差异记在
/// `docs/relay-perf-acceptance.md` 的装置缺陷里，不要当它不存在。
fn spliced_body(prefix: Bytes, rest: Bytes) -> RelayBody {
    let frames: [Result<Frame<Bytes>, RelayError>; 2] =
        [Ok(Frame::data(prefix)), Ok(Frame::data(rest))];
    RelayBody::Streaming(StreamBody::new(stream::iter(frames)).boxed())
}

async fn handle(State(kernel): State<Arc<Kernel>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();

    let surface = match validate(&parts.method, parts.uri.path(), &parts.headers) {
        Ok(surface) => surface,
        Err(err) => return err.status().into_response(),
    };

    let limit = kernel.options.request_buffer_limit;
    let body = match RelayBody::from_body(body, limit).await {
        Ok(body) => body,
        Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    };

    // 全链路唯一一次请求体解析（缺陷 #15）。
    let spec = RequestSpec::parse(surface, body.peek());

    // 定点插入 `stream_options.include_usage`，取代 `ensure_include_usage()`
    // 的整树 serde_json 往返（缺陷 #4）。不该插时 `None`，一个字节都不动。
    let body = match body {
        RelayBody::Buffered(bytes) => {
            match splice_include_usage(&bytes, &spec, kernel.target.dialect, kernel.policy) {
                Some(spliced) => spliced_body(spliced.prefix, spliced.rest),
                None => RelayBody::Buffered(bytes),
            }
        }
        streaming => streaming,
    };

    let target = parts
        .uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| PathAndQuery::from_static("/"));

    let (probe, _handle) = SseUsageProbe::new(UsageShape::OpenAi);
    let relayed = kernel
        .engine
        .relay(
            RelayRequest {
                method: parts.method,
                target,
                headers: parts.headers,
                body,
            },
            &kernel.target,
            Some(Box::new(LoudProbe(probe))),
        )
        .await;

    let upstream = match relayed {
        Ok(response) => response,
        Err(err) => return (StatusCode::BAD_GATEWAY, err.to_string()).into_response(),
    };

    let mut response = Response::new(Body::new(upstream.body.into_http_body()));
    *response.status_mut() = upstream.status;
    copy_preserving_multivalue(response.headers_mut(), &upstream.headers);
    response
}

/// failover handler。**与 [`handle`] 是两个函数，不是一个带 `if` 的函数** ——
/// T1–T13 那条路径必须一个字节都不多，否则验收数字里会混进这一档的分支。
///
/// 上游按 `?fail_first=N` 回 N 次 429；本函数带着 `x-perf-attempt` 重试，
/// 直到 200 或用完 `attempts`。**最后一次的响应原样回给客户端，不管它是几** ——
/// 上游的 4xx 是一个响应而不是错误（缺陷 #3）。
async fn failover(State(kernel): State<Arc<Kernel>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();

    if let Err(err) = validate(&parts.method, parts.uri.path(), &parts.headers) {
        return err.status().into_response();
    }
    let limit = kernel.options.request_buffer_limit;
    let payload = match RelayBody::from_body(body, limit).await {
        Ok(RelayBody::Buffered(bytes)) => bytes,
        // 流式请求体不可重放（`RelayBody::is_replayable()`），这一档不测它。
        Ok(RelayBody::Streaming(_)) => {
            return (StatusCode::BAD_REQUEST, "failover 档要求可重放的请求体").into_response();
        }
        Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    };
    let target = parts
        .uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| PathAndQuery::from_static("/"));

    let mut last = None;
    for attempt in 0..kernel.attempts {
        let mut headers = parts.headers.clone();
        headers.insert(
            HeaderName::from_static("x-perf-attempt"),
            HeaderValue::from(attempt),
        );
        // 这一行就是本档要称的东西：refcount 加一，还是一次全量 memcpy。
        let body = if kernel.copy_on_retry {
            Bytes::copy_from_slice(&payload)
        } else {
            payload.clone()
        };
        let (probe, _handle) = SseUsageProbe::new(UsageShape::OpenAi);
        let relayed = kernel
            .engine
            .relay(
                RelayRequest {
                    method: parts.method.clone(),
                    target: target.clone(),
                    headers,
                    body: RelayBody::Buffered(body),
                },
                &kernel.target,
                Some(Box::new(LoudProbe(probe))),
            )
            .await;
        match relayed {
            Ok(response) => {
                let retryable = response.status == StatusCode::TOO_MANY_REQUESTS
                    || response.status.is_server_error();
                last = Some(response);
                if !retryable {
                    break;
                }
            }
            Err(err) => return (StatusCode::BAD_GATEWAY, err.to_string()).into_response(),
        }
    }

    let Some(upstream) = last else {
        return (StatusCode::BAD_GATEWAY, "attempts 为 0").into_response();
    };
    let mut response = Response::new(Body::new(upstream.body.into_http_body()));
    *response.status_mut() = upstream.status;
    copy_preserving_multivalue(response.headers_mut(), &upstream.headers);
    response
}

/// TLS + h2 档专用的 [`Transport`]。
///
/// 为什么不能直接用 [`gw_relay::engine::ReqwestTransport`]：它的 client 由
/// `shared_client()` 建，配置写死在 `gw-relay` 里，**没有放行自签证书的口子**，
/// 而 mock 上游只能用自签证书。所以这里照抄它的池配置，逐字一致，只多一条
/// `danger_accept_invalid_certs(true)`。
///
/// 抄的是 `crates/gw-relay/src/engine.rs::shared_client`：每 host 100 条空闲
/// 连接、空闲 90 秒回收、TCP keepalive 60 秒、h2 心跳 30/20 秒 + while_idle。
/// **改了那边要跟着改这里** —— 这一档量的是"生产 transport 在 h2 上多花多少"，
/// 池配置一漂就不是同一个东西了。
struct TlsTransport {
    client: reqwest::Client,
}

impl TlsTransport {
    fn new(connect: Duration) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(connect)
                .pool_max_idle_per_host(100)
                .pool_idle_timeout(Duration::from_secs(90))
                .tcp_keepalive(Duration::from_secs(60))
                .http2_keep_alive_interval(Duration::from_secs(30))
                .http2_keep_alive_timeout(Duration::from_secs(20))
                .http2_keep_alive_while_idle(true)
                .danger_accept_invalid_certs(true)
                .build()?,
        })
    }
}

#[async_trait::async_trait]
impl Transport for TlsTransport {
    async fn send(&self, req: UpstreamRequest) -> Result<UpstreamHead, RelayTransportError> {
        let built = self
            .client
            .request(req.method, req.url)
            .headers(req.headers)
            .body(req.body.into_upstream());
        let response = match tokio::time::timeout(req.timeouts.request, built.send()).await {
            Err(_) => return Err(RelayTransportError::Idle(req.timeouts.request)),
            Ok(Err(err)) => return Err(RelayTransportError::Connect(err)),
            Ok(Ok(response)) => response,
        };
        let status = response.status();
        let headers = response.headers().clone();
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

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() -> anyhow::Result<()> {
    counting_alloc::init_from_env();

    let port: u16 = env_or("PERF_PORT", 18088);
    let admin_port: u16 = env_or("PERF_ADMIN_PORT", 18098);
    let workers: usize = env_or("PERF_WORKERS", 4);
    let upstream =
        std::env::var("PERF_UPSTREAM").unwrap_or_else(|_| "http://127.0.0.1:18081".to_owned());
    let policy = if env_or::<u8>("PERF_RELAY_INCLUDE_USAGE", 1) == 1 {
        IncludeUsagePolicy::Force
    } else {
        IncludeUsagePolicy::RespectClient
    };
    let attempts: u32 = env_or("PERF_RELAY_FAILOVER", 0);
    let copy_on_retry = std::env::var("PERF_RELAY_BODY_MODE").as_deref() == Ok("vec");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?;

    rt.block_on(async move {
        let options = RelayOptions::default();
        let engine: Box<dyn Relay> = if std::env::var("PERF_TLS").as_deref() == Ok("1") {
            Box::new(RelayEngine::with_transport(
                TlsTransport::new(RelayTimeouts::default().connect)?,
                options,
            ))
        } else {
            Box::new(RelayEngine::new(options))
        };
        let kernel = Arc::new(Kernel {
            engine,
            options,
            target: UpstreamTarget {
                origin: upstream.parse()?,
                credential: Credential::Bearer("perf-upstream-key".to_owned()),
                timeouts: RelayTimeouts::default(),
                dialect: UpstreamDialect::OpenAiChat,
            },
            policy,
            attempts,
            copy_on_retry,
        });

        // 三个入口全挂上，与 `gw-relay` 的收敛结论一致。压测只打第一个，
        // 另外两个挂着是为了让路由表的形状与生产一致（axum 的路由匹配开销
        // 随路由条数变化，只挂一条会把这部分开销藏起来）。
        let entry = if attempts > 1 { post(failover) } else { post(handle) };
        let app = Router::new()
            .route("/v1/chat/completions", entry.clone())
            .route("/v1/responses", entry.clone())
            .route("/v1/messages", entry)
            .with_state(kernel);

        let admin_addr: SocketAddr = ([127, 0, 0, 1], admin_port).into();
        tokio::spawn(async move {
            if let Err(err) = perfkit::admin::serve(admin_addr).await {
                eprintln!("admin server stopped: {err}");
            }
        });

        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        let listener = tokio::net::TcpListener::bind(addr).await?;
        eprintln!(
            "relay workers={workers} include_usage={policy:?} failover={attempts} \
             copy_on_retry={copy_on_retry} alloc_count={} \
             listening on http://{addr} (admin :{admin_port}) -> {upstream}",
            counting_alloc::enabled()
        );
        axum::serve(listener, app).await?;
        Ok::<(), anyhow::Error>(())
    })
}

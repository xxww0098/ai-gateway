//! 理论下界对照组：一个**纯 Bytes 中继**的 axum 反代。
//!
//! 它做且只做转发该做的事：
//!   1. 请求体**不缓冲**，直接把 axum 的 body stream 交给 reqwest；
//!   2. 转发 inbound header（去掉 hop-by-hop 与 host / content-length）；
//!   3. 响应体**不缓冲**，`bytes_stream()` 直接回给客户端。
//!
//! 没有 JSON 解析、没有计费、没有 usage 抽取、没有 `Vec<u8>` 落地。
//! 它就是「同样的 tokio + hyper + reqwest 栈能做到的最好水平」。
//!
//! **网关自身开销 = 网关实测 − 本对照组实测。** 这是本任务里唯一能把
//! "运行时/内核/mock 的成本" 从 "gw-proxy 的成本" 里减掉的办法。
//!
//! 环境变量与 `gateway` 同名同义（`PERF_PORT` / `PERF_ADMIN_PORT` /
//! `PERF_UPSTREAM` / `PERF_WORKERS` / `PERF_COUNT_ALLOC`）。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use http::{HeaderMap, HeaderName, StatusCode};
use perfkit::counting_alloc::{self, Counting};

#[global_allocator]
static ALLOC: Counting = Counting;

struct Floor {
    client: reqwest::Client,
    upstream: String,
}

/// 与 `gw_provider::types::is_skipped_proxy_header` 同一份语义，
/// 免得对照组因为多转/少转 header 而不可比。
fn skip(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "authorization"
            | "connection"
            | "content-length"
            | "host"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn forwardable(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::with_capacity(src.len());
    for name in src.keys() {
        if skip(name) {
            continue;
        }
        for value in src.get_all(name) {
            out.append(name.clone(), value.clone());
        }
    }
    out
}

async fn relay(State(state): State<Arc<Floor>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("{}{}", state.upstream, path_and_query);

    // 请求体全程流式：既不 `to_bytes`，也不 `Vec<u8>` 落地。
    let outbound = reqwest::Body::wrap_stream(body.into_data_stream());
    let sent = state
        .client
        .post(url)
        .headers(forwardable(&parts.headers))
        .body(outbound)
        .send()
        .await;

    let upstream = match sent {
        Ok(r) => r,
        Err(err) => {
            return (StatusCode::BAD_GATEWAY, err.to_string()).into_response();
        }
    };

    let status = upstream.status();
    let headers = upstream.headers().clone();
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    for (name, value) in headers.iter() {
        if skip(name) {
            continue;
        }
        response.headers_mut().insert(name, value.clone());
    }
    response
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() -> anyhow::Result<()> {
    counting_alloc::init_from_env();

    let port: u16 = env_or("PERF_PORT", 18082);
    let admin_port: u16 = env_or("PERF_ADMIN_PORT", 18092);
    let workers: usize = env_or("PERF_WORKERS", 4);
    let upstream =
        std::env::var("PERF_UPSTREAM").unwrap_or_else(|_| "http://127.0.0.1:18081".to_owned());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?;

    rt.block_on(async move {
        // 与 gw-provider 的 `shared_client()` 同配置，否则连接池差异会污染对比。
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(100)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;
        let state = Arc::new(Floor {
            client,
            upstream: upstream.clone(),
        });

        let app = Router::new()
            .route("/v1/chat/completions", post(relay))
            .with_state(state);

        let admin_addr: SocketAddr = ([127, 0, 0, 1], admin_port).into();
        tokio::spawn(async move {
            if let Err(err) = perfkit::admin::serve(admin_addr).await {
                eprintln!("admin server stopped: {err}");
            }
        });

        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        let listener = tokio::net::TcpListener::bind(addr).await?;
        eprintln!(
            "floor workers={workers} alloc_count={} listening on http://{addr} \
             (admin :{admin_port}) -> {upstream}",
            counting_alloc::enabled()
        );
        axum::serve(listener, app).await?;
        Ok::<(), anyhow::Error>(())
    })
}

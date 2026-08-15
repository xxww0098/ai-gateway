//! 被测网关：**真** `gw_proxy::router()` + **真** `gw_provider` OpenAI executor，
//! 上游指向 `mock-upstream`。crates/ 下一行都没改，全部是只读引用。
//!
//! # 两种拓扑，为的是把中间件开销和转发开销拆开
//!
//! * `PERF_MODE=full` —— `gw_proxy::router()` 原样：access → hold → dispatch。
//!   这是生产拓扑。
//! * `PERF_MODE=nomw` —— 只挂 `routes::chat_completions`，不挂两层中间件。
//!   handler 里 `billing` 与 `PeekedBody` 都取不到，于是既不预扣也不结算，
//!   body 由 handler 自己读。**它与 full 的差值就是 access+hold 的净成本。**
//!
//! # 环境变量
//!
//! | 变量 | 默认 | 含义 |
//! | --- | --- | --- |
//! | `PERF_PORT` | 18080 | 被测端口 |
//! | `PERF_ADMIN_PORT` | 18090 | 分配计数端口 |
//! | `PERF_UPSTREAM` | `http://127.0.0.1:18081` | mock 上游 |
//! | `PERF_MODE` | full | `full` / `nomw` |
//! | `PERF_IDEMPOTENCY` | 0 | 1 = 挂上幂等管理器（量 `capture_body` 的全量响应缓冲） |
//! | `PERF_WORKERS` | 4 | tokio worker 线程数，钉死以便复现 |
//! | `PERF_COUNT_ALLOC` | 0 | 1 = 打开分配计数 |

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::post;
use gw_provider::common::ProviderConfig;
use gw_provider::openai::OpenAiCompatibleProvider;
use gw_provider::types::Provider;
use gw_proxy::channel::{ChannelHealth, ChannelPolicyCache, ChannelPool};
use gw_proxy::hold::HoldMiddleware;
use gw_proxy::idempotency::IdempotencyManager;
use gw_proxy::routes::Dispatcher;
use gw_proxy::usage::Settlement;
use gw_proxy::{AccessProvider, ProxyState};
use perfkit::counting_alloc::{self, Counting};
use perfkit::stubs::{
    AllowAllRateLimiter, ClosedBreaker, EmptyPolicyStore, FlatCalculator, MemIdempotencyStore,
    NullLedger, NullUsageStore, OneAuthStore, OneModelCatalog, RealCrypto, StaticDirectory,
};
use tokio_util::task::TaskTracker;

#[global_allocator]
static ALLOC: Counting = Counting;

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn build_state(upstream: &str, idempotency: bool) -> anyhow::Result<ProxyState> {
    let ledger = Arc::new(NullLedger::default());
    let calc = Arc::new(FlatCalculator);
    let usage_store = Arc::new(NullUsageStore::default());
    let crypto = Arc::new(RealCrypto);
    let directory = Arc::new(StaticDirectory::new());

    let settlement = Arc::new(Settlement::new(
        ledger.clone(),
        calc.clone(),
        usage_store.clone(),
    ));

    let health = Arc::new(ChannelHealth::new(0, Duration::from_secs(30)));
    let policies = Arc::new(ChannelPolicyCache::new(Arc::new(EmptyPolicyStore)));
    let channels = Arc::new(ChannelPool::new(health).with_policies(policies));

    let provider = OpenAiCompatibleProvider::new(
        &ProviderConfig {
            base_url: upstream.to_owned(),
            api_key: "perf-upstream-key".to_owned(),
            enabled: true,
        },
        60,
    )
    .map_err(|err| anyhow::anyhow!("building the upstream executor failed: {err}"))?;

    let dispatch = Arc::new(
        Dispatcher::new(
            vec![Arc::new(provider) as Arc<dyn Provider>],
            OneAuthStore::new("perf-upstream-key"),
            channels,
            settlement.clone(),
        )
        .with_circuit_breaker(Arc::new(ClosedBreaker))
        .with_catalog(Arc::new(OneModelCatalog)),
    );

    let mut hold = HoldMiddleware::new(
        ledger.clone(),
        calc.clone(),
        settlement.clone(),
        Duration::from_secs(300),
    )
    .with_rate_limiter(Arc::new(AllowAllRateLimiter))
    .with_circuit_breaker(Arc::new(ClosedBreaker));
    if idempotency {
        hold = hold.with_idempotency(Arc::new(IdempotencyManager::new(
            Arc::new(MemIdempotencyStore::default()),
            crypto.clone(),
            Duration::from_secs(60),
        )));
    }

    Ok(ProxyState::new(
        Arc::new(AccessProvider::new(directory, crypto)),
        Arc::new(hold),
        dispatch,
        TaskTracker::new(),
    ))
}

/// `nomw` 拓扑：同一批 handler，去掉两层中间件。
fn bare_router(state: ProxyState) -> Router {
    Router::new()
        .route(
            "/v1/chat/completions",
            post(gw_proxy::routes::chat_completions),
        )
        .with_state(state)
}

fn main() -> anyhow::Result<()> {
    counting_alloc::init_from_env();

    let port: u16 = env_or("PERF_PORT", 18080);
    let admin_port: u16 = env_or("PERF_ADMIN_PORT", 18090);
    let workers: usize = env_or("PERF_WORKERS", 4);
    let upstream =
        std::env::var("PERF_UPSTREAM").unwrap_or_else(|_| "http://127.0.0.1:18081".to_owned());
    let mode = std::env::var("PERF_MODE").unwrap_or_else(|_| "full".to_owned());
    let idempotency = env_or::<u8>("PERF_IDEMPOTENCY", 0) == 1;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?;

    rt.block_on(async move {
        let state = build_state(&upstream, idempotency)?;
        let app = match mode.as_str() {
            "nomw" => bare_router(state),
            "full" => gw_proxy::router(state),
            other => anyhow::bail!("unknown PERF_MODE {other}, expected full|nomw"),
        };

        let admin_addr: SocketAddr = ([127, 0, 0, 1], admin_port).into();
        tokio::spawn(async move {
            if let Err(err) = perfkit::admin::serve(admin_addr).await {
                eprintln!("admin server stopped: {err}");
            }
        });

        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        let listener = tokio::net::TcpListener::bind(addr).await?;
        eprintln!(
            "gateway mode={mode} idempotency={idempotency} workers={workers} alloc_count={} \
             listening on http://{addr} (admin :{admin_port}) -> {upstream}",
            counting_alloc::enabled()
        );
        axum::serve(listener, app).await?;
        Ok::<(), anyhow::Error>(())
    })
}

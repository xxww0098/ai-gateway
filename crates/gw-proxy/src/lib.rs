//! The proxy kernel: tenant auth, pre-flight hold, upstream dispatch, streaming
//! relay, settlement. The kernel is implemented here end to end.
//!
//! # 三个客户端入口（收敛结论，`docs/relay-surface-plan.md` §2：12 条路由删 6 留 6）
//!
//! | 入口 | 路径 | 方言 |
//! | --- | --- | --- |
//! | A | `POST /v1/chat/completions` | OpenAI Chat Completions |
//! | B | `POST /v1/responses` | OpenAI Responses |
//! | C | `POST /v1/messages` | Anthropic Messages |
//!
//! 另有三条**非推理**路由随入口一起留下，且**全部不计费**
//! （`hold::is_billable`）：`POST /v1/messages/count_tokens`（入口 C 的附属端点）、
//! `GET /v1/models`、`GET /v1/models/{model}`。
//!
//! 只有一个前缀：`/v1/`（[`access::is_proxy_path`]）。
//!
//! # 已知缺口（已接受，不要试图去修）
//!
//! 面板 `frontend/src/features/user-dashboard/components/QuickIntegrationPanel.tsx:80`
//! 的 Anthropic tab 仍然把 `${origin}/v1beta` 印给用户，而 `/v1beta/**` 已被硬删，
//! 照着面板配的用户会拿到 404。前端冻结，改不了。
//!
//! 补充事实：**那行文案今天本来就是错的** —— `/v1beta` 是 Google 的版本段，
//! 给 Anthropic 客户端本来就 404（`@anthropic-ai/sdk` 自己会拼 `/v1/messages`，
//! 它需要的 base 是裸 `${origin}`）。收敛只是把「错但碰巧有个路由在」
//! 变成「错且路由也没了」。这是已知且已接受的代价。
//!
//! 反过来，`GET /v1/models` **必须保留**，理由不是「前端在调」（前端对 `/v1` 的
//! HTTP 调用数是 0），而是面板 `QuickIntegrationPanel.tsx:79` 把 `${origin}/v1`
//! 作为 Base URL 印给用户 —— 所有 OpenAI 兼容客户端拿到 base 之后的第一个请求
//! 就是 `GET {base}/models`。删了它，照面板指引配置的客户端在**连接测试阶段**就失败。
//!
//! Request order (must match the B1 fix):
//!   access-auth -> hold -> execute -> parse usage -> settle | release
//!
//! # Layout
//!
//! | module | role |
//! | --- | --- |
//! | [`kernel`] | request state machine + single [`RelayCtx`] |
//! | [`access`] | tenant authentication |
//! | [`hold`] | pre-flight reservation + quota gate |
//! | [`usage`] | usage parsing + settlement |
//! | [`channel`] | account selection, health, policy cache |
//! | [`idempotency`] | idempotent replay of cached responses |
//! | [`budget_token`] | process-local batch budget |
//! | [`reconcile`] | orphaned-hold recovery |
//! | [`settlectx`] | per-request billing state |
//! | [`ports`] | the collaborator traits |
//! | [`adapters`] | concrete implementations of [`ports`] |
//! | [`routes`] | the `/v1` handlers |
//!
//! Collaborators that used to be concrete types (`*ledger.Ledger`,
//! `*pricing.Calculator`, `*infra.RateLimiter`) sit behind the traits in
//! [`ports`]. [`adapters`] implements every one of them over the real crates,
//! so the composition root only has to hand the pieces to
//! [`ProxyState::new`].
//!
//! What this crate deliberately does NOT own: `/metrics/prometheus`. `gw-server`
//! owns the route, the counters and the exposition format; this crate only
//! pushes its two gauges through [`ports::MetricsSink`].
//!
//! OWNER: worker `proxy-kernel`.

// Rule 5.3 ratchet: this crate is at zero `todo!()` / `unimplemented!()`, so it
// carries its own `deny` even though the workspace lint is still `warn` for the
// crates that have not caught up yet.
#![deny(clippy::todo, clippy::unimplemented)]

pub mod access;
pub mod adapters;
pub mod budget_token;
pub mod channel;
pub mod error;
pub mod hold;
pub mod idempotency;
pub mod kernel;
pub mod ports;
pub mod reconcile;
pub mod routes;
pub mod settlectx;
pub mod usage;

#[cfg(test)]
mod testsupport;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use tokio_util::task::TaskTracker;

pub use access::AccessProvider;
pub use hold::HoldMiddleware;
pub use kernel::{Phase, RelayCtx};
pub use ports::{DiscardMetrics, MetricsSink};
pub use routes::Dispatcher;
pub use settlectx::{RequestBilling, SettleCtx};
pub use usage::Settlement;

/// Everything the `/v1` surface needs, cloned into each handler.
///
/// The bag of collaborators the composition root threads through.
#[derive(Clone)]
pub struct ProxyState {
    /// Tenant authentication for `/v1/*`.
    pub access: Arc<AccessProvider>,
    /// Pre-flight reservation and quota gate.
    pub hold: Arc<HoldMiddleware>,
    /// Upstream account selection, execution and settlement.
    pub dispatch: Arc<Dispatcher>,
    /// Tracker for detached settlement tasks.
    ///
    /// **This is load-bearing, not bookkeeping.** `StreamSettler::drop` cannot
    /// `.await`, so a client that hangs up mid-stream — and every unary
    /// settle, which uses the same drop path so the HTTP response is not
    /// blocked on ledger I/O — is settled from a detached task. A bare
    /// `tokio::spawn` dies silently the moment the runtime is dropped —
    /// `serve()` returns, `main` ends, and every in-flight `Settle` is
    /// aborted with its hold left to expire on TTL. That is free
    /// upstream output, i.e. a violation of the billing invariants in
    /// `AGENTS.md`.
    ///
    /// The composition root creates ONE [`TaskTracker`], hands a clone here,
    /// and waits it out after graceful shutdown (`gw_server::drain`). Clones
    /// share one task set, so closing it there closes this view too — and
    /// closing does not block later spawns, so a `drop` that lands *during* the
    /// drain is still tracked and still waited on.
    pub drain: TaskTracker,
    /// Where the two gauges this crate observes are published.
    pub metrics: Arc<dyn MetricsSink>,
}

impl ProxyState {
    /// Assembles the `/v1` surface's collaborators.
    ///
    /// Assembles the access provider, hold middleware, usage plugin and auth
    /// manager.
    ///
    /// `drain` must be the tracker the composition root drains after shutdown —
    /// see the field docs. Passing a fresh `TaskTracker::new()` here compiles
    /// and then loses settlements at shutdown, which is exactly the bug the
    /// parameter exists to prevent.
    pub fn new(
        access: Arc<AccessProvider>,
        hold: Arc<HoldMiddleware>,
        dispatch: Arc<Dispatcher>,
        drain: TaskTracker,
    ) -> Self {
        Self {
            access,
            hold,
            dispatch,
            drain,
            metrics: Arc::new(DiscardMetrics),
        }
    }

    /// Publishes gauges to `sink` instead of discarding them.
    #[must_use]
    pub fn with_metrics(mut self, sink: Arc<dyn MetricsSink>) -> Self {
        self.metrics = sink;
        self
    }

    /// Pushes the live benched-account count into the metrics sink.
    ///
    /// The route lives in another crate now, so the composition root calls this
    /// on a ticker (or immediately before serving a scrape) to keep the gauge
    /// fresh.
    pub fn publish_gauges(&self) {
        self.metrics
            .set_channel_benched(self.dispatch.channels().health().benched_count());
    }
}

/// Builds the metered proxy routes — 只有 `/v1/*` —— with the billing
/// middleware stack attached.
///
/// 六条路由，一条不多：三个推理入口 + `count_tokens` + 两条 catalogue 读。
/// 被删掉的六条（`POST /v1/completions`、`POST /v1/embeddings`、
/// `POST /v1/models/{model}`、`GET /v1beta/models`、`GET /v1beta/models/{model}`、
/// `POST /v1beta/models/{model}`）是**硬删**，不是 410 过渡 —— 判定表见
/// `docs/relay-surface-plan.md` §2，已知代价见 crate 级 doc。
///
/// 热路径只挂 **一层** [`kernel::layer`]。鉴权→预扣的顺序写在状态机里
/// （[`kernel::Phase`]），不再靠两个 `.layer()` 的挂载顺序维持 B1。
/// `access::layer` / `hold::layer` 仍在，给只想测其中一层的用例用。
///
/// Request counting is NOT layered here: `gw_server::metrics::track` wraps the
/// merged router and already scopes itself to `/v1/*`. A second layer would
/// double-count every request into an instance nothing exports. For the same
/// reason there is no `/metrics/prometheus` route here — `gw-server` registers
/// it, and registering it twice is an axum duplicate-route panic at merge time.
pub fn router(state: ProxyState) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(routes::chat_completions))
        .route("/v1/responses", post(routes::responses))
        .route("/v1/messages", post(routes::messages))
        .route("/v1/messages/count_tokens", post(routes::count_tokens))
        .route("/v1/models", get(routes::models))
        // 只挂 `.get()`。历史上这条路径还挂了 `.post(gemini_generate)`
        // —— Google Generative Language API 的 GA 别名 —— 它随 `/v1beta` 一起删了。
        .route("/v1/models/{model}", get(routes::model_detail))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            kernel::layer,
        ))
        .with_state(state)
}

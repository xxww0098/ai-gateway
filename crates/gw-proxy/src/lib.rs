//! The proxy kernel: tenant auth, pre-flight hold, upstream dispatch, streaming
//! relay, settlement. The kernel is implemented here end to end.
//!
//! Two client-facing prefixes, both metered identically
//! ([`access::is_proxy_path`]): `/v1/*` for the OpenAI and Anthropic dialects,
//! and `/v1beta/*` for Gemini's, because that is the version segment Google's
//! Generative Language API uses and the dashboard advertises it verbatim.
//!
//! What `/v1beta` does **not** cover: Google's non-generation actions
//! (`:countTokens`, `:embedContent`, `:batchEmbedContents`). [`gw_provider`]'s
//! Gemini executor only builds `:generateContent` / `:streamGenerateContent`
//! endpoints, so those actions would be dispatched — and billed — as a
//! generation. Serving them means teaching the provider the other endpoints
//! first; until then they are as absent as they were under the SDK.
//!
//! Request order (must match the B1 fix):
//!   access-auth -> hold -> execute -> parse usage -> settle | release
//!
//! # Layout
//!
//! | module | role |
//! | --- | --- |
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
    /// `.await`, so a client that hangs up mid-stream is settled from a
    /// detached task. A bare `tokio::spawn` dies silently the moment the
    /// runtime is dropped — `serve()` returns, `main` ends, and every in-flight
    /// `Settle` is aborted with its hold left to expire on TTL. That is free
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

/// Builds the metered proxy routes — `/v1/*` and `/v1beta/*` — with the
/// billing middleware stack attached.
///
/// **The layer order is the whole point.** axum applies `.layer()` outermost-
/// last, so the calls below read bottom-up as the execution order:
/// access-auth -> hold -> handler. Getting it backwards is blocker B1 — with
/// hold ahead of authentication, every `/v1` request aborts with a pre-auth 401
/// and the billing path never runs. `access::tests` and `routes::tests` both
/// pin the order so it cannot regress in one place while looking correct in the
/// other, which is exactly why the billing middleware is a single definition.
///
/// Request counting is NOT layered here: `gw_server::metrics::track` wraps the
/// merged router and already scopes itself to `/v1/*`. A second layer would
/// double-count every request into an instance nothing exports. For the same
/// reason there is no `/metrics/prometheus` route here — `gw-server` registers
/// it, and registering it twice is an axum duplicate-route panic at merge time.
pub fn router(state: ProxyState) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(routes::chat_completions))
        .route("/v1/completions", post(routes::completions))
        .route("/v1/responses", post(routes::responses))
        .route("/v1/embeddings", post(routes::embeddings))
        .route("/v1/messages", post(routes::messages))
        .route("/v1/messages/count_tokens", post(routes::count_tokens))
        .route("/v1/models", get(routes::models))
        .route(
            "/v1/models/{model}",
            get(routes::model_detail).post(routes::gemini_generate),
        )
        // The Gemini native surface. Google versions its Generative Language
        // API at `v1beta`, so this is a sibling prefix rather than a `/v1`
        // sub-path — which is exactly why the two middleware gates key on
        // `access::is_proxy_path` and not on `"/v1/"`.
        .route("/v1beta/models", get(routes::gemini_models))
        .route(
            "/v1beta/models/{model}",
            get(routes::gemini_model_detail).post(routes::gemini_generate),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            hold::layer,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            access::layer,
        ))
        .with_state(state)
}

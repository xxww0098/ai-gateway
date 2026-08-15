//! `/api/health` and `/api/health/ready`.
//!
//! The two must not be conflated: a liveness failure restarts the pod, a
//! readiness failure only de-registers it.
//!
//! The backing stores are injected as [`StoreProbe`]s rather than concrete
//! pool handles, so this crate does not depend on how `gw-infra` shapes its
//! Postgres/Redis clients — and so the 503 path is unit-testable without a
//! database.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use tokio::time::Instant;

use crate::response::success;

/// The 2s timeout shared across both readiness pings.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// A future returned by [`StoreProbe::ping`].
pub type ProbeFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// One backing store the readiness probe can reach.
///
/// The `Err` payload is only logged, never returned to the caller (the JSON
/// body says `down`, nothing more, so a probe cannot leak connection strings to
/// an unauthenticated endpoint).
pub trait StoreProbe: Send + Sync + 'static {
    fn ping(&self) -> ProbeFuture<'_>;
}

/// Any owned closure is a probe, so `gw-infra` can wire one without declaring a
/// type:
///
/// ```ignore
/// let pool = pool.clone();
/// health.with_database(Arc::new(move || -> ProbeFuture<'static> {
///     let pool = pool.clone();
///     Box::pin(async move { pool.ping().await.map_err(|e| e.to_string()) })
/// }));
/// ```
impl<F> StoreProbe for F
where
    F: Fn() -> ProbeFuture<'static> + Send + Sync + 'static,
{
    fn ping(&self) -> ProbeFuture<'_> {
        self()
    }
}

/// The stores `/api/health/ready` reports on. A `None` store reports
/// `not_configured` and makes the instance NOT ready — a half-wired process is
/// never allowed to join a load balancer.
#[derive(Clone, Default)]
pub struct HealthState {
    database: Option<Arc<dyn StoreProbe>>,
    redis: Option<Arc<dyn StoreProbe>>,
}

impl std::fmt::Debug for HealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthState")
            .field("database", &self.database.is_some())
            .field("redis", &self.redis.is_some())
            .finish()
    }
}

impl HealthState {
    /// A state with no store wired yet: readiness reports `not_configured`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach the Postgres probe (`gw-infra`'s pool).
    pub fn with_database(mut self, probe: Arc<dyn StoreProbe>) -> Self {
        self.database = Some(probe);
        self
    }

    /// Attach the Redis probe (`gw-infra`'s client).
    pub fn with_redis(mut self, probe: Arc<dyn StoreProbe>) -> Self {
        self.redis = Some(probe);
        self
    }
}

/// The readiness body. NOT wrapped in the `code`/`message`/`data` envelope,
/// which is what the existing probes parse.
#[derive(Debug, Serialize)]
struct Readiness {
    status: &'static str,
    checks: BTreeMap<&'static str, &'static str>,
}

/// `GET /api/health` — the cheap liveness check: the process is up.
pub async fn liveness() -> Response {
    success(serde_json::json!({ "status": "ok" }))
}

/// `GET /healthz` — the same liveness answer, for callers that expect the
/// conventional path.
///
/// # Why this exists at all
///
/// There is no handler to port: the CPA SDK's own HTTP server registered it,
/// and `api.RegisterPanelRoutes` says so out loud —
/// *"`/healthz` is already registered by the SDK's internal server; only
/// register `/api/health` to avoid a duplicate-route panic."* Dropping the SDK
/// dropped the route with it, while `frontend/vite.config.ts` still proxies
/// `/healthz` to the backend, so the path has to keep answering.
///
/// # Why the body is not the panel envelope
///
/// Nothing in this repository parses it — the vite entry is a bare proxy — so
/// the shape is ours to pick, and the useful one for a k8s/Docker probe is a
/// flat `{"status":"ok"}` with the verdict in the status code. That is the same
/// call [`readiness`] already makes, and for the same reason: probes read
/// status codes, not `code: 0`. The enveloped answer stays available at
/// `/api/health`, the shape that was deliberately fixed.
///
/// Liveness only: this deliberately does not ping Postgres or Redis. A probe
/// that fails on a lost dependency restarts the pod instead of de-registering
/// it, which is what [`readiness`] is for.
pub async fn healthz() -> Response {
    Json(serde_json::json!({ "status": "ok" })).into_response()
}

/// `GET /api/health/ready` — pings every backing store; 200 when all are
/// reachable, 503 otherwise so a load balancer stops routing here.
pub async fn readiness(State(state): State<HealthState>) -> Response {
    // One shared 2s budget for both probes (a per-probe timeout would double
    // the worst case an LB has to wait for).
    let deadline = Instant::now() + PROBE_TIMEOUT;

    let mut checks = BTreeMap::new();
    let mut ready = true;

    for (name, probe) in [("database", &state.database), ("redis", &state.redis)] {
        let status = match probe {
            None => {
                ready = false;
                "not_configured"
            }
            Some(probe) => match tokio::time::timeout_at(deadline, probe.ping()).await {
                Ok(Ok(())) => "up",
                Ok(Err(err)) => {
                    tracing::warn!(store = name, error = %err, "readiness probe failed");
                    ready = false;
                    "down"
                }
                Err(_elapsed) => {
                    tracing::warn!(store = name, "readiness probe timed out");
                    ready = false;
                    "down"
                }
            },
        };
        checks.insert(name, status);
    }

    let (code, status) = if ready {
        (StatusCode::OK, "ready")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not_ready")
    };

    (code, Json(Readiness { status, checks })).into_response()
}

#[cfg(test)]
mod tests;

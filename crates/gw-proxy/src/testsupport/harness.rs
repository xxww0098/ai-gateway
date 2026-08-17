//! A fully-wired [`crate::ProxyState`] over the in-memory doubles, plus the
//! request helpers the router tests drive it with.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request as HttpRequest, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use gw_authcore::AuthRecord;
use http_body_util::BodyExt;
use tokio_util::task::TaskTracker;

use crate::channel::{ChannelHealth, ChannelPolicyCache, ChannelPool};
use crate::hold::HoldMiddleware;
use crate::idempotency::IdempotencyManager;
use crate::ports::{ApiKeyRow, AuthCrypto as _, Id, ModelEntry};
use crate::routes::Dispatcher;
use crate::testsupport::{
    FakeAuthStore, FakeCalculator, FakeCatalog, FakeCircuitBreaker, FakeCrypto, FakeDirectory,
    FakeIdempotencyStore, FakeLedger, FakePolicyStore, FakeProvider, FakeQuotaStore,
    FakeRateLimiter, FakeUsageStore, RecordingMetrics, auth_record,
};
use crate::usage::Settlement;
use crate::{AccessProvider, ProxyState};

/// A fully-wired [`ProxyState`] over in-memory doubles.
///
/// This is the equivalent of the `newTestHoldMiddleware` / `newTestPlugin`
/// helpers, gathered in one place.
pub(crate) struct Harness {
    pub(crate) state: ProxyState,
    pub(crate) ledger: Arc<FakeLedger>,
    pub(crate) calc: Arc<FakeCalculator>,
    pub(crate) usage_store: Arc<FakeUsageStore>,
    pub(crate) directory: Arc<FakeDirectory>,
    pub(crate) quota: Arc<FakeQuotaStore>,
    pub(crate) rate_limiter: Arc<FakeRateLimiter>,
    pub(crate) breaker: Arc<FakeCircuitBreaker>,
    pub(crate) idempotency: Arc<FakeIdempotencyStore>,
    pub(crate) provider: Arc<FakeProvider>,
    /// 上游凭证表，用来量化它被加载了多少次（热点 #5）。
    pub(crate) auth_store: Arc<FakeAuthStore>,
    /// The Gemini upstream. Separate from
    /// [`Self::provider`] so a test can tell which dialect actually dispatched.
    pub(crate) gemini: Arc<FakeProvider>,
    /// The Anthropic upstream, for the `/v1/messages` dialect.
    pub(crate) claude: Arc<FakeProvider>,
    pub(crate) catalog: Arc<FakeCatalog>,
    pub(crate) settlement: Arc<Settlement>,
    pub(crate) health: Arc<ChannelHealth>,
    /// The composition root's tracker, as `gw_server::AppState::drain` would
    /// hand it over.
    pub(crate) drain: TaskTracker,
    pub(crate) metrics: Arc<RecordingMetrics>,
}

/// The plaintext key every harness test authenticates with.
pub(crate) const TEST_API_KEY: &str = "cpa-testkey";
/// The tenant that key belongs to.
pub(crate) const TEST_USER_ID: Id = 7;

impl Harness {
    /// Builds the default happy-path wiring: an active key for an active user,
    /// a funded balance, an allowing limiter and a closed breaker.
    pub(crate) fn build() -> Self {
        Self::build_with(vec![auth_record("acct-1", "openai")])
    }

    pub(crate) fn build_with(auths: Vec<AuthRecord>) -> Self {
        Self::build_routed(auths, None)
    }

    /// 装上四级链的 L1/L2/L3 数据源。`None` = 一键回滚到纯前缀猜测（L4），
    /// 也就是收敛前的行为。
    pub(crate) fn build_routed(
        auths: Vec<AuthRecord>,
        resolver: Option<Arc<dyn gw_relay::endpoint::upstream::ChannelResolver>>,
    ) -> Self {
        let ledger = FakeLedger::with_balance(100.0);
        let calc = FakeCalculator::shared();
        let usage_store = FakeUsageStore::shared();
        let directory = FakeDirectory::shared();
        let crypto = FakeCrypto::shared();
        let quota = FakeQuotaStore::shared();
        let rate_limiter = FakeRateLimiter::allowing();
        let breaker = FakeCircuitBreaker::closed();
        let idempotency = FakeIdempotencyStore::shared();
        let provider = FakeProvider::new("openai");
        let claude = FakeProvider::new("claude");
        let gemini = FakeProvider::new("gemini");
        let catalog = Arc::new(FakeCatalog::default());
        catalog.models.lock().push(ModelEntry {
            id: "gpt-4o".to_owned(),
            created: 0,
            owned_by: "openai".to_owned(),
            ..ModelEntry::default()
        });

        directory.with_active_key(
            &crypto.hash_api_key(TEST_API_KEY),
            ApiKeyRow {
                id: 3,
                user_id: TEST_USER_ID,
                group_id: None,
                status: "active".to_owned(),
            },
        );

        let settlement = Arc::new(Settlement::new(
            ledger.clone(),
            calc.clone(),
            usage_store.clone(),
        ));
        let health = Arc::new(ChannelHealth::new(0, Duration::from_secs(30)));
        let policies = Arc::new(ChannelPolicyCache::new(
            Arc::new(FakePolicyStore::default()),
        ));
        let channels = Arc::new(ChannelPool::new(health.clone()).with_policies(policies));

        let hold = Arc::new(
            HoldMiddleware::new(
                ledger.clone(),
                calc.clone(),
                settlement.clone(),
                Duration::from_secs(300),
            )
            .with_quota_store(quota.clone())
            .with_rate_limiter(rate_limiter.clone())
            .with_circuit_breaker(breaker.clone())
            .with_idempotency(Arc::new(IdempotencyManager::new(
                idempotency.clone(),
                crypto.clone(),
                Duration::ZERO,
            ))),
        );

        let auth_store = FakeAuthStore::with(auths);
        let dispatch = Dispatcher::new(
            vec![provider.clone(), claude.clone(), gemini.clone()],
            auth_store.clone(),
            channels,
            settlement.clone(),
        )
        .with_circuit_breaker(breaker.clone())
        .with_catalog(catalog.clone());
        let dispatch = Arc::new(match resolver {
            Some(resolver) => dispatch.with_channel_resolver(resolver),
            None => dispatch,
        });

        let drain = TaskTracker::new();
        let metrics = Arc::new(RecordingMetrics::default());
        let state = ProxyState::new(
            Arc::new(AccessProvider::new(directory.clone(), crypto.clone())),
            hold,
            dispatch,
            drain.clone(),
        )
        .with_metrics(metrics.clone());

        Self {
            state,
            ledger,
            calc,
            usage_store,
            directory,
            quota,
            rate_limiter,
            breaker,
            idempotency,
            provider,
            auth_store,
            gemini,
            claude,
            catalog,
            settlement,
            health,
            drain,
            metrics,
        }
    }

    /// The production router, for end-to-end ordering and failover tests.
    pub(crate) fn router(&self) -> Router {
        crate::router(self.state.clone())
    }

    /// A router with the same middleware stack but a stub handler, so the hold
    /// layer can be exercised without an upstream.
    pub(crate) fn stub_router(&self, status: StatusCode) -> Router {
        let state = self.state.clone();
        Router::new()
            .route(
                "/v1/chat/completions",
                post(move || async move {
                    (status, axum::Json(serde_json::json!({"stub": true}))).into_response()
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::kernel::layer,
            ))
            .with_state(state)
    }
}

/// Builds an authenticated JSON POST to `path`.
pub(crate) fn signed_request(path: &str, body: serde_json::Value) -> HttpRequest<Body> {
    HttpRequest::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .body(Body::from(body.to_string()))
        .expect("request builds")
}

/// Builds an authenticated GET of `path`, for the catalogue routes.
pub(crate) fn signed_get(path: &str) -> HttpRequest<Body> {
    HttpRequest::builder()
        .uri(path)
        .header("authorization", format!("Bearer {TEST_API_KEY}"))
        .body(Body::empty())
        .expect("request builds")
}

/// Builds an unauthenticated JSON POST to `path`.
pub(crate) fn anonymous_request(path: &str, body: serde_json::Value) -> HttpRequest<Body> {
    HttpRequest::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request builds")
}

/// Sends `request` through `router` and returns the status plus decoded body.
pub(crate) async fn send(
    router: Router,
    request: HttpRequest<Body>,
) -> (StatusCode, serde_json::Value) {
    use tower::ServiceExt;
    let response = router.oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// A minimal chat-completions payload.
pub(crate) fn chat_body(model: &str) -> serde_json::Value {
    serde_json::json!({"model": model, "messages": [{"role": "user", "content": "hi"}]})
}

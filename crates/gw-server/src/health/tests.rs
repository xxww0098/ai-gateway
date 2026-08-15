use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::ServiceExt as _;

use super::*;
use crate::AppState;

/// A probe with a fixed answer.
struct Fixed(Result<(), String>);

impl StoreProbe for Fixed {
    fn ping(&self) -> ProbeFuture<'_> {
        let answer = self.0.clone();
        Box::pin(async move { answer })
    }
}

/// A probe that never answers in time — a wedged connection pool, not a
/// refused one.
struct Hangs;

impl StoreProbe for Hangs {
    fn ping(&self) -> ProbeFuture<'_> {
        Box::pin(async {
            tokio::time::sleep(PROBE_TIMEOUT * 4).await;
            Ok(())
        })
    }
}

fn up() -> Arc<dyn StoreProbe> {
    Arc::new(Fixed(Ok(())))
}

fn down() -> Arc<dyn StoreProbe> {
    Arc::new(Fixed(Err("connection refused".to_owned())))
}

async fn get_readiness(health: HealthState) -> (u16, Value) {
    let state = AppState {
        health,
        ..AppState::default()
    };
    let response = crate::base_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/health/ready")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");

    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, serde_json::from_slice(&bytes).expect("json body"))
}

#[tokio::test]
async fn liveness_is_the_standard_envelope() {
    let response = crate::base_router(AppState::default())
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("json body");
    // The frontend and the backend agree on this exact shape.
    assert_eq!(
        body,
        serde_json::json!({"code": 0, "message": "ok", "data": {"status": "ok"}})
    );
}

/// `GET` a path off `base_router` with no store wired at all.
async fn get_unwired(uri: &str) -> (u16, Value) {
    let response = crate::base_router(AppState::default())
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, serde_json::from_slice(&bytes).expect("json body"))
}

#[tokio::test]
async fn healthz_answers_so_the_vite_proxy_entry_is_not_dead() {
    // The SDK used to serve this path; `frontend/vite.config.ts` still proxies
    // it. A 404 here is the whole bug this route exists to prevent.
    let (status, body) = get_unwired("/healthz").await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn healthz_is_liveness_only_and_ignores_the_stores() {
    // Every store unwired — the state readiness reports `not_configured` for.
    // Liveness must still be 200, or a lost dependency would restart the pod
    // instead of de-registering it.
    let (live, _) = get_unwired("/healthz").await;
    let (ready, _) = get_readiness(HealthState::new()).await;
    assert_eq!(live, 200);
    assert_eq!(ready, 503);
}

#[tokio::test]
async fn healthz_is_flat_while_api_health_is_enveloped() {
    // Two shapes on purpose: `/api/health` is the fixed shape, `/healthz` is
    // the probe shape. Collapsing them into one would break whichever caller
    // was reading the other.
    let (_, healthz) = get_unwired("/healthz").await;
    let (_, api_health) = get_unwired("/api/health").await;
    assert!(healthz.get("code").is_none());
    assert_eq!(api_health["data"], healthz);
}

#[tokio::test]
async fn readiness_is_200_when_every_store_answers() {
    let (status, body) =
        get_readiness(HealthState::new().with_database(up()).with_redis(up())).await;

    assert_eq!(status, 200);
    assert_eq!(body["status"], "ready");
    assert_eq!(body["checks"]["database"], "up");
    assert_eq!(body["checks"]["redis"], "up");
}

#[tokio::test]
async fn readiness_is_503_when_a_store_is_down() {
    let (status, body) =
        get_readiness(HealthState::new().with_database(up()).with_redis(down())).await;

    // 503 de-registers the instance from the load balancer without restarting
    // it — the whole reason readiness is separate from liveness.
    assert_eq!(status, 503);
    assert_eq!(body["status"], "not_ready");
    assert_eq!(body["checks"]["database"], "up");
    assert_eq!(body["checks"]["redis"], "down");
}

#[tokio::test]
async fn an_unwired_store_is_reported_and_blocks_readiness() {
    let (status, body) = get_readiness(HealthState::new().with_redis(up())).await;

    assert_eq!(status, 503);
    assert_eq!(body["checks"]["database"], "not_configured");
    assert_eq!(body["checks"]["redis"], "up");
}

#[tokio::test]
async fn readiness_body_never_leaks_the_probe_error() {
    let (_, body) = get_readiness(
        HealthState::new()
            .with_database(Arc::new(Fixed(Err(
                "host=db user=gw password=hunter2".to_owned()
            ))))
            .with_redis(up()),
    )
    .await;

    // The endpoint is unauthenticated: "down" is all a caller may learn.
    assert!(!body.to_string().contains("hunter2"));
}

#[tokio::test]
async fn wedged_stores_share_one_deadline() {
    // Both stores hang. The two pings share a single 2s context, so the
    // whole handler must return in ~one timeout — a per-probe timeout would
    // take twice as long and blow past the load balancer's own budget.
    let started = std::time::Instant::now();
    let (status, body) = get_readiness(
        HealthState::new()
            .with_database(Arc::new(Hangs))
            .with_redis(Arc::new(Hangs)),
    )
    .await;

    assert_eq!(status, 503);
    assert_eq!(body["checks"]["database"], "down");
    assert_eq!(body["checks"]["redis"], "down");
    assert!(
        started.elapsed() < PROBE_TIMEOUT * 2,
        "two hung probes took {:?}, which means they did not share a deadline",
        started.elapsed()
    );
}

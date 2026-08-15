//! Crate-level wiring tests: the routing table and the state plumbing that
//! holds the endpoint modules together.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

use super::*;

async fn status_of(uri: &str) -> StatusCode {
    app_router(AppState::default(), None)
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds")
        .status()
}

#[tokio::test]
async fn every_owned_endpoint_is_mounted_at_its_path() {
    // These five paths are a contract: the container HEALTHCHECK, the k8s
    // readiness probe, the panel dashboard, the Prometheus scrape config and
    // the vite dev proxy all hard-code them.
    assert_eq!(status_of("/api/health").await, StatusCode::OK);
    // `/healthz` came from the SDK; it is proxied by
    // `frontend/vite.config.ts` and must not 404 now that the SDK is gone.
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
    assert_eq!(status_of("/metrics").await, StatusCode::OK);
    assert_eq!(status_of("/metrics/prometheus").await, StatusCode::OK);
    // No stores are wired into a default AppState, so readiness is honest
    // about it rather than reporting a green instance.
    assert_eq!(
        status_of("/api/health/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn unknown_paths_are_not_served_by_this_crate() {
    // /v1/* and /api/panel/** belong to the merged routers; until those land,
    // nothing here may answer for them.
    assert_eq!(
        status_of("/v1/chat/completions").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of("/api/panel/user/profile").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(status_of("/nope").await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_metrics_layer_does_not_disturb_owned_routes() {
    // app_router wraps everything in the metrics layer; base_router does not.
    // Both must answer identically on this crate's own endpoints.
    let layered = status_of("/api/health").await;
    let bare = base_router(AppState::default())
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds")
        .status();
    assert_eq!(layered, bare);
}

#[tokio::test]
async fn merged_domain_routes_are_served_and_counted() {
    // Stands in for gw_proxy::router() / gw_panel::router(). The metrics layer
    // is applied AFTER the merge, so it must wrap these too — in axum a layer
    // only covers the routes registered before it, and getting this backwards
    // silently zeroes every panel and proxy metric.
    let state = AppState::default();
    let metrics = Arc::clone(&state.metrics);
    let domains = Router::new()
        .route(
            "/api/panel/user/profile",
            axum::routing::get(|| async { "panel" }),
        )
        .route("/v1/models", axum::routing::get(|| async { "proxy" }));

    let app = app_router(state, Some(domains));

    for uri in ["/api/panel/user/profile", "/v1/models"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router responds");
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
    }

    // ...and this crate's own endpoints still answer after the merge.
    assert_eq!(
        app.oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .expect("request")
        )
        .await
        .expect("router responds")
        .status(),
        StatusCode::OK
    );

    let snapshot = metrics.snapshot();
    assert_eq!(
        snapshot.total_requests, 1,
        "the panel request was not counted"
    );
    let mut text = String::new();
    metrics.write_prometheus(&mut text);
    assert!(
        text.contains("cpa_v1_requests_total{status_class=\"2xx\"} 1"),
        "the /v1 request was not counted: {text}"
    );
}

#[test]
fn app_state_hands_each_extractor_the_shared_instance() {
    let state = AppState::default();
    let metrics: Arc<Metrics> = FromRef::from_ref(&state);
    metrics.set_orphaned_holds(3);

    // gw-proxy and gw-ledger publish gauges through their own clone of the
    // Arc; a per-extractor copy would silently drop those writes.
    let second: Arc<Metrics> = FromRef::from_ref(&state);
    let mut text = String::new();
    second.write_prometheus(&mut text);
    assert!(text.contains("cpa_orphaned_holds 3"), "{text}");
}

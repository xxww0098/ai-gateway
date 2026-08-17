use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use serde_json::Value;
use tower::ServiceExt as _;

use super::*;
use crate::AppState;

fn prometheus_text(metrics: &Metrics) -> String {
    let mut out = String::new();
    metrics.write_prometheus(&mut out);
    out
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json body")
}

// ---------------------------------------------------------------------------
// The JSON snapshot (panel surface)
// ---------------------------------------------------------------------------

#[test]
fn a_fresh_snapshot_reports_no_traffic() {
    let snapshot = Metrics::new().snapshot();
    assert_eq!(snapshot.total_requests, 0);
    assert_eq!(snapshot.in_flight, 0);
    assert_eq!(snapshot.total_latency_ms, 0);
    assert_eq!(snapshot.average_latency_ms, 0.0, "no divide-by-zero");
    assert!(snapshot.status_counts.is_empty());
    assert!(snapshot.path_counts.is_empty());
    // RFC3339, UTC, seconds precision.
    assert!(
        snapshot.started_at.ends_with('Z'),
        "{}",
        snapshot.started_at
    );
}

#[test]
fn in_flight_rises_between_start_and_finish() {
    let metrics = Metrics::new();
    metrics.panel_start("/api/panel/user/profile");
    metrics.panel_start("/api/panel/user/profile");
    assert_eq!(metrics.snapshot().in_flight, 2);

    metrics.panel_finish(200, Duration::from_millis(10));
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.in_flight, 1);
    assert_eq!(
        snapshot.total_requests, 2,
        "total counts starts, not finishes"
    );
}

#[test]
fn in_flight_never_goes_negative() {
    // A finish without a start (a panicking
    // handler, a mid-flight reload) must not drive the gauge below zero and
    // make the panel show a negative in-flight count forever.
    let metrics = Metrics::new();
    metrics.panel_finish(500, Duration::from_millis(1));
    assert_eq!(metrics.snapshot().in_flight, 0);
}

#[test]
fn latency_is_summed_and_averaged_over_all_requests() {
    let metrics = Metrics::new();
    for _ in 0..4 {
        metrics.panel_start("/api/panel/x");
        metrics.panel_finish(200, Duration::from_millis(50));
    }
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.total_latency_ms, 200);
    assert_eq!(snapshot.average_latency_ms, 50.0);
}

#[test]
fn status_and_path_counts_accumulate_per_key() {
    let metrics = Metrics::new();
    metrics.panel_start("/api/panel/a");
    metrics.panel_finish(200, Duration::ZERO);
    metrics.panel_start("/api/panel/a");
    metrics.panel_finish(404, Duration::ZERO);
    metrics.panel_start("/api/panel/b");
    metrics.panel_finish(200, Duration::ZERO);

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.status_counts[&200], 2);
    assert_eq!(snapshot.status_counts[&404], 1);
    assert_eq!(snapshot.path_counts["/api/panel/a"], 2);
    assert_eq!(snapshot.path_counts["/api/panel/b"], 1);
}

#[test]
fn an_empty_path_is_recorded_as_unmatched() {
    let metrics = Metrics::new();
    metrics.panel_start("");
    assert_eq!(metrics.snapshot().path_counts["unmatched"], 1);
}

#[test]
fn path_counts_cannot_grow_without_bound() {
    // The key is the request path, which a caller controls; spraying unique
    // URLs must not turn the metrics map into a memory leak.
    let metrics = Metrics::new();
    for i in 0..(PATH_COUNTS_CAP * 3) {
        metrics.panel_start(&format!("/api/panel/{i}"));
    }
    let snapshot = metrics.snapshot();
    assert!(snapshot.path_counts.len() <= PATH_COUNTS_CAP + 1);
    assert!(snapshot.path_counts.contains_key(OVERFLOW_PATH));
    // Nothing is lost, only bucketed: every start is still counted somewhere.
    let counted: u64 = snapshot.path_counts.values().sum();
    assert_eq!(counted, snapshot.total_requests);
}

#[test]
fn status_counts_serialize_with_numeric_keys_as_strings() {
    // The map is marshaled with stringified keys; the panel parses them
    // that way.
    let metrics = Metrics::new();
    metrics.panel_start("/api/panel/a");
    metrics.panel_finish(503, Duration::ZERO);

    let json = serde_json::to_value(metrics.snapshot()).expect("snapshot serializes");
    assert_eq!(json["status_counts"]["503"], 1);
}

// ---------------------------------------------------------------------------
// Prometheus exposition (/v1 surface)
// ---------------------------------------------------------------------------

#[test]
fn v1_requests_are_bucketed_by_status_class() {
    let metrics = Metrics::new();
    for status in [200, 201, 302] {
        metrics.record_v1(status, Duration::ZERO);
    }
    for status in [400, 401, 429] {
        metrics.record_v1(status, Duration::ZERO);
    }
    // Status 0 means "the handler never wrote one" — counted as 5xx so a
    // dropped connection cannot masquerade as success.
    for status in [500, 502, 0] {
        metrics.record_v1(status, Duration::ZERO);
    }

    let text = prometheus_text(&metrics);
    assert!(
        text.contains("agw_v1_requests_total{status_class=\"2xx\"} 3"),
        "{text}"
    );
    assert!(
        text.contains("agw_v1_requests_total{status_class=\"4xx\"} 3"),
        "{text}"
    );
    assert!(
        text.contains("agw_v1_requests_total{status_class=\"5xx\"} 3"),
        "{text}"
    );
}

#[test]
fn v1_latency_is_reported_in_seconds() {
    let metrics = Metrics::new();
    metrics.record_v1(200, Duration::from_millis(1500));
    metrics.record_v1(200, Duration::from_millis(500));

    let text = prometheus_text(&metrics);
    assert!(
        text.contains("agw_v1_request_duration_seconds_sum 2.000000"),
        "{text}"
    );
    assert!(
        text.contains("agw_v1_request_duration_seconds_count 2"),
        "{text}"
    );
}

#[test]
fn exposition_matches_the_established_handler_byte_for_byte() {
    // Golden copied from the Prometheus exposition + the two gauges that the
    // /metrics/prometheus endpoint appends.
    // Dashboards and alerts match on these exact metric names and types.
    let metrics = Metrics::new();
    metrics.set_channel_benched(2);
    metrics.set_orphaned_holds(7);

    let expected = "\
# HELP agw_v1_requests_total Total /v1 proxy requests by HTTP status class.
# TYPE agw_v1_requests_total counter
agw_v1_requests_total{status_class=\"2xx\"} 0
agw_v1_requests_total{status_class=\"4xx\"} 0
agw_v1_requests_total{status_class=\"5xx\"} 0
# HELP agw_v1_request_duration_seconds Cumulative /v1 request latency.
# TYPE agw_v1_request_duration_seconds summary
agw_v1_request_duration_seconds_sum 0.000000
agw_v1_request_duration_seconds_count 0
# HELP agw_channel_benched_total Upstream accounts currently benched by health checks.
# TYPE agw_channel_benched_total gauge
agw_channel_benched_total 2
# HELP agw_orphaned_holds Holds orphaned by a prior crash (read-only B8 detection).
# TYPE agw_orphaned_holds gauge
agw_orphaned_holds 7
";

    let text = prometheus_text(&metrics);
    assert!(
        text.starts_with(expected),
        "the parity block must come first and unchanged:\n{text}"
    );
}

#[test]
fn the_rust_only_metrics_are_appended_after_the_parity_block() {
    // agw_abandoned_settlements has no upstream counterpart (the finalizer runs on
    // the server's own task and cannot be dropped by a runtime teardown). It is
    // appended, never interleaved, so a dashboard matching the existing exposition
    // keeps working — Prometheus exposition is additive.
    let metrics = Metrics::new();
    metrics.set_abandoned_settlements(4);

    let text = prometheus_text(&metrics);
    let expected = "\
# HELP agw_abandoned_settlements Settlements still running when the shutdown drain timed out.
# TYPE agw_abandoned_settlements gauge
agw_abandoned_settlements 4
";
    assert!(text.ends_with(expected), "{text}");
    assert!(
        text.find("agw_orphaned_holds").expect("go block") < text.find(expected).expect("suffix"),
        "the Rust-only block must not be interleaved with the parity one",
    );
}

// ---------------------------------------------------------------------------
// Endpoints + middleware
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_endpoint_wraps_the_snapshot_in_the_envelope() {
    let response = crate::base_router(AppState::default())
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["code"], 0);
    assert_eq!(body["message"], "ok");
    for field in [
        "started_at",
        "uptime_seconds",
        "total_requests",
        "in_flight",
        "status_counts",
        "path_counts",
        "total_latency_ms",
        "average_latency_ms",
    ] {
        assert!(!body["data"][field].is_null(), "missing {field}");
    }
}

#[tokio::test]
async fn prometheus_endpoint_announces_the_exposition_content_type() {
    let response = crate::base_router(AppState::default())
        .oneshot(
            Request::builder()
                .uri("/metrics/prometheus")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::OK);
    // A scraper rejects the payload without version=0.0.4.
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some(PROMETHEUS_CONTENT_TYPE)
    );
}

#[tokio::test]
async fn the_layer_counts_panel_and_v1_traffic_only() {
    let metrics = Arc::new(Metrics::new());
    let app = Router::new()
        .route("/api/panel/thing", get(|| async { "panel" }))
        .route("/v1/chat/completions", get(|| async { "proxy" }))
        .route("/api/health", get(|| async { "health" }))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&metrics),
            track,
        ));

    for uri in ["/api/panel/thing", "/v1/chat/completions", "/api/health"] {
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

    let snapshot = metrics.snapshot();
    // The JSON metrics scopes to the /api/panel group and the Prometheus
    // counters to /v1/, leaving health checks and scrapes uncounted.
    assert_eq!(snapshot.total_requests, 1);
    assert_eq!(snapshot.in_flight, 0);
    assert!(snapshot.path_counts.contains_key("/api/panel/thing"));
    assert!(
        prometheus_text(&metrics).contains("agw_v1_requests_total{status_class=\"2xx\"} 1"),
        "the /v1 request was not counted"
    );
}

#[tokio::test]
async fn the_layer_records_the_response_status() {
    let metrics = Arc::new(Metrics::new());
    let app = Router::new()
        .route(
            "/api/panel/boom",
            get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
        )
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&metrics),
            track,
        ));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/panel/boom")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    assert_eq!(metrics.snapshot().status_counts[&500], 1);
}

//! `/metrics` (JSON) and `/metrics/prometheus` (text exposition).
//!
//! Two separate collectors, scraped from the same process:
//!
//! * a JSON snapshot of the `/api/panel/**` surface (the panel dashboard reads
//!   it);
//! * Prometheus text for the `/v1/*` proxy surface, plus the
//!   `agw_channel_benched_total` / `agw_orphaned_holds` gauges.
//!
//! Hand-rolling the exposition format (rather than taking a Prometheus client
//! dependency) is deliberate and keeps the output strictly compatible.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;

use crate::response::success;

/// Prometheus text exposition format version.
pub const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// The bucket key for requests matching no tracked route.
const UNMATCHED_PATH: &str = "unmatched";

/// Distinct paths tracked in `path_counts` before the rest is folded into a
/// single bucket.
///
/// Keys this map by request path. Routing to the matched ROUTE would be
/// bounded by the routing table, but axum's `MatchedPath` extractor needs a
/// cargo feature this crate does not enable, so the request path is used
/// instead — that is attacker-controlled and unbounded, hence the cap. Without
/// it, spraying unique URLs would grow the map without limit.
const PATH_COUNTS_CAP: usize = 512;

/// The bucket that absorbs paths beyond [`PATH_COUNTS_CAP`].
const OVERFLOW_PATH: &str = "other";

/// Process-wide request metrics. Cheap enough to sit on the hot path: counters
/// are atomics and only the two maps take a short lock.
#[derive(Debug)]
pub struct Metrics {
    started_at: DateTime<Utc>,
    started_instant: Instant,

    // Panel surface (JSON snapshot).
    total_requests: AtomicU64,
    in_flight: AtomicI64,
    total_latency_nanos: AtomicU64,
    status_counts: Mutex<BTreeMap<u16, u64>>,
    path_counts: Mutex<BTreeMap<String, u64>>,

    // /v1 proxy surface (Prometheus).
    v1_2xx: AtomicU64,
    v1_4xx: AtomicU64,
    v1_5xx: AtomicU64,
    v1_duration_nanos: AtomicU64,
    v1_duration_count: AtomicU64,

    // Gauges fed by other subsystems.
    channel_benched: AtomicI64,
    orphaned_holds: AtomicI64,
    abandoned_settlements: AtomicI64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// `started_at` is captured once, at boot.
    pub fn new() -> Self {
        Self {
            started_at: Utc::now(),
            started_instant: Instant::now(),
            total_requests: AtomicU64::new(0),
            in_flight: AtomicI64::new(0),
            total_latency_nanos: AtomicU64::new(0),
            status_counts: Mutex::new(BTreeMap::new()),
            path_counts: Mutex::new(BTreeMap::new()),
            v1_2xx: AtomicU64::new(0),
            v1_4xx: AtomicU64::new(0),
            v1_5xx: AtomicU64::new(0),
            v1_duration_nanos: AtomicU64::new(0),
            v1_duration_count: AtomicU64::new(0),
            channel_benched: AtomicI64::new(0),
            orphaned_holds: AtomicI64::new(0),
            abandoned_settlements: AtomicI64::new(0),
        }
    }

    /// Record that a panel request started.
    pub fn panel_start(&self, path: &str) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.in_flight.fetch_add(1, Ordering::Relaxed);

        let key = if path.is_empty() {
            UNMATCHED_PATH
        } else {
            path
        };
        let mut counts = self.path_counts.lock().expect("path_counts lock");
        if let Some(count) = counts.get_mut(key) {
            *count += 1;
        } else if counts.len() < PATH_COUNTS_CAP {
            counts.insert(key.to_owned(), 1);
        } else {
            *counts.entry(OVERFLOW_PATH.to_owned()).or_insert(0) += 1;
        }
    }

    /// Record that a panel request finished.
    pub fn panel_finish(&self, status: u16, latency: Duration) {
        // Saturating at zero keeps a finish-without-start from underflowing
        // the gauge.
        let _ = self
            .in_flight
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                (current > 0).then_some(current - 1)
            });
        *self
            .status_counts
            .lock()
            .expect("status_counts lock")
            .entry(status)
            .or_insert(0) += 1;
        self.total_latency_nanos
            .fetch_add(latency.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Bucket by status class and accumulate latency for the `/v1` proxy
    /// surface.
    pub fn record_v1(&self, status: u16, latency: Duration) {
        let bucket = if status >= 500 || status == 0 {
            &self.v1_5xx
        } else if status >= 400 {
            &self.v1_4xx
        } else {
            &self.v1_2xx
        };
        bucket.fetch_add(1, Ordering::Relaxed);
        if !latency.is_zero() {
            self.v1_duration_nanos
                .fetch_add(latency.as_nanos() as u64, Ordering::Relaxed);
        }
        self.v1_duration_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Upstream accounts currently benched by health checks. Fed by
    /// `gw-proxy`'s channel health.
    pub fn set_channel_benched(&self, count: i64) {
        self.channel_benched.store(count, Ordering::Relaxed);
    }

    /// Holds orphaned by a prior crash. Fed by the startup + 5-minute scan.
    pub fn set_orphaned_holds(&self, count: i64) {
        self.orphaned_holds.store(count, Ordering::Relaxed);
    }

    /// Settlements still running when the shutdown drain timed out — see
    /// [`crate::drain`]. Each one is a hold left to expire by TTL, i.e. unbilled
    /// upstream usage.
    ///
    /// Detached settlement tasks have no other failure mode to count here. The
    /// gauge is written after the listener has already closed, so the `warn!`
    /// this pairs with is the signal operators actually see; the gauge exists
    /// for a long drain that is still being scraped, and for an in-process
    /// exporter later.
    pub fn set_abandoned_settlements(&self, count: i64) {
        self.abandoned_settlements.store(count, Ordering::Relaxed);
    }

    /// The JSON snapshot of the panel surface.
    pub fn snapshot(&self) -> MetricsSnapshot {
        let total_requests = self.total_requests.load(Ordering::Relaxed);
        let total_latency_ms =
            (self.total_latency_nanos.load(Ordering::Relaxed) / 1_000_000) as i64;
        let average_latency_ms = if total_requests > 0 {
            total_latency_ms as f64 / total_requests as f64
        } else {
            0.0
        };

        MetricsSnapshot {
            started_at: self.started_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            uptime_seconds: self.started_instant.elapsed().as_secs() as i64,
            total_requests,
            in_flight: self.in_flight.load(Ordering::Relaxed),
            status_counts: self
                .status_counts
                .lock()
                .expect("status_counts lock")
                .clone(),
            path_counts: self.path_counts.lock().expect("path_counts lock").clone(),
            total_latency_ms,
            average_latency_ms,
        }
    }

    /// Prometheus text for the `/v1` proxy surface plus the two gauges
    /// appended below. Byte-for-byte identical output, so existing dashboards
    /// and alerts keep matching.
    pub fn write_prometheus(&self, out: &mut String) {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);

        out.push_str(
            "# HELP agw_v1_requests_total Total /v1 proxy requests by HTTP status class.\n",
        );
        out.push_str("# TYPE agw_v1_requests_total counter\n");
        let _ = writeln!(
            out,
            "agw_v1_requests_total{{status_class=\"2xx\"}} {}",
            load(&self.v1_2xx)
        );
        let _ = writeln!(
            out,
            "agw_v1_requests_total{{status_class=\"4xx\"}} {}",
            load(&self.v1_4xx)
        );
        let _ = writeln!(
            out,
            "agw_v1_requests_total{{status_class=\"5xx\"}} {}",
            load(&self.v1_5xx)
        );

        out.push_str("# HELP agw_v1_request_duration_seconds Cumulative /v1 request latency.\n");
        out.push_str("# TYPE agw_v1_request_duration_seconds summary\n");
        // Print exactly six decimals.
        let _ = writeln!(
            out,
            "agw_v1_request_duration_seconds_sum {:.6}",
            load(&self.v1_duration_nanos) as f64 / 1e9
        );
        let _ = writeln!(
            out,
            "agw_v1_request_duration_seconds_count {}",
            load(&self.v1_duration_count)
        );

        out.push_str(
            "# HELP agw_channel_benched_total Upstream accounts currently benched by health checks.\n",
        );
        out.push_str("# TYPE agw_channel_benched_total gauge\n");
        let _ = writeln!(
            out,
            "agw_channel_benched_total {}",
            self.channel_benched.load(Ordering::Relaxed)
        );

        out.push_str(
            "# HELP agw_orphaned_holds Holds orphaned by a prior crash (read-only B8 detection).\n",
        );
        out.push_str("# TYPE agw_orphaned_holds gauge\n");
        let _ = writeln!(
            out,
            "agw_orphaned_holds {}",
            self.orphaned_holds.load(Ordering::Relaxed)
        );

        // Rust-only, appended AFTER the parity block above so existing
        // dashboards keep matching (Prometheus exposition is additive).
        out.push_str(
            "# HELP agw_abandoned_settlements Settlements still running when the shutdown drain timed out.\n",
        );
        out.push_str("# TYPE agw_abandoned_settlements gauge\n");
        let _ = writeln!(
            out,
            "agw_abandoned_settlements {}",
            self.abandoned_settlements.load(Ordering::Relaxed)
        );
    }
}

/// Lets `gw-proxy` publish its two gauges without depending on this crate.
///
/// The proxy holds an `Arc<dyn MetricsSink>` (`gw_proxy::ports::MetricsSink`)
/// so it never learns the exposition format; the composition root hands it the
/// same [`Metrics`] instance `/metrics/prometheus` scrapes. The adapter exists
/// because the two live in different crates with a one-way dependency.
impl gw_proxy::MetricsSink for Metrics {
    fn set_channel_benched(&self, count: i64) {
        Metrics::set_channel_benched(self, count);
    }

    fn set_orphaned_holds(&self, count: i64) {
        Metrics::set_orphaned_holds(self, count);
    }
}

/// The `/metrics` payload. Field names and order are the established wire
/// contract.
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub started_at: String,
    pub uptime_seconds: i64,
    pub total_requests: u64,
    pub in_flight: i64,
    pub status_counts: BTreeMap<u16, u64>,
    pub path_counts: BTreeMap<String, u64>,
    pub total_latency_ms: i64,
    pub average_latency_ms: f64,
}

/// The metrics middleware, merged into one layer.
///
/// Scope is exact: the JSON snapshot counts `/api/panel/**` only (the panel
/// group's middleware) and the Prometheus counters count `/v1/*` only. Health
/// checks and scrapes are deliberately not counted.
pub async fn track(State(metrics): State<Arc<Metrics>>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let panel = path.starts_with("/api/panel");
    let v1 = path.starts_with("/v1/");

    if !panel && !v1 {
        return next.run(request).await;
    }

    let start = Instant::now();
    if panel {
        metrics.panel_start(&path);
    }

    let response = next.run(request).await;

    let latency = start.elapsed();
    let status = response.status().as_u16();
    if panel {
        metrics.panel_finish(status, latency);
    }
    if v1 {
        metrics.record_v1(status, latency);
    }
    response
}

/// `GET /metrics` — JSON snapshot, wrapped in the standard envelope.
pub async fn json(State(metrics): State<Arc<Metrics>>) -> Response {
    success(metrics.snapshot())
}

/// `GET /metrics/prometheus` — text exposition for the `/v1` proxy surface.
pub async fn prometheus(State(metrics): State<Arc<Metrics>>) -> Response {
    let mut body = String::new();
    metrics.write_prometheus(&mut body);
    ([(CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)], body).into_response()
}

#[cfg(test)]
mod tests;

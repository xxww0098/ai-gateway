//! Draining detached settlement tasks at shutdown.
//!
//! # Why this module exists
//!
//! `gw-proxy`'s `StreamSettler::drop` settles a request whose client hung up
//! mid-stream by spawning a detached task — `drop` cannot `.await`. Those tasks
//! are the ONLY thing standing between a mid-stream disconnect and a lost
//! `Settle`, and a detached task dies silently when the runtime is dropped:
//! `run()` returns, `main` ends, the runtime tears down, and every in-flight
//! settlement is aborted. The hold then sits there until its TTL expires and
//! the tenant got free upstream output — a violation of the billing invariants
//! in `AGENTS.md`.
//!
//!
//! The fix is a shared [`TaskTracker`]: `gw-proxy` spawns settlements into it
//! instead of calling `tokio::spawn`, and this module waits it out before the
//! process exits.
//!
//! # Ordering (load-bearing)
//!
//! ```text
//! axum graceful shutdown returns   // connections closed, response bodies
//!                                  // dropped, StreamSettler::drop has run
//!                                  // and spawned its settlements
//!   -> tracker.close()             // now, and not one instant earlier
//!   -> tracker.wait()
//! ```
//!
//! [`TaskTracker::wait`] completes when the tracker is **closed and empty**.
//! Closing early risks catching a momentary zero — `wait()` returns, a late
//! `drop` spawns its settlement afterwards, and that one is lost anyway.
//! Closing does not block later spawns, so a settlement that arrives *during*
//! the drain is still waited on; that is exactly what we want here.

use std::time::{Duration, Instant};

use gw_config::BillingConfig;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};

/// Hard ceiling on the shutdown drain, regardless of configuration.
///
/// A drain runs inside the orchestrator's termination grace period (30s by
/// default on Kubernetes); overshooting it converts a graceful shutdown into a
/// `SIGKILL`, which loses strictly more settlements than giving up early does.
pub const DRAIN_TIMEOUT_CAP: Duration = Duration::from_secs(30);

/// How long to wait for in-flight settlements: the hold TTL, capped by
/// [`DRAIN_TIMEOUT_CAP`].
///
/// The hold TTL is the upper bound that matters — past it the reservation has
/// expired on its own, so waiting longer settles nothing.
pub fn drain_timeout(billing: &BillingConfig) -> Duration {
    billing.hold_ttl().min(DRAIN_TIMEOUT_CAP)
}

/// What the drain managed to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainOutcome {
    /// Settlements that completed while we waited.
    pub drained: usize,
    /// Settlements still running when the timeout fired. **Every one of these
    /// is a hold that will not be settled** — real money, not a warning.
    pub abandoned: usize,
    /// Wall-clock time spent draining.
    pub elapsed: Duration,
}

impl DrainOutcome {
    /// Whether every tracked settlement finished.
    pub fn is_complete(&self) -> bool {
        self.abandoned == 0
    }
}

/// Close `tracker` and wait for the settlements already spawned into it.
///
/// MUST be called only after the HTTP server's graceful shutdown has returned
/// — see the module docs for why the ordering is load-bearing.
pub async fn drain_settlements(tracker: &TaskTracker, timeout: Duration) -> DrainOutcome {
    let started = Instant::now();
    let pending = tracker.len();

    tracker.close();

    if pending > 0 {
        info!(
            pending,
            timeout_ms = timeout.as_millis() as u64,
            "draining in-flight settlements",
        );
        // Timing out is not an error path we can recover from: the tasks keep
        // running until the runtime drops, we simply stop waiting.
        let _ = tokio::time::timeout(timeout, tracker.wait()).await;
    }

    let abandoned = tracker.len();
    let outcome = DrainOutcome {
        drained: pending.saturating_sub(abandoned),
        abandoned,
        elapsed: started.elapsed(),
    };

    if outcome.abandoned > 0 {
        warn!(
            drained = outcome.drained,
            abandoned = outcome.abandoned,
            elapsed_ms = outcome.elapsed.as_millis() as u64,
            "settlement drain timed out — abandoned settlements leave their holds to expire by TTL, which means unbilled upstream usage",
        );
    } else if outcome.drained > 0 {
        info!(
            drained = outcome.drained,
            elapsed_ms = outcome.elapsed.as_millis() as u64,
            "drained in-flight settlements",
        );
    }

    outcome
}

#[cfg(test)]
mod tests;

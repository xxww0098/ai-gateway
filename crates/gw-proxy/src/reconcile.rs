//! Recovery of holds abandoned by a crashed request.
//!
//! Charging a crash-orphaned request is a policy choice, so the scan always
//! runs (it feeds the `cpa_orphaned_holds` gauge) while the settlement half is
//! opt-in through `BILLING_AUTO_RECONCILE_HOLDS`.
//!
//! Reconciliation is idempotent and safe to run concurrently: a hold whose
//! request already produced a `usage_logs` row is skipped, and the existence
//! check is re-evaluated inside the same transaction as the debit
//! ([`crate::ports::SettlementCommit::skip_if_already_logged`]), so a hold can
//! never be charged twice.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tokio_util::task::TaskTracker;

use crate::ports::{Id, MetricsSink, SettleReceipt, SettlementCommit, UsageLogEntry};
use crate::usage::Settlement;

/// A Redis reservation that outlived its TTL without being settled or released.
/// Matches `gw_ledger::StaleHold`.
#[derive(Debug, Clone, PartialEq)]
pub struct StaleHold {
    pub user_id: Id,
    pub request_id: String,
    pub amount: f64,
}

/// Environment flag that arms automatic reconciliation.
pub const AUTO_RECONCILE_ENV: &str = "BILLING_AUTO_RECONCILE_HOLDS";

/// How stale a hold must be before the scan considers it orphaned.
pub const DEFAULT_STALE_AFTER: Duration = Duration::from_secs(30 * 60);

/// How often the scan runs (5 minutes).
pub const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Source of orphaned reservations.
#[async_trait::async_trait]
pub trait StaleHoldScanner: Send + Sync {
    /// Holds still present well past their TTL, never settled or released.
    async fn scan_stale_holds(&self, older_than: Duration) -> anyhow::Result<Vec<StaleHold>>;
}

/// Whether automatic reconciliation is armed.
pub fn auto_reconcile_enabled() -> bool {
    std::env::var(AUTO_RECONCILE_ENV).is_ok_and(|v| v == "true")
}

/// Settles the given orphaned holds, charging the reserved amount — a
/// conservative estimate, never more — and clearing the reservation.
///
/// Returns how many were reconciled.
pub async fn reconcile_orphaned_holds(settlement: &Settlement, holds: &[StaleHold]) -> usize {
    let mut settled = 0;
    for hold in holds {
        if hold.request_id.is_empty() || hold.user_id == 0 || hold.amount <= 0.0 {
            continue;
        }
        let entry = UsageLogEntry {
            user_id: hold.user_id,
            request_id: hold.request_id.clone(),
            total_cost: hold.amount,
            actual_cost: hold.amount,
            cost: hold.amount,
            rate_multiplier: 1.0,
            failed: false,
            raw_metadata: Some(json!({
                "reconciled": true,
                "reason": "orphaned_hold",
                "timestamp": Utc::now().to_rfc3339(),
            })),
            ..UsageLogEntry::default()
        };
        let commit = SettlementCommit {
            user_id: hold.user_id,
            request_id: hold.request_id.clone(),
            actual_cost: hold.amount,
            entry,
            subscription_id: None,
            // The guard that makes this idempotent: re-checked inside the
            // transaction, so a concurrent run cannot double-charge.
            skip_if_already_logged: true,
        };

        match settlement.store().commit_settlement(&commit).await {
            Ok(SettleReceipt::Committed { .. }) => {
                if let Err(err) = settlement
                    .store()
                    .clear_hold(hold.user_id, &hold.request_id)
                    .await
                {
                    tracing::warn!(user_id = hold.user_id, request_id = %hold.request_id, %err,
                        "clear hold after reconcile failed");
                }
                settled += 1;
            }
            // Already settled, or an infrastructure error: either way do not
            // clear the reservation and do not count it.
            Ok(SettleReceipt::AlreadySettled) => {}
            Err(err) => {
                tracing::warn!(user_id = hold.user_id, request_id = %hold.request_id, %err,
                    "reconcile settle failed");
            }
        }
    }
    settled
}

/// One scan: publish the gauge, and reconcile when armed.
pub async fn scan_once(
    scanner: &dyn StaleHoldScanner,
    settlement: &Settlement,
    metrics: &dyn MetricsSink,
    auto_reconcile: bool,
    stale_after: Duration,
) -> usize {
    let stale = match scanner.scan_stale_holds(stale_after).await {
        Ok(stale) => stale,
        Err(err) => {
            tracing::warn!(%err, "stale hold scan failed");
            return 0;
        }
    };
    metrics.set_orphaned_holds(stale.len() as i64);
    if stale.is_empty() {
        return 0;
    }
    tracing::warn!(
        count = stale.len(),
        auto_reconcile,
        "orphaned holds detected (likely a prior crash)",
    );
    if !auto_reconcile {
        return 0;
    }
    let n = reconcile_orphaned_holds(settlement, &stale).await;
    if n > 0 {
        tracing::info!(count = n, "auto-reconciled orphaned holds");
    }
    n
}

/// Runs [`scan_once`] immediately, then on a ticker until `shutdown` resolves.
///
/// The loop ends when the root context is cancelled on SIGINT/SIGTERM.
///
/// # Why the loop is NOT on the drain tracker
///
/// It never finishes on its own. [`tokio_util::task::TaskTracker::wait`]
/// completes only when the tracker is closed **and empty**, so registering a
/// perpetual loop makes every shutdown block until the drain's timeout, and
/// then report the loop as an abandoned settlement — poisoning the one alarm
/// that is supposed to fire only when a real charge was lost. The loop instead
/// exits on `shutdown` and simply ends.
///
/// What *does* go on the tracker is the scan that can move money: with
/// `auto_reconcile` on, a reconcile straddling the shutdown instant would
/// otherwise be aborted mid-debit when the runtime drops. With it off nothing
/// is charged, so the scan runs inline and the drain stays free of read-only
/// work in the default configuration.
///
/// `auto_reconcile` is a parameter rather than an
/// [`auto_reconcile_enabled`] call inside — the environment is read once and
/// closed over — letting the tests drive both paths without touching
/// process-wide state.
pub fn spawn_scanner(
    scanner: Arc<dyn StaleHoldScanner>,
    settlement: Arc<Settlement>,
    metrics: Arc<dyn MetricsSink>,
    interval: Duration,
    drain: TaskTracker,
    auto_reconcile: bool,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                // Biased so a shutdown that arrives together with a due tick
                // wins, rather than starting a scan the drain then waits for.
                biased;
                () = &mut shutdown => break,
                _ = ticker.tick() => {}
            }

            if auto_reconcile {
                let job = drain.spawn(scan_owned(
                    scanner.clone(),
                    settlement.clone(),
                    metrics.clone(),
                    true,
                    DEFAULT_STALE_AFTER,
                ));
                // Awaited, not fire-and-forget: two overlapping reconciles
                // would race for the same orphaned holds.
                if let Err(err) = job.await {
                    tracing::warn!(%err, "orphaned-hold reconcile task failed");
                }
            } else {
                scan_once(
                    scanner.as_ref(),
                    settlement.as_ref(),
                    metrics.as_ref(),
                    false,
                    DEFAULT_STALE_AFTER,
                )
                .await;
            }
        }
        tracing::debug!("orphaned-hold scanner stopped");
    })
}

/// [`scan_once`] over owned handles, so it can be spawned onto the tracker.
async fn scan_owned(
    scanner: Arc<dyn StaleHoldScanner>,
    settlement: Arc<Settlement>,
    metrics: Arc<dyn MetricsSink>,
    auto_reconcile: bool,
    stale_after: Duration,
) -> usize {
    scan_once(
        scanner.as_ref(),
        settlement.as_ref(),
        metrics.as_ref(),
        auto_reconcile,
        stale_after,
    )
    .await
}

#[cfg(test)]
mod tests;

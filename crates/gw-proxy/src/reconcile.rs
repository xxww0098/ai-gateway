//! Recovery of billing operations abandoned by a crashed request.
//!
//! Charging a crash-orphaned request is a policy choice, so the scan always
//! runs (it feeds the `agw_orphaned_holds` gauge) while the settlement half is
//! opt-in through `BILLING_AUTO_RECONCILE_HOLDS`.
//!
//! # The scan reads Postgres
//!
//! The input is the set of non-terminal `billing_operations` rows, **not** a
//! Redis `SCAN` for surviving reservations. A reservation is a cache entry: it
//! expires on a TTL, it can be evicted, and it dies with its box — none of
//! which is evidence about whether the money was accounted for. The `held` row
//! is that evidence, and it is the only thing that outlives a crash.
//!
//! # It cannot double-charge
//!
//! Not because the caller remembers to ask. `commit_settlement` moves the
//! operation to a terminal state inside the same transaction as the debit, so
//! a second reconciler — or the request's own late settle — gets
//! [`SettleReceipt::AlreadyTerminal`] and moves nothing. There is no flag to
//! forget.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tokio_util::task::TaskTracker;

use gw_ledger::BillingOperationId;

use crate::ports::{Id, MetricsSink, SettleReceipt, SettlementCommit, UsageLogEntry};
use crate::usage::Settlement;

/// A `billing_operations` row still `held` long after its request should have
/// finished. Matches `gw_ledger::NonTerminalOperation`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrphanedOperation {
    pub user_id: Id,
    /// The money key. Everything below charges against this and nothing else.
    pub operation: BillingOperationId,
    /// What the client saw, carried through so the reconciled usage row still
    /// joins to the tenant's own logs.
    pub client_trace_id: String,
    /// The reserved upper bound — the most this operation may be charged.
    pub reserved_amount: f64,
}

/// Environment flag that arms automatic reconciliation.
pub const AUTO_RECONCILE_ENV: &str = "BILLING_AUTO_RECONCILE_HOLDS";

/// How old a non-terminal operation must be before the scan treats its request
/// as dead.
pub const DEFAULT_STALE_AFTER: Duration = Duration::from_secs(30 * 60);

/// How often the scan runs (5 minutes).
pub const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How many operations one scan may take on.
///
/// Bounded so a large backlog cannot turn a periodic job into an unbounded
/// read; whatever is left is picked up on the next tick. When a scan comes
/// back full, [`scan_once`] says so in the log rather than reporting a
/// truncated count as if it were the whole picture.
pub const DEFAULT_SCAN_LIMIT: i64 = 500;

/// Source of orphaned operations. Backed by Postgres, never by Redis.
#[async_trait::async_trait]
pub trait NonTerminalOperationScanner: Send + Sync {
    /// `billing_operations` rows still `held` and older than `older_than`, at
    /// most `limit` of them.
    async fn scan_non_terminal(
        &self,
        older_than: Duration,
        limit: i64,
    ) -> anyhow::Result<Vec<OrphanedOperation>>;
}

/// Whether automatic reconciliation is armed.
pub fn auto_reconcile_enabled() -> bool {
    std::env::var(AUTO_RECONCILE_ENV).is_ok_and(|v| v == "true")
}

/// Settles the given orphaned operations, charging the reserved amount — the
/// bound that was admitted, never more — and clearing the reservation.
///
/// Returns how many were reconciled.
pub async fn reconcile_orphaned_operations(
    settlement: &Settlement,
    operations: &[OrphanedOperation],
) -> usize {
    let mut settled = 0;
    for op in operations {
        if op.user_id == 0 || op.reserved_amount <= 0.0 {
            continue;
        }
        let entry = UsageLogEntry {
            user_id: op.user_id,
            request_id: op.client_trace_id.clone(),
            // Never empty on a settled row: this is the money key.
            event_key: op.operation.to_string(),
            total_cost: op.reserved_amount,
            actual_cost: op.reserved_amount,
            cost: op.reserved_amount,
            rate_multiplier: 1.0,
            failed: false,
            raw_metadata: Some(json!({
                "reconciled": true,
                "reason": "orphaned_operation",
                "timestamp": Utc::now().to_rfc3339(),
            })),
            ..UsageLogEntry::default()
        };
        let commit = SettlementCommit {
            user_id: op.user_id,
            operation: op.operation.clone(),
            actual_cost: op.reserved_amount,
            entry,
            subscription_id: None,
        };

        match settlement.store().commit_settlement(&commit).await {
            Ok(SettleReceipt::Committed { .. }) => {
                if let Err(err) = settlement
                    .store()
                    .clear_hold(op.user_id, &op.operation)
                    .await
                {
                    tracing::warn!(user_id = op.user_id, operation = %op.operation, %err,
                        "clear reservation after reconcile failed");
                }
                settled += 1;
            }
            // The request settled itself first, or a concurrent reconciler won.
            // Either way this call moved no money and must not count.
            Ok(SettleReceipt::AlreadyTerminal) => {}
            Err(err) => {
                tracing::warn!(user_id = op.user_id, operation = %op.operation, %err,
                    "reconcile settle failed");
            }
        }
    }
    settled
}

/// One scan: publish the gauge, and reconcile when armed.
pub async fn scan_once(
    scanner: &dyn NonTerminalOperationScanner,
    settlement: &Settlement,
    metrics: &dyn MetricsSink,
    auto_reconcile: bool,
    stale_after: Duration,
) -> usize {
    let stale = match scanner
        .scan_non_terminal(stale_after, DEFAULT_SCAN_LIMIT)
        .await
    {
        Ok(stale) => stale,
        Err(err) => {
            tracing::warn!(%err, "non-terminal operation scan failed");
            return 0;
        }
    };
    metrics.set_orphaned_holds(stale.len() as i64);
    if stale.is_empty() {
        return 0;
    }
    if stale.len() as i64 >= DEFAULT_SCAN_LIMIT {
        // Say it out loud. A capped count reported as the whole picture is how
        // a growing backlog looks like a steady state.
        tracing::warn!(
            limit = DEFAULT_SCAN_LIMIT,
            "orphaned-operation scan hit its limit; more remain for the next tick",
        );
    }
    tracing::warn!(
        count = stale.len(),
        auto_reconcile,
        "orphaned billing operations detected (likely a prior crash)",
    );
    if !auto_reconcile {
        return 0;
    }
    let n = reconcile_orphaned_operations(settlement, &stale).await;
    if n > 0 {
        tracing::info!(count = n, "auto-reconciled orphaned billing operations");
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
    scanner: Arc<dyn NonTerminalOperationScanner>,
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
    scanner: Arc<dyn NonTerminalOperationScanner>,
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

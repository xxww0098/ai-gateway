//! Orphaned-hold recovery. The property that matters is idempotence: a hold
//! that has already produced a usage row must never be charged again.

use std::sync::Arc;

use super::*;
use crate::testsupport::{
    FakeCalculator, FakeLedger, FakeScanner, FakeUsageStore, RecordingMetrics,
};

struct Fixture {
    settlement: Settlement,
    store: Arc<FakeUsageStore>,
}

fn fixture() -> Fixture {
    let store = FakeUsageStore::shared();
    Fixture {
        settlement: Settlement::new(
            FakeLedger::with_balance(100.0),
            FakeCalculator::shared(),
            store.clone(),
        ),
        store,
    }
}

fn hold(request_id: &str, amount: f64) -> StaleHold {
    StaleHold {
        user_id: 7,
        request_id: request_id.to_owned(),
        amount,
    }
}

#[tokio::test]
async fn an_orphaned_hold_is_charged_at_the_reserved_amount_and_then_cleared() {
    // The reservation is a conservative estimate, so charging it is the most
    // the tenant can owe — never more.
    let fixture = fixture();
    let settled = reconcile_orphaned_holds(&fixture.settlement, &[hold("req-1", 1.25)]).await;

    assert_eq!(settled, 1);
    let commits = fixture.store.commits.lock();
    assert_eq!(commits[0].actual_cost, 1.25);
    assert!(
        commits[0].skip_if_already_logged,
        "the in-transaction existence check is what makes this safe to re-run",
    );
    assert_eq!(fixture.store.cleared_holds.lock().as_slice(), ["req-1"]);
}

#[tokio::test]
async fn the_reconciled_row_is_marked_so_it_is_not_mistaken_for_real_traffic() {
    let fixture = fixture();
    reconcile_orphaned_holds(&fixture.settlement, &[hold("req-1", 1.0)]).await;
    let logs = fixture.store.logs.lock();
    let metadata = logs[0].raw_metadata.as_ref().expect("annotated");
    assert_eq!(metadata["reconciled"].as_bool(), Some(true));
    assert_eq!(metadata["reason"].as_str(), Some("orphaned_hold"));
    assert!(!logs[0].failed);
}

#[tokio::test]
async fn re_running_the_reconciler_cannot_double_charge() {
    let fixture = fixture();
    let holds = [hold("req-1", 1.0)];

    assert_eq!(
        reconcile_orphaned_holds(&fixture.settlement, &holds).await,
        1
    );
    assert_eq!(
        reconcile_orphaned_holds(&fixture.settlement, &holds).await,
        0,
        "a hold whose request already produced a usage row must be skipped",
    );
    assert_eq!(fixture.store.commits.lock().len(), 1);
}

#[tokio::test]
async fn malformed_holds_are_ignored_rather_than_charged() {
    let fixture = fixture();
    let bogus = [
        hold("", 1.0),
        hold("req-2", 0.0),
        hold("req-3", -1.0),
        StaleHold {
            user_id: 0,
            request_id: "req-4".to_owned(),
            amount: 1.0,
        },
    ];
    assert_eq!(
        reconcile_orphaned_holds(&fixture.settlement, &bogus).await,
        0
    );
    assert!(fixture.store.commits.lock().is_empty());
}

#[tokio::test]
async fn a_failed_settle_leaves_the_reservation_in_place() {
    let fixture = fixture();
    *fixture.store.commit_fails.lock() = true;
    assert_eq!(
        reconcile_orphaned_holds(&fixture.settlement, &[hold("req-1", 1.0)]).await,
        0,
    );
    assert!(
        fixture.store.cleared_holds.lock().is_empty(),
        "clearing a hold whose debit rolled back would lose the money",
    );
}

#[tokio::test]
async fn the_scan_always_publishes_the_gauge_even_when_reconciliation_is_disarmed() {
    let fixture = fixture();
    let metrics = RecordingMetrics::default();
    let scanner = FakeScanner::default();
    *scanner.stale.lock() = vec![hold("req-1", 1.0), hold("req-2", 2.0)];

    let settled = scan_once(
        &scanner,
        &fixture.settlement,
        &metrics,
        false,
        DEFAULT_STALE_AFTER,
    )
    .await;

    assert_eq!(settled, 0, "detection is always on; charging is opt-in");
    assert_eq!(metrics.orphaned(), 2);
    assert!(fixture.store.commits.lock().is_empty());
}

#[tokio::test]
async fn an_armed_scan_reconciles_what_it_finds() {
    let fixture = fixture();
    let metrics = RecordingMetrics::default();
    let scanner = FakeScanner::default();
    *scanner.stale.lock() = vec![hold("req-1", 1.0)];

    let settled = scan_once(
        &scanner,
        &fixture.settlement,
        &metrics,
        true,
        DEFAULT_STALE_AFTER,
    )
    .await;
    assert_eq!(settled, 1);
    assert_eq!(metrics.orphaned(), 1);
}

#[tokio::test]
async fn a_clean_run_resets_the_gauge() {
    let fixture = fixture();
    let metrics = RecordingMetrics::default();
    crate::ports::MetricsSink::set_orphaned_holds(&metrics, 9);
    let scanner = FakeScanner::default();

    scan_once(
        &scanner,
        &fixture.settlement,
        &metrics,
        true,
        DEFAULT_STALE_AFTER,
    )
    .await;
    assert_eq!(metrics.orphaned(), 0);
}

#[tokio::test]
async fn a_failed_scan_leaves_the_previous_reading_untouched() {
    // Reporting zero on a scan failure would silently clear a real alert.
    let fixture = fixture();
    let metrics = RecordingMetrics::default();
    crate::ports::MetricsSink::set_orphaned_holds(&metrics, 5);
    let scanner = FakeScanner::default();
    *scanner.errors.lock() = true;

    scan_once(
        &scanner,
        &fixture.settlement,
        &metrics,
        true,
        DEFAULT_STALE_AFTER,
    )
    .await;
    assert_eq!(metrics.orphaned(), 5);
}

// ---------------------------------------------------------------- the loop

use std::time::Duration as StdDuration;

use tokio_util::task::TaskTracker;

/// A scanner that blocks until released, so a test can observe the tracker
/// while a scan is genuinely in flight instead of racing an instant fake.
struct BlockingScanner {
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
    stale: parking_lot::Mutex<Vec<StaleHold>>,
}

impl Default for BlockingScanner {
    fn default() -> Self {
        Self {
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
            stale: parking_lot::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl StaleHoldScanner for BlockingScanner {
    async fn scan_stale_holds(&self, _older_than: StdDuration) -> anyhow::Result<Vec<StaleHold>> {
        self.entered.add_permits(1);
        let permit = self.release.acquire().await.expect("never closed");
        permit.forget();
        Ok(self.stale.lock().clone())
    }
}

/// An interval long enough that only the immediate first tick fires.
const ONE_TICK: StdDuration = StdDuration::from_secs(3600);

#[tokio::test]
async fn the_ticker_loop_stops_on_shutdown_without_ever_blocking_the_drain() {
    // The regression this guards: registering the perpetual loop on the tracker
    // makes `wait()` hang until the drain times out, stalling every shutdown by
    // the full timeout and then reporting the loop as an abandoned settlement.
    let fixture = fixture();
    let drain = TaskTracker::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let scanner = Arc::new(BlockingScanner::default());
    let metrics = Arc::new(RecordingMetrics::default());

    let loop_handle = spawn_scanner(
        scanner.clone(),
        Arc::new(fixture.settlement),
        metrics,
        ONE_TICK,
        drain.clone(),
        false, // nothing is charged, so nothing needs draining
        async move {
            let _ = shutdown_rx.await;
        },
    );

    // Observed while the loop is demonstrably alive — it is parked inside the
    // scan. THIS is the invariant: the loop must not be registered at all.
    // Asserting only that `wait()` returns after shutdown is too weak, because
    // a loop that is both tracked and shutdown-aware still drains; the hang
    // appears when a tracked loop outlives the signal.
    let _ = scanner
        .entered
        .acquire()
        .await
        .expect("the first tick scans");
    assert!(
        drain.is_empty(),
        "the perpetual loop must never be registered on the drain tracker",
    );

    scanner.release.add_permits(1);
    shutdown_tx.send(()).expect("the loop is still listening");
    tokio::time::timeout(StdDuration::from_secs(5), loop_handle)
        .await
        .expect("the loop must observe shutdown, not run forever")
        .expect("and must not panic on the way out");

    drain.close();
    tokio::time::timeout(StdDuration::from_secs(5), drain.wait())
        .await
        .expect("the drain must not be waiting on a loop that never ends");
}

#[tokio::test]
async fn a_scan_that_can_move_money_is_registered_on_the_drain() {
    // With auto-reconcile armed the scan debits, so a shutdown landing mid-scan
    // must wait for it rather than abort the runtime out from under the debit.
    let fixture = fixture();
    let drain = TaskTracker::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let scanner = Arc::new(BlockingScanner::default());
    *scanner.stale.lock() = vec![hold("req-1", 1.0)];

    let loop_handle = spawn_scanner(
        scanner.clone(),
        Arc::new(fixture.settlement),
        Arc::new(RecordingMetrics::default()),
        ONE_TICK,
        drain.clone(),
        true,
        async move {
            let _ = shutdown_rx.await;
        },
    );

    // Observed while the scan is genuinely blocked inside the tracker.
    let _ = scanner
        .entered
        .acquire()
        .await
        .expect("the first tick scans");
    assert_eq!(
        drain.len(),
        1,
        "a reconcile that debits must be on the tracker the drain waits out",
    );

    scanner.release.add_permits(1);
    shutdown_tx.send(()).expect("the loop is still listening");
    tokio::time::timeout(StdDuration::from_secs(5), loop_handle)
        .await
        .expect("the loop stops")
        .expect("without panicking");

    drain.close();
    tokio::time::timeout(StdDuration::from_secs(5), drain.wait())
        .await
        .expect("and the drain still completes once the reconcile finishes");
    assert_eq!(
        fixture.store.commits.lock().len(),
        1,
        "the reconcile the drain waited for must actually have settled",
    );
}

#[tokio::test]
async fn a_shutdown_already_signalled_starts_no_scan_at_all() {
    // `biased` in the select: a shutdown racing the due tick wins, so the drain
    // is never handed work that arrives after it began closing.
    let fixture = fixture();
    let drain = TaskTracker::new();
    let scanner = Arc::new(BlockingScanner::default());

    let loop_handle = spawn_scanner(
        scanner.clone(),
        Arc::new(fixture.settlement),
        Arc::new(RecordingMetrics::default()),
        ONE_TICK,
        drain.clone(),
        true,
        std::future::ready(()),
    );

    tokio::time::timeout(StdDuration::from_secs(5), loop_handle)
        .await
        .expect("the loop exits immediately")
        .expect("without panicking");
    assert_eq!(drain.len(), 0);
    assert!(fixture.store.commits.lock().is_empty());
}

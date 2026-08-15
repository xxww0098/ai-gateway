use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gw_config::Config;

use super::*;
use crate::AppState;

/// A settlement that takes `millis` and bumps `counter` when it finishes.
fn spawn_settlement(tracker: &TaskTracker, counter: &Arc<AtomicUsize>, millis: u64) {
    let counter = Arc::clone(counter);
    tracker.spawn(async move {
        tokio::time::sleep(Duration::from_millis(millis)).await;
        counter.fetch_add(1, Ordering::SeqCst);
    });
}

#[tokio::test]
async fn draining_waits_for_settlements_already_in_flight() {
    let tracker = TaskTracker::new();
    let settled = Arc::new(AtomicUsize::new(0));
    for _ in 0..3 {
        spawn_settlement(&tracker, &settled, 50);
    }

    let outcome = drain_settlements(&tracker, Duration::from_secs(5)).await;

    // The whole point: none of these would have run at all under a bare
    // `tokio::spawn` + runtime drop.
    assert_eq!(settled.load(Ordering::SeqCst), 3);
    assert_eq!(outcome.drained, 3);
    assert_eq!(outcome.abandoned, 0);
    assert!(outcome.is_complete());
}

#[tokio::test]
async fn a_settlement_spawned_during_the_drain_is_still_waited_on() {
    // `close()` does not prevent later spawns, and `wait()` completes only when
    // the tracker is closed AND empty — so a StreamSettler that drops a beat
    // late still gets settled.
    let tracker = TaskTracker::new();
    let settled = Arc::new(AtomicUsize::new(0));

    let late = tracker.clone();
    let late_settled = Arc::clone(&settled);
    tracker.spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        spawn_settlement(&late, &late_settled, 50);
    });

    let outcome = drain_settlements(&tracker, Duration::from_secs(5)).await;

    assert_eq!(settled.load(Ordering::SeqCst), 1, "the late settlement ran");
    assert!(outcome.is_complete());
}

#[tokio::test]
async fn draining_gives_up_at_the_timeout_and_reports_the_loss() {
    let tracker = TaskTracker::new();
    let settled = Arc::new(AtomicUsize::new(0));
    spawn_settlement(&tracker, &settled, 50);
    spawn_settlement(&tracker, &settled, 60_000); // never finishes in test time

    let outcome = drain_settlements(&tracker, Duration::from_millis(300)).await;

    // A wedged settlement must not hold the process past the orchestrator's
    // grace period, but it must be COUNTED — that is the unbilled request.
    assert_eq!(outcome.abandoned, 1);
    assert_eq!(outcome.drained, 1);
    assert!(!outcome.is_complete());
    assert!(outcome.elapsed < Duration::from_secs(5), "{outcome:?}");
}

#[tokio::test]
async fn draining_an_idle_tracker_is_instant() {
    let tracker = TaskTracker::new();
    let outcome = drain_settlements(&tracker, Duration::from_secs(30)).await;
    assert_eq!(
        outcome,
        DrainOutcome {
            drained: 0,
            abandoned: 0,
            elapsed: outcome.elapsed
        }
    );
    assert!(outcome.elapsed < Duration::from_secs(1));
    assert!(
        tracker.is_closed(),
        "an idle drain still closes the tracker"
    );
}

#[test]
fn the_timeout_is_the_hold_ttl_capped_by_the_grace_period() {
    // The shipped config holds for an hour; waiting an hour to shut down turns
    // a graceful stop into a SIGKILL, which loses more than giving up does.
    let shipped = Config::parse_yaml("billing:\n  hold_ttl_seconds: 3600\n").expect("yaml");
    assert_eq!(drain_timeout(&shipped.billing), DRAIN_TIMEOUT_CAP);

    // Below the cap the hold TTL wins: past it the reservation has expired on
    // its own and there is nothing left to settle.
    let brief = Config::parse_yaml("billing:\n  hold_ttl_seconds: 5\n").expect("yaml");
    assert_eq!(drain_timeout(&brief.billing), Duration::from_secs(5));

    // Unset/zero falls back to the ledger's own default (5 min), still capped.
    let unset = Config::parse_yaml("billing:\n  hold_amount: 1\n").expect("yaml");
    assert_eq!(drain_timeout(&unset.billing), DRAIN_TIMEOUT_CAP);
}

// ---------------------------------------------------------------------------
// End to end: the guarantee `run()` depends on
// ---------------------------------------------------------------------------

fn test_config(hold_ttl_seconds: i32) -> Config {
    let mut config = Config::default();
    // Port 0 lets the OS pick a free port, so the test never fights a real one.
    config.server.port = 0;
    config.billing.hold_ttl_seconds = hold_ttl_seconds;
    config
}

#[tokio::test]
async fn serve_returns_only_after_in_flight_settlements_finish() {
    let config = test_config(300);
    let state = AppState::default();
    let settled = Arc::new(AtomicUsize::new(0));
    spawn_settlement(&state.drain, &settled, 250);

    // Shutdown fires immediately: the server binds, is told to stop, and the
    // only thing left to wait for is the settlement.
    crate::serve_with_shutdown(&config, state, None, std::future::ready(()))
        .await
        .expect("serve");

    assert_eq!(
        settled.load(Ordering::SeqCst),
        1,
        "serve() returned before the in-flight settlement completed — under a real shutdown the runtime would now be dropped and that Settle lost"
    );
}

#[tokio::test]
async fn serve_records_abandoned_settlements_for_operators() {
    let config = test_config(1); // drain timeout = 1s
    let state = AppState::default();
    let metrics = Arc::clone(&state.metrics);
    let settled = Arc::new(AtomicUsize::new(0));
    spawn_settlement(&state.drain, &settled, 60_000);

    let started = std::time::Instant::now();
    crate::serve_with_shutdown(&config, state, None, std::future::ready(()))
        .await
        .expect("serve");

    assert!(
        started.elapsed() < Duration::from_secs(30),
        "a wedged settlement must not pin the process open"
    );
    let mut exposition = String::new();
    metrics.write_prometheus(&mut exposition);
    assert!(
        exposition.contains("cpa_abandoned_settlements 1"),
        "{exposition}"
    );
}

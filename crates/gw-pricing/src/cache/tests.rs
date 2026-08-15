//! Tests for the cache's normalization and refresh lifecycle.

use std::sync::Arc;
use std::time::Duration;

use gw_model::ModelPrice;
use sqlx::postgres::PgPoolOptions;

use super::ModelPriceCache;
use crate::testsupport::priced;

/// A row with only its identity set; the prices are irrelevant to the
/// normalization and lifecycle invariants below.
fn price(model_id: &str) -> ModelPrice {
    priced(model_id, 1.0, 2.0, 0.5, 4.0)
}

/// A pool that resolves nowhere and gives up quickly, so a refresh tick fails
/// fast instead of blocking the test on sqlx's 30s default acquire timeout.
fn unreachable_pool() -> sqlx::PgPool {
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy("postgres://gw:gw@127.0.0.1:1/gw")
        .expect("connection string parses")
}

/// A model id is looked up case- and whitespace-insensitively: whatever an
/// upstream sends, it must resolve to the same row the admin priced.
#[test]
fn lookup_ignores_case_and_surrounding_whitespace() {
    let cache = ModelPriceCache::from_rows([price("GPT-4o")]);

    for probe in ["GPT-4o", "gpt-4o", "  gpt-4O\t", "\nGPT-4O  "] {
        assert!(
            cache.get(probe).is_some(),
            "{probe:?} must resolve to the cached row"
        );
    }
    assert!(cache.get("gpt-4o-mini").is_none());
}

/// The stored `model_id` keeps the operator's original casing even though the
/// key is normalized — the panel renders this value back to admins.
#[test]
fn stored_row_preserves_the_original_model_id() {
    let cache = ModelPriceCache::from_rows([price("  GPT-4o  ")]);
    let row = cache.get("gpt-4o").expect("normalized lookup hits");
    assert_eq!(row.model_id, "  GPT-4o  ");
}

/// A blank model id can never be looked up (the normalized key would be empty
/// and would swallow every blank probe), so such rows are dropped on load.
#[test]
fn blank_model_ids_are_dropped_and_never_match() {
    let cache = ModelPriceCache::from_rows([price(""), price("   "), price("real")]);

    assert_eq!(cache.len(), 1);
    assert!(cache.get("").is_none());
    assert!(cache.get("   ").is_none());
    assert!(cache.get("real").is_some());
}

/// Two rows whose ids normalize to the same key collapse to one entry — the
/// unique index on `model_prices.model_id` is case-sensitive, so the cache has
/// to pick a winner rather than serve both.
#[test]
fn ids_colliding_after_normalization_collapse_to_one_entry() {
    let cache = ModelPriceCache::from_rows([price("Claude"), price("CLAUDE")]);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.list().len(), 1);
}

#[test]
fn empty_cache_reports_itself_empty() {
    let cache = ModelPriceCache::empty();
    assert!(cache.is_empty());
    assert_eq!(cache.len(), 0);
    assert!(cache.list().is_empty());
    assert!(cache.get("anything").is_none());
}

/// `list` returns every distinct entry exactly once.
#[test]
fn list_returns_every_entry() {
    let ids = ["a", "b", "c", "d"];
    let cache = ModelPriceCache::from_rows(ids.map(price));

    let mut listed: Vec<String> = cache.list().iter().map(|p| p.model_id.clone()).collect();
    listed.sort();
    assert_eq!(listed, ids);
}

/// A degenerate interval must not spawn a refresher. (Rust `Duration`s cannot
/// be negative, so zero is the whole degenerate case.)
#[tokio::test]
async fn start_refresh_is_a_noop_for_a_zero_interval() {
    let cache = Arc::new(ModelPriceCache::from_rows([price("m")]));
    assert!(
        cache
            .start_refresh(unreachable_pool(), Duration::ZERO)
            .is_none()
    );
}

/// A refresh tick that cannot reach the database keeps the last good snapshot.
/// Losing prices on a transient outage would silently reprice every request at
/// the default rate, which is the failure this guards.
#[tokio::test]
async fn a_failing_refresh_preserves_the_previous_snapshot() {
    let cache = Arc::new(ModelPriceCache::from_rows([price("m")]));
    let handle = cache
        .start_refresh(unreachable_pool(), Duration::from_millis(20))
        .expect("a positive interval spawns a refresher");

    // Long enough for several ticks to have failed against the dead pool.
    tokio::time::sleep(Duration::from_millis(400)).await;

    assert_eq!(cache.len(), 1, "snapshot must survive failed reloads");
    assert!(cache.get("m").is_some());

    handle.abort();
}

/// The refresher's lifetime is tied to the cache's: once the last handle is
/// dropped the task must exit on its own, so a caller that forgets the
/// `JoinHandle` cannot leak a task that outlives what it refreshes.
#[tokio::test]
async fn dropping_the_last_handle_stops_the_refresher() {
    let cache = Arc::new(ModelPriceCache::from_rows([price("m")]));
    let handle = cache
        .start_refresh(unreachable_pool(), Duration::from_millis(20))
        .expect("a positive interval spawns a refresher");

    drop(cache);

    // The task can be mid-tick (bounded by the pool's acquire timeout), so
    // poll rather than sleeping a fixed amount.
    for _ in 0..200 {
        if handle.is_finished() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    handle.abort();
    panic!("refresher outlived the cache it refreshes");
}

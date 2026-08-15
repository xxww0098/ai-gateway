//! Guard clauses, TTL wiring, audit metadata shape and Lua error
//! classification — everything about the ledger that is decided before any
//! Redis or Postgres round-trip.
//!
//! The round-trips themselves are covered by the `tests/` integration
//! binaries, which are `#[ignore]`d behind a local Redis / Postgres.

use std::time::Duration;

use serde_json::json;
use sqlx::postgres::PgPoolOptions;

use super::{Ledger, audit_metadata, is_cache_miss, is_insufficient_balance};
use crate::scripts::{CACHE_MISS, INSUFFICIENT_BALANCE};
use crate::{DEFAULT_BALANCE_TTL, DEFAULT_HOLD_TTL, LedgerError};

/// A pool that never dials. Every assertion below is reached before any query
/// runs, so the ledger only needs a pool to *exist*.
fn offline_pool() -> sqlx::PgPool {
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(50))
        .connect_lazy("postgres://gw:gw@127.0.0.1:1/gw")
        .expect("connection string parses")
}

/// A ledger with no Redis: the shape used when holds are disabled.
fn redisless() -> Ledger {
    Ledger::new(offline_pool(), None)
}

// ------------------------------------------------------------------- config

/// A zero TTL means "use the default", not "expire immediately" — a config
/// file that omits the value must not make every reservation stale on arrival.
#[tokio::test]
async fn zero_ttls_fall_back_to_the_defaults() {
    let ledger = Ledger::with_config(offline_pool(), None, Duration::ZERO, Duration::ZERO);
    assert_eq!(ledger.hold_ttl(), DEFAULT_HOLD_TTL);
    assert_eq!(ledger.balance_ttl, DEFAULT_BALANCE_TTL);
}

/// A configured TTL is honoured verbatim. This is the value both Lua scripts
/// use as their expiry cutoff, so it must not be silently normalized.
#[tokio::test]
async fn configured_ttls_are_used_verbatim() {
    let ledger = Ledger::with_config(
        offline_pool(),
        None,
        Duration::from_secs(11),
        Duration::from_secs(100),
    );
    assert_eq!(ledger.hold_ttl(), Duration::from_secs(100));
    assert_eq!(ledger.balance_ttl, Duration::from_secs(11));
}

// ------------------------------------------------------------ guard clauses

/// An empty request id is a programmer error, and it must not be reported as
/// "no hold present" — the two are indistinguishable to a caller reading only
/// the amount, and the fallback-settle path would silently under-charge.
#[tokio::test]
async fn an_empty_request_id_is_rejected_rather_than_read_as_absent() {
    let err = redisless()
        .active_hold_amount(42, "")
        .await
        .expect_err("empty request id must error");
    assert!(matches!(err, LedgerError::InvalidArgument(_)), "{err:?}");
}

/// The request-id guard is checked before the Redis one, so the error names
/// the caller's actual mistake instead of blaming the deployment.
#[tokio::test]
async fn the_request_id_guard_precedes_the_redis_guard() {
    for err in [
        redisless().release(1, "").await.unwrap_err(),
        redisless().settle(1, "", 1.0).await.unwrap_err(),
    ] {
        assert!(matches!(err, LedgerError::InvalidArgument(_)), "{err:?}");
    }
}

/// Holds have nowhere to live without Redis, so the hold path refuses outright
/// rather than pretending to reserve.
#[tokio::test]
async fn holding_without_redis_is_refused() {
    let err = redisless()
        .hold(1, 1.0, "req", Duration::from_secs(60))
        .await
        .unwrap_err();
    assert!(matches!(err, LedgerError::RedisNotConfigured), "{err:?}");

    let err = redisless().active_hold_amount(1, "req").await.unwrap_err();
    assert!(matches!(err, LedgerError::RedisNotConfigured), "{err:?}");
}

/// Releasing and clearing, by contrast, succeed without Redis: there is
/// nothing to release, and failing would make a Redis-less deployment unable
/// to finish a request.
#[tokio::test]
async fn releasing_without_redis_succeeds() {
    redisless().release(1, "req").await.expect("release");
    redisless().clear_hold(1, "req").await.expect("clear_hold");
}

/// With no Redis there are no reservations, so a scan is empty rather than an
/// error — an ops scan must not fail a deployment that runs without holds.
#[tokio::test]
async fn scanning_without_redis_finds_nothing() {
    let stale = redisless()
        .scan_stale_holds(Duration::from_secs(600))
        .await
        .expect("scan");
    assert!(stale.is_empty());
}

/// Amount guards reject before any state moves. Every rejected call below must
/// leave the ledger untouched, which is why they are checked first.
#[tokio::test]
async fn non_positive_amounts_are_rejected() {
    for amount in [0.0, -1.0] {
        let err = redisless().credit(1, amount, "ref").await.unwrap_err();
        assert!(matches!(err, LedgerError::InvalidArgument(_)), "{err:?}");

        let err = redisless().debit(1, amount, "ref").await.unwrap_err();
        assert!(matches!(err, LedgerError::InvalidArgument(_)), "{err:?}");
    }
}

// --------------------------------------------------------- audit metadata

/// Every audit row carries who it belongs to and when it happened, so a row
/// remains attributable even if its surrounding context is lost.
#[test]
fn audit_metadata_always_identifies_the_user_and_the_moment() {
    let meta = audit_metadata(77, None);
    assert_eq!(meta["user_id"], json!(77));
    assert!(
        meta["timestamp"].as_str().is_some_and(|t| t.ends_with('Z')),
        "timestamp must be an explicit-UTC instant, got {:?}",
        meta["timestamp"]
    );
}

/// Extras are merged alongside the base fields rather than nested under them —
/// that flat shape is what the shortfall SQL reads with `->> 'shortfall_usd'`.
#[test]
fn audit_metadata_merges_extras_at_the_top_level() {
    let meta = audit_metadata(5, Some(json!({ "shortfall_usd": 1.5, "actual_cost": 2.0 })));
    assert_eq!(meta["shortfall_usd"], json!(1.5));
    assert_eq!(meta["actual_cost"], json!(2.0));
    assert_eq!(meta["user_id"], json!(5), "base fields must survive");
    assert!(meta["timestamp"].is_string());
}

/// A non-object extra is ignored rather than replacing the payload, so a
/// miswired caller cannot strip the identifying fields off an audit row.
#[test]
fn a_non_object_extra_cannot_displace_the_base_fields() {
    let meta = audit_metadata(5, Some(json!("just a string")));
    assert_eq!(meta["user_id"], json!(5));
    assert!(meta["timestamp"].is_string());
}

// ----------------------------------------------- Lua error classification

/// The cache-miss reply is what drives "load the balance from Postgres and
/// retry". Misclassifying it would turn every cold cache into a hard failure,
/// so the check tolerates both shapes redis-rs may report a custom
/// `error_reply` in.
#[test]
fn the_cache_miss_reply_is_recognised() {
    assert!(is_cache_miss(&extension_error(CACHE_MISS)));
    assert!(!is_cache_miss(&extension_error(INSUFFICIENT_BALANCE)));
    assert!(!is_cache_miss(&extension_error("WRONGTYPE")));
}

/// The refusal reply carries the available balance after a colon; the
/// classifier must still recognise it, and must not confuse it with anything
/// else.
#[test]
fn the_insufficient_balance_reply_is_recognised_with_its_payload() {
    assert!(is_insufficient_balance(&extension_error(&format!(
        "{INSUFFICIENT_BALANCE}:12.5"
    ))));
    assert!(is_insufficient_balance(&extension_error(
        INSUFFICIENT_BALANCE
    )));
    assert!(!is_insufficient_balance(&extension_error(CACHE_MISS)));
    assert!(!is_insufficient_balance(&extension_error(
        "INVALID_BALANCE"
    )));
}

/// Builds the error shape redis-rs produces for a Lua `redis.error_reply`.
fn extension_error(marker: &str) -> redis::RedisError {
    redis::RedisError::from((
        redis::ErrorKind::ExtensionError,
        "An error was signalled by the server",
        marker.to_string(),
    ))
}

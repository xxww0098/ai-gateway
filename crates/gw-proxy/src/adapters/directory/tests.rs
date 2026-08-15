//! Cache policy and column projection.
//!
//! The pool below is `connect_lazy` against an address nothing listens on, so
//! any test here that passes has provably never reached Postgres — which is
//! exactly the property the cache-hit tests are asserting.

use std::str::FromStr as _;

use chrono::TimeZone as _;
use sqlx::postgres::PgConnectOptions;

use super::*;

/// A pool that would fail on first use. Nothing in these tests may use it.
fn unreachable_db() -> Db {
    let opts = PgConnectOptions::from_str("postgres://127.0.0.1:1/nonexistent")
        .expect("a syntactically valid DSN");
    sqlx::postgres::PgPoolOptions::new().connect_lazy_with(opts)
}

fn directory() -> (SqlTenantDirectory, ApiKeyCache, UserStatusCache) {
    let api_keys = ApiKeyCache::new();
    let users = UserStatusCache::new();
    (
        SqlTenantDirectory::new(unreachable_db(), api_keys.clone(), users.clone()),
        api_keys,
        users,
    )
}

#[tokio::test]
async fn a_cached_key_is_served_without_touching_postgres() {
    let (directory, api_keys, _) = directory();
    api_keys.set(
        "hash-1",
        CachedKey {
            user_id: 7,
            api_key_id: 3,
            group_id: Some(5),
            rate_mult: 1.0,
            status: "active".to_owned(),
            expires_at: Utc::now() + CACHE_TTL,
        },
    );

    let row = directory
        .api_key_by_hash("hash-1")
        .await
        .expect("the cache hit must not reach the database")
        .expect("an entry exists");
    assert_eq!(row.id, 3);
    assert_eq!(row.user_id, 7);
    assert_eq!(row.group_id, Some(5));
    assert_eq!(row.status, "active");
}

#[tokio::test]
async fn a_cached_but_deactivated_key_still_surfaces_its_status() {
    // The caller — not this type — decides that a non-active status is a
    // rejection. Swallowing it here would let a revoked key ride the cache.
    let (directory, api_keys, _) = directory();
    api_keys.set(
        "hash-1",
        CachedKey {
            user_id: 7,
            api_key_id: 3,
            group_id: None,
            rate_mult: 1.0,
            status: "revoked".to_owned(),
            expires_at: Utc::now() + CACHE_TTL,
        },
    );

    let row = directory
        .api_key_by_hash("hash-1")
        .await
        .expect("cache hit")
        .expect("an entry exists");
    assert_eq!(row.status, "revoked");
}

#[tokio::test]
async fn a_cached_user_status_is_served_without_touching_postgres() {
    let (directory, _, users) = directory();
    users.set(9, "active", CACHE_TTL);
    assert_eq!(
        directory.user_status(9).await.expect("cache hit"),
        Some("active".to_owned()),
    );
}

#[tokio::test]
async fn the_missing_sentinel_reads_back_as_an_absent_row_not_as_a_status() {
    // Caching the negative is what stops a guessing burst from hammering the
    // database; it must not come back looking like a real `users.status`.
    let (directory, _, users) = directory();
    users.set(404, USER_STATUS_MISSING, CACHE_TTL);
    assert_eq!(directory.user_status(404).await.expect("cache hit"), None);
}

#[tokio::test]
async fn a_suspended_user_reads_back_verbatim() {
    let (directory, _, users) = directory();
    users.set(9, "banned", CACHE_TTL);
    assert_eq!(
        directory.user_status(9).await.expect("cache hit"),
        Some("banned".to_owned()),
    );
}

#[test]
fn a_zero_timestamp_means_this_period_never_rotates() {
    // The schema stores the zero timestamp for an unconfigured period. Reading
    // it as an elapsed boundary would zero the counter on every request.
    assert_eq!(normalise_reset_at(None), None);
    assert_eq!(normalise_reset_at(Some(chrono::DateTime::UNIX_EPOCH)), None);
}

#[test]
fn a_real_boundary_survives_projection() {
    let boundary = Utc.with_ymd_and_hms(2026, 8, 16, 0, 0, 0).unwrap();
    assert_eq!(normalise_reset_at(Some(boundary)), Some(boundary));
}

#[tokio::test]
async fn touching_a_key_never_blocks_and_ignores_the_zero_id() {
    // The bump is detached precisely so a slow write cannot show up as auth
    // latency; with an unreachable pool it must still return immediately.
    let (directory, _, _) = directory();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        directory.touch_api_key(0).await;
        directory.touch_api_key(3).await;
    })
    .await
    .expect("touch_api_key must not await the write");
}

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
    sqlx::postgres::PgPoolOptions::new()
        // Tests that assert the *error* path would otherwise sit through the
        // 30-second acquire default before failing.
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy_with(opts)
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

/// 三个新缓存命中时整个查表不碰 Postgres（连的是不可达库，触库即失败）。
/// 阴性（无订阅 / 无组行）同样是命中 —— 这正是生产里最常见的形态。
#[tokio::test]
async fn cached_auth_lookups_are_served_without_touching_postgres() {
    let (directory, _, _) = directory();

    let quota = SubscriptionQuota {
        id: 11,
        group_id: 5,
        ..SubscriptionQuota::default()
    };
    directory.subscriptions.insert(7, Some(quota.clone()));
    directory.subscriptions.insert(8, None);
    directory.group_mults.insert(5, Some(1.5));
    directory.group_mults.insert(6, None);
    directory.entitlements.insert((7, 5), true);

    assert_eq!(
        directory.active_subscription(7).await.ok(),
        Some(Some(quota))
    );
    assert_eq!(
        directory.active_subscription(8).await.ok(),
        Some(None),
        "「无订阅」的阴性结果必须也是缓存命中"
    );
    assert_eq!(directory.group_rate_multiplier(5).await.ok(), Some(Some(1.5)));
    assert_eq!(directory.group_rate_multiplier(6).await.ok(), Some(None));
    assert_eq!(
        directory.holds_group_entitlement(7, 5).await.ok(),
        Some(true)
    );
}

/// 未命中时必须走真查询：不可达库上的每一条路径都得报错，而不是被某个
/// 默认值静默兜住。
#[tokio::test]
async fn uncached_auth_lookups_must_reach_for_postgres() {
    let (directory, _, _) = directory();

    assert!(directory.active_subscription(7).await.is_err());
    assert!(directory.group_rate_multiplier(5).await.is_err());
    assert!(directory.holds_group_entitlement(7, 5).await.is_err());
}

/// 第二次 touch 落在去抖窗口内：标记只有一份，不随请求数增长。
#[tokio::test]
async fn touch_is_debounced_within_the_window() {
    let (directory, _, _) = directory();

    directory.touch_api_key(3).await;
    directory.touch_api_key(3).await;
    directory.touch_api_key(4).await;

    assert_eq!(directory.touched.len(), 2, "同一把钥匙只留一个去抖标记");
}

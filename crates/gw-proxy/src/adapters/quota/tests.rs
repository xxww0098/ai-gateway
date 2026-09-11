//! Locking, rotation and persistence of the quota counters.

use super::*;
use crate::adapters::usage::SqlUsageStore;
use crate::hold::evaluate_quota;
use crate::ports::{SettleReceipt, SettlementCommit, UsageLogEntry, UsageStore};
use crate::testsupport::{fresh_db, seed_user};
use gw_ledger::Ledger;
use std::sync::Arc;

/// Seeds one subscription whose three reset boundaries are set individually,
/// so a test can make exactly one period elapse.
async fn seed_subscription(
    pool: &sqlx::PgPool,
    id: Id,
    daily_reset: &str,
    weekly_reset: &str,
    monthly_reset: &str,
) {
    sqlx::query(&format!(
        "INSERT INTO subscriptions (id, user_id, package_id, group_id, group_name, status, \
                starts_at, expires_at, \
                daily_usage_usd, daily_reset_at, weekly_usage_usd, weekly_reset_at, \
                monthly_usage_usd, monthly_reset_at, daily_limit_usd, \
                funding_source, funding_reference, price_paid_usd, notes, created_at, updated_at) \
         VALUES ($1, 7, 1, 3, '', 'active', NOW(), NOW() + INTERVAL '30 days', \
                 5, {daily_reset}, 5, {weekly_reset}, 5, {monthly_reset}, 100, \
                 '', '', 0, '', NOW(), NOW())"
    ))
    .bind(id)
    .execute(pool)
    .await
    .expect("seeding a subscription");
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_missing_subscription_is_permissive_rather_than_an_error() {
    // The quota system is opt-in; a user without a row bills purely from their
    // balance, so this must not read as "denied".
    let pool = fresh_db("quota_missing").await;
    let rotated = SqlSubscriptionQuotaStore::new(pool.clone())
        .lock_and_rotate(404, Utc::now())
        .await
        .expect("a missing row is not a failure");
    assert_eq!(rotated, None);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_live_period_is_returned_untouched() {
    let pool = fresh_db("quota_live").await;
    seed_subscription(
        &pool,
        1,
        "NOW() + INTERVAL '1 hour'",
        "NOW() + INTERVAL '2 days'",
        "NOW() + INTERVAL '20 days'",
    )
    .await;

    let quota = SqlSubscriptionQuotaStore::new(pool.clone())
        .lock_and_rotate(1, Utc::now())
        .await
        .expect("locks")
        .expect("the row exists");
    assert_eq!(
        quota.daily_usage_usd, 5.0,
        "a running period keeps its counter"
    );
    assert_eq!(quota.daily_limit_usd, Some(100.0));
    assert_eq!(quota.group_id, 3);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn an_elapsed_period_is_zeroed_and_the_reset_is_persisted() {
    let pool = fresh_db("quota_rotates").await;
    seed_subscription(
        &pool,
        1,
        "NOW() - INTERVAL '1 hour'", // elapsed
        "NOW() + INTERVAL '2 days'",
        "NOW() + INTERVAL '20 days'",
    )
    .await;
    let store = SqlSubscriptionQuotaStore::new(pool.clone());

    let quota = store
        .lock_and_rotate(1, Utc::now())
        .await
        .expect("locks")
        .expect("the row exists");
    assert_eq!(quota.daily_usage_usd, 0.0, "the elapsed period is zeroed");
    assert_eq!(
        quota.weekly_usage_usd, 5.0,
        "a running period is left alone"
    );

    // The rotation has to survive the transaction, or the next request rotates
    // again and the counter never accumulates.
    let persisted: (gw_model::compat::Money, gw_model::compat::Money) =
        sqlx::query_as("SELECT daily_usage_usd, weekly_usage_usd FROM subscriptions WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("reading back");
    assert_eq!((persisted.0.0, persisted.1.0), (0.0, 5.0));

    let again = store
        .lock_and_rotate(1, Utc::now())
        .await
        .expect("locks")
        .expect("the row exists");
    assert_eq!(
        again.daily_usage_usd, 0.0,
        "a second pass within the same period must be a no-op, not another reset",
    );
    assert!(
        again.daily_reset_at.expect("advanced") > Utc::now(),
        "the new boundary must be in the future",
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_zero_boundary_means_the_period_never_rotates() {
    // The schema stores the zero timestamp for an unconfigured period; treating
    // it as elapsed would zero that counter on every single request.
    let pool = fresh_db("quota_zero_boundary").await;
    seed_subscription(
        &pool,
        1,
        "TIMESTAMPTZ 'epoch'",
        "TIMESTAMPTZ 'epoch'",
        "TIMESTAMPTZ 'epoch'",
    )
    .await;

    let quota = SqlSubscriptionQuotaStore::new(pool.clone())
        .lock_and_rotate(1, Utc::now())
        .await
        .expect("locks")
        .expect("the row exists");
    assert_eq!(quota.daily_usage_usd, 5.0);
    assert_eq!(quota.daily_reset_at, None);
    assert_eq!(
        sqlx::query_as::<_, (gw_model::compat::Money,)>(
            "SELECT daily_usage_usd FROM subscriptions WHERE id = 1"
        )
        .fetch_one(&pool)
        .await
        .expect("reading back")
        .0
        .0,
        5.0,
    );
}

/// D0 characterization: lock_and_rotate commits before evaluate_quota, so two
/// 1.0 admissions against 9/10 both pass and usage lands at 11. Slice 06 only
/// if the product wants a hard cap. Kept green for `--ignored` CI.
#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn concurrent_quota_admission_currently_overshoots_the_daily_limit() {
    let pool = fresh_db("quota_d0").await;
    seed_user(&pool, 7, 100.0).await;
    sqlx::query(
        "INSERT INTO subscriptions (id, user_id, package_id, group_id, group_name, status,              starts_at, expires_at, daily_usage_usd, daily_limit_usd, daily_reset_at,              weekly_usage_usd, weekly_reset_at, monthly_usage_usd, monthly_reset_at,              funding_source, funding_reference, price_paid_usd, notes, created_at, updated_at)          VALUES (1, 7, 1, 3, '', 'active', NOW(), NOW() + INTERVAL '30 days',                  9, 10, NOW() + INTERVAL '1 day', 0, NOW() + INTERVAL '7 days',                  0, NOW() + INTERVAL '30 days', '', '', 0, '', NOW(), NOW())",
    )
    .execute(&pool)
    .await
    .expect("seed quota");

    let quota_store = SqlSubscriptionQuotaStore::new(pool.clone());
    let usage = SqlUsageStore::new(pool.clone(), Arc::new(Ledger::new(pool.clone(), None)));
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for i in 0..2 {
        let quota_store = quota_store.clone();
        let usage = usage.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let snapshot = quota_store
                .lock_and_rotate(1, Utc::now())
                .await
                .expect("lock")
                .expect("row");
            if evaluate_quota(&snapshot, 1.0).is_some() {
                return false;
            }
            let request_id = format!("req-d0-{i}");
            let commit = SettlementCommit {
                user_id: 7,
                request_id: request_id.clone(),
                actual_cost: 1.0,
                entry: UsageLogEntry {
                    user_id: 7,
                    api_key_id: 1,
                    request_id,
                    model: "gpt-4o".to_owned(),
                    provider: "openai".to_owned(),
                    cost: 1.0,
                    rate_multiplier: 1.0,
                    ..UsageLogEntry::default()
                },
                subscription_id: Some(1),
            };
            matches!(
                usage.commit_settlement(&commit).await.expect("settle"),
                SettleReceipt::Committed { .. }
            )
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }

    let used: (gw_model::compat::Money,) =
        sqlx::query_as("SELECT daily_usage_usd FROM subscriptions WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("used");
    assert!(
        (used.0.0 - 11.0).abs() < 1e-9,
        "soft-cap window currently admits both requests, got {}",
        used.0.0
    );
}

//! 锁、轮转、持久化，以及**「比较发生在锁里」**这条性质。

use std::sync::Arc;

use gw_ledger::BillingOperationId;

use super::*;
use crate::testsupport::fresh_db;

/// Seeds one subscription whose three reset boundaries are set individually,
/// so a test can make exactly one period elapse.
async fn seed_subscription(
    pool: &sqlx::PgPool,
    id: Id,
    daily_reset: &str,
    weekly_reset: &str,
    monthly_reset: &str,
) {
    seed_subscription_with_limit(pool, id, daily_reset, weekly_reset, monthly_reset, 100.0).await;
}

async fn seed_subscription_with_limit(
    pool: &sqlx::PgPool,
    id: Id,
    daily_reset: &str,
    weekly_reset: &str,
    monthly_reset: &str,
    daily_limit: f64,
) {
    sqlx::query(&format!(
        "INSERT INTO subscriptions (id, user_id, package_id, group_id, group_name, status, \
                starts_at, expires_at, \
                daily_usage_usd, daily_reset_at, weekly_usage_usd, weekly_reset_at, \
                monthly_usage_usd, monthly_reset_at, daily_limit_usd, \
                funding_source, funding_reference, price_paid_usd, notes, created_at, updated_at) \
         VALUES ($1, 7, 1, 3, '', 'active', NOW(), NOW() + INTERVAL '30 days', \
                 5, {daily_reset}, 5, {weekly_reset}, 5, {monthly_reset}, $2, \
                 '', '', 0, '', NOW(), NOW())"
    ))
    .bind(id)
    .bind(daily_limit)
    .execute(pool)
    .await
    .expect("seeding a subscription");
}

/// 预留一笔，断言它被接受。
async fn reserve(store: &SqlSubscriptionQuotaStore, id: Id, amount: f64) -> BillingOperationId {
    let operation = BillingOperationId::mint();
    assert_eq!(
        store
            .reserve(id, &operation, amount, Utc::now())
            .await
            .expect("reserve runs"),
        QuotaAdmission::Reserved,
    );
    operation
}

/// 这个订阅上还压着多少在途预留。
async fn outstanding(pool: &sqlx::PgPool, id: Id) -> f64 {
    sqlx::query_as::<_, (gw_model::compat::Money,)>(
        "SELECT COALESCE(SUM(reserved_amount), 0) FROM quota_reservations WHERE subscription_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("reading reservations")
    .0
    .0
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_missing_subscription_is_permissive_rather_than_an_error() {
    // The quota system is opt-in; a user without a row bills purely from their
    // balance, so this must not read as "denied".
    let pool = fresh_db("quota_missing").await;
    let admission = SqlSubscriptionQuotaStore::new(pool.clone())
        .reserve(404, &BillingOperationId::mint(), 1.0, Utc::now())
        .await
        .expect("a missing row is not a failure");
    assert_eq!(admission, QuotaAdmission::NoSubscription);
    assert_eq!(outstanding(&pool, 404).await, 0.0);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_live_period_keeps_its_counter_and_the_reservation_lands() {
    let pool = fresh_db("quota_live").await;
    seed_subscription(
        &pool,
        1,
        "NOW() + INTERVAL '1 hour'",
        "NOW() + INTERVAL '2 days'",
        "NOW() + INTERVAL '20 days'",
    )
    .await;
    let store = SqlSubscriptionQuotaStore::new(pool.clone());

    reserve(&store, 1, 3.0).await;
    assert_eq!(outstanding(&pool, 1).await, 3.0);

    // 运行中的周期不该被动过。
    let used: (gw_model::compat::Money,) =
        sqlx::query_as("SELECT daily_usage_usd FROM subscriptions WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("reading back");
    assert_eq!(used.0.0, 5.0, "a running period keeps its counter");
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

    reserve(&store, 1, 1.0).await;

    // The rotation has to survive the transaction, or the next request rotates
    // again and the counter never accumulates.
    let persisted: (
        gw_model::compat::Money,
        gw_model::compat::Money,
        Option<chrono::DateTime<Utc>>,
    ) = sqlx::query_as(
        "SELECT daily_usage_usd, weekly_usage_usd, daily_reset_at FROM subscriptions WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("reading back");
    assert_eq!(persisted.0.0, 0.0, "the elapsed period is zeroed");
    assert_eq!(persisted.1.0, 5.0, "a running period is left alone");
    assert!(
        persisted.2.expect("advanced") > Utc::now(),
        "the new boundary must be in the future",
    );

    reserve(&store, 1, 1.0).await;
    let again: (gw_model::compat::Money,) =
        sqlx::query_as("SELECT daily_usage_usd FROM subscriptions WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("reading back");
    assert_eq!(
        again.0.0, 0.0,
        "a second pass within the same period must be a no-op, not another reset",
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

    reserve(&SqlSubscriptionQuotaStore::new(pool.clone()), 1, 1.0).await;

    let row: (gw_model::compat::Money, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("SELECT daily_usage_usd, daily_reset_at FROM subscriptions WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("reading back");
    assert_eq!(row.0.0, 5.0);
    assert_eq!(
        row.1,
        Some(chrono::DateTime::<Utc>::UNIX_EPOCH),
        "零边界原样留着，轮转对它是关的",
    );
}

/// **并发请求抢最后一格额度，只有一个能过。**
///
/// 这是这把刀要根除的那个 bug：收敛前是「锁行 → 轮转 → 提交 → 事务外面比限额」，
/// 行锁在提交那一刻就放了，于是并发的请求都读到同一个「已用」，都放行。
///
/// 用一个 [`Barrier`](tokio::sync::Barrier) 把 [`CONTENDERS`] 个请求压到同一
/// 瞬间放出去，再让它们抢同一格额度：只要那个比较不在行锁里，就一定有人多过。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn concurrent_reserves_for_the_last_slot_admit_exactly_one() {
    /// 一起抢的请求数。多于 2 是因为「没上锁」是个时序 bug ——
    /// 只放两个进去，它们可能恰好一前一后，bug 就藏过去了。
    const CONTENDERS: usize = 8;

    let pool = fresh_db("quota_last_slot").await;
    // 已用 5，限额 10，每笔 4：单独任何一笔都过得去，两笔一起就顶穿。
    seed_subscription_with_limit(
        &pool,
        1,
        "NOW() + INTERVAL '1 hour'",
        "NOW() + INTERVAL '2 days'",
        "NOW() + INTERVAL '20 days'",
        10.0,
    )
    .await;
    let store = Arc::new(SqlSubscriptionQuotaStore::new(pool.clone()));
    let gate = Arc::new(tokio::sync::Barrier::new(CONTENDERS));

    let mut tasks = Vec::new();
    for _ in 0..CONTENDERS {
        let store = Arc::clone(&store);
        let gate = Arc::clone(&gate);
        tasks.push(tokio::spawn(async move {
            gate.wait().await;
            store
                .reserve(1, &BillingOperationId::mint(), 4.0, Utc::now())
                .await
                .expect("reserve runs")
        }));
    }

    let mut reserved = 0;
    let mut exceeded = 0;
    for task in tasks {
        match task.await.expect("task joins") {
            QuotaAdmission::Reserved => reserved += 1,
            QuotaAdmission::Exceeded { .. } => exceeded += 1,
            QuotaAdmission::NoSubscription => panic!("the subscription exists"),
        }
    }
    assert_eq!(reserved, 1, "只有一个请求能拿到最后一格额度");
    assert_eq!(exceeded, CONTENDERS - 1);
    assert_eq!(outstanding(&pool, 1).await, 4.0, "被拒的那些不许留下预留行",);
}

/// 超限**整个回滚**：一行预留都不留下。
#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn an_exceeded_reserve_leaves_no_row_behind() {
    let pool = fresh_db("quota_exceeded").await;
    seed_subscription_with_limit(
        &pool,
        1,
        "NOW() + INTERVAL '1 hour'",
        "NOW() + INTERVAL '2 days'",
        "NOW() + INTERVAL '20 days'",
        10.0,
    )
    .await;
    let store = SqlSubscriptionQuotaStore::new(pool.clone());

    let admission = store
        .reserve(1, &BillingOperationId::mint(), 99.0, Utc::now())
        .await
        .expect("reserve runs");
    assert!(matches!(admission, QuotaAdmission::Exceeded { reason } if reason.contains("daily")));
    assert_eq!(outstanding(&pool, 1).await, 0.0);
}

/// 在途预留必须**算进**限额比较，否则一千个在途请求对配额是隐形的。
#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn outstanding_reservations_count_against_the_limit() {
    let pool = fresh_db("quota_inflight").await;
    seed_subscription_with_limit(
        &pool,
        1,
        "NOW() + INTERVAL '1 hour'",
        "NOW() + INTERVAL '2 days'",
        "NOW() + INTERVAL '20 days'",
        10.0,
    )
    .await;
    let store = SqlSubscriptionQuotaStore::new(pool.clone());

    // 已用 5 + 在途 4 = 9，限额 10。
    let first = reserve(&store, 1, 4.0).await;
    assert!(
        matches!(
            store
                .reserve(1, &BillingOperationId::mint(), 4.0, Utc::now())
                .await
                .expect("reserve runs"),
            QuotaAdmission::Exceeded { .. },
        ),
        "在途那 4 块必须挡住第二笔",
    );

    // 释放之后额度回来了 —— 释放**不**累加任何计数器。
    store
        .release_reservation(&first)
        .await
        .expect("release runs");
    assert_eq!(outstanding(&pool, 1).await, 0.0);
    let used: (gw_model::compat::Money,) =
        sqlx::query_as("SELECT daily_usage_usd FROM subscriptions WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("reading back");
    assert_eq!(used.0.0, 5.0, "释放不许把预留算成实际用量");
    reserve(&store, 1, 4.0).await;
}

/// 同一个操作重复预留是**恢复**，不是第二笔：额度不许被自己顶穿。
#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn re_reserving_the_same_operation_resumes_instead_of_double_reserving() {
    let pool = fresh_db("quota_resume").await;
    seed_subscription_with_limit(
        &pool,
        1,
        "NOW() + INTERVAL '1 hour'",
        "NOW() + INTERVAL '2 days'",
        "NOW() + INTERVAL '20 days'",
        10.0,
    )
    .await;
    let store = SqlSubscriptionQuotaStore::new(pool.clone());

    let operation = BillingOperationId::mint();
    for round in 0..2 {
        assert_eq!(
            store
                .reserve(1, &operation, 4.0, Utc::now())
                .await
                .expect("reserve runs"),
            QuotaAdmission::Reserved,
            "第 {round} 次预留同一个操作应当是恢复",
        );
    }
    assert_eq!(outstanding(&pool, 1).await, 4.0, "一次操作只该压着一格额度",);
}

/// 释放一笔不存在的预留是成功 —— 调用方已经在错误路径上，没有东西要还。
#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn releasing_an_unknown_reservation_is_a_no_op() {
    let pool = fresh_db("quota_release_unknown").await;
    SqlSubscriptionQuotaStore::new(pool.clone())
        .release_reservation(&BillingOperationId::mint())
        .await
        .expect("releasing nothing is not a failure");
}

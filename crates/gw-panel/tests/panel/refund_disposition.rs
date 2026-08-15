//! 退款审批：一份申请只有一个终态，且审批**不动余额**。
//!
//! 对应原实现的 refund_persistence 测试。

use crate::common::{
    balance_log_count, balance_of, fresh_db, seed_group, seed_refund, seed_subscription, seed_user,
};
use chrono::{Days, Utc};
use gw_panel::commerce::refund::{Disposition, apply_disposition};

async fn seed_pending(pool: &sqlx::PgPool, tag: &str) -> (i64, i64) {
    let user = seed_user(pool, &format!("{tag}@example.com"), 50.0).await;
    let group = seed_group(pool, &format!("g-{tag}"), 1.0).await;
    let expires = Utc::now().checked_add_days(Days::new(30)).expect("date");
    let sub = seed_subscription(pool, user, group, "active", expires).await;
    (user, seed_refund(pool, user, sub).await)
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_first_disposition_applies_and_the_second_is_rejected() {
    let pool = fresh_db("refund_single_disposition").await;
    let (_, refund) = seed_pending(&pool, "once").await;

    assert_eq!(
        apply_disposition(&pool, refund, "approved", 1)
            .await
            .expect("apply"),
        Disposition::Applied
    );
    assert_eq!(
        apply_disposition(&pool, refund, "rejected", 1)
            .await
            .expect("apply"),
        Disposition::AlreadyProcessed,
        "第二次审批必须被拒 —— 否则一份申请能既批又拒"
    );

    let status: String =
        sqlx::query_scalar("SELECT COALESCE(status,'') FROM refunds WHERE id = $1")
            .bind(refund)
            .fetch_one(&pool)
            .await
            .expect("read refund");
    assert_eq!(status, "approved", "终态必须是第一次的那个");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn approval_records_who_and_when_without_touching_the_balance() {
    // 既有模型的注释：审批只写处置结果，实际打款走线下。这里钉住"不动余额"。
    let pool = fresh_db("refund_no_money").await;
    let (user, refund) = seed_pending(&pool, "nomoney").await;
    let before = balance_of(&pool, user).await;
    let logs_before = balance_log_count(&pool, user).await;

    apply_disposition(&pool, refund, "approved", 99)
        .await
        .expect("apply");

    let (processed_by, processed_at): (Option<i64>, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("SELECT processed_by, processed_at FROM refunds WHERE id = $1")
            .bind(refund)
            .fetch_one(&pool)
            .await
            .expect("read refund");
    assert_eq!(processed_by, Some(99));
    assert!(processed_at.is_some());

    assert!(
        (balance_of(&pool, user).await - before).abs() < 1e-9,
        "余额必须原封不动"
    );
    assert_eq!(balance_log_count(&pool, user).await, logs_before);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn concurrent_approvals_produce_exactly_one_applied() {
    let pool = fresh_db("refund_concurrent").await;
    let (_, refund) = seed_pending(&pool, "race").await;

    let mut handles = Vec::new();
    for admin in 0..6_i64 {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            apply_disposition(&pool, refund, "approved", admin + 1).await
        }));
    }
    let mut applied = 0;
    for handle in handles {
        if handle.await.expect("join").expect("apply") == Disposition::Applied {
            applied += 1;
        }
    }
    assert_eq!(applied, 1);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_unknown_refund_is_reported_as_missing_not_as_already_processed() {
    // 两者对管理员是不同的信息：404 与 409。混在一起会让"这张单去哪了"无从判断。
    let pool = fresh_db("refund_missing").await;
    assert_eq!(
        apply_disposition(&pool, 123_456, "approved", 1)
            .await
            .expect("apply"),
        Disposition::Missing
    );
}

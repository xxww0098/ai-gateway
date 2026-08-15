//! 分组权益：非基线分组必须有一份未过期的 active 订阅。
//!
//! 对应原实现的 available_groups / apikey_rebind_entitlement 测试里那个共同的谓词。它同时守着两条路由（可用分组列表、API Key 改绑），所以
//! 写错一次会同时打开两个越权口子。

use crate::common::{fresh_db, seed_group, seed_subscription, seed_user};
use chrono::{Days, Utc};
use gw_panel::identity::entitlement::user_holds_entitlement;

fn in_days(days: u64) -> chrono::DateTime<Utc> {
    Utc::now().checked_add_days(Days::new(days)).expect("date")
}

fn days_ago(days: u64) -> chrono::DateTime<Utc> {
    Utc::now().checked_sub_days(Days::new(days)).expect("date")
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_baseline_group_is_held_by_everyone() {
    let pool = fresh_db("entitlement_baseline").await;
    let user = seed_user(&pool, "anyone@example.com", 0.0).await;
    let baseline = seed_group(&pool, "default", 1.0).await;

    assert!(
        user_holds_entitlement(&pool, user, baseline)
            .await
            .expect("probe"),
        "倍率为 1 的分组人人可绑，不需要订阅"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_discounted_group_needs_an_active_unexpired_subscription() {
    let pool = fresh_db("entitlement_discounted").await;
    let user = seed_user(&pool, "buyer@example.com", 0.0).await;
    let pro = seed_group(&pool, "pro", 0.95).await;

    assert!(
        !user_holds_entitlement(&pool, user, pro)
            .await
            .expect("probe"),
        "没有订阅就不该持有折扣分组"
    );

    seed_subscription(&pool, user, pro, "active", in_days(30)).await;
    assert!(
        user_holds_entitlement(&pool, user, pro)
            .await
            .expect("probe")
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_expired_or_revoked_subscription_confers_nothing() {
    let pool = fresh_db("entitlement_lapsed").await;
    let pro = seed_group(&pool, "pro", 0.95).await;

    let expired_user = seed_user(&pool, "expired@example.com", 0.0).await;
    seed_subscription(&pool, expired_user, pro, "active", days_ago(1)).await;
    assert!(
        !user_holds_entitlement(&pool, expired_user, pro)
            .await
            .expect("probe"),
        "过期的订阅不再授予权益"
    );

    let revoked_user = seed_user(&pool, "revoked@example.com", 0.0).await;
    seed_subscription(&pool, revoked_user, pro, "revoked", in_days(30)).await;
    assert!(
        !user_holds_entitlement(&pool, revoked_user, pro)
            .await
            .expect("probe"),
        "被撤销的订阅不再授予权益"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn one_users_subscription_never_entitles_another() {
    let pool = fresh_db("entitlement_isolation").await;
    let pro = seed_group(&pool, "pro", 0.95).await;
    let subscriber = seed_user(&pool, "sub@example.com", 0.0).await;
    let stranger = seed_user(&pool, "stranger@example.com", 0.0).await;
    seed_subscription(&pool, subscriber, pro, "active", in_days(30)).await;

    assert!(
        !user_holds_entitlement(&pool, stranger, pro)
            .await
            .expect("probe")
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_vanished_group_is_unbindable_rather_than_an_error() {
    // 管理员可以合法地删掉分组，而租户手上还留着指向它的陈旧引用。
    // 那种情况必须是"不可绑"，不是 500。
    let pool = fresh_db("entitlement_vanished").await;
    let user = seed_user(&pool, "u@example.com", 0.0).await;
    assert!(
        !user_holds_entitlement(&pool, user, 999_999)
            .await
            .expect("probe")
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn zero_ids_mean_no_entitlement_rather_than_an_error() {
    let pool = fresh_db("entitlement_zero").await;
    let baseline = seed_group(&pool, "default", 1.0).await;
    assert!(
        !user_holds_entitlement(&pool, 0, baseline)
            .await
            .expect("probe")
    );
    assert!(!user_holds_entitlement(&pool, 1, 0).await.expect("probe"));
}

//! 一次性管理员引导：只在"还没有任何管理员"时生效，之后永久失效。
//!
//! 对应原实现的 bootstrap 测试。这条路径是唯一能在没有管理员的系统上创造
//! 管理员的机制，所以它的**失效条件**比它的生效条件更重要。

use crate::common::{fresh_db, role_of, seed_user, seed_user_with};
use gw_panel::identity::bootstrap::{any_active_admin_exists, ensure_bootstrap_admin};

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn promotes_the_configured_user_when_no_admin_exists() {
    let pool = fresh_db("bootstrap_promotes").await;
    let user = seed_user(&pool, "founder@example.com", 0.0).await;
    assert!(!any_active_admin_exists(&pool).await.expect("probe"));

    ensure_bootstrap_admin(&pool, "founder@example.com")
        .await
        .expect("bootstrap");

    assert_eq!(role_of(&pool, user).await, "admin");
    assert!(any_active_admin_exists(&pool).await.expect("probe"));
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn is_inert_once_any_admin_exists() {
    // 这是整条路径的安全属性：它**不能**在一个已经有人管的系统上抬升任何人。
    let pool = fresh_db("bootstrap_inert").await;
    seed_user_with(&pool, "boss@example.com", 0.0, "admin", "active").await;
    let candidate = seed_user(&pool, "founder@example.com", 0.0).await;

    ensure_bootstrap_admin(&pool, "founder@example.com")
        .await
        .expect("bootstrap");

    assert_eq!(
        role_of(&pool, candidate).await,
        "user",
        "已经有管理员时绝不能再提升任何人"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_suspended_admin_does_not_count_as_an_admin() {
    // `anyActiveAdminExists` 同时看 role 和 status：唯一的管理员被停用之后，
    // 引导路径必须重新可用，否则系统就永远锁死了。
    let pool = fresh_db("bootstrap_suspended_admin").await;
    seed_user_with(&pool, "boss@example.com", 0.0, "admin", "suspended").await;
    let candidate = seed_user(&pool, "founder@example.com", 0.0).await;

    assert!(!any_active_admin_exists(&pool).await.expect("probe"));
    ensure_bootstrap_admin(&pool, "founder@example.com")
        .await
        .expect("bootstrap");
    assert_eq!(role_of(&pool, candidate).await, "admin");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn is_idempotent() {
    // 每次启动都会调一次；第二次必须什么都不做（而不是报错或再写一遍）。
    let pool = fresh_db("bootstrap_idempotent").await;
    let user = seed_user(&pool, "founder@example.com", 0.0).await;
    for _ in 0..3 {
        ensure_bootstrap_admin(&pool, "founder@example.com")
            .await
            .expect("bootstrap");
    }
    assert_eq!(role_of(&pool, user).await, "admin");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_absent_or_unconfigured_email_is_a_no_op() {
    let pool = fresh_db("bootstrap_noop").await;
    let user = seed_user(&pool, "someone@example.com", 0.0).await;

    // 没配置 → 什么都不做。
    ensure_bootstrap_admin(&pool, "   ").await.expect("blank");
    // 配了但那个人还没注册 → 也什么都不做（而且不报错）。
    ensure_bootstrap_admin(&pool, "nobody@example.com")
        .await
        .expect("absent user");

    assert_eq!(role_of(&pool, user).await, "user");
    assert!(!any_active_admin_exists(&pool).await.expect("probe"));
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_configured_email_is_matched_case_insensitively() {
    let pool = fresh_db("bootstrap_case").await;
    let user = seed_user(&pool, "founder@example.com", 0.0).await;
    // 配置里写的是大写；库里存的是小写。
    ensure_bootstrap_admin(&pool, "  Founder@Example.COM  ")
        .await
        .expect("bootstrap");
    assert_eq!(role_of(&pool, user).await, "admin");
}

//! 一次性 `super_admin` 引导：只在还没有任何 super_admin 时生效，之后永久失效。

use crate::common::{fresh_db, role_of, seed_user, seed_user_with};
use gw_model::seed::ensure_bootstrap_admin;
use gw_panel::identity::Role;
use gw_panel::identity::bootstrap::any_active_super_admin_exists;

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn promotes_the_configured_user_when_no_super_admin_exists() {
    let pool = fresh_db("bootstrap_promotes").await;
    let user = seed_user(&pool, "founder@example.com", 0.0).await;
    assert!(!any_active_super_admin_exists(&pool).await.expect("probe"));

    ensure_bootstrap_admin(&pool, "founder@example.com", None)
        .await
        .expect("bootstrap");

    assert_eq!(role_of(&pool, user).await, Role::SuperAdmin.as_str());
    assert!(any_active_super_admin_exists(&pool).await.expect("probe"));
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn is_inert_once_any_super_admin_exists() {
    let pool = fresh_db("bootstrap_inert").await;
    seed_user_with(
        &pool,
        "boss@example.com",
        0.0,
        Role::SuperAdmin.as_str(),
        "active",
    )
    .await;
    let candidate = seed_user(&pool, "founder@example.com", 0.0).await;

    ensure_bootstrap_admin(&pool, "founder@example.com", None)
        .await
        .expect("bootstrap");

    assert_eq!(
        role_of(&pool, candidate).await,
        Role::User.as_str(),
        "已经有 super_admin 时绝不能再提升任何人"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_admin_does_not_block_owner_bootstrap() {
    let pool = fresh_db("bootstrap_admin_not_owner").await;
    seed_user_with(
        &pool,
        "ops@example.com",
        0.0,
        Role::Admin.as_str(),
        "active",
    )
    .await;
    let candidate = seed_user(&pool, "founder@example.com", 0.0).await;

    ensure_bootstrap_admin(&pool, "founder@example.com", None)
        .await
        .expect("bootstrap");
    assert_eq!(role_of(&pool, candidate).await, Role::SuperAdmin.as_str());
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_suspended_super_admin_does_not_count() {
    let pool = fresh_db("bootstrap_suspended_admin").await;
    seed_user_with(
        &pool,
        "boss@example.com",
        0.0,
        Role::SuperAdmin.as_str(),
        "suspended",
    )
    .await;
    let candidate = seed_user(&pool, "founder@example.com", 0.0).await;

    assert!(!any_active_super_admin_exists(&pool).await.expect("probe"));
    ensure_bootstrap_admin(&pool, "founder@example.com", None)
        .await
        .expect("bootstrap");
    assert_eq!(role_of(&pool, candidate).await, Role::SuperAdmin.as_str());
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn is_idempotent() {
    let pool = fresh_db("bootstrap_idempotent").await;
    let user = seed_user(&pool, "founder@example.com", 0.0).await;
    for _ in 0..3 {
        ensure_bootstrap_admin(&pool, "founder@example.com", None)
            .await
            .expect("bootstrap");
    }
    assert_eq!(role_of(&pool, user).await, Role::SuperAdmin.as_str());
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_absent_or_unconfigured_email_is_a_no_op() {
    let pool = fresh_db("bootstrap_noop").await;
    let user = seed_user(&pool, "someone@example.com", 0.0).await;

    ensure_bootstrap_admin(&pool, "   ", None)
        .await
        .expect("blank");
    ensure_bootstrap_admin(&pool, "nobody@example.com", None)
        .await
        .expect("absent user");

    assert_eq!(role_of(&pool, user).await, Role::User.as_str());
    assert!(!any_active_super_admin_exists(&pool).await.expect("probe"));
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_configured_email_is_matched_case_insensitively() {
    let pool = fresh_db("bootstrap_case").await;
    let user = seed_user(&pool, "founder@example.com", 0.0).await;
    ensure_bootstrap_admin(&pool, "  Founder@Example.COM  ", None)
        .await
        .expect("bootstrap");
    assert_eq!(role_of(&pool, user).await, Role::SuperAdmin.as_str());
}

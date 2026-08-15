//! API Key：明文只出现一次，库里只有摘要。
//!
//! 原实现没有一条专门的测试盯这个，但它是本域最直接的安全属性：一旦
//! `GenerateAPIKey` 把明文写进了 `key_hash`，所有历史 key 都等同于泄露。

use crate::common::{fresh_db, seed_user};
use gw_panel::identity::apikey::generate_api_key;

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_plaintext_never_reaches_the_database() {
    let pool = fresh_db("apikey_no_plaintext").await;
    let user = seed_user(&pool, "owner@example.com", 0.0).await;

    let (plaintext, row) = generate_api_key(&pool, user, "笔记本", None)
        .await
        .expect("generate");

    let (key_hash, key_prefix): (String, String) =
        sqlx::query_as("SELECT key_hash, key_prefix FROM api_keys WHERE id = $1")
            .bind(row.id)
            .fetch_one(&pool)
            .await
            .expect("read key");

    assert_ne!(key_hash, plaintext, "库里存的必须是摘要，不是明文");
    assert!(!plaintext.contains(&key_hash), "摘要不该是明文的一个子串");
    assert!(
        plaintext.starts_with(&key_prefix),
        "前缀必须是明文的开头，否则列表页对不上号"
    );
    assert!(key_prefix.len() < plaintext.len(), "前缀必须严格短于明文");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn two_keys_for_one_user_are_distinct() {
    let pool = fresh_db("apikey_distinct").await;
    let user = seed_user(&pool, "owner@example.com", 0.0).await;

    let (first, first_row) = generate_api_key(&pool, user, "a", None).await.expect("a");
    let (second, second_row) = generate_api_key(&pool, user, "b", None).await.expect("b");

    assert_ne!(first, second);
    assert_ne!(first_row.id, second_row.id);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_new_key_starts_active_unbound_and_never_used() {
    // 新 key 不绑分组是有意的：要绑得走改绑接口，那里才有权益校验。
    let pool = fresh_db("apikey_defaults").await;
    let user = seed_user(&pool, "owner@example.com", 0.0).await;
    let (_, row) = generate_api_key(&pool, user, "默认", None)
        .await
        .expect("generate");

    let (status, group_id, last_used): (
        String,
        Option<i64>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT COALESCE(status,''), group_id, last_used_at FROM api_keys WHERE id = $1",
    )
    .bind(row.id)
    .fetch_one(&pool)
    .await
    .expect("read key");

    assert_eq!(status, "active");
    assert_eq!(group_id, None, "新 key 不该自动绑到任何分组");
    assert_eq!(last_used, None);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_key_can_be_created_already_bound_to_a_group() {
    // 管理员/种子路径会用到这个入参；用户自助路径永远传 None。
    let pool = fresh_db("apikey_bound").await;
    let user = seed_user(&pool, "owner@example.com", 0.0).await;
    let group = crate::common::seed_group(&pool, "pro", 0.95).await;

    let (_, row) = generate_api_key(&pool, user, "绑定", Some(group))
        .await
        .expect("generate");
    let group_id: Option<i64> = sqlx::query_scalar("SELECT group_id FROM api_keys WHERE id = $1")
        .bind(row.id)
        .fetch_one(&pool)
        .await
        .expect("read key");
    assert_eq!(group_id, Some(group));
}

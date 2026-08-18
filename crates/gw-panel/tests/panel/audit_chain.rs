//! 不变量：审计行被改过，就验不过。
//!
//! 对应原实现的 audit 测试（`TestAuditLog_TamperEvident` / `TestAuditLog_NoKeyDisablesHashing`）。
//!
//! 这条必须连真库，而且必须是 **Postgres**，不能是别的引擎：`metadata` 是
//! `jsonb`，Postgres 会按自己的规则重排键并重新渲染空白。哈希覆盖的是**列里存的
//! 字节**，所以校验时读的是 `metadata::text` 而不是先解析成 `Value` 再序列化
//! ——后者会按 serde 的顺序重排，让每一行都验不过。这个决定只有连库才能被证伪，
//! 单测再多也测不到。

use chrono::{DateTime, SubsecRound as _, TimeDelta, Utc};
use gw_panel::audit::{OperationEntry, SOURCE_PANEL, derive_audit_key, entry_hash};
use gw_panel::identity::oplog::stored_metadata_bytes;
use gw_panel::ops::audit_log::verify_audit_log;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::fresh_db;

/// 与凭证加密密钥同源的派生密钥。空口令等于关闭哈希。
fn key() -> Vec<u8> {
    derive_audit_key("test-credential-encryption-secret").expect("非空口令应派生出密钥")
}

fn entry(action: &str, target: &str, metadata: Option<Value>) -> OperationEntry {
    OperationEntry {
        source: SOURCE_PANEL.to_owned(),
        actor_id: 7,
        actor_email: "admin@example.test".to_owned(),
        actor_role: "admin".to_owned(),
        action: action.to_owned(),
        target: target.to_owned(),
        method: "PUT".to_owned(),
        path: "/api/panel/admin/users/{id}".to_owned(),
        status_code: 200,
        ip_address: "10.0.0.1".to_owned(),
        request_id: "trace-1".to_owned(),
        // 与 handler 侧一致：先编成字节再哈希。
        metadata: metadata
            .as_ref()
            .map(|value| value.to_string().into_bytes())
            .unwrap_or_default(),
        // 截到整秒，否则驱动的亚秒精度会让刚写的行自己验不过。
        created_at: Utc::now().trunc_subsecs(0),
    }
}

/// 按 `entry` 写一行，`hash_key` 为 `None` 时 `entry_hash` 留空（等于没配密钥）。
async fn insert(pool: &PgPool, entry: &OperationEntry, hash_key: Option<&[u8]>) -> i64 {
    let metadata: Option<Value> = if entry.metadata.is_empty() {
        None
    } else {
        Some(serde_json::from_slice(&entry.metadata).expect("metadata 必须是合法 JSON"))
    };
    let mut hashed = entry.clone();
    hashed.metadata = stored_metadata_bytes(pool, metadata.as_ref()).await;
    sqlx::query_scalar(
        "INSERT INTO operation_logs (source, actor_id, actor_email, actor_role, action, target, \
          method, path, status_code, ip_address, request_id, metadata, created_at, entry_hash) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) RETURNING id",
    )
    .bind(&entry.source)
    .bind(entry.actor_id)
    .bind(&entry.actor_email)
    .bind(&entry.actor_role)
    .bind(&entry.action)
    .bind(&entry.target)
    .bind(&entry.method)
    .bind(&entry.path)
    .bind(i64::from(entry.status_code))
    .bind(&entry.ip_address)
    .bind(&entry.request_id)
    .bind(&metadata)
    .bind(entry.created_at)
    .bind(entry_hash(hash_key, &hashed))
    .fetch_one(pool)
    .await
    .expect("insert operation_log")
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_clean_chain_verifies() {
    let pool = fresh_db("audit_chain_clean").await;
    let key = key();

    insert(
        &pool,
        &entry("admin.user.update", "user:1", Some(json!({"before": "x"}))),
        Some(&key),
    )
    .await;
    insert(
        &pool,
        &entry("admin.user.delete", "user:2", None),
        Some(&key),
    )
    .await;

    let tampered = verify_audit_log(&pool, &key).await.expect("verify");
    assert!(tampered.is_empty(), "干净的审计链被判为篡改: {tampered:?}");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_edited_row_is_caught() {
    // 拿到写权限的攻击者会直接改列，而不是走 handler。
    let pool = fresh_db("audit_chain_tampered").await;
    let key = key();

    let victim = insert(
        &pool,
        &entry("admin.user.delete", "user:1", None),
        Some(&key),
    )
    .await;
    let innocent = insert(
        &pool,
        &entry("admin.user.update", "user:2", None),
        Some(&key),
    )
    .await;

    sqlx::query("UPDATE operation_logs SET action = $1 WHERE id = $2")
        .bind("admin.user.read")
        .bind(victim)
        .execute(&pool)
        .await
        .expect("tamper");

    let tampered = verify_audit_log(&pool, &key).await.expect("verify");
    assert_eq!(tampered, vec![victim], "只有被改的那一行该被报出来");
    assert!(!tampered.contains(&innocent));
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_jsonb_metadata_column_still_verifies_after_a_round_trip() {
    // 本文件存在的主要理由。Postgres 会重排 jsonb 的键；如果校验时先解析再重新
    // 序列化，这一行就验不过 —— 而单测里 metadata 从没进过 jsonb，看不出来。
    let pool = fresh_db("audit_chain_jsonb").await;
    let key = key();

    // 故意用一组不按字母序、且长度各异的键，去撞 jsonb 的排序规则。
    let metadata = json!({
        "zeta": 1,
        "a": "x",
        "middle_length_key": [1, 2, 3],
        "nested": {"b": true, "aa": null},
    });
    insert(
        &pool,
        &entry("admin.user.update", "user:1", Some(metadata)),
        Some(&key),
    )
    .await;

    let tampered = verify_audit_log(&pool, &key).await.expect("verify");
    assert!(
        tampered.is_empty(),
        "带 jsonb metadata 的行经过一次往返就验不过了：说明哈希覆盖的不是列里的字节"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_unhashed_row_is_skipped_rather_than_reported() {
    // 特性上线前写的行没有哈希。把它们报成「篡改」会让告警从第一天起就没人看。
    let pool = fresh_db("audit_chain_legacy").await;
    let key = key();

    let legacy = insert(&pool, &entry("admin.legacy", "x", None), None).await;
    insert(&pool, &entry("admin.current", "y", None), Some(&key)).await;

    let tampered = verify_audit_log(&pool, &key).await.expect("verify");
    assert!(!tampered.contains(&legacy));
    assert!(tampered.is_empty());
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn verification_without_a_key_refuses_rather_than_reporting_clean() {
    // 原实现 `TestAuditLog_NoKeyDisablesHashing` 的后半段。没有密钥却回
    // 「一切正常」，比报错危险得多。
    let pool = fresh_db("audit_chain_nokey").await;
    insert(&pool, &entry("admin.x", "y", None), None).await;

    assert!(verify_audit_log(&pool, &[]).await.is_err());
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_wrong_key_does_not_verify_anything() {
    // 换了凭证加密密钥而没重放审计链，应该是「全红」，不是「全绿」。
    let pool = fresh_db("audit_chain_wrongkey").await;
    let key = key();
    let first = insert(&pool, &entry("admin.a", "x", None), Some(&key)).await;
    let second = insert(&pool, &entry("admin.b", "y", None), Some(&key)).await;

    let other = derive_audit_key("a-different-secret").expect("非空口令");
    let tampered = verify_audit_log(&pool, &other).await.expect("verify");
    assert_eq!(tampered.len(), 2);
    assert!(tampered.contains(&first) && tampered.contains(&second));
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn moving_a_row_in_time_is_caught() {
    // `created_at` 也在哈希里：把一条操作挪到别的时间窗口，是最省事的一种掩盖。
    let pool = fresh_db("audit_chain_timeshift").await;
    let key = key();
    let victim = insert(
        &pool,
        &entry("admin.user.delete", "user:1", None),
        Some(&key),
    )
    .await;

    let shifted: DateTime<Utc> = Utc::now().trunc_subsecs(0) - TimeDelta::days(30);
    sqlx::query("UPDATE operation_logs SET created_at = $1 WHERE id = $2")
        .bind(shifted)
        .bind(victim)
        .execute(&pool)
        .await
        .expect("tamper");

    assert_eq!(
        verify_audit_log(&pool, &key).await.expect("verify"),
        vec![victim]
    );
}

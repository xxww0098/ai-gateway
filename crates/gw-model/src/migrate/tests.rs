use crate::testsupport::fresh_db;

use super::*;

/// 光是 `sqlx::migrate!` 展开成功还不够 —— 它只保证文件名可解析、目录存在。
/// 这里卡住的是「迁移集合本身」的性质：非空、版本号严格递增、没有重名。
#[test]
fn migrations_are_ordered_and_unique() {
    let versions: Vec<i64> = MIGRATOR.iter().map(|m| m.version).collect();
    assert!(!versions.is_empty(), "rust/migrations/ 下一个迁移都没有");
    assert!(
        versions.windows(2).all(|w| w[0] < w[1]),
        "版本号必须严格递增：{versions:?}"
    );
}

/// 这份迁移的硬要求：能直接跑在「已经建过表」的库上。
/// 换句话说每一条建表 / 建列 / 建索引都必须带 IF NOT EXISTS。
///
/// 这不是在复述实现，而是在守一条部署契约 —— 未来谁加了一条裸 `CREATE TABLE`，
/// 在 CI 里就会红，而不是在别人的生产库上红。
#[test]
fn migrations_are_idempotent() {
    for m in MIGRATOR.iter() {
        for (idx, raw) in m.sql.lines().enumerate() {
            let line = raw.trim();
            let upper = line.to_uppercase();
            if line.starts_with("--") {
                continue;
            }
            let where_ = || format!("{}_{} 第 {} 行: {line}", m.version, m.description, idx + 1);
            if upper.starts_with("CREATE TABLE") {
                assert!(
                    upper.starts_with("CREATE TABLE IF NOT EXISTS"),
                    "{}",
                    where_()
                );
            }
            if upper.starts_with("CREATE INDEX") || upper.starts_with("CREATE UNIQUE INDEX") {
                assert!(upper.contains("IF NOT EXISTS"), "{}", where_());
            }
            if upper.contains("ADD COLUMN") {
                assert!(upper.contains("ADD COLUMN IF NOT EXISTS"), "{}", where_());
            }
            // DROP / TRUNCATE 会毁掉现有库里的数据，这份迁移里不该出现。
            assert!(
                !upper.starts_with("DROP ") && !upper.starts_with("TRUNCATE "),
                "{}",
                where_()
            );
        }
    }
}

/// 建表迁移必须覆盖历史建库列出的全部 22 张表。
/// 少一张，Rust 起来之后就是运行时 "relation does not exist"。
#[test]
fn every_table_is_created() {
    // 表名取自历史建库的调用顺序。
    const TABLES: &[&str] = &[
        "users",
        "api_keys",
        "groups",
        "balance_logs",
        "usage_logs",
        "operation_logs",
        "subscription_packages",
        "subscriptions",
        "tickets",
        "ticket_replies",
        "model_prices",
        "model_catalog_entries",
        "redeem_codes",
        "refunds",
        "user_token_versions",
        "announcements",
        "payment_orders",
        "ampcode_configs",
        "o_auth_sessions",
        "provider_configs",
        "auth_records",
        "channel_policies",
    ];
    let all_sql: String = MIGRATOR.iter().map(|m| m.sql.to_string()).collect();
    for table in TABLES {
        assert!(
            all_sql.contains(&format!("CREATE TABLE IF NOT EXISTS \"{table}\"")),
            "迁移里没有建表 {table}"
        );
    }
}

/// 空库 → 迁移 → 再迁移一次，必须都成功且版本表稳定。
#[sqlx::test]
#[ignore = "需要本地 Postgres（见 testsupport::fresh_db 的用法说明）"]
async fn migrator_runs_twice_on_empty_database() {
    let pool = fresh_db("migrate_twice").await;

    run(&pool).await.expect("首次迁移应当成功");
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("应当有版本表");
    assert_eq!(applied as usize, MIGRATOR.iter().count());

    run(&pool).await.expect("重复迁移应当是 no-op");
    let again: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("应当有版本表");
    assert_eq!(again, applied, "重复迁移不应新增版本记录");
}

/// 真正的验收条件：跑在一个**已经建过表**的库上。
///
/// 这里用「先把迁移 SQL 原样执行一遍（模拟 schema 已建好，但 `_sqlx_migrations`
/// 里一条记录都没有），再让 MIGRATOR 从零跑一遍」来复现那个场景 —— 只要有一条
/// 语句不是 IF NOT EXISTS，这个测试就会红。
#[sqlx::test]
#[ignore = "需要本地 Postgres（见 testsupport::fresh_db 的用法说明）"]
async fn migrator_tolerates_an_existing_schema() {
    let pool = fresh_db("legacy_schema").await;

    for m in MIGRATOR.iter() {
        sqlx::raw_sql(&m.sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("预置 {} 失败: {e}", m.description));
    }
    // 再模拟一个更老的库：少一列、少一个索引。
    sqlx::raw_sql(
        "ALTER TABLE operation_logs DROP COLUMN entry_hash;
         DROP INDEX idx_usage_logs_user_created;",
    )
    .execute(&pool)
    .await
    .expect("模拟老库应当成功");

    run(&pool).await.expect("在既有 schema 上迁移必须成功");

    let has_column: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns
          WHERE table_name = 'operation_logs' AND column_name = 'entry_hash')",
    )
    .fetch_one(&pool)
    .await
    .expect("查列");
    assert!(has_column, "迁移应当把缺的列补回来");

    let has_index: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_usage_logs_user_created')",
    )
    .fetch_one(&pool)
    .await
    .expect("查索引");
    assert!(has_index, "迁移应当把缺的索引补回来");
}

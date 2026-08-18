use crate::testsupport::fresh_db;
use sqlx::PgPool;

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

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let mut on_disk: Vec<i64> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("读不到 {}: {e}", dir.display()))
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            if !name.ends_with(".sql") {
                return None;
            }
            name.split('_').next()?.parse().ok()
        })
        .collect();
    on_disk.sort();
    assert_eq!(
        on_disk, versions,
        "嵌进二进制的迁移必须和 migrations/*.sql 一一对应；对不上说明 cargo 没重编"
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

/// `/v1` 每条计费请求都会打的两条查找，0003 的单列/错前缀索引盖不住。
/// 守的是「迁移之后库里真有能按这些谓词做 Index Cond 的索引」，不是文件名。
#[sqlx::test]
#[ignore = "需要本地 Postgres（见 testsupport::fresh_db 的用法说明）"]
async fn hot_path_lookups_use_covering_indexes() {
    let pool = fresh_db("hot_path_idx").await;
    run(&pool).await.expect("迁移应当成功");

    let sub_defs = indexdefs(&pool, "subscriptions").await;
    assert!(
        sub_defs
            .iter()
            .any(|d| column_order(d, &["user_id", "status", "expires_at"])),
        "subscriptions 必须有 (user_id, status, expires_at) 复合索引，单列盖不住 \
         `status='active' AND expires_at>NOW()`：{sub_defs:?}"
    );

    let cat_defs = indexdefs(&pool, "model_catalog_entries").await;
    assert!(
        cat_defs.iter().any(|d| leading_column(d, "model_id")),
        "model_catalog_entries 必须有以 model_id 打头的索引，\
         唯一键 (channel_key, model_id) 服务不了按模型反查渠道：{cat_defs:?}"
    );

    sqlx::query(
        "INSERT INTO subscriptions (user_id, package_id, group_id, status, starts_at, expires_at)
         SELECT (g % 200) + 1, 1, 1,
                CASE WHEN g % 5 = 0 THEN 'active' ELSE 'expired' END,
                NOW() - INTERVAL '30 days',
                CASE WHEN g % 5 = 0 THEN NOW() + INTERVAL '30 days' ELSE NOW() - INTERVAL '1 day' END
         FROM generate_series(1, 8000) AS g",
    )
    .execute(&pool)
    .await
    .expect("灌 subscriptions");

    sqlx::query(
        "INSERT INTO model_catalog_entries (channel_key, model_id, visible, created_at)
         SELECT 'ch-' || (g % 40), 'model-' || (g / 40), TRUE, NOW()
         FROM generate_series(0, 3999) AS g",
    )
    .execute(&pool)
    .await
    .expect("灌 model_catalog_entries");

    for table in ["subscriptions", "model_catalog_entries"] {
        sqlx::query(&format!("ANALYZE {table}"))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("ANALYZE {table}: {e}"));
    }

    // 同一条连接上关 seqscan：证明「有索引能按这些列做 Index Cond」。
    // 只靠 user_id / (channel_key, model_id) 时 Cond 里不会同时出现下面这些列。
    let mut conn = pool.acquire().await.expect("借连接");
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *conn)
        .await
        .expect("enable_seqscan");

    let sub_plan = explain(
        &mut conn,
        "SELECT id FROM subscriptions
         WHERE user_id = 42 AND status = 'active' AND expires_at > NOW()
         ORDER BY expires_at DESC LIMIT 1",
    )
    .await;
    assert!(
        index_cond_mentions(&sub_plan, &["user_id", "status"]),
        "活跃订阅查找的 Index Cond 必须带上 user_id 和 status，否则还在扫单列 user_id：{sub_plan}"
    );

    let cat_plan = explain(
        &mut conn,
        "SELECT DISTINCT channel_key FROM model_catalog_entries
         WHERE model_id = 'model-5' AND model_id <> '__models_url__'
         ORDER BY channel_key",
    )
    .await;
    assert!(
        index_cond_mentions(&cat_plan, &["model_id"]),
        "按 model_id 反查渠道的 Index Cond 必须带上 model_id：{cat_plan}"
    );
}

async fn indexdefs(pool: &PgPool, table: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE tablename = $1")
        .bind(table)
        .fetch_all(pool)
        .await
        .unwrap_or_else(|e| panic!("pg_indexes({table}): {e}"))
}

fn column_order(indexdef: &str, cols: &[&str]) -> bool {
    let mut rest = indexdef;
    for col in cols {
        let Some(at) = rest.find(col) else {
            return false;
        };
        rest = &rest[at + col.len()..];
    }
    true
}

fn leading_column(indexdef: &str, col: &str) -> bool {
    let Some(open) = indexdef.find('(') else {
        return false;
    };
    indexdef[open + 1..]
        .trim_start()
        .trim_start_matches('"')
        .starts_with(col)
}

async fn explain(conn: &mut sqlx::PgConnection, sql: &str) -> serde_json::Value {
    sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {sql}"))
        .fetch_one(&mut *conn)
        .await
        .unwrap_or_else(|e| panic!("EXPLAIN 失败: {e}"))
}

fn index_cond_mentions(plan: &serde_json::Value, cols: &[&str]) -> bool {
    fn walk(node: &serde_json::Value, cols: &[&str]) -> bool {
        if let Some(cond) = node.get("Index Cond").and_then(|v| v.as_str())
            && cols.iter().all(|c| cond.contains(c))
        {
            return true;
        }
        if let Some(plans) = node.get("Plans").and_then(|v| v.as_array()) {
            return plans.iter().any(|p| walk(p, cols));
        }
        false
    }
    let root = plan
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.get("Plan"))
        .unwrap_or(plan);
    walk(root, cols)
}

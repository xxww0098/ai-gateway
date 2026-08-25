//! Scaffolding for the database-backed adapter tests.
//!
//! Every caller is `#[ignore]`d (rule 2.9: fail loud with instructions, never
//! silently skip), so a missing connection string panics with [`PG_HOWTO`]
//! rather than reporting a pass.

use crate::ports::Id;

/// 连接串的两个变量名，按优先级。
///
/// `GW_TEST_DATABASE_URL` 是工作区里跑 `--ignored` 的统一口径
/// （`gw-authcore` / `gw-infra` / `gw-ledger` / `gw-panel` 都读它）；
/// `DATABASE_URL` 留作兼容单独跑本 crate 的习惯，也是 `gw-model` 读的那个。
const PG_ENV: [&str; 2] = ["GW_TEST_DATABASE_URL", "DATABASE_URL"];

/// How to run the database-backed adapter tests. They are `#[ignore]`d rather
/// than silently skipped (rule 2.9), so a missing variable fails loudly with
/// this text instead of quietly reporting a pass.
pub(crate) const PG_HOWTO: &str = "database-backed adapter tests need GW_TEST_DATABASE_URL (or DATABASE_URL), e.g.\n  \
     GW_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:5432/postgres \
     cargo test -p gw-proxy -- --ignored";

/// Creates an empty database named after `tag`, migrates it, and connects.
///
/// Each test owns its own database and drops any leftover first, so a previous
/// failed run cannot contaminate this one.
pub(crate) async fn fresh_db(tag: &str) -> sqlx::PgPool {
    use std::str::FromStr as _;

    assert!(
        tag.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "the database name is interpolated, so a tag may only be [a-z0-9_]",
    );

    let url = PG_ENV
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| panic!("{PG_HOWTO}"));
    let opts = sqlx::postgres::PgConnectOptions::from_str(&url)
        .expect("DATABASE_URL is not a valid Postgres connection string");
    let admin = sqlx::PgPool::connect_with(opts.clone())
        .await
        .expect("cannot reach the server DATABASE_URL points at");

    let name = format!("gw_proxy_test_{tag}");
    for statement in [
        format!(r#"DROP DATABASE IF EXISTS "{name}""#),
        format!(r#"CREATE DATABASE "{name}""#),
    ] {
        sqlx::query(&statement)
            .execute(&admin)
            .await
            .unwrap_or_else(|err| panic!("{statement}: {err}"));
    }
    admin.close().await;

    let pool = sqlx::PgPool::connect_with(opts.database(&name))
        .await
        .expect("cannot connect to the freshly created database");
    gw_model::run_migrations(&pool)
        .await
        .expect("migrations must apply to an empty database");
    pool
}

/// Inserts the user rows the settlement tests debit against.
pub(crate) async fn seed_user(pool: &sqlx::PgPool, user_id: Id, balance: f64) {
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, role, username, balance, status, \
                            concurrency, created_at, updated_at) \
         VALUES ($1, $2, '', 'user', 'tester', $3, 'active', 0, NOW(), NOW())",
    )
    .bind(user_id)
    .bind(format!("user{user_id}@example.test"))
    .bind(balance)
    .execute(pool)
    .await
    .expect("seeding a user");
}

//! 连库集成测试的公共脚手架。
//!
//! 规范 2.3：共享辅助必须放 `tests/<binary>/common/mod.rs`，**不是**
//! `tests/common.rs`（后者会被当成一个跑 0 个测试的空二进制）。
//!
//! 规范 2.9：这里所有用到 [`fresh_db`] 的测试都标了
//! `#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]`，
//! **绝不允许「读不到环境变量就 return」** —— 那会让覆盖率变成假的。跑法：
//!
//! ```bash
//! GW_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:5432/postgres \
//!   CARGO_TARGET_DIR=/tmp/cargo-panel-identity \
//!   cargo test -p gw-panel --test panel -- --ignored
//! ```
//!
//! 每个测试用一个**独占的、以测试名命名的库**：先 DROP 再 CREATE，所以上一次
//! 跑失败留下的残骸不会污染下一次。

use std::str::FromStr as _;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use gw_ledger::Ledger;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;

/// 缺环境变量时的报错文案。fail-loud 的一半 —— 另一半是 `#[ignore]`。
const HOWTO: &str = "连库集成测试需要 GW_TEST_DATABASE_URL，例如：\n  \
     GW_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:5432/postgres \
     cargo test -p gw-panel --test panel -- --ignored";

/// 建一个空库、跑完迁移并连上去。`tag` 必须在整个测试二进制内唯一。
pub(crate) async fn fresh_db(tag: &str) -> PgPool {
    assert!(
        tag.chars()
            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
        "tag 只能是小写字母/数字/下划线，收到 {tag:?}"
    );

    let url = std::env::var("GW_TEST_DATABASE_URL").expect(HOWTO);
    let opts = PgConnectOptions::from_str(&url).expect("GW_TEST_DATABASE_URL 不是合法连接串");

    let admin = PgPool::connect_with(opts.clone())
        .await
        .expect("连不上 GW_TEST_DATABASE_URL 指向的库");
    let name = format!("gw_panel_test_{tag}");
    for stmt in [
        format!(r#"DROP DATABASE IF EXISTS "{name}""#),
        format!(r#"CREATE DATABASE "{name}""#),
    ] {
        sqlx::query(&stmt)
            .execute(&admin)
            .await
            .unwrap_or_else(|e| panic!("{stmt} 失败: {e}"));
    }
    admin.close().await;

    let pool = PgPool::connect_with(opts.database(&name))
        .await
        .expect("连不上刚建好的测试库");
    gw_model::run_migrations(&pool)
        .await
        .expect("迁移失败（rust/migrations 与 gw_model::MIGRATOR 不一致？）");
    pool
}

/// 一个**没有 Redis** 的账本。
///
/// `Ledger::new(pg, None)` 是被支持的形态（每个 Redis 支撑的组件都能优雅降级），
/// 所以计费相关的集成测试只需要 Postgres。这也是这些测试值得存在的原因之一：
/// 它们跑得起来。
pub(crate) fn ledger_without_redis(pool: &PgPool) -> Arc<Ledger> {
    Arc::new(Ledger::new(pool.clone(), None))
}

/// 插一个用户，返回它的 id。
pub(crate) async fn seed_user(pool: &PgPool, email: &str, balance: f64) -> i64 {
    seed_user_with(pool, email, balance, "user", "active").await
}

/// 插一个指定角色/状态的用户。
pub(crate) async fn seed_user_with(
    pool: &PgPool,
    email: &str,
    balance: f64,
    role: &str,
    status: &str,
) -> i64 {
    let now = Utc::now();
    sqlx::query_scalar(
        "INSERT INTO users \
             (email, password_hash, role, username, balance, status, concurrency, created_at, updated_at) \
         VALUES ($1, 'x', $2, '', $3, $4, 1, $5, $5) RETURNING id",
    )
    .bind(email)
    .bind(role)
    .bind(balance)
    .bind(status)
    .bind(now)
    .fetch_one(pool)
    .await
    .expect("seed user")
}

/// 读回 `users.balance`（持久化列，不是账本的可用余额）。
pub(crate) async fn balance_of(pool: &PgPool, user_id: i64) -> f64 {
    sqlx::query_scalar("SELECT COALESCE(balance,0)::float8 FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("read balance")
}

/// 数一个用户名下的 `balance_logs` 行数。
pub(crate) async fn balance_log_count(pool: &PgPool, user_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*)::bigint FROM balance_logs WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("count balance logs")
}

/// 取一个用户名下所有 `balance_logs` 的 `(amount, reference)`，按 id 升序。
pub(crate) async fn balance_log_entries(pool: &PgPool, user_id: i64) -> Vec<(f64, String)> {
    sqlx::query_as(
        "SELECT COALESCE(amount,0)::float8, COALESCE(reference,'') FROM balance_logs \
         WHERE user_id = $1 ORDER BY id ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .expect("list balance logs")
}

/// 写一条余额流水（绕过账本，测试里用来铺数据）。
pub(crate) async fn seed_balance_log(
    pool: &PgPool,
    user_id: i64,
    amount: f64,
    kind: &str,
    reference: &str,
    created_at: DateTime<Utc>,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(user_id)
    .bind(amount)
    .bind(kind)
    .bind(reference)
    .bind(created_at)
    .fetch_one(pool)
    .await
    .expect("seed balance log")
}

/// 插一张 `pending` 充值单，返回 id。
pub(crate) async fn seed_payment_order(pool: &PgPool, user_id: i64, amount_usd: f64) -> i64 {
    let now = Utc::now();
    sqlx::query_scalar(
        "INSERT INTO payment_orders \
             (user_id, provider, amount_usd, amount_local, currency, status, created_at, updated_at) \
         VALUES ($1, 'stripe', $2, $2, 'USD', 'pending', $3, $3) RETURNING id",
    )
    .bind(user_id)
    .bind(amount_usd)
    .bind(now)
    .fetch_one(pool)
    .await
    .expect("seed payment order")
}

/// 插一张未使用的兑换码，返回 id。
pub(crate) async fn seed_redeem_code(pool: &PgPool, code: &str, amount: f64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO redeem_codes (code, amount, status, created_at) \
         VALUES ($1, $2, 'unused', $3) RETURNING id",
    )
    .bind(code)
    .bind(amount)
    .bind(Utc::now())
    .fetch_one(pool)
    .await
    .expect("seed redeem code")
}

/// 插一份 `pending` 退款申请，返回 id。
pub(crate) async fn seed_refund(pool: &PgPool, user_id: i64, subscription_id: i64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO refunds \
             (user_id, subscription_id, amount, reason, status, days_used, total_days, \
              daily_rate, created_at) \
         VALUES ($1, $2, 0, '', 'pending', 0, 0, 0, $3) RETURNING id",
    )
    .bind(user_id)
    .bind(subscription_id)
    .bind(Utc::now())
    .fetch_one(pool)
    .await
    .expect("seed refund")
}

/// 插一个分组，返回 id。
pub(crate) async fn seed_group(pool: &PgPool, name: &str, rate_multiplier: f64) -> i64 {
    let now = Utc::now();
    sqlx::query_scalar(
        "INSERT INTO groups (name, rate_multiplier, quota_limit, created_at, updated_at) \
         VALUES ($1, $2, 0, $3, $3) RETURNING id",
    )
    .bind(name)
    .bind(rate_multiplier)
    .bind(now)
    .fetch_one(pool)
    .await
    .expect("seed group")
}

/// 插一份订阅，返回 id。
pub(crate) async fn seed_subscription(
    pool: &PgPool,
    user_id: i64,
    group_id: i64,
    status: &str,
    expires_at: DateTime<Utc>,
) -> i64 {
    let now = Utc::now();
    sqlx::query_scalar(
        "INSERT INTO subscriptions \
             (user_id, package_id, group_id, group_name, status, starts_at, expires_at, \
              daily_usage_usd, daily_reset_at, weekly_usage_usd, weekly_reset_at, \
              monthly_usage_usd, monthly_reset_at, funding_source, funding_reference, \
              price_paid_usd, notes, created_at, updated_at) \
         VALUES ($1, 1, $2, 'Test', $3, $4, $5, 0, $4, 0, $4, 0, $4, '', '', 0, '', $4, $4) \
         RETURNING id",
    )
    .bind(user_id)
    .bind(group_id)
    .bind(status)
    .bind(now)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .expect("seed subscription")
}

/// 读一张兑换码的 `(status, used_by_id)`。
pub(crate) async fn redeem_code_state(pool: &PgPool, id: i64) -> (String, Option<i64>) {
    sqlx::query_as("SELECT COALESCE(status,''), used_by_id FROM redeem_codes WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read redeem code")
}

/// 读一张充值单的 `status`。
pub(crate) async fn order_status(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT COALESCE(status,'') FROM payment_orders WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read order")
}

/// 读一个用户的 `role`。
pub(crate) async fn role_of(pool: &PgPool, user_id: i64) -> String {
    sqlx::query_scalar("SELECT COALESCE(role,'') FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("read role")
}

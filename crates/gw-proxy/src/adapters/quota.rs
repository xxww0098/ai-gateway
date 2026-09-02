//! [`SubscriptionQuotaStore`] over Postgres —— 锁里比，锁里留。
//!
//! # 收敛前那把锁保护错了东西
//!
//! 此前的流程是：`SELECT ... FOR UPDATE` 锁住订阅行 → 轮转过期的周期计数器
//! → 持久化 → **提交** → 在事务外面拿限额和估算比。行锁在提交那一刻就放了，
//! 于是两个抢最后一格额度的并发请求会读到同一个「已用」，两个都放行。
//! 锁保护的是轮转，不是那个比较 —— 而比较才是要串行化的那件事。
//!
//! 现在整条链都在**一个事务**里：
//!
//! ```text
//! BEGIN
//!   SELECT ... FOR UPDATE          -- 排队
//!   rotate_counters                -- 轮转（规则来自 crate::hold，不在 SQL 里重写）
//!   UPDATE subscriptions           -- 轮转脏了才写
//!   SELECT SUM(reserved_amount)    -- 这个订阅上还有多少在途预留
//!   evaluate_quota(已用, 在途, 这一笔)
//!   INSERT quota_reservations      -- 通过才落行；超限直接 ROLLBACK
//! COMMIT
//! ```
//!
//! 并发的第二个请求在行锁上排队，拿到锁时已经看得见第一个请求落下的预留行。
//!
//! # 第二个洞：在途请求对限额是隐形的
//!
//! 预扣从不预留配额，结算才 `daily_usage_usd += actual`。一千个在途请求
//! 因此完全不占额度，限额只在它们陆续结算之后才追上来。`quota_reservations`
//! 就是那份在途负债，[`crate::hold::evaluate_quota`] 把它一起算进比较。
//!
//! **轮转规则不在这里重写。** 它来自 [`crate::hold::rotate_counters`]，
//! 与 hold 中间件的测试盯的是同一个函数。把「下一个 UTC 零点 / 下周一 /
//! 下月一号，严格晚于现在」写成 SQL 是第二份边界算术实现 —— 周一零点那一格
//! 是经典陷阱，而且没法单测。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gw_infra::Db;
use gw_ledger::BillingOperationId;

use crate::adapters::directory::normalise_reset_at;
use crate::hold::{evaluate_quota, rotate_counters};
use crate::ports::{Id, QuotaAdmission, SubscriptionQuota, SubscriptionQuotaStore};

/// Row-locking quota reservations for the hold pre-flight.
#[derive(Debug, Clone)]
pub struct SqlSubscriptionQuotaStore {
    db: Db,
}

impl SqlSubscriptionQuotaStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SubscriptionQuotaStore for SqlSubscriptionQuotaStore {
    async fn reserve(
        &self,
        subscription_id: Id,
        operation: &BillingOperationId,
        amount: f64,
        now: DateTime<Utc>,
    ) -> anyhow::Result<QuotaAdmission> {
        let mut tx = self.db.begin().await?;

        let row = sqlx::query_as::<_, LockedRow>(
            "SELECT id, group_id, daily_usage_usd, weekly_usage_usd, monthly_usage_usd, \
                    daily_limit_usd, weekly_limit_usd, monthly_limit_usd, \
                    daily_reset_at, weekly_reset_at, monthly_reset_at \
             FROM subscriptions WHERE id = $1 FOR UPDATE",
        )
        .bind(subscription_id)
        .fetch_optional(&mut *tx)
        .await?;

        // A missing subscription is permissive, not an error: the quota system
        // is opt-in, and a user without one is billed purely from their balance.
        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(QuotaAdmission::NoSubscription);
        };

        let mut quota = row.into_quota();
        if rotate_counters(&mut quota, now) {
            sqlx::query(
                "UPDATE subscriptions SET \
                    daily_usage_usd = $2, daily_reset_at = $3, \
                    weekly_usage_usd = $4, weekly_reset_at = $5, \
                    monthly_usage_usd = $6, monthly_reset_at = $7, \
                    updated_at = NOW() \
                 WHERE id = $1",
            )
            .bind(subscription_id)
            .bind(quota.daily_usage_usd)
            .bind(quota.daily_reset_at)
            .bind(quota.weekly_usage_usd)
            .bind(quota.weekly_reset_at)
            .bind(quota.monthly_usage_usd)
            .bind(quota.monthly_reset_at)
            .execute(&mut *tx)
            .await?;
        }

        // 同一个操作再来一次是**恢复**（重试、失败后重入），不是第二笔：
        // 它的金额已经在下面那个求和里了，再比一次等于把自己算两遍。
        let existing: Option<(gw_model::compat::Money,)> = sqlx::query_as(
            "SELECT reserved_amount FROM quota_reservations WHERE billing_operation_id = $1",
        )
        .bind(operation.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        if existing.is_some() {
            tx.commit().await?;
            return Ok(QuotaAdmission::Reserved);
        }

        let (reserved,): (gw_model::compat::Money,) = sqlx::query_as(
            "SELECT COALESCE(SUM(reserved_amount), 0) FROM quota_reservations \
             WHERE subscription_id = $1",
        )
        .bind(subscription_id)
        .fetch_one(&mut *tx)
        .await?;

        if let Some(reason) = evaluate_quota(&quota, reserved.0, amount) {
            // 超限就整个回滚 —— 连轮转一起。下一个请求会重新轮转一次
            // （幂等），而**一行预留都不会留下**。
            tx.rollback().await?;
            return Ok(QuotaAdmission::Exceeded { reason });
        }

        sqlx::query(
            // 显式转 numeric：不转的话 Postgres 会把 `numeric` 列拓宽成二进制
            // 浮点来解析 `float8` 字面量，那正是这份代码对钱唯一不许做的算术。
            "INSERT INTO quota_reservations \
                (billing_operation_id, subscription_id, reserved_amount, created_at) \
             VALUES ($1, $2, CAST($3 AS numeric), NOW())",
        )
        .bind(operation.as_str())
        .bind(subscription_id)
        .bind(amount)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(QuotaAdmission::Reserved)
    }

    async fn release_reservation(&self, operation: &BillingOperationId) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM quota_reservations WHERE billing_operation_id = $1")
            .bind(operation.as_str())
            .execute(&self.db)
            .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct LockedRow {
    id: Id,
    group_id: Id,
    #[sqlx(try_from = "gw_model::compat::Money")]
    daily_usage_usd: f64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    weekly_usage_usd: f64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    monthly_usage_usd: f64,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    daily_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    weekly_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    monthly_limit_usd: Option<f64>,
    daily_reset_at: Option<DateTime<Utc>>,
    weekly_reset_at: Option<DateTime<Utc>>,
    monthly_reset_at: Option<DateTime<Utc>>,
}

impl LockedRow {
    fn into_quota(self) -> SubscriptionQuota {
        SubscriptionQuota {
            id: self.id,
            group_id: self.group_id,
            daily_usage_usd: self.daily_usage_usd,
            weekly_usage_usd: self.weekly_usage_usd,
            monthly_usage_usd: self.monthly_usage_usd,
            daily_limit_usd: self.daily_limit_usd,
            weekly_limit_usd: self.weekly_limit_usd,
            monthly_limit_usd: self.monthly_limit_usd,
            daily_reset_at: normalise_reset_at(self.daily_reset_at),
            weekly_reset_at: normalise_reset_at(self.weekly_reset_at),
            monthly_reset_at: normalise_reset_at(self.monthly_reset_at),
        }
    }
}

#[cfg(test)]
mod tests;

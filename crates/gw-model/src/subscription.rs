//! 订阅套餐、订阅实例，以及三个配额重置时刻的算法。

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::compat;

/// `subscription_packages` 的实体。
///
/// 三个 `*_limit_usd` 是 `*float64`：NULL 表示「该周期不限额」,
/// 与 0 不是一回事，所以是 `Option<f64>` 而不是 `f64`。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SubscriptionPackage {
    pub id: Id,
    pub name: String,
    #[sqlx(try_from = "compat::Text")]
    pub description: String,
    pub group_id: Id,
    #[sqlx(try_from = "compat::Money")]
    pub rate_multiplier: f64,
    #[sqlx(try_from = "compat::Int")]
    pub default_validity_days: i64,
    #[sqlx(try_from = "compat::MoneyOpt")]
    pub daily_limit_usd: Option<f64>,
    #[sqlx(try_from = "compat::MoneyOpt")]
    pub weekly_limit_usd: Option<f64>,
    #[sqlx(try_from = "compat::MoneyOpt")]
    pub monthly_limit_usd: Option<f64>,
    #[sqlx(try_from = "compat::Money")]
    pub subscription_price_usd: f64,
    #[sqlx(try_from = "compat::Bool")]
    pub enabled: bool,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

/// `subscriptions` 的实体，一次购买 = 一行。
///
/// `*_usage_usd` 是滚动累加的用量计数器，`*_reset_at` 是下一次归零的时刻（UTC）；
/// 两者成对，配额判定靠 `UsagePlugin` 在结算后累加。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Subscription {
    pub id: Id,
    pub user_id: Id,
    pub package_id: Id,
    pub group_id: Id,
    #[sqlx(try_from = "compat::Text")]
    pub group_name: String,
    #[sqlx(try_from = "compat::Text")]
    pub status: String,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Money")]
    pub daily_usage_usd: f64,
    #[sqlx(try_from = "compat::Ts")]
    pub daily_reset_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Money")]
    pub weekly_usage_usd: f64,
    #[sqlx(try_from = "compat::Ts")]
    pub weekly_reset_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Money")]
    pub monthly_usage_usd: f64,
    #[sqlx(try_from = "compat::Ts")]
    pub monthly_reset_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::MoneyOpt")]
    pub daily_limit_usd: Option<f64>,
    #[sqlx(try_from = "compat::MoneyOpt")]
    pub weekly_limit_usd: Option<f64>,
    #[sqlx(try_from = "compat::MoneyOpt")]
    pub monthly_limit_usd: Option<f64>,
    #[sqlx(try_from = "compat::Text")]
    pub funding_source: String,
    #[sqlx(try_from = "compat::Text")]
    pub funding_reference: String,
    #[sqlx(try_from = "compat::Money")]
    pub price_paid_usd: f64,
    #[sqlx(try_from = "compat::Text")]
    pub notes: String,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

/// 严格晚于 `t` 的下一个 UTC 零点。
///
/// 「严格晚于」是关键 —— `t` 正好是零点时返回的是**第二天**零点，否则重置时刻会
/// 卡在原地，配额永远不归零。
pub fn next_daily_reset_after(t: DateTime<Utc>) -> DateTime<Utc> {
    add_days(midnight_utc(t), 1)
}

/// 严格晚于 `t` 的下一个 UTC 周一 00:00
/// （ISO 周，周一为一周之始）。
pub fn next_weekly_reset_after(t: DateTime<Utc>) -> DateTime<Utc> {
    let midnight = midnight_utc(t);
    // isoDay ∈ 1..=7（周一=1），daysUntilNextMonday = 8 - isoDay。
    // chrono 的 num_days_from_monday() 是 0..=6，正好是 isoDay - 1。
    let days_until_next_monday = 7 - u64::from(midnight.weekday().num_days_from_monday());
    add_days(midnight, days_until_next_monday)
}

/// 严格晚于 `t` 的下一个 UTC 月初 00:00。
pub fn next_monthly_reset_after(t: DateTime<Utc>) -> DateTime<Utc> {
    let first_of_this_month = NaiveDate::from_ymd_opt(t.year(), t.month(), 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|d| d.and_utc())
        .unwrap_or(t);
    first_of_this_month
        .checked_add_months(Months::new(1))
        .unwrap_or(first_of_this_month)
}

fn midnight_utc(t: DateTime<Utc>) -> DateTime<Utc> {
    t.date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|d| d.and_utc())
        .unwrap_or(t)
}

fn add_days(t: DateTime<Utc>, days: u64) -> DateTime<Utc> {
    t.checked_add_days(Days::new(days)).unwrap_or(t)
}

#[cfg(test)]
mod tests;

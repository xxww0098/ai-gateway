//! `/admin/dashboard` — the six numbers the operator console opens with.
//!
//! Corresponds to `AdminDashboardHandler`.
//!
//! The three usage figures are deliberately raw `cost`, not the
//! charged-cost fallback the usage tables use: the column is summed directly
//! (`COALESCE(SUM(cost), 0)`), and a dashboard that disagreed with the usage
//! page would be worse than one that is consistently simple.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, Local, TimeZone, Utc};
use serde_json::json;

use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

/// 对应 `apiErrorInternal`。
const ERR_INTERNAL: i32 = 5000;

/// `users.status` / `api_keys.status` counted as active. 对应 `userStatusActive`
/// 与 key 计数中的字面量 `"active"`。
const STATUS_ACTIVE: &str = "active";

/// How far back "this week" reaches, inclusive of today. 旧实现为
/// `todayStart.AddDate(0, 0, -6)` —— 七个自然日，而非滚动的 168 小时。
const WEEK_DAYS_BACK: i64 = 6;

/// `GET /admin/dashboard`. 对应 `AdminDashboardHandler`。
pub async fn dashboard(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    let (today_start, week_start) = day_bounds(Local::now());

    // 旧实现发出六条独立查询，并把每个失败映射到各自的消息。
    // The messages are what an operator sees, so they are kept apart.
    let user_total = match count(&state, "SELECT COUNT(*) FROM users").await {
        Ok(value) => value,
        Err(error) => return internal(error, "统计用户失败，请稍后重试"),
    };
    let user_active = match count_status(&state, "users").await {
        Ok(value) => value,
        Err(error) => return internal(error, "统计活跃用户失败，请稍后重试"),
    };
    let key_total = match count(&state, "SELECT COUNT(*) FROM api_keys").await {
        Ok(value) => value,
        Err(error) => return internal(error, "统计 API Key 失败，请稍后重试"),
    };
    let key_active = match count_status(&state, "api_keys").await {
        Ok(value) => value,
        Err(error) => return internal(error, "统计活跃 API Key 失败，请稍后重试"),
    };
    let today_requests = match count_since(&state, today_start).await {
        Ok(value) => value,
        Err(error) => return internal(error, "加载今日用量失败，请稍后重试"),
    };
    let today_cost = match cost_since(&state, today_start).await {
        Ok(value) => value,
        Err(error) => return internal(error, "加载今日用量费用失败，请稍后重试"),
    };
    let week_requests = match count_since(&state, week_start).await {
        Ok(value) => value,
        Err(error) => return internal(error, "加载本周用量失败，请稍后重试"),
    };

    ok(json!({
        "users": {"total": user_total, "active": user_active},
        "api_keys": {"total": key_total, "active": key_active},
        "usage": {
            "today_requests": today_requests,
            "today_cost": today_cost,
            "week_requests": week_requests,
        },
    }))
}

fn internal(error: sqlx::Error, message: &str) -> Response {
    tracing::error!(%error, "dashboard query failed");
    err(StatusCode::INTERNAL_SERVER_ERROR, ERR_INTERNAL, message)
}

/// Local midnight today, and local midnight [`WEEK_DAYS_BACK`] days earlier.
///
/// Local, not UTC: the boundaries are built with `time.Date(..., today.Location())`,
/// so "today" is the operator's day.
#[must_use]
pub fn day_bounds(now: DateTime<Local>) -> (DateTime<Utc>, DateTime<Utc>) {
    let today = local_midnight(now.date_naive());
    let week = local_midnight(now.date_naive() - chrono::Duration::days(WEEK_DAYS_BACK));
    (today, week)
}

/// Local midnight of `day`, tolerating a DST transition that deletes it.
fn local_midnight(day: chrono::NaiveDate) -> DateTime<Utc> {
    let naive = day.and_hms_opt(0, 0, 0).unwrap_or_default();
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(at) => at.with_timezone(&Utc),
        chrono::LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
        chrono::LocalResult::None => (1..=23)
            .find_map(|hour| {
                Local
                    .from_local_datetime(&day.and_hms_opt(hour, 0, 0)?)
                    .earliest()
            })
            .map(|at| at.with_timezone(&Utc))
            .unwrap_or_else(Utc::now),
    }
}

async fn count(state: &PanelState, sql: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(sql).fetch_one(&state.pg).await
}

/// `SELECT COUNT(*) FROM <table> WHERE status = 'active'`.
///
/// The table name is interpolated because it is one of two compile-time
/// literals chosen here, never anything a caller supplies.
async fn count_status(state: &PanelState, table: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE status = $1"))
        .bind(STATUS_ACTIVE)
        .fetch_one(&state.pg)
        .await
}

async fn count_since(state: &PanelState, since: DateTime<Utc>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM usage_logs WHERE created_at >= $1")
        .bind(since)
        .fetch_one(&state.pg)
        .await
}

/// `cost` is `numeric`, so the SUM needs an explicit `::float8` — the entity's
/// money adapter is not in play for a scalar aggregate (CONTRACT §3.5).
async fn cost_since(state: &PanelState, since: DateTime<Utc>) -> Result<f64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost), 0)::float8 FROM usage_logs WHERE created_at >= $1",
    )
    .bind(since)
    .fetch_one(&state.pg)
    .await
}

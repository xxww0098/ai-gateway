//! `/admin/usage-logs`, `/admin/usage/trend`, `/admin/usage/models` — the
//! operator's view of what the proxy billed.
//!
//! 对应 `AdminUsageLogsHandler`、`AdminUsageTrendHandler` /
//! `AdminUsageModelsHandler`，以及它们所调的两个聚合器
//! `buildUsageTrend` / `buildUsageModels`。
//!
//! # Why the aggregation is not a `GROUP BY`
//!
//! 旧实现按**进程的当地日期**分桶（`row.CreatedAt.In(time.Local)`）。
//! Pushing that into SQL would bucket by the *database session's* `TimeZone`
//! instead, which is a different setting on a different machine. The row set is
//! bounded by `days ≤ 90` on an admin-only route, so the rows are folded here,
//! exactly where the original folds them.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

// `paging` is panel-wide vocabulary that `panel-identity` parked in its own
// domain until the coordinator promotes it to the crate root. Reaching for it
// rather than re-deriving `queryInt` keeps one declaration of the clamping
// rule (rule 1.9); the import moves when the module does.
use crate::paging::{ListPage, page_params, query_int};
use crate::{AdminUser, PanelState, err, ok};

// The user-facing half of the same views. Split out for size (rule 1.10 — this
// file was already at 500 lines), NOT because "admin" and "user" are different
// domains: they read one table under one set of column-fallback rules, and
// those rules live here, shared through `super::` rather than copied (rule 1.9).
pub mod user;

#[cfg(test)]
mod tests;

/// 对应 `apiErrorInternal`。
const ERR_INTERNAL: i32 = 5000;

// ---------------------------------------------------------------- usage logs

/// Query string of `GET /admin/usage-logs`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct UsageLogsQuery {
    pub page: Option<String>,
    pub page_size: Option<String>,
    pub model: Option<String>,
    pub status: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

/// Translates `?status=` into the `failed` predicate.
///
/// 旧实现依次测 `status == "success"`、`status == "failed"`，其余一律忽略，
/// anything else — including the empty string — leaving the query unfiltered.
#[must_use]
pub fn status_filter(status: Option<&str>) -> Option<bool> {
    match status.map(str::trim) {
        Some("success") => Some(false),
        Some("failed") => Some(true),
        _ => None,
    }
}

/// Trims a query parameter and drops it when it is empty, so `?model=` behaves
/// like an absent `model`. 对应 `strings.TrimSpace(c.Query(...)) != ""`.
fn non_empty(raw: Option<&String>) -> Option<&str> {
    raw.map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

/// The columns `/admin/usage-logs` reads. Narrower than the entity because the
/// response only maps these.
#[derive(Debug, sqlx::FromRow)]
struct UsageLogRow {
    id: i64,
    request_id: Option<String>,
    user_id: i64,
    api_key_id: i64,
    model: Option<String>,
    provider: Option<String>,
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    cached_tokens: Option<i64>,
    input_cost: Option<f64>,
    output_cost: Option<f64>,
    total_cost: Option<f64>,
    actual_cost: Option<f64>,
    cost: Option<f64>,
    rate_multiplier: Option<f64>,
    stream: Option<bool>,
    duration_ms: Option<i64>,
    failed: Option<bool>,
    created_at: DateTime<Utc>,
}

impl UsageLogRow {
    /// 对应 `usageInputTokens` — the newer four-column counter wins, falling
    /// back to the legacy `tokens_in` when it was never populated.
    fn input_tokens(&self) -> i64 {
        prefer_positive(self.input_tokens, self.tokens_in)
    }

    /// 对应 `usageOutputTokens`.
    fn output_tokens(&self) -> i64 {
        prefer_positive(self.output_tokens, self.tokens_out)
    }

    /// 对应 `usageTotalCost`.
    fn total_cost(&self) -> f64 {
        prefer_positive_f64(self.total_cost, self.cost)
    }

    /// 对应 `usageChargedCost` — what the user was actually billed, which is
    /// what the trend and per-model aggregates sum.
    fn charged_cost(&self) -> f64 {
        prefer_positive_f64(self.actual_cost, self.cost)
    }

    /// 对应 `usageRateMultiplier` — a missing or non-positive multiplier reads
    /// as the baseline, never as zero (which would render every cost free).
    fn rate_multiplier(&self) -> f64 {
        match self.rate_multiplier {
            Some(value) if value > 0.0 => value,
            _ => 1.0,
        }
    }
}

/// `primary` when it is strictly positive, else `fallback`. NULL columns read
/// as `0`（与既有列解码行为一致）。
fn prefer_positive(primary: Option<i64>, fallback: Option<i64>) -> i64 {
    match primary {
        Some(value) if value > 0 => value,
        _ => fallback.unwrap_or_default(),
    }
}

/// [`prefer_positive`] for the money columns.
fn prefer_positive_f64(primary: Option<f64>, fallback: Option<f64>) -> f64 {
    match primary {
        Some(value) if value > 0.0 => value,
        _ => fallback.unwrap_or_default(),
    }
}

/// Money columns are `numeric`; `::float8` is required for the scalar the
/// entity's `compat::Money` adapter is not decoding here.
const USAGE_LOG_COLUMNS: &str = "id, request_id, user_id, api_key_id, model, provider, \
     tokens_in, tokens_out, input_tokens, output_tokens, reasoning_tokens, cached_tokens, \
     input_cost::float8 AS input_cost, output_cost::float8 AS output_cost, \
     total_cost::float8 AS total_cost, actual_cost::float8 AS actual_cost, \
     cost::float8 AS cost, rate_multiplier::float8 AS rate_multiplier, \
     stream, duration_ms, failed, created_at";

/// The optional filters, written so an absent parameter is a NULL bind rather
/// than a different statement. One prepared statement, four toggles.
const USAGE_LOG_FILTER: &str = "($1::text IS NULL OR model = $1) \
     AND ($2::bool IS NULL OR failed = $2) \
     AND ($3::date IS NULL OR created_at::date >= $3) \
     AND ($4::date IS NULL OR created_at::date <= $4)";

/// `GET /admin/usage-logs`. 对应 `AdminUsageLogsHandler`.
pub async fn list_usage_logs(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<UsageLogsQuery>,
) -> Response {
    let (page, page_size) = page_params(query.page.as_deref(), query.page_size.as_deref(), 30);
    let offset = crate::paging::offset(page, page_size);

    let model = non_empty(query.model.as_ref());
    let failed = status_filter(query.status.as_deref());
    let start_date = non_empty(query.start_date.as_ref());
    let end_date = non_empty(query.end_date.as_ref());

    let total: Result<i64, _> = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM usage_logs WHERE {USAGE_LOG_FILTER}"
    ))
    .bind(model)
    .bind(failed)
    .bind(start_date)
    .bind(end_date)
    .fetch_one(&state.pg)
    .await;
    let total = match total {
        Ok(total) => total,
        Err(error) => {
            tracing::error!(%error, "failed to count usage logs");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "统计用量日志失败，请稍后重试",
            );
        }
    };

    let rows: Result<Vec<UsageLogRow>, _> = sqlx::query_as(&format!(
        "SELECT {USAGE_LOG_COLUMNS} FROM usage_logs WHERE {USAGE_LOG_FILTER} \
         ORDER BY created_at DESC LIMIT $5 OFFSET $6"
    ))
    .bind(model)
    .bind(failed)
    .bind(start_date)
    .bind(end_date)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pg)
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "failed to list usage logs");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "获取用量日志失败，请稍后重试",
            );
        }
    };

    // Admin views span every user, so key names are resolved without a
    // `user_id` scope. 旧实现把这项放进一个独立 helper，与用户侧的那版
    // one precisely so the unscoped version cannot be reached from a
    // user-owned endpoint.
    let key_ids: Vec<i64> = {
        let mut ids: Vec<i64> = rows
            .iter()
            .map(|row| row.api_key_id)
            .filter(|id| *id != 0)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let key_names = match api_key_names(&state, &key_ids).await {
        Ok(names) => names,
        Err(error) => {
            tracing::error!(%error, "failed to resolve api key names");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "加载 API Key 名称失败，请稍后重试",
            );
        }
    };

    let items: Vec<_> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.id,
                "request_id": row.request_id.clone().unwrap_or_default(),
                "user_id": row.user_id,
                "api_key_id": row.api_key_id,
                // A key deleted since the request was logged has no name; the
                // original's map lookup yields "" rather than null.
                "api_key_name": key_names.get(&row.api_key_id).cloned().unwrap_or_default(),
                "model": row.model.clone().unwrap_or_default(),
                "provider": row.provider.clone().unwrap_or_default(),
                "input_tokens": row.input_tokens(),
                "output_tokens": row.output_tokens(),
                "reasoning_tokens": row.reasoning_tokens.unwrap_or_default(),
                "cached_tokens": row.cached_tokens.unwrap_or_default(),
                "input_cost": row.input_cost.unwrap_or_default(),
                "output_cost": row.output_cost.unwrap_or_default(),
                "total_cost": row.total_cost(),
                "actual_cost": row.charged_cost(),
                "cost": row.cost.unwrap_or_default(),
                "rate_multiplier": row.rate_multiplier(),
                "stream": row.stream.unwrap_or_default(),
                "duration_ms": row.duration_ms.unwrap_or_default(),
                "failed": row.failed.unwrap_or_default(),
                "created_at": row.created_at,
            })
        })
        .collect();

    ok(ListPage::new(items, total, page, page_size))
}

/// 对应 `apiKeyNamesAcrossUsers`。Unscoped by design — admin views span users.
async fn api_key_names(
    state: &PanelState,
    ids: &[i64],
) -> Result<HashMap<i64, String>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, Option<String>)> =
        sqlx::query_as("SELECT id, name FROM api_keys WHERE id = ANY($1)")
            .bind(ids)
            .fetch_all(&state.pg)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| (id, name.unwrap_or_default()))
        .collect())
}

// ---------------------------------------------------------------- aggregates

/// Query string of the two aggregate routes.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct DaysQuery {
    pub days: Option<String>,
}

/// One day of `/admin/usage/trend`. 对应 `trendPoint`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrendPoint {
    pub date: String,
    pub requests: i64,
    pub tokens: i64,
    pub cost: f64,
}

/// One model of `/admin/usage/models`. 对应 `modelPoint`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelPoint {
    pub model: String,
    pub requests: i64,
    pub tokens: i64,
    pub cost: f64,
}

/// How many models `/admin/usage/models` returns at most. 对应 `buildUsageModels`
/// 里 `if len(items) > 20 { items = items[:20] }` 的截断。
const MODEL_POINT_LIMIT: usize = 20;

/// Bucket label a row falls into. 对应 `row.CreatedAt.In(time.Local).Format("2006-01-02")`.
fn local_day(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).date_naive().to_string()
}

/// Midnight, local time, `days - 1` days ago — the inclusive lower bound both
/// aggregates scan from. 对应 `time.Now().AddDate(0,0,-(days-1))`，再截断到
/// the local day.
fn window_start(now: DateTime<Local>, days: i64) -> DateTime<Utc> {
    let day = now.date_naive() - chrono::Duration::days(days - 1);
    local_midnight(day)
}

/// Local midnight of `day` as an instant.
///
/// A DST spring-forward can delete local midnight; the earliest instant that
/// exists on that date is then the right lower bound, and a fold picks the
/// earlier of the two ambiguous instants. 既有实现里的 `time.Date` 宁可就近取整
/// 也不报错，so this must not panic either.
fn local_midnight(day: NaiveDate) -> DateTime<Utc> {
    match Local.from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap_or_default()) {
        chrono::LocalResult::Single(at) => at.with_timezone(&Utc),
        chrono::LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
        chrono::LocalResult::None => {
            // Midnight does not exist on this date; step forward until it does.
            (1..=23)
                .find_map(|hour| {
                    let naive = day.and_hms_opt(hour, 0, 0)?;
                    Local.from_local_datetime(&naive).earliest()
                })
                .map(|at| at.with_timezone(&Utc))
                .unwrap_or_else(Utc::now)
        }
    }
}

/// The columns the aggregates need. Deliberately not the whole row.
const AGGREGATE_COLUMNS: &str = "model, tokens_in, tokens_out, input_tokens, output_tokens, \
     actual_cost::float8 AS actual_cost, cost::float8 AS cost, created_at";

/// The columns the two folds read. `pub(crate)` only because they appear in
/// the folds' signatures; nothing outside this module constructs one.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct AggregateRow {
    model: Option<String>,
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    actual_cost: Option<f64>,
    cost: Option<f64>,
    created_at: DateTime<Utc>,
}

impl AggregateRow {
    fn tokens(&self) -> i64 {
        prefer_positive(self.input_tokens, self.tokens_in)
            + prefer_positive(self.output_tokens, self.tokens_out)
    }

    fn charged_cost(&self) -> f64 {
        prefer_positive_f64(self.actual_cost, self.cost)
    }
}

/// Loads the rows both folds run over.
///
/// `user_id` 对应旧实现的 `if userID > 0 { q = q.Where("user_id = ?", userID) }`：
/// the admin routes pass `None` (every user), the `/user/usage/**` routes pass
/// the caller. One query, one place where the scoping predicate is written.
async fn load_window(
    state: &PanelState,
    start: DateTime<Utc>,
    user_id: Option<i64>,
) -> Result<Vec<AggregateRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {AGGREGATE_COLUMNS} FROM usage_logs \
         WHERE created_at >= $1 AND ($2::bigint IS NULL OR user_id = $2)"
    ))
    .bind(start)
    .bind(user_id)
    .fetch_all(&state.pg)
    .await
}

/// `GET /admin/usage/trend`. 对应 `AdminUsageTrendHandler` → `buildUsageTrend(0, days)`.
///
/// The payload is a bare array under `data`. Every day in the window is present
/// even with no traffic, so the chart has no gaps.
pub async fn usage_trend(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<DaysQuery>,
) -> Response {
    let days = query_int(query.days.as_deref(), 7, 1, 30);
    let now = Local::now();
    match load_window(&state, window_start(now, days), None).await {
        Ok(rows) => ok(build_trend(&rows, now, days)),
        Err(error) => {
            tracing::error!(%error, "failed to load usage trend");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "加载用量趋势失败，请稍后重试",
            )
        }
    }
}

/// `GET /admin/usage/models`. 对应 `AdminUsageModelsHandler` → `buildUsageModels(0, days)`.
pub async fn usage_models(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<DaysQuery>,
) -> Response {
    let days = query_int(query.days.as_deref(), 30, 1, 90);
    let now = Local::now();
    match load_window(&state, window_start(now, days), None).await {
        Ok(rows) => ok(build_models(&rows)),
        Err(error) => {
            tracing::error!(%error, "failed to load usage models");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "加载用量模型失败，请稍后重试",
            )
        }
    }
}

/// Folds rows into one point per local day, oldest first.
///
/// 对应 `buildUsageTrend`。Rows outside the pre-seeded bucket set are dropped
/// (they can only be rows that arrived after the window was computed).
#[must_use]
pub(crate) fn build_trend(
    rows: &[AggregateRow],
    now: DateTime<Local>,
    days: i64,
) -> Vec<TrendPoint> {
    let first = now.date_naive() - chrono::Duration::days(days - 1);
    let mut points: Vec<TrendPoint> = (0..days)
        .map(|offset| TrendPoint {
            date: (first + chrono::Duration::days(offset)).to_string(),
            requests: 0,
            tokens: 0,
            cost: 0.0,
        })
        .collect();
    let index: HashMap<String, usize> = points
        .iter()
        .enumerate()
        .map(|(index, point)| (point.date.clone(), index))
        .collect();

    for row in rows {
        let Some(&slot) = index.get(&local_day(row.created_at)) else {
            continue;
        };
        let point = &mut points[slot];
        point.requests += 1;
        point.tokens += row.tokens();
        point.cost += row.charged_cost();
    }
    points
}

/// Folds rows into one point per model, busiest first.
///
/// 对应 `buildUsageModels` — ties break on the model name so the order is
/// deterministic, a blank model becomes `unknown`, and the list is truncated.
#[must_use]
pub(crate) fn build_models(rows: &[AggregateRow]) -> Vec<ModelPoint> {
    let mut table: HashMap<String, ModelPoint> = HashMap::new();
    for row in rows {
        let name = row.model.as_deref().unwrap_or_default().trim();
        let name = if name.is_empty() { "unknown" } else { name };
        let point = table.entry(name.to_owned()).or_insert_with(|| ModelPoint {
            model: name.to_owned(),
            requests: 0,
            tokens: 0,
            cost: 0.0,
        });
        point.requests += 1;
        point.tokens += row.tokens();
        point.cost += row.charged_cost();
    }

    let mut items: Vec<ModelPoint> = table.into_values().collect();
    items.sort_by(|left, right| {
        right
            .requests
            .cmp(&left.requests)
            .then_with(|| left.model.cmp(&right.model))
    });
    items.truncate(MODEL_POINT_LIMIT);
    items
}

/// Test-only constructor so the fold functions can be exercised without a
/// database. Not `pub`: nothing outside this module builds an `AggregateRow`.
#[cfg(test)]
impl AggregateRow {
    fn at(created_at: DateTime<Utc>, model: &str, tokens: i64, cost: f64) -> Self {
        Self {
            model: Some(model.to_owned()),
            tokens_in: Some(tokens),
            tokens_out: Some(0),
            input_tokens: None,
            output_tokens: None,
            actual_cost: Some(cost),
            cost: None,
            created_at,
        }
    }
}

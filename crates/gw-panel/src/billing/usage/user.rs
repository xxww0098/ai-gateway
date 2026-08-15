//! `/user/usage`, `/user/usage/detail`, `/user/usage/stats`, `/user/usage/trend`,
//! `/user/usage/models` —— 用户看自己的账。
//!
//! 对应 `UsageHandler` / `UsageDetailHandler` / `UsageStatsHandler` /
//! `UserUsageTrendHandler` / `UserUsageModelsHandler`
//! 以及它们调的 `applyUsageDetailFilters` / `usageDetailStatsForQuery` /
//! `apiKeyNamesForUsageLogs` / `usageStatsSince`。
//!
//! # 为什么不在 `billing/` 之外
//!
//! 规则 1.6：删掉一个功能应该等于删掉一个文件夹。这五条路由读的是
//! [`super`] 里管理员那三条读的**同一张 `usage_logs`、同一套四列口径**
//! （`input_tokens>0 ? input_tokens : tokens_in` 这类回退规则一字不差）。
//! 按 admin/user 的角色把它们劈到两个域，等于让「用量口径」这一个概念横跨两处，
//! 下次改回退规则就必然漏改一边。
//!
//! # 为什么另起一个文件而不是塞进 `usage.rs`
//!
//! 规则 1.10：单文件超 1,000 行就该停下来看一眼。父模块已经 500 行，这五条再进去
//! 就顶到线上。所以拆成子模块 —— 但**仍在同一个域目录里**，共享口径（
//! [`super::prefer_positive`]、[`super::build_trend`]、[`super::AggregateRow`]）
//! 直接走 `super::`，不复制一份（规则 1.9）。
//!
//! # 用户侧与管理员侧唯一不能共用的那一段
//!
//! API Key 名字的解析：管理员那条走 `apiKeyNamesAcrossUsers`（不带 `user_id`
//! 约束），用户这条必须走带约束的版本。旧实现专门写成两个函数并在注释里点名
//! 「it must never be used on a user-owned endpoint」—— 共用会让用户从自己的
//! 用量明细里读出别人的 Key 名字。

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use base64::Engine as _;
use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::paging::{Page, page_params, query_int};
use crate::{AuthUser, PanelState, err, ok};

use super::{
    DaysQuery, ERR_INTERNAL, build_models, build_trend, load_window, local_midnight, non_empty,
    prefer_positive, prefer_positive_f64, window_start,
};

#[cfg(test)]
mod tests;

/// 对应 `apiErrorBadRequest`。
const ERR_BAD_REQUEST: i32 = 4000;

// ---------------------------------------------------------------- /user/usage

/// `GET /user/usage` 与 `/user/usage/detail` 共用的翻页参数。
#[derive(Debug, Default, Clone, Deserialize)]
pub struct UsageQuery {
    pub page: Option<String>,
    pub page_size: Option<String>,
    // ── 只有 /detail 会用到的筛选项 ──
    pub api_key_id: Option<String>,
    pub model: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub status: Option<String>,
}

/// `usage_logs` 的**整行**，用来复刻旧实现直接把 `[]model.UsageLog` 塞进响应的那条路由。
///
/// `raw_metadata` 取的是 `::text` 而不是 `jsonb`：旧实现的字段是 `[]byte`，
/// `encoding/json` 把它编成 **base64 字符串**，编的正是驱动交回来的那串原文。
/// 解成 `serde_json::Value` 再自己序列化会得到一个 JSON 对象 —— 那是另一种形状。
#[derive(Debug, sqlx::FromRow)]
struct EntityRow {
    id: i64,
    user_id: i64,
    api_key_id: i64,
    group_id: Option<i64>,
    request_id: Option<String>,
    idempotency_key: Option<String>,
    event_key: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    auth_id: Option<String>,
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
    ip_address: Option<String>,
    raw_metadata: Option<String>,
    failed: Option<bool>,
    created_at: DateTime<Utc>,
}

/// 只在测试里存在的构造器：把整行拉成"全 NULL"，让每个用例只填它关心的那一列。
/// 库外没有人构造 [`EntityRow`]，所以它不是公开 API。
#[cfg(test)]
impl EntityRow {
    fn blank(created_at: DateTime<Utc>) -> Self {
        Self {
            id: 1,
            user_id: 2,
            api_key_id: 0,
            group_id: None,
            request_id: None,
            idempotency_key: None,
            event_key: None,
            model: None,
            provider: None,
            auth_id: None,
            tokens_in: None,
            tokens_out: None,
            input_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            cached_tokens: None,
            input_cost: None,
            output_cost: None,
            total_cost: None,
            actual_cost: None,
            cost: None,
            rate_multiplier: None,
            stream: None,
            duration_ms: None,
            ip_address: None,
            raw_metadata: None,
            failed: None,
            created_at,
        }
    }
}

/// `model.UsageLog` 上**一个 json tag 都没有**，所以 `encoding/json` 用的是字段名
/// 原样：`ID` / `UserID` / `ApiKeyID` / `IPAddress` / `TokensIn` …… 这不是笔误，
/// 也不能"顺手改成" snake_case —— 那是 `/user/usage/detail` 的形状，两条路由在
/// 旧实现里就是不一样的。serde 的 `rename_all = "PascalCase"` 也救不了：它会把
/// `ID` 写成 `Id`、`IPAddress` 写成 `IpAddress`。所以逐键手写。
///
/// 空值语义同样照抄：`GroupID *uint` 为 nil → `null`；`RawMetadata []byte` 为
/// nil → `null`，非 nil → base64；其余标量列 NULL 在旧实现那边是零值。
#[must_use]
fn entity_json(row: &EntityRow) -> Value {
    json!({
        "ID": row.id,
        "UserID": row.user_id,
        "ApiKeyID": row.api_key_id,
        "GroupID": row.group_id,
        "RequestID": row.request_id.clone().unwrap_or_default(),
        "IdempotencyKey": row.idempotency_key.clone().unwrap_or_default(),
        "EventKey": row.event_key.clone().unwrap_or_default(),
        "Model": row.model.clone().unwrap_or_default(),
        "Provider": row.provider.clone().unwrap_or_default(),
        "AuthID": row.auth_id.clone().unwrap_or_default(),
        "TokensIn": row.tokens_in.unwrap_or_default(),
        "TokensOut": row.tokens_out.unwrap_or_default(),
        "InputTokens": row.input_tokens.unwrap_or_default(),
        "OutputTokens": row.output_tokens.unwrap_or_default(),
        "ReasoningTokens": row.reasoning_tokens.unwrap_or_default(),
        "CachedTokens": row.cached_tokens.unwrap_or_default(),
        "InputCost": row.input_cost.unwrap_or_default(),
        "OutputCost": row.output_cost.unwrap_or_default(),
        "TotalCost": row.total_cost.unwrap_or_default(),
        "ActualCost": row.actual_cost.unwrap_or_default(),
        "Cost": row.cost.unwrap_or_default(),
        "RateMultiplier": row.rate_multiplier.unwrap_or_default(),
        "Stream": row.stream.unwrap_or_default(),
        "DurationMs": row.duration_ms.unwrap_or_default(),
        "IPAddress": row.ip_address.clone().unwrap_or_default(),
        "RawMetadata": row
            .raw_metadata
            .as_ref()
            .map(|raw| base64::engine::general_purpose::STANDARD.encode(raw)),
        "Failed": row.failed.unwrap_or_default(),
        "CreatedAt": row.created_at,
    })
}

/// 整行的列清单。金额列是 `numeric`，标量解码要 `::float8`；`raw_metadata` 要
/// `::text`（见 [`EntityRow`]）。
const ENTITY_COLUMNS: &str = "id, user_id, api_key_id, group_id, request_id, idempotency_key, \
     event_key, model, provider, auth_id, tokens_in, tokens_out, input_tokens, output_tokens, \
     reasoning_tokens, cached_tokens, input_cost::float8 AS input_cost, \
     output_cost::float8 AS output_cost, total_cost::float8 AS total_cost, \
     actual_cost::float8 AS actual_cost, cost::float8 AS cost, \
     rate_multiplier::float8 AS rate_multiplier, stream, duration_ms, ip_address, \
     raw_metadata::text AS raw_metadata, failed, created_at";

/// `GET /user/usage`。对应 `UsageHandler`。
///
/// 不带任何筛选，信封是带 `total_pages` 的那一种（[`Page`]），`items` 是**实体原样**。
pub async fn list_usage(
    State(state): State<PanelState>,
    user: AuthUser,
    Query(query): Query<UsageQuery>,
) -> Response {
    let (page, page_size) = page_params(query.page.as_deref(), query.page_size.as_deref(), 20);
    let offset = crate::paging::offset(page, page_size);

    let total: Result<i64, _> =
        sqlx::query_scalar("SELECT COUNT(*) FROM usage_logs WHERE user_id = $1")
            .bind(user.user_id)
            .fetch_one(&state.pg)
            .await;
    let total = match total {
        Ok(total) => total,
        Err(error) => {
            tracing::error!(%error, "failed to count user usage");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "查询用量失败，请稍后重试",
            );
        }
    };

    let rows: Result<Vec<EntityRow>, _> = sqlx::query_as(&format!(
        "SELECT {ENTITY_COLUMNS} FROM usage_logs WHERE user_id = $1 \
         ORDER BY created_at DESC LIMIT $2 OFFSET $3"
    ))
    .bind(user.user_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pg)
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "failed to list user usage");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "获取用量记录失败，请稍后重试",
            );
        }
    };

    let items: Vec<Value> = rows.iter().map(entity_json).collect();
    ok(Page::new(items, page, page_size, total))
}

// -------------------------------------------------------- /user/usage/detail

/// `/user/usage/detail` 的筛选条件，全部校验过。
///
/// Ports `applyUsageDetailFilters` 的产物。空 `Option` = 该条件不生效，在 SQL 里
/// 是一个 NULL 绑参而不是另一条语句 —— 一条预编译语句，五个开关。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DetailFilters {
    pub api_key_id: Option<i64>,
    /// 已经拼好 `%…%` 的 ILIKE pattern。
    pub model_like: Option<String>,
    pub start: Option<DateTime<Utc>>,
    /// **开区间上界**：旧实现用的是 `created_at < end+1day`，不是 `<=`。
    pub end_exclusive: Option<DateTime<Utc>>,
    pub failed: Option<bool>,
}

/// `?status=` → `failed` 谓词。
///
/// 与管理员那条（[`super::status_filter`]）**故意不同**：用户这条对无法识别的值
/// 返回 400「状态无效」，管理员那条静默忽略。旧实现就是这么写的，别统一。
///
/// # Errors
/// 无法识别的值返回 `Err(())`，由调用方翻成 400。
fn user_status_filter(status: Option<&str>) -> Result<Option<bool>, ()> {
    match status.map(str::trim).unwrap_or("") {
        "" | "all" => Ok(None),
        "success" => Ok(Some(false)),
        "failed" => Ok(Some(true)),
        _ => Err(()),
    }
}

/// `YYYY-MM-DD` → 当地零点。对应 `time.ParseInLocation("2006-01-02", …, time.Local)`。
///
/// # Errors
/// 格式不合法返回 `Err(())`。
fn parse_local_date(raw: &str) -> Result<NaiveDate, ()> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| ())
}

/// 纯解析部分的 [`DetailFilters`] 构造：不查库，因此可单测。
///
/// `api_key_id` 的**归属校验**留给调用方 —— 那一步要查库，而且旧实现把它排在最前面。
///
/// # Errors
/// 返回旧实现那句一模一样的中文错误文案。
fn parse_detail_filters(query: &UsageQuery) -> Result<DetailFilters, &'static str> {
    let mut filters = DetailFilters::default();

    if let Some(raw) = non_empty(query.api_key_id.as_ref()) {
        // 对应 `ParseUint`，且把 0 也算错（自增主键从 1 起）。
        filters.api_key_id = Some(crate::paging::parse_id(raw).ok_or("无效的 API Key ID")?);
    }

    if let Some(raw) = non_empty(query.model.as_ref()) {
        // 旧实现不转义 `%` / `_`。照抄：转义会让「搜 `gpt_4`」的结果和旧实现不一样。
        filters.model_like = Some(format!("%{raw}%"));
    }

    if let Some(raw) = non_empty(query.start_date.as_ref()) {
        let day = parse_local_date(raw).map_err(|()| "开始日期无效")?;
        filters.start = Some(local_midnight(day));
    }

    if let Some(raw) = non_empty(query.end_date.as_ref()) {
        let day = parse_local_date(raw).map_err(|()| "结束日期无效")?;
        // 对应 `end.AddDate(0, 0, 1)` —— 上界是**次日零点，开区间**，这样「结束日期」
        // 这一整天是含在内的。写成 `<= end` 会把当天的记录全丢掉。
        let next = day.succ_opt().ok_or("结束日期无效")?;
        filters.end_exclusive = Some(local_midnight(next));
    }

    filters.failed = user_status_filter(query.status.as_deref()).map_err(|()| "状态无效")?;

    Ok(filters)
}

/// 五个可选筛选项，与 [`DetailFilters`] 的绑参顺序一一对应（`$1` 是 `user_id`）。
const DETAIL_FILTER: &str = "user_id = $1 \
     AND ($2::bigint IS NULL OR api_key_id = $2) \
     AND ($3::text IS NULL OR model ILIKE $3) \
     AND ($4::timestamptz IS NULL OR created_at >= $4) \
     AND ($5::timestamptz IS NULL OR created_at < $5) \
     AND ($6::bool IS NULL OR failed = $6)";

/// `usageDetailStats` 的 Rust 版。字段名即 JSON 键。
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct UsageDetailStats {
    pub total_requests: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub total_actual_cost: f64,
    pub success_count: i64,
    pub fail_count: i64,
    pub avg_duration_ms: f64,
}

/// 聚合出来的原始七列；`total_tokens` 与 `total_requests` 由外面补。
#[derive(Debug, sqlx::FromRow)]
struct StatsRow {
    input_tokens: i64,
    output_tokens: i64,
    total_cost: f64,
    total_actual_cost: f64,
    success_count: i64,
    fail_count: i64,
    avg_duration_ms: f64,
}

/// `usageDetailStatsForQuery` 的那段 SELECT，逐字照抄。
///
/// 三处不能改：
/// * 四列口径的回退（`CASE WHEN input_tokens > 0 …`）必须在 SQL 里，不能在 Rust 里
///   重算 —— 那样会把整表拉进内存；
/// * `AVG(NULLIF(duration_ms, 0))` 把 0 排除在分母外，不是把 0 算进去；
/// * 金额列是 `numeric`，聚合出来仍是 `numeric`，要 `::float8` 才能解成 `f64`。
const DETAIL_STATS_SELECT: &str = "\
     COALESCE(SUM(CASE WHEN input_tokens > 0 THEN input_tokens ELSE tokens_in END), 0)::bigint AS input_tokens, \
     COALESCE(SUM(CASE WHEN output_tokens > 0 THEN output_tokens ELSE tokens_out END), 0)::bigint AS output_tokens, \
     COALESCE(SUM(CASE WHEN total_cost > 0 THEN total_cost ELSE cost END), 0)::float8 AS total_cost, \
     COALESCE(SUM(CASE WHEN actual_cost > 0 THEN actual_cost ELSE cost END), 0)::float8 AS total_actual_cost, \
     COALESCE(SUM(CASE WHEN failed = false THEN 1 ELSE 0 END), 0)::bigint AS success_count, \
     COALESCE(SUM(CASE WHEN failed = true THEN 1 ELSE 0 END), 0)::bigint AS fail_count, \
     COALESCE(AVG(NULLIF(duration_ms, 0)), 0)::float8 AS avg_duration_ms";

/// `GET /user/usage/detail`。对应 `UsageDetailHandler`。
pub async fn usage_detail(
    State(state): State<PanelState>,
    user: AuthUser,
    Query(query): Query<UsageQuery>,
) -> Response {
    let (page, page_size) = page_params(query.page.as_deref(), query.page_size.as_deref(), 20);
    let offset = crate::paging::offset(page, page_size);

    let filters = match parse_detail_filters(&query) {
        Ok(filters) => filters,
        Err(message) => return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, message),
    };

    // 归属校验。旧实现在拼 WHERE 之前先做，因为「这个 key 不是你的」是 400 而不是
    // 「查出 0 行」—— 后者会变成一个存在性预言机。
    if let Some(api_key_id) = filters.api_key_id {
        let owned: Result<i64, _> =
            sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE id = $1 AND user_id = $2")
                .bind(api_key_id)
                .bind(user.user_id)
                .fetch_one(&state.pg)
                .await;
        match owned {
            Ok(0) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    ERR_BAD_REQUEST,
                    "该 API Key 不属于当前用户",
                );
            }
            Ok(_) => {}
            Err(error) => {
                tracing::error!(%error, "failed to verify api key ownership");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ERR_INTERNAL,
                    "校验 API Key 失败，请稍后重试",
                );
            }
        }
    }

    let total: Result<i64, _> = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM usage_logs WHERE {DETAIL_FILTER}"
    ))
    .bind(user.user_id)
    .bind(filters.api_key_id)
    .bind(filters.model_like.as_deref())
    .bind(filters.start)
    .bind(filters.end_exclusive)
    .bind(filters.failed)
    .fetch_one(&state.pg)
    .await;
    let total = match total {
        Ok(total) => total,
        Err(error) => {
            tracing::error!(%error, "failed to count usage detail");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "查询用量明细失败，请稍后重试",
            );
        }
    };

    let rows: Result<Vec<EntityRow>, _> = sqlx::query_as(&format!(
        "SELECT {ENTITY_COLUMNS} FROM usage_logs WHERE {DETAIL_FILTER} \
         ORDER BY created_at DESC LIMIT $7 OFFSET $8"
    ))
    .bind(user.user_id)
    .bind(filters.api_key_id)
    .bind(filters.model_like.as_deref())
    .bind(filters.start)
    .bind(filters.end_exclusive)
    .bind(filters.failed)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pg)
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "failed to list usage detail");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "获取用量明细失败，请稍后重试",
            );
        }
    };

    // 统计跑在**筛选后的同一个集合**上（旧实现复用了那个 `base` session），不是跑在
    // 当前这一页上：翻到第 3 页时 `total_cost` 仍然是整个筛选结果的总额。
    let stats: Result<StatsRow, _> = sqlx::query_as(&format!(
        "SELECT {DETAIL_STATS_SELECT} FROM usage_logs WHERE {DETAIL_FILTER}"
    ))
    .bind(user.user_id)
    .bind(filters.api_key_id)
    .bind(filters.model_like.as_deref())
    .bind(filters.start)
    .bind(filters.end_exclusive)
    .bind(filters.failed)
    .fetch_one(&state.pg)
    .await;
    let stats = match stats {
        Ok(stats) => UsageDetailStats {
            total_requests: total,
            total_input_tokens: stats.input_tokens,
            total_output_tokens: stats.output_tokens,
            total_tokens: stats.input_tokens + stats.output_tokens,
            total_cost: stats.total_cost,
            total_actual_cost: stats.total_actual_cost,
            success_count: stats.success_count,
            fail_count: stats.fail_count,
            avg_duration_ms: stats.avg_duration_ms,
        },
        Err(error) => {
            tracing::error!(%error, "failed to aggregate usage detail");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "加载用量明细统计失败，请稍后重试",
            );
        }
    };

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
    let key_names = match own_api_key_names(&state, user.user_id, &key_ids).await {
        Ok(names) => names,
        Err(error) => {
            tracing::error!(%error, "failed to resolve own api key names");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "加载 API Key 名称失败，请稍后重试",
            );
        }
    };

    // 明细这条**不是**实体原样：旧实现手写了一个 snake_case 的投影，字段集也更窄。
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.id,
                "request_id": row.request_id.clone().unwrap_or_default(),
                "provider": row.provider.clone().unwrap_or_default(),
                "model": row.model.clone().unwrap_or_default(),
                "api_key_id": row.api_key_id,
                // Key 已被删/不属于本人 → 旧实现的 map 取不到，得到 ""，不是 null。
                "api_key_name": key_names.get(&row.api_key_id).cloned().unwrap_or_default(),
                "input_tokens": prefer_positive(row.input_tokens, row.tokens_in),
                "output_tokens": prefer_positive(row.output_tokens, row.tokens_out),
                "reasoning_tokens": row.reasoning_tokens.unwrap_or_default(),
                "cached_tokens": row.cached_tokens.unwrap_or_default(),
                "input_cost": row.input_cost.unwrap_or_default(),
                "output_cost": row.output_cost.unwrap_or_default(),
                "total_cost": prefer_positive_f64(row.total_cost, row.cost),
                "actual_cost": prefer_positive_f64(row.actual_cost, row.cost),
                "cost": row.cost.unwrap_or_default(),
                "rate_multiplier": match row.rate_multiplier {
                    Some(value) if value > 0.0 => value,
                    _ => 1.0,
                },
                "stream": row.stream.unwrap_or_default(),
                "duration_ms": row.duration_ms.unwrap_or_default(),
                "failed": row.failed.unwrap_or_default(),
                "created_at": row.created_at,
            })
        })
        .collect();

    ok(json!({
        "items": items,
        "page": page,
        "page_size": page_size,
        "total": total,
        "total_pages": crate::paging::total_pages(total, page_size),
        "stats": stats,
    }))
}

/// 对应 `apiKeyNamesForUsageLogs` —— **带 `user_id` 约束**的那一个。
///
/// 与 [`super::api_key_names`] 的区别只有 `AND user_id = $2`，而那一行就是全部意义：
/// 少了它，用户能从自己的用量明细里读出别人给 Key 起的名字。
async fn own_api_key_names(
    state: &PanelState,
    user_id: i64,
    ids: &[i64],
) -> Result<HashMap<i64, String>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, Option<String>)> =
        sqlx::query_as("SELECT id, name FROM api_keys WHERE user_id = $1 AND id = ANY($2)")
            .bind(user_id)
            .bind(ids)
            .fetch_all(&state.pg)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| (id, name.unwrap_or_default()))
        .collect())
}

// --------------------------------------------------------- /user/usage/stats

/// 一个时间窗的用量小计。对应 `usageStatsSince` 返回的那个响应对象。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageWindowStats {
    pub requests: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens: i64,
    pub cost: f64,
}

#[derive(Debug, sqlx::FromRow)]
struct WindowRow {
    requests: i64,
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
}

/// `usageStatsSince` 的 SELECT，逐字照抄（金额列补 `::float8`）。
const WINDOW_STATS_SELECT: &str = "COUNT(*)::bigint AS requests, \
     COALESCE(SUM(CASE WHEN input_tokens > 0 THEN input_tokens ELSE tokens_in END), 0)::bigint AS tokens_in, \
     COALESCE(SUM(CASE WHEN output_tokens > 0 THEN output_tokens ELSE tokens_out END), 0)::bigint AS tokens_out, \
     COALESCE(SUM(CASE WHEN actual_cost > 0 THEN actual_cost ELSE cost END), 0)::float8 AS cost";

/// 对应 `usageStatsSince`。
async fn stats_since(
    state: &PanelState,
    user_id: i64,
    since: DateTime<Utc>,
) -> Result<UsageWindowStats, sqlx::Error> {
    let row: WindowRow = sqlx::query_as(&format!(
        "SELECT {WINDOW_STATS_SELECT} FROM usage_logs WHERE user_id = $1 AND created_at >= $2"
    ))
    .bind(user_id)
    .bind(since)
    .fetch_one(&state.pg)
    .await?;

    Ok(UsageWindowStats {
        requests: row.requests,
        tokens_in: row.tokens_in,
        tokens_out: row.tokens_out,
        tokens: row.tokens_in + row.tokens_out,
        cost: row.cost,
    })
}

/// 三个窗口的起点。对应 `today` / `today-6d` / `today-29d`，全部按**当地**零点。
///
/// 注意 `week` 不是「本周一」、`month` 不是「本月 1 号」—— 是滚动 7 天 / 30 天。
/// 键名叫 `week` / `month` 只是前端的叫法。
#[must_use]
fn stats_windows(now: DateTime<Local>) -> (DateTime<Utc>, DateTime<Utc>, DateTime<Utc>) {
    let today = now.date_naive();
    (
        local_midnight(today),
        local_midnight(today - chrono::Duration::days(6)),
        local_midnight(today - chrono::Duration::days(29)),
    )
}

/// `GET /user/usage/stats`。对应 `UsageStatsHandler`。
pub async fn usage_stats(State(state): State<PanelState>, user: AuthUser) -> Response {
    let (today, week, month) = stats_windows(Local::now());

    let mut out = Vec::with_capacity(3);
    for (label, since, message) in [
        ("today", today, "加载今日用量统计失败，请稍后重试"),
        ("week", week, "加载本周用量统计失败，请稍后重试"),
        ("month", month, "加载本月用量统计失败，请稍后重试"),
    ] {
        match stats_since(&state, user.user_id, since).await {
            Ok(stats) => out.push((label, stats)),
            Err(error) => {
                tracing::error!(%error, window = label, "failed to load usage stats");
                return err(StatusCode::INTERNAL_SERVER_ERROR, ERR_INTERNAL, message);
            }
        }
    }

    ok(json!({
        "today": out[0].1,
        "week": out[1].1,
        "month": out[2].1,
    }))
}

// ------------------------------------------------- /user/usage/{trend,models}

/// `GET /user/usage/trend`。对应 `UserUsageTrendHandler` → `buildUsageTrend(uid, days)`。
///
/// 与管理员那条（[`super::usage_trend`]）是**同一个折叠函数**，只是多一个
/// `user_id` 约束 —— 旧实现也是同一个 `buildUsageTrend`，`userID > 0` 时加约束。
pub async fn usage_trend(
    State(state): State<PanelState>,
    user: AuthUser,
    Query(query): Query<DaysQuery>,
) -> Response {
    let days = query_int(query.days.as_deref(), 7, 1, 30);
    let now = Local::now();
    match load_window(&state, window_start(now, days), Some(user.user_id)).await {
        Ok(rows) => ok(build_trend(&rows, now, days)),
        Err(error) => {
            tracing::error!(%error, "failed to load user usage trend");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "加载用量趋势失败，请稍后重试",
            )
        }
    }
}

/// `GET /user/usage/models`。对应 `UserUsageModelsHandler` → `buildUsageModels(uid, days)`。
pub async fn usage_models(
    State(state): State<PanelState>,
    user: AuthUser,
    Query(query): Query<DaysQuery>,
) -> Response {
    let days = query_int(query.days.as_deref(), 30, 1, 90);
    let now = Local::now();
    match load_window(&state, window_start(now, days), Some(user.user_id)).await {
        Ok(rows) => ok(build_models(&rows)),
        Err(error) => {
            tracing::error!(%error, "failed to load user usage models");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "加载用量模型失败，请稍后重试",
            )
        }
    }
}

//! `GET /api/service/users/{user_id}/usage` 与 `.../usage/by-idempotency-key/{key}`。
//!
//! 契约 §3.4 / §3.5。两条路由读的是 `billing::usage::user` 那张 `usage_logs`、同一套
//! 四列口径（`input_tokens > 0 ? input_tokens : tokens_in` 这类回退一字不差）——
//! 规则 1.9：一个概念只声明一处，改口径不该漏改一边。
//!
//! # 扣款口径
//!
//! `cost` 是价表算出的金额，`actual_cost` 是实际扣款。当前结算路径成功时三列金额
//! 相同，失败时三列都写 0；投影里额外显式判 `failed`，把"失败请求的 `actual_cost`
//! 恒为 `0.00000000`"变成投影层的不变量，而不是依赖每个写入方都记得清零。
//! ozon-pod 以 `actual_cost` 记账，两者都原样透传，本端点不做任何金额运算。
//!
//! # 未命中不是错误
//!
//! `by-idempotency-key` 未命中返回 `200 + found:false, row:null`：对 ozon-pod 表示
//! "还没结算完，稍后再问"。只有未知 `user_id` 才是 `404 + 4004`。

use axum::extract::{Path, Query, State};
use axum::response::Response;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use gw_model::compat::Ts;

use super::{ServiceCaller, amount_str, bad_request, db_failure, not_found, timestamp, user_exists};
use crate::PanelState;
use crate::ok;
use crate::paging::{offset, parse_id, query_int};

/// 不带任何参数时的窗口：最近 7 天。
const DEFAULT_WINDOW_DAYS: i64 = 7;
/// 窗口上限（契约 §3.4），超限 4000。
const MAX_WINDOW_DAYS: i64 = 90;
/// 默认页大小。
const DEFAULT_PAGE_SIZE: i64 = 50;
/// 页大小上限。契约写的是 200，比面板通用的 100 宽 —— 照契约，不要"统一"。
const MAX_PAGE_SIZE: i64 = 200;

/// 契约 §3.4 的查询参数。全部可选；解析失败走默认值（`query_int` 的语义）。
#[derive(Debug, Default, Clone, Deserialize)]
pub struct UsageQuery {
    /// RFC3339；缺省 = `end - 7 天`。
    pub start: Option<String>,
    /// RFC3339；缺省 = 现在。
    pub end: Option<String>,
    pub page: Option<String>,
    pub page_size: Option<String>,
}

/// 已校验的查询窗口，闭区间 `[start, end]`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageWindow {
    /// 闭区间下界（含）。
    pub start: DateTime<Utc>,
    /// 闭区间上界（含）。
    pub end: DateTime<Utc>,
}

/// 契约 §3.4 的 `data`。
///
/// **没有** `total_pages`：夹具钉死的键集合就是 `page` / `page_size` / `total` /
/// `rows`，多一个键就是破坏性变更（契约 §6）。
#[derive(Debug, Serialize)]
pub struct UsagePage {
    /// 当前页（从 1 起）。
    pub page: i64,
    /// 本页请求的页大小（夹紧到 200 之后的值）。
    pub page_size: i64,
    /// 时间窗内的总行数。
    pub total: i64,
    /// 本页的行，按 `created_at DESC, id DESC`。
    pub rows: Vec<UsageRowView>,
}

/// 契约 §3.5 的 `data`。
#[derive(Debug, Serialize)]
pub struct UsageLookup {
    /// 命中为 `true`；未命中 `false` + `row:null`，HTTP 仍是 200。
    pub found: bool,
    /// 命中的那一行；未命中为 `null`。
    pub row: Option<UsageRowView>,
}

/// 契约 §3.4 的用量行。
#[derive(Debug, Serialize)]
pub struct UsageRowView {
    pub id: i64,
    pub request_id: String,
    pub idempotency_key: String,
    pub model: String,
    pub provider: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    /// `tokens_in + tokens_out`。
    pub tokens: i64,
    /// 价表算出额，8 位小数。
    pub cost: String,
    /// 实际扣款，8 位小数；失败请求恒为 `"0.00000000"`。
    pub actual_cost: String,
    /// 金额的币种：部署级 `service.currency`，与余额端点同一取值。
    pub currency: String,
    pub failed: bool,
    pub duration_ms: i64,
    /// 秒精度 RFC3339 UTC。
    pub created_at: String,
}

/// `usage_logs` 的投影列。
///
/// 回退规则与 `billing::usage::user` 相同，另外把 `failed` 显式写进 `actual_cost`
/// 的投影（见模块文档）。所有金额列都 `::float8`：`numeric` 无法直接解成 `f64`
/// （CONTRACT §3.5）。
const USAGE_COLUMNS: &str = "\
     id, \
     COALESCE(request_id, '') AS request_id, \
     COALESCE(idempotency_key, '') AS idempotency_key, \
     COALESCE(model, '') AS model, \
     COALESCE(provider, '') AS provider, \
     COALESCE(CASE WHEN input_tokens > 0 THEN input_tokens ELSE tokens_in END, 0)::bigint AS tokens_in, \
     COALESCE(CASE WHEN output_tokens > 0 THEN output_tokens ELSE tokens_out END, 0)::bigint AS tokens_out, \
     COALESCE(CASE WHEN total_cost > 0 THEN total_cost ELSE cost END, 0)::float8 AS cost, \
     COALESCE(CASE WHEN COALESCE(failed, FALSE) THEN 0 \
                   WHEN actual_cost > 0 THEN actual_cost \
                   ELSE cost END, 0)::float8 AS actual_cost, \
     COALESCE(failed, FALSE) AS failed, \
     COALESCE(duration_ms, 0)::bigint AS duration_ms, \
     created_at";

/// 投影出来的一行；字段与 [`USAGE_COLUMNS`] 一一对应。
#[derive(Debug, sqlx::FromRow)]
struct UsageRow {
    id: i64,
    request_id: String,
    idempotency_key: String,
    model: String,
    provider: String,
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
    actual_cost: f64,
    failed: bool,
    duration_ms: i64,
    #[sqlx(try_from = "Ts")]
    created_at: DateTime<Utc>,
}

impl UsageRowView {
    /// 行里只存金额，币种归部署级 `service.currency`（契约 §3.4 与 §3.6 同一取值）。
    fn of(row: &UsageRow, currency: &str) -> Self {
        Self {
            id: row.id,
            request_id: row.request_id.clone(),
            idempotency_key: row.idempotency_key.clone(),
            model: row.model.clone(),
            provider: row.provider.clone(),
            tokens_in: row.tokens_in,
            tokens_out: row.tokens_out,
            tokens: row.tokens_in + row.tokens_out,
            cost: amount_str(row.cost),
            actual_cost: amount_str(row.actual_cost),
            currency: currency.to_owned(),
            failed: row.failed,
            duration_ms: row.duration_ms,
            created_at: timestamp(row.created_at),
        }
    }
}

/// 解析并校验时间窗：缺省最近 7 天，窗口不得超过 90 天，`start` 不得晚于 `end`。
///
/// # Errors
/// 返回给调用方的 4000 文案。
pub(super) fn parse_window(
    start_raw: Option<&str>,
    end_raw: Option<&str>,
    now: DateTime<Utc>,
) -> Result<UsageWindow, &'static str> {
    let start = match start_raw.map(str::trim).filter(|raw| !raw.is_empty()) {
        Some(raw) => Some(parse_rfc3339(raw)?),
        None => None,
    };
    let end = match end_raw.map(str::trim).filter(|raw| !raw.is_empty()) {
        Some(raw) => Some(parse_rfc3339(raw)?),
        None => None,
    };

    let end = end.unwrap_or(now);
    let start = start.unwrap_or(end - Duration::days(DEFAULT_WINDOW_DAYS));
    if end < start {
        return Err("时间窗口无效：start 不能晚于 end");
    }
    if end - start > Duration::days(MAX_WINDOW_DAYS) {
        return Err("时间窗口不能超过 90 天");
    }
    Ok(UsageWindow { start, end })
}

/// RFC3339 → UTC。
///
/// # Errors
/// 格式不合法。
fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>, &'static str> {
    DateTime::parse_from_rfc3339(raw)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|_| "时间格式无效（需要 RFC3339）")
}

/// `GET /api/service/users/{user_id}/usage`。
pub async fn list(
    State(state): State<PanelState>,
    _caller: ServiceCaller,
    Path(user_id): Path<String>,
    Query(query): Query<UsageQuery>,
) -> Response {
    let Some(user_id) = parse_id(&user_id) else {
        return bad_request("无效的用户 ID");
    };
    let window = match parse_window(query.start.as_deref(), query.end.as_deref(), Utc::now()) {
        Ok(window) => window,
        Err(message) => return bad_request(message),
    };
    let page = query_int(query.page.as_deref(), 1, 1, 1_000_000);
    let page_size = query_int(
        query.page_size.as_deref(),
        DEFAULT_PAGE_SIZE,
        1,
        MAX_PAGE_SIZE,
    );

    match user_exists(&state.pg, user_id).await {
        Ok(true) => {}
        Ok(false) => return not_found("用户不存在"),
        Err(error) => return db_failure("service_usage_user_lookup", &error, "查询用量失败，请稍后重试"),
    }

    let total: Result<i64, _> = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM usage_logs \
         WHERE user_id = $1 AND created_at >= $2 AND created_at <= $3",
    )
    .bind(user_id)
    .bind(window.start)
    .bind(window.end)
    .fetch_one(&state.pg)
    .await;
    let total = match total {
        Ok(total) => total,
        Err(error) => return db_failure("service_usage_count", &error, "查询用量失败，请稍后重试"),
    };

    let rows: Result<Vec<UsageRow>, _> = sqlx::query_as(&format!(
        "SELECT {USAGE_COLUMNS} FROM usage_logs \
         WHERE user_id = $1 AND created_at >= $2 AND created_at <= $3 \
         ORDER BY created_at DESC, id DESC LIMIT $4 OFFSET $5"
    ))
    .bind(user_id)
    .bind(window.start)
    .bind(window.end)
    .bind(page_size)
    .bind(offset(page, page_size))
    .fetch_all(&state.pg)
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            return db_failure("service_usage_list", &error, "查询用量失败，请稍后重试");
        }
    };

    ok(UsagePage {
        page,
        page_size,
        total,
        rows: rows
            .iter()
            .map(|row| UsageRowView::of(row, state.cfg.service.effective_currency()))
            .collect(),
    })
}

/// `GET /api/service/users/{user_id}/usage/by-idempotency-key/{key}`。
///
/// `key` 里的 `:` 等字符由调用方百分号编码，axum 解出来的是原值，直接与
/// `usage_logs.idempotency_key` 比较（契约 §3.5）。
pub async fn by_idempotency_key(
    State(state): State<PanelState>,
    _caller: ServiceCaller,
    Path((user_id, key)): Path<(String, String)>,
) -> Response {
    let Some(user_id) = parse_id(&user_id) else {
        return bad_request("无效的用户 ID");
    };

    match user_exists(&state.pg, user_id).await {
        Ok(true) => {}
        Ok(false) => return not_found("用户不存在"),
        Err(error) => {
            return db_failure("service_usage_lookup_user", &error, "查询用量失败，请稍后重试");
        }
    }

    let row: Result<Option<UsageRow>, _> = sqlx::query_as(&format!(
        "SELECT {USAGE_COLUMNS} FROM usage_logs \
         WHERE user_id = $1 AND idempotency_key = $2 \
         ORDER BY created_at DESC, id DESC LIMIT 1"
    ))
    .bind(user_id)
    .bind(key.trim())
    .fetch_optional(&state.pg)
    .await;

    match row {
        Ok(row) => ok(UsageLookup {
            found: row.is_some(),
            row: row
                .as_ref()
                .map(|row| UsageRowView::of(row, state.cfg.service.effective_currency())),
        }),
        Err(error) => db_failure("service_usage_lookup", &error, "查询用量失败，请稍后重试"),
    }
}

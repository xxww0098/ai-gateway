//! 余额流水：用户自己的分页视图 + 管理员看某个用户的近百条。
//!
//! 对应 `BalanceHistoryHandler` + `AdminUsersBalanceHistoryHandler`。
//!
//! # 累计余额在 SQL 里算，不在内存里算
//!
//! 用户视图要显示"这一笔之后余额是多少"。朴素做法是把这个用户的全部流水读进内存、
//! 累加、再切页 —— 一个重度用户有几万行，那就是每次请求几万行的分配。这里用窗口
//! 函数 `SUM(amount) OVER (ORDER BY created_at, id)` 在**全集**上算出累计值，
//! 外层再 `ORDER BY … DESC LIMIT/OFFSET` 只取一页。
//!
//! 排序键必须是 `(created_at, id)` 而不是只有 `created_at`：同一毫秒内的两笔
//! 流水顺序不定，累计值会在两次请求间跳变。

use axum::extract::{Path, Query, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;

use gw_infra::Db;

use crate::identity::{bad_request, db_failure};
use crate::paging::{Page, offset, page_params, parse_id};
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 用户流水的默认页大小（既有实现里 query 参数名是 `page_size`，默认 20）。
const USER_HISTORY_DEFAULT_PAGE_SIZE: i64 = 20;

/// 管理员视图一次最多回多少条（既有实现固定 100 条），且**没有分页**。
const ADMIN_HISTORY_LIMIT: i64 = 100;

/// 对应 `balanceHistoryItem`。
///
/// 有三对字段是**同一个值的两个名字**（`type`/`kind`、`reference`/`note`），
/// 因为前端不同页面读的键不一样。别去重，去掉哪个都会让某个页面空掉。
/// `operator_email` 恒为空串（旧实现从没填过它）。
#[derive(Debug, Serialize)]
pub struct BalanceHistoryItem {
    pub id: i64,
    pub user_id: i64,
    pub amount: f64,
    #[serde(rename = "type")]
    pub kind_type: String,
    pub kind: String,
    pub reference: String,
    pub note: String,
    pub balance_before: f64,
    pub balance_after: f64,
    pub operator_email: String,
    pub created_at: DateTime<Utc>,
}

/// 管理员视图的一行。**键名与用户视图不同**（没有 `type` / `reference`，
/// `operator_email` 是 `null` 而不是空串，`balance_*` 恒为 0），照抄旧实现里键恒在、值可为 `null` 的形状。
#[derive(Debug, Serialize)]
pub struct AdminBalanceHistoryItem {
    pub id: i64,
    pub kind: String,
    pub amount: f64,
    pub balance_before: f64,
    pub balance_after: f64,
    pub operator_email: Option<String>,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct RunningBalanceRow {
    id: i64,
    user_id: i64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    amount: f64,
    #[sqlx(rename = "type", try_from = "gw_model::compat::Text")]
    kind: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    reference: String,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
    #[sqlx(try_from = "gw_model::compat::Money")]
    balance_after: f64,
}

/// `GET /user/balance-history` —— 我的流水，带累计余额。
///
/// Ports `BalanceHistoryHandler`。`?kind=` 过滤的是 `balance_logs.type` 列
/// （查询参数叫 kind、列叫 type，两边名字不一样，照抄）。
pub async fn history_own(
    State(state): State<PanelState>,
    user: AuthUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (page, page_size) = page_params(
        params.get("page").map(String::as_str),
        params.get("page_size").map(String::as_str),
        USER_HISTORY_DEFAULT_PAGE_SIZE,
    );
    let kind = params.get("kind").map(|s| s.trim()).unwrap_or_default();

    match history_page(&state.pg, user.user_id, kind, page, page_size).await {
        Ok((items, total)) => ok(Page::new(items, page, page_size, total)),
        Err(error) => db_failure("list_balance_logs", &error, "获取余额流水失败，请稍后重试"),
    }
}

/// 取一页余额流水，并把累计余额一并算好。
///
/// 参数收窄到 `&Db`（不是 `&PanelState`）：累计余额的正确性只跟 SQL 有关，
/// 一个只有 Postgres 的测试就该能把"翻到第二页时累计值仍然对得上"验完 ——
/// 这正是既有连库测 `TestBalanceHistoryRunningBalancePaginated` 盯的性质。
///
/// `kind` 为空串表示不按类型过滤。返回 `(这一页, 过滤后的总条数)`。
///
/// # Errors
/// 计数或分页查询失败。
pub async fn history_page(
    pg: &Db,
    user_id: i64,
    kind: &str,
    page: i64,
    page_size: i64,
) -> Result<(Vec<BalanceHistoryItem>, i64), sqlx::Error> {
    let filter = "user_id = $1 AND ($2 = '' OR type = $2)";

    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*)::bigint FROM balance_logs WHERE {filter}"
    ))
    .bind(user_id)
    .bind(kind)
    .fetch_one(pg)
    .await?;

    // 窗口函数在**过滤后的全集**上按时间正序累加，外层再倒序取一页。
    // 排序键必须是 (created_at, id)：只用 created_at 的话，同一毫秒内的两笔顺序
    // 不定，累计值会在两次请求之间跳变。
    let rows: Vec<RunningBalanceRow> = sqlx::query_as(&format!(
        "SELECT id, user_id, amount, type, reference, created_at, \
             SUM(amount) OVER (ORDER BY created_at ASC, id ASC) AS balance_after \
         FROM balance_logs WHERE {filter} \
         ORDER BY created_at DESC, id DESC LIMIT $3 OFFSET $4"
    ))
    .bind(user_id)
    .bind(kind)
    .bind(page_size)
    .bind(offset(page, page_size))
    .fetch_all(pg)
    .await?;

    Ok((
        rows.into_iter()
            .map(|row| BalanceHistoryItem {
                id: row.id,
                user_id: row.user_id,
                amount: row.amount,
                kind_type: row.kind.clone(),
                kind: row.kind,
                reference: row.reference.clone(),
                note: row.reference,
                // 「这笔之前」= 「这笔之后」减去这笔本身。
                balance_before: row.balance_after - row.amount,
                balance_after: row.balance_after,
                operator_email: String::new(),
                created_at: row.created_at,
            })
            .collect(),
        total,
    ))
}

/// 管理员视图的一行原始数据。命名而不是用五元组，纯粹为了让查询读得懂。
#[derive(Debug, sqlx::FromRow)]
struct AdminHistoryRow {
    id: i64,
    #[sqlx(rename = "type", try_from = "gw_model::compat::Text")]
    kind: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    amount: f64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    reference: String,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
}

/// `GET /admin/users/{id}/balance-history` —— 管理员看某个用户最近 100 条。
///
/// Ports `AdminUsersBalanceHistoryHandler`。信封是 `{"entries": [...]}`
/// —— **不是** `items`，也没有分页字段。
pub async fn admin_history(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    let Some(user_id) = parse_id(&id) else {
        return bad_request("无效的用户 ID");
    };

    let rows: Result<Vec<AdminHistoryRow>, _> = sqlx::query_as(
        "SELECT id, type, amount, reference, created_at FROM balance_logs \
         WHERE user_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(ADMIN_HISTORY_LIMIT)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(serde_json::json!({
            "entries": rows
                .into_iter()
                .map(|row| AdminBalanceHistoryItem {
                    id: row.id,
                    kind: row.kind,
                    amount: row.amount,
                    // 旧实现这里就是写死的 0 / null：管理员视图不算累计余额。
                    balance_before: 0.0,
                    balance_after: 0.0,
                    operator_email: None,
                    note: row.reference,
                    created_at: row.created_at,
                })
                .collect::<Vec<_>>(),
        })),
        Err(error) => db_failure(
            "admin_list_balance_logs",
            &error,
            "获取余额流水失败，请稍后重试",
        ),
    }
}

//! 退款：用户申请与查询、管理员列表与审批。
//!
//! 对应 `UserRefund*` / `AdminListRefunds` / `processRefund`。
//!
//! # 审批只写处置结果，**不动余额**
//!
//! 既有模型的注释写得很清楚：`refunds` 行记录的是"这次申请被批了还是被拒了"，
//! 实际打款走线下。移植时最容易好心办坏事的地方就是在 approve 分支里顺手加一笔
//! `Credit` —— 那会让每一次审批都凭空多出一笔钱。
//!
//! # 一次申请只能有一个处置
//!
//! `UPDATE … WHERE status = 'pending'` 是那把锁：并发或重复审批里只有一个能改到
//! 行，其余得到 `rows_affected == 0` → **409**。

use axum::extract::{Path, Query, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use gw_infra::Db;

use crate::identity::{bad_request, conflict, db_failure, not_found, parse_json_body};
use crate::paging::{ListPage, offset, page_params, parse_id};
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 管理员退款列表的默认页大小（既有实现默认 15）。
const ADMIN_REFUNDS_DEFAULT_PAGE_SIZE: i64 = 15;

const STATUS_PENDING: &str = "pending";
const STATUS_APPROVED: &str = "approved";
const STATUS_REJECTED: &str = "rejected";

/// 对应 `refundRecord`。
///
/// `days_used` / `total_days` / `daily_rate` 是按比例退款的计算依据，申请时全是 0
/// —— 旧实现的 `UserRefundApplyHandler` 只写 `user_id` / `subscription_id` /
/// `reason` / `status`，其余留给人工填。别在移植时"顺手算一下"。
#[derive(Debug, Serialize)]
pub struct RefundRecord {
    pub id: i64,
    pub user_id: i64,
    pub subscription_id: i64,
    pub amount: f64,
    pub reason: String,
    pub status: String,
    pub days_used: i64,
    pub total_days: i64,
    pub daily_rate: f64,
    pub processed_at: Option<DateTime<Utc>>,
    pub processed_by: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct RefundRow {
    id: i64,
    user_id: i64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    subscription_id: i64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    amount: f64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    reason: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    #[sqlx(try_from = "gw_model::compat::Int")]
    days_used: i64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    total_days: i64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    daily_rate: f64,
    processed_at: Option<DateTime<Utc>>,
    processed_by: Option<i64>,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
}

const REFUND_COLUMNS: &str = "id, user_id, subscription_id, amount, reason, status, days_used, \
     total_days, daily_rate, processed_at, processed_by, created_at";

impl From<RefundRow> for RefundRecord {
    fn from(row: RefundRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            subscription_id: row.subscription_id,
            amount: row.amount,
            reason: row.reason,
            status: row.status,
            days_used: row.days_used,
            total_days: row.total_days,
            daily_rate: row.daily_rate,
            processed_at: row.processed_at,
            processed_by: row.processed_by,
            created_at: row.created_at,
        }
    }
}

/// 对应 `applyRefundRequest`。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ApplyRequest {
    subscription_id: i64,
    reason: String,
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// `GET /refund/list` —— 我的退款申请。
///
/// Ports `UserRefundListHandler`。信封是 `{"items": [...]}`，**没有分页字段**。
pub async fn list_own(State(state): State<PanelState>, user: AuthUser) -> Response {
    let rows: Result<Vec<RefundRow>, _> = sqlx::query_as(&format!(
        "SELECT {REFUND_COLUMNS} FROM refunds WHERE user_id = $1 ORDER BY id DESC"
    ))
    .bind(user.user_id)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(serde_json::json!({
            "items": rows.into_iter().map(RefundRecord::from).collect::<Vec<_>>(),
        })),
        Err(error) => db_failure("list_refunds", &error, "获取退款失败，请稍后重试"),
    }
}

/// `POST /refund/apply` —— 提交一份申请。
///
/// Ports `UserRefundApplyHandler`。先确认这份订阅**确实属于调用者**（否则任何人
/// 都能对别人的订阅提申请），再写一行 `pending`。
pub async fn apply(
    State(state): State<PanelState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> Response {
    let req: ApplyRequest = match parse_json_body(&body, "退款申请无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if req.subscription_id == 0 {
        return bad_request("退款申请无效");
    }

    let owned: Result<Option<i64>, _> =
        sqlx::query_scalar("SELECT id FROM subscriptions WHERE id = $1 AND user_id = $2 LIMIT 1")
            .bind(req.subscription_id)
            .bind(user.user_id)
            .fetch_optional(&state.pg)
            .await;
    match owned {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("未找到该订阅"),
        Err(error) => {
            return db_failure(
                "refund_load_subscription",
                &error,
                "查询订阅失败，请稍后重试",
            );
        }
    }

    let created: Result<(i64, String), _> = sqlx::query_as(
        "INSERT INTO refunds \
             (user_id, subscription_id, amount, reason, status, days_used, total_days, \
              daily_rate, created_at) \
         VALUES ($1, $2, 0, $3, $4, 0, 0, 0, $5) RETURNING id, status",
    )
    .bind(user.user_id)
    .bind(req.subscription_id)
    .bind(req.reason.trim())
    .bind(STATUS_PENDING)
    .bind(Utc::now())
    .fetch_one(&state.pg)
    .await;

    match created {
        Ok((id, status)) => ok(serde_json::json!({ "id": id, "status": status })),
        Err(error) => db_failure("create_refund", &error, "创建退款申请失败，请稍后重试"),
    }
}

/// `GET /admin/refunds` —— 分页 + 状态过滤。Ports `AdminListRefundsHandler`。
pub async fn admin_list(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (page, page_size) = page_params(
        params.get("page").map(String::as_str),
        params.get("page_size").map(String::as_str),
        ADMIN_REFUNDS_DEFAULT_PAGE_SIZE,
    );
    let status = params.get("status").map(|s| s.trim()).unwrap_or_default();
    let filter = "($1 = '' OR status = $1)";

    let total: Result<i64, _> = sqlx::query_scalar(&format!(
        "SELECT COUNT(*)::bigint FROM refunds WHERE {filter}"
    ))
    .bind(status)
    .fetch_one(&state.pg)
    .await;
    let total = match total {
        Ok(total) => total,
        Err(error) => return db_failure("count_refunds", &error, "统计退款失败，请稍后重试"),
    };

    let rows: Result<Vec<RefundRow>, _> = sqlx::query_as(&format!(
        "SELECT {REFUND_COLUMNS} FROM refunds WHERE {filter} ORDER BY id DESC LIMIT $2 OFFSET $3"
    ))
    .bind(status)
    .bind(page_size)
    .bind(offset(page, page_size))
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(ListPage::new(
            rows.into_iter().map(RefundRecord::from).collect(),
            total,
            page,
            page_size,
        )),
        Err(error) => db_failure("list_refunds", &error, "获取退款失败，请稍后重试"),
    }
}

/// `PUT /admin/refund/{id}/approve`。
pub async fn admin_approve(
    State(state): State<PanelState>,
    admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    process(&state, &admin, &id, STATUS_APPROVED).await
}

/// `PUT /admin/refund/{id}/reject`。
pub async fn admin_reject(
    State(state): State<PanelState>,
    admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    process(&state, &admin, &id, STATUS_REJECTED).await
}

/// 一次审批的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum Disposition {
    /// 抢到了那份 `pending` 申请，处置已写入。
    Applied,
    /// 这张单已经有终态了（重复审批 / 并发审批的败者）。
    AlreadyProcessed,
    /// 没有这张单。
    Missing,
}

/// 给一份 `pending` 申请打上终态。Ports `processRefund` 的写入部分。
///
/// 参数收窄到 `&Db`，好让"一份申请只能有一个处置"这条不变量能被一个只需要
/// Postgres 的测试并发地撞（既有连库测 `TestRefundPersistedAndSingleDisposition` 对应这点）。
///
/// # Errors
/// 查询或更新失败。
pub async fn apply_disposition(
    pg: &Db,
    refund_id: i64,
    new_status: &str,
    processed_by: i64,
) -> Result<Disposition, sqlx::Error> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM refunds WHERE id = $1")
        .bind(refund_id)
        .fetch_optional(pg)
        .await?;
    if exists.is_none() {
        return Ok(Disposition::Missing);
    }

    let updated = sqlx::query(
        "UPDATE refunds SET status = $2, processed_at = $3, processed_by = $4 \
         WHERE id = $1 AND status = $5",
    )
    .bind(refund_id)
    .bind(new_status)
    .bind(Utc::now())
    .bind(processed_by)
    .bind(STATUS_PENDING)
    .execute(pg)
    .await?;

    if updated.rows_affected() == 0 {
        Ok(Disposition::AlreadyProcessed)
    } else {
        Ok(Disposition::Applied)
    }
}

/// handler 侧：处置 + 回读最新一行。
async fn process(
    state: &PanelState,
    admin: &AdminUser,
    raw_id: &str,
    new_status: &str,
) -> Response {
    let Some(id) = parse_id(raw_id) else {
        return bad_request("无效的 ID");
    };

    // 「这张单不存在」（404）与「已经处理过」（409）对管理员是两条不同的信息。
    match apply_disposition(&state.pg, id, new_status, admin.0.user_id).await {
        Ok(Disposition::Missing) => return not_found("未找到该退款"),
        Ok(Disposition::AlreadyProcessed) => return conflict("该退款已处理"),
        Ok(Disposition::Applied) => {}
        Err(error) => return db_failure("update_refund", &error, "更新退款失败，请稍后重试"),
    }

    let reloaded: Result<Option<RefundRow>, _> = sqlx::query_as(&format!(
        "SELECT {REFUND_COLUMNS} FROM refunds WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&state.pg)
    .await;

    match reloaded {
        Ok(Some(row)) => ok(RefundRecord::from(row)),
        Ok(None) => not_found("未找到该退款"),
        Err(error) => db_failure("reload_refund", &error, "重新加载退款失败，请稍后重试"),
    }
}

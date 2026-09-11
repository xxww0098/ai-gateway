//! 欠款：管理员列表与零额注销。
//!
//! # 为什么必须有这两个端点
//!
//! `settle` 按 `min(balance, actual)` 扣款，扣不动的部分写成一条带
//! `metadata.shortfall_usd` 的标记行；只要它没被配对的 credit 注销，
//! `has_unresolved_shortfall` 就一直为真，用户在 `/v1` 和订阅购买两处都被 402
//! `outstanding_debt` 挡死。在这两个端点之前，面板既看不到这些行，也没有任何
//! 途径写出那条配对 credit（普通充值的 reference 前缀对不上），账号只能改库救。
//!
//! # 注销不给钱
//!
//! [`gw_ledger::Ledger::write_off_shortfall`] 写的是一条 `amount = 0` 的 credit：
//! 门解开、余额一分不变、审计留痕。这是已定的产品决策 —— 公司吃掉这笔没收回来的
//! 服务成本，而不是再送用户一笔等额余额。别在这里"顺手"补一笔 `Credit`。
//!
//! # 一条欠款只能注销一次
//!
//! 幂等不靠 handler 先查后写，靠 `0008` 迁移里那个分区唯一索引：并发的第二次
//! 注销撞唯一约束，得到 [`WriteOff::AlreadyResolved`] → **409**。

use axum::extract::{Path, Query, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;

use gw_ledger::WriteOff;

use crate::identity::{bad_request, conflict, internal, not_found};
use crate::paging::{Page, offset, page_params, parse_id};
use crate::{AdminUser, PanelState, ok, ok_empty};

/// 欠款列表的默认页大小。
const SHORTFALLS_DEFAULT_PAGE_SIZE: i64 = 20;

/// 列表里的一行。`reference` 就是产生欠款的那个 request id，运维要靠它去
/// `usage_logs` 里对账，所以原样透出。
#[derive(Debug, Serialize)]
pub struct ShortfallRecord {
    pub id: i64,
    pub user_id: i64,
    pub reference: String,
    pub shortfall_usd: f64,
    pub created_at: Option<DateTime<Utc>>,
}

/// `GET /admin/shortfalls` —— 分页列出全部未清偿欠款。
pub async fn admin_list(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (page, page_size) = page_params(
        params.get("page").map(String::as_str),
        params.get("page_size").map(String::as_str),
        SHORTFALLS_DEFAULT_PAGE_SIZE,
    );

    match state
        .ledger
        .list_unresolved_shortfalls(page_size, offset(page, page_size))
        .await
    {
        Ok((rows, total)) => {
            let items: Vec<ShortfallRecord> = rows
                .into_iter()
                .map(|row| ShortfallRecord {
                    id: row.log_id,
                    user_id: row.user_id,
                    reference: row.reference,
                    shortfall_usd: row.shortfall_usd,
                    created_at: row.created_at,
                })
                .collect();
            ok(Page::new(items, page, page_size, total))
        }
        Err(error) => {
            tracing::warn!(event = "list_shortfalls_failed", error = %error);
            internal("获取欠款列表失败，请稍后重试")
        }
    }
}

/// `POST /admin/shortfalls/{log_id}/write-off` —— 零额注销一条欠款。
///
/// 调用方只能给一个 `log_id`：user_id、reference、金额全部由 SQL 从欠款行派生，
/// 伪造不了。操作者的 user id 落进 metadata 备查。
pub async fn admin_write_off(
    State(state): State<PanelState>,
    admin: AdminUser,
    Path(raw_id): Path<String>,
) -> Response {
    let Some(log_id) = parse_id(&raw_id) else {
        return bad_request("无效的 ID");
    };

    match state
        .ledger
        .write_off_shortfall(log_id, admin.0.user_id)
        .await
    {
        Ok(WriteOff::Written) => {
            tracing::info!(
                event = "shortfall_written_off",
                shortfall_log_id = log_id,
                operator_id = admin.0.user_id,
            );
            ok_empty()
        }
        Ok(WriteOff::AlreadyResolved) => conflict("该欠款已注销"),
        Ok(WriteOff::Missing) => not_found("未找到该欠款记录"),
        Err(error) => {
            tracing::warn!(
                event = "write_off_shortfall_failed",
                shortfall_log_id = log_id,
                error = %error,
            );
            internal("注销欠款失败，请稍后重试")
        }
    }
}

//! `GET /api/service/users/{user_id}/balance` —— 落库余额 + 币种标签。
//!
//! 契约 §3.3：`data` 只有两个键，`balance` 是 8 位小数的 decimal 字符串，`currency`
//! 取自 `service.currency`（默认 `USD`）。
//!
//! # 读的是 `users.balance`，不是账本可用余额
//!
//! 面板的 `/user/profile` 返回 `available_balance`（已扣掉在途预扣的账本口径）。
//! 服务面要的是**对账口径**：与 `balance_logs` 的累计、与一次 `credits` 之后的余额
//! 完全一致的那个数，所以在途 hold 不减 —— 它只是一个还没结算的预估，不是已经花掉
//! 的钱。

use axum::extract::{Path, State};
use axum::response::Response;
use serde::Serialize;

use gw_infra::Db;
use gw_model::compat::Money;

use super::{ServiceCaller, amount_str, bad_request, db_failure, not_found};
use crate::PanelState;
use crate::ok;
use crate::paging::parse_id;

/// 契约 §3.3 的 `data`。
#[derive(Debug, Serialize)]
pub struct BalanceView {
    /// 8 位小数的 decimal 字符串，如 `"12.34000000"`。
    pub balance: String,
    /// `service.currency`，只作展示与透传。
    pub currency: String,
}

/// `GET /api/service/users/{user_id}/balance`。
pub async fn show(
    State(state): State<PanelState>,
    _caller: ServiceCaller,
    Path(user_id): Path<String>,
) -> Response {
    let Some(user_id) = parse_id(&user_id) else {
        return bad_request("无效的用户 ID");
    };

    match load_balance(&state.pg, user_id).await {
        Ok(Some(balance)) => ok(BalanceView {
            balance: amount_str(balance),
            currency: state.cfg.service.effective_currency().to_owned(),
        }),
        Ok(None) => not_found("用户不存在"),
        Err(error) => db_failure("service_balance", &error, "查询余额失败，请稍后重试"),
    }
}

/// 读 `users.balance` 的持久化值；`None` = 没有这个用户。
///
/// 列是 `numeric`，sqlx 不能直接解成 `f64`（CONTRACT §3.5），所以走
/// [`Money`]，它同时也把历史 NULL 行读成 `0`。
///
/// # Errors
/// 查询失败。
pub async fn load_balance(pg: &Db, user_id: i64) -> Result<Option<f64>, sqlx::Error> {
    let row: Option<(Money,)> = sqlx::query_as("SELECT balance FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pg)
        .await?;
    Ok(row.map(|(balance,)| balance.0))
}

//! `POST /api/service/users/{user_id}/credits` —— 幂等入账。
//!
//! 契约 §3.6：`amount > 0` 且 `<= service.max_credit`；`idempotency_key` 非空且
//! `<= 128` 字符；reference 固定为 `service_credit:<idempotency_key>`；重复入账返回
//! `applied:false, duplicate:true` + 当前余额；同一 reference 换 user 或换金额是
//! `409 + 4009`，绝不静默改账。
//!
//! # 幂等由账本执行，不在这里"先查再写"
//!
//! [`Ledger::credit_tx`](gw_ledger::Ledger::credit_tx) 在用户行锁之后做去重预查，
//! 0015 的部分唯一索引是库级兜底。本端点只负责：把 reference 拼对、把账本的三类
//! 结果翻译成契约的两个成功形状与两个错误码。
//!
//! 跨 user 的并发竞态（两边预查都看不到对方、插入时才撞索引）也走同一个 409：
//! 唯一索引冲突不是"内部错误"，它正是"这笔 reference 已经属于别人"的证据。
//!
//! # `note` 只做兼容占位
//!
//! 契约的请求体带一个可省的 `note`。`credit_tx` 的入账流水没有 metadata 参数（账本
//! 只写 user / 金额 / reference / 时间），所以这里接受并忽略它 —— 不为一个展示字段
//! 绕开账本自己写 `balance_logs`。真要把备注落账，扩的是 `credit_tx` 的签名。

use axum::extract::{Path, State};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use gw_ledger::{BalanceChange, LedgerError};

use super::balance::load_balance;
use super::{
    ServiceCaller, amount_str, bad_request, conflict, db_failure, internal, not_found,
    parse_json_body,
};
use crate::PanelState;
use crate::ok;
use crate::paging::parse_id;

/// `idempotency_key` 的长度上限（契约 §3.6）。
const MAX_IDEMPOTENCY_KEY_CHARS: usize = 128;

/// 契约 §3.6 的请求体。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CreditRequest {
    /// 契约给的是 decimal 字符串；解析见 [`CreditRequest::amount`]。
    amount: Option<Value>,
    /// 幂等身份。reference = `service_credit:<idempotency_key>`。
    idempotency_key: String,
    /// 可选备注，当前不落库（见模块文档）。
    note: Option<String>,
}

impl CreditRequest {
    /// 把 `amount` 解析成 `f64`。
    ///
    /// 契约与夹具都把金额写成 decimal 字符串，这里也接受 JSON 数字：两者表达的是
    /// 同一个 decimal 值，拒绝数字只会把调用方的序列化选择变成 400。
    ///
    /// # Errors
    /// 缺字段、类型不对，或字符串不是数字。`NaN` / `inf` 这类字符串**能**解析成功，
    /// 由调用方按"不是有限正数"拒绝 —— 校验只在一个地方做。
    fn amount(&self) -> Result<f64, &'static str> {
        match self.amount.as_ref() {
            Some(Value::String(raw)) => raw.trim().parse::<f64>().map_err(|_| "amount 无效"),
            Some(Value::Number(number)) => number.as_f64().ok_or("amount 无效"),
            _ => Err("amount 必填"),
        }
    }
}

/// 契约 §3.6 的 `data`。
#[derive(Debug, Serialize)]
pub struct CreditResult {
    /// 本次是否真的入账（`false` = 命中既有 reference）。
    pub applied: bool,
    /// 是否重复入账。与 `applied` 恒为相反值，契约要求两个键都在。
    pub duplicate: bool,
    /// 入账后的余额（重复时是当前余额），8 位小数。
    pub balance: String,
}

/// `POST /api/service/users/{user_id}/credits`。
pub async fn create(
    State(state): State<PanelState>,
    _caller: ServiceCaller,
    Path(user_id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(user_id) = parse_id(&user_id) else {
        return bad_request("无效的用户 ID");
    };
    let req: CreditRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };

    let amount = match req.amount() {
        Ok(amount) => amount,
        Err(message) => return bad_request(message),
    };
    // `!(amount > 0.0)` 式判断才挡得住 NaN —— 见 identity::is_positive_amount 的注释。
    if !amount.is_finite() || amount <= 0.0 {
        return bad_request("amount 必须是大于 0 的有限数");
    }
    let max_credit = state.cfg.service.effective_max_credit();
    if amount > max_credit {
        return bad_request(&format!("amount 超过单次入账上限 {max_credit}"));
    }

    let idempotency_key = req.idempotency_key.trim();
    if idempotency_key.is_empty() || idempotency_key.chars().count() > MAX_IDEMPOTENCY_KEY_CHARS
    {
        return bad_request("idempotency_key 必填且不超过 128 字符");
    }
    let reference = gw_ledger::service_credit_reference(idempotency_key);

    let mut tx = match state.pg.begin().await {
        Ok(tx) => tx,
        Err(error) => return db_failure("service_credit_begin", &error, "入账失败，请稍后重试"),
    };

    match state
        .ledger
        .credit_tx(&mut tx, user_id, amount, &reference)
        .await
    {
        Ok(BalanceChange::Applied {
            balance_after,
            balance_version,
        }) => {
            if let Err(error) = tx.commit().await {
                return db_failure("service_credit_commit", &error, "入账失败，请稍后重试");
            }
            let _ = state
                .ledger
                .publish_balance(user_id, balance_after, balance_version)
                .await;
            ok(CreditResult {
                applied: true,
                duplicate: false,
                balance: amount_str(balance_after),
            })
        }
        Ok(BalanceChange::AlreadyApplied) => {
            // 什么都没写，但行锁还攥在这个事务里 —— 先回滚再读余额。
            let _ = tx.rollback().await;
            match load_balance(&state.pg, user_id).await {
                Ok(Some(balance)) => ok(CreditResult {
                    applied: false,
                    duplicate: true,
                    balance: amount_str(balance),
                }),
                Ok(None) => not_found("用户不存在"),
                Err(error) => {
                    db_failure("service_credit_balance", &error, "入账失败，请稍后重试")
                }
            }
        }
        Err(LedgerError::UserNotFound) => {
            let _ = tx.rollback().await;
            not_found("用户不存在")
        }
        Err(LedgerError::InvalidArgument(_)) => {
            let _ = tx.rollback().await;
            conflict("该 idempotency_key 已用于另一笔入账（user 或金额不一致）")
        }
        Err(LedgerError::Db(error)) => {
            let _ = tx.rollback().await;
            if error
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
            {
                // 跨 user 并发：双方预查都在对方提交之前，插入时才撞 0015 的索引。
                return conflict("该 idempotency_key 已用于另一笔入账（user 或金额不一致）");
            }
            db_failure("service_credit_failed", &error, "入账失败，请稍后重试")
        }
        Err(error) => {
            let _ = tx.rollback().await;
            tracing::warn!(event = "service_credit_failed", %error);
            internal("入账失败，请稍后重试")
        }
    }
}

//! 三种「钱进来 / 钱出去」的单据。
//!
//! 三者的共同点是**状态机 + 条件 UPDATE 保证幂等**：充值单 pending→paid 只有一次
//! 能成功，兑换码 unused→used 只有一个人能抢到，退款 pending→approved/rejected 只
//! 记录处置结果、本身不动余额。放在一个文件里是因为它们在业务上是一族
//! （规范 1.6：删掉"商务单据"应该等于删掉一个文件，而不是散在三处）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::compat;

/// `payment_orders` 的实体，第三方支付充值单。
///
/// `status` 只有 pending / paid / failed 三个值；credit 用户余额的唯一入口是
/// pending→paid 那一次条件 UPDATE，重复回调不会重复入账。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaymentOrder {
    pub id: Id,
    pub user_id: Id,
    /// stripe | alipay | wechat
    pub provider: String,
    #[sqlx(try_from = "compat::Money")]
    pub amount_usd: f64,
    #[sqlx(try_from = "compat::Money")]
    pub amount_local: f64,
    #[sqlx(try_from = "compat::Text")]
    pub currency: String,
    /// pending | paid | failed
    pub status: String,
    pub transaction_id: Option<String>,
    /// 历史实现是 `*string` + `type:text`（不是 jsonb），保持 `Option<String>`。
    pub metadata: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

/// `redeem_codes` 的实体，管理员生成、用户兑换的预付码。
///
/// `status` unused | used。兑换靠 `UPDATE … WHERE status='unused'` 的条件更新，
/// 并发下只有一个人能赢，与实例数无关。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RedeemCode {
    pub id: Id,
    pub code: String,
    #[sqlx(try_from = "compat::Money")]
    pub amount: f64,
    pub status: String,
    pub used_by_id: Option<Id>,
    pub used_by: Option<String>,
    pub used_at: Option<DateTime<Utc>>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
}

/// `refunds` 的实体，订阅退款申请。
///
/// 审批只写处置结果（`status` / `processed_*`），**不移动账户余额**，
/// 别在这里顺手加扣款语义。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Refund {
    pub id: Id,
    pub user_id: Id,
    #[sqlx(try_from = "compat::Int")]
    pub subscription_id: i64,
    #[sqlx(try_from = "compat::Money")]
    pub amount: f64,
    #[sqlx(try_from = "compat::Text")]
    pub reason: String,
    /// pending | approved | rejected
    pub status: String,
    #[sqlx(try_from = "compat::Int")]
    pub days_used: i64,
    #[sqlx(try_from = "compat::Int")]
    pub total_days: i64,
    #[sqlx(try_from = "compat::Money")]
    pub daily_rate: f64,
    pub processed_at: Option<DateTime<Utc>>,
    pub processed_by: Option<Id>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
}

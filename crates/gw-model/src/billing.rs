//! 三条日志流：余额变动、面板操作、代理用量。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Id;
use crate::compat;

/// `balance_logs` 的实体，账本的唯一真相。
///
/// `metadata` 是 jsonb，且带 GIN 索引（`idx_balance_logs_metadata`）：欠款核销靠
/// 在里面查 `shortfall_usd`，别把它换成 text。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BalanceLog {
    pub id: Id,
    pub user_id: Id,
    #[sqlx(try_from = "compat::Money")]
    pub amount: f64,
    /// 列名就是 `type`。Rust 侧字段名避开关键字，靠 `rename` 对上列。
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub kind: String,
    /// 外部引用 id，例如 `shortfall_resolve:<requestID>:<debitLogID>`。
    #[sqlx(try_from = "compat::Text")]
    pub reference: String,
    pub metadata: Option<Value>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
}

/// `operation_logs` 的实体，面板侧操作审计。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OperationLog {
    pub id: Id,
    pub source: String,
    /// 未登录时为 0（注册 / 登录失败），不是 NULL。
    #[sqlx(try_from = "compat::Int")]
    pub actor_id: i64,
    #[sqlx(try_from = "compat::Text")]
    pub actor_email: String,
    #[sqlx(try_from = "compat::Text")]
    pub actor_role: String,
    pub action: String,
    #[sqlx(try_from = "compat::Text")]
    pub target: String,
    #[sqlx(try_from = "compat::Text")]
    pub method: String,
    #[sqlx(try_from = "compat::Text")]
    pub path: String,
    #[sqlx(try_from = "compat::Int")]
    pub status_code: i64,
    #[sqlx(try_from = "compat::Text")]
    pub ip_address: String,
    #[sqlx(try_from = "compat::Text")]
    pub request_id: String,
    pub metadata: Option<Value>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    /// 行内容的 keyed HMAC，用来让审计链可验伪。**这一列是后加的**，老库里在它
    /// 之前写入的行是 NULL —— 所以走 `compat::Text`（NULL → `""`）。
    #[sqlx(try_from = "compat::Text")]
    pub entry_hash: String,
}

/// `usage_logs` 的实体，每一次 `/v1/*` 请求一行。
///
/// 四列 token（input/output/reasoning/cached）与四列成本一一对应；`tokens_in` /
/// `tokens_out` 是更早的兼容列，仍在写。`raw_metadata` 里放 `billing_fallback`、
/// `shortfall_usd` 这类计费旁证。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UsageLog {
    pub id: Id,
    pub user_id: Id,
    pub api_key_id: Id,
    pub group_id: Option<Id>,
    #[sqlx(try_from = "compat::Text")]
    pub request_id: String,
    #[sqlx(try_from = "compat::Text")]
    pub idempotency_key: String,
    #[sqlx(try_from = "compat::Text")]
    pub event_key: String,
    #[sqlx(try_from = "compat::Text")]
    pub model: String,
    #[sqlx(try_from = "compat::Text")]
    pub provider: String,
    #[sqlx(try_from = "compat::Text")]
    pub auth_id: String,
    #[sqlx(try_from = "compat::Int")]
    pub tokens_in: i64,
    #[sqlx(try_from = "compat::Int")]
    pub tokens_out: i64,
    #[sqlx(try_from = "compat::Int")]
    pub input_tokens: i64,
    #[sqlx(try_from = "compat::Int")]
    pub output_tokens: i64,
    #[sqlx(try_from = "compat::Int")]
    pub reasoning_tokens: i64,
    #[sqlx(try_from = "compat::Int")]
    pub cached_tokens: i64,
    #[sqlx(try_from = "compat::Money")]
    pub input_cost: f64,
    #[sqlx(try_from = "compat::Money")]
    pub output_cost: f64,
    #[sqlx(try_from = "compat::Money")]
    pub total_cost: f64,
    #[sqlx(try_from = "compat::Money")]
    pub actual_cost: f64,
    #[sqlx(try_from = "compat::Money")]
    pub cost: f64,
    #[sqlx(try_from = "compat::Money")]
    pub rate_multiplier: f64,
    #[sqlx(try_from = "compat::Bool")]
    pub stream: bool,
    #[sqlx(try_from = "compat::Int")]
    pub duration_ms: i64,
    #[sqlx(try_from = "compat::Text")]
    pub ip_address: String,
    pub raw_metadata: Option<Value>,
    #[sqlx(try_from = "compat::Bool")]
    pub failed: bool,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
}

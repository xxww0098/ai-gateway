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

/// `billing_operations` 的实体 —— 一次计费操作的持久状态。
///
/// 这是**非终态操作的唯一真相**：对账扫的是这张表，不是 Redis 的 TTL。
/// Redis 里的 hold 只是预留缓存，掉了就重建，掉了也不改变这一行说的话。
///
/// `billing_operation_id` 由服务端生成，与观测用的 `client_trace_id`
/// （入站 `X-Trace-ID`）是**两个不同的东西**：前者是钱的键，客户端碰不到；
/// 后者只进日志与响应头。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BillingOperation {
    pub id: Id,
    #[sqlx(try_from = "compat::Text")]
    pub billing_operation_id: String,
    pub user_id: Id,
    /// `held` | `settled` | `released`。状态机在 `gw-ledger`，这里只存字面量。
    #[sqlx(try_from = "compat::Text")]
    pub state: String,
    /// 实际预留住的上限。预付模式下等于 `admitted_liability`。
    #[sqlx(try_from = "compat::Money")]
    pub reserved_amount: f64,
    /// 准入时认可的责任上限。
    #[sqlx(try_from = "compat::Money")]
    pub admitted_liability: f64,
    /// 请求指纹。同 id 不同指纹再预扣 = 冲突。
    #[sqlx(try_from = "compat::Text")]
    pub request_fingerprint: String,
    /// 观测用，不参与任何判定。
    #[sqlx(try_from = "compat::Text")]
    pub client_trace_id: String,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
    /// 进入终态的时刻。`None` = 仍然非终态，对账要扫它。
    pub terminal_at: Option<DateTime<Utc>>,
}

//! 上游凭证与 SDK 管理相关的四张表。
//!
//! 这四张表是网关鉴权与 SDK 管理的持久化面：`auth_records` 是
//! 上游凭证的登记行，`gw-authcore` 的 `AuthStore` 就落在它上面。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Id;
use crate::compat;

/// `auth_records` 的实体（表名被显式钉成 `auth_records`，
/// 不走复数化推导）。
///
/// 主键是 `id`（`varchar(128)`）而不是自增整数：id 由凭证本身决定。
/// 运行时字段（Index / FileName / Storage / Runtime / 计数器）**不落库**。
///
/// 五个 jsonb 列原来对应 `json.RawMessage`：nil 就是 SQL NULL，所以是
/// `Option<Value>`。其中 `metadata` 存的是凭证明文的加密载荷（AES-GCM，见
/// `gw-authcore`），别当普通 JSON 直接回显给前端。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuthRecord {
    pub id: String,
    pub provider: String,
    #[sqlx(try_from = "compat::Text")]
    pub prefix: String,
    #[sqlx(try_from = "compat::Text")]
    pub label: String,
    #[sqlx(try_from = "compat::Text")]
    pub status: String,
    #[sqlx(try_from = "compat::Text")]
    pub status_message: String,
    pub disabled: bool,
    pub unavailable: bool,
    #[sqlx(try_from = "compat::Text")]
    pub proxy_url: String,
    pub attributes: Option<Value>,
    pub metadata: Option<Value>,
    pub quota: Option<Value>,
    pub model_states: Option<Value>,
    pub last_error: Option<Value>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
    /// 历史实现是非指针 `time.Time`，从没刷新过时是零值而不是 NULL。
    #[sqlx(try_from = "compat::Ts")]
    pub last_refreshed_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub next_refresh_after: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub next_retry_after: DateTime<Utc>,
}

/// `provider_configs` 的实体，按 provider 名唯一的 JSON 配置块。
///
/// 启动时的种子会写一行 `provider = "sdk_config"`（见 [`crate::seed`]）。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderConfig {
    pub id: Id,
    pub provider: String,
    pub config_data: Option<Value>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

/// `o_auth_sessions` 的实体。
///
/// ⚠️ 表名是 `o_auth_sessions`：历史建库的命名策略把 `OAuthSession` 断成
/// `o_auth_session` 再复数化。不是 `oauth_sessions`。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OAuthSession {
    pub id: Id,
    pub provider: String,
    pub state: String,
    #[sqlx(try_from = "compat::Text")]
    pub auth_url: String,
    #[sqlx(try_from = "compat::Text")]
    pub status: String,
    pub auth_id: Option<String>,
    pub config_data: Option<Value>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// `ampcode_configs` 的实体，单行（id = 1）配置块。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AmpcodeConfig {
    pub id: Id,
    pub config_data: Option<Value>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

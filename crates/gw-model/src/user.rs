//! 用户、API Key、分组与 token 版本号。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::compat;

/// `users` 的实体。
///
/// 注意 `email` / `password_hash` 是 `text` 而不是 `varchar(255)`：历史建库的 tag
/// 写的是 `size=255`（等号），建表脚本解析不到，于是建成了 `text`。这里保持一致。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: Id,
    pub email: String,
    pub password_hash: String,
    #[sqlx(try_from = "compat::Text")]
    pub role: String,
    #[sqlx(try_from = "compat::Text")]
    pub username: String,
    #[sqlx(try_from = "compat::Money")]
    pub balance: f64,
    #[sqlx(try_from = "compat::Text")]
    pub status: String,
    #[sqlx(try_from = "compat::Int")]
    pub concurrency: i64,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

/// `api_keys` 的实体。表名是 `api_keys`、外键列是 `api_key_id`
/// （命名规则把 `ApiKeyID` 拆成 `api_key_id`，不是 `apikey_id`）。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ApiKey {
    pub id: Id,
    pub user_id: Id,
    pub key_hash: String,
    pub key_prefix: String,
    #[sqlx(try_from = "compat::Text")]
    pub name: String,
    #[sqlx(try_from = "compat::Text")]
    pub status: String,
    pub group_id: Option<Id>,
    pub last_used_at: Option<DateTime<Utc>>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

/// `groups` 的实体。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Group {
    pub id: Id,
    pub name: String,
    #[sqlx(try_from = "compat::Money")]
    pub rate_multiplier: f64,
    #[sqlx(try_from = "compat::Money")]
    pub quota_limit: f64,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

/// `user_token_versions` 的实体。
///
/// 主键是 `user_id`（没有独立 `id` 列）。缺行等价于 version 0，这是「从未吊销过
/// token 的用户不受影响」的前提，别给它加 NOT NULL 之外的语义。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserTokenVersion {
    pub user_id: Id,
    pub version: i64,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

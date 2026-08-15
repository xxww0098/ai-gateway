//! `channel_policies` —— 上游账号（SDK auth）的负载均衡策略。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::compat;

/// `channel_policies` 的实体，按 `auth_id` 唯一。
///
/// **缺行 = 默认值**（weight 1 / priority 0 / enabled）。读取方必须把「查不到」
/// 当默认处理，而不是当错误 —— 大多数上游账号根本没有这一行。
///
/// * `weight`：同优先级内的相对流量份额，越大分到越多。
/// * `priority`：越高越优先；低优先级是备份，只有高优先级全不健康时才用。
/// * `enabled`：临时下线一个账号而不用删凭证。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChannelPolicy {
    pub id: Id,
    pub auth_id: String,
    #[sqlx(try_from = "compat::Int")]
    pub weight: i64,
    #[sqlx(try_from = "compat::Int")]
    pub priority: i64,
    #[sqlx(try_from = "compat::Bool")]
    pub enabled: bool,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

impl ChannelPolicy {
    /// 某个 `auth_id` 没有策略行时的默认策略 —— 「缺行即默认」
    /// （weight 1, priority 0, enabled）。
    pub fn default_for(auth_id: impl Into<String>) -> Self {
        Self {
            id: 0,
            auth_id: auth_id.into(),
            weight: 1,
            priority: 0,
            enabled: true,
            created_at: compat::zero_time(),
            updated_at: compat::zero_time(),
        }
    }
}

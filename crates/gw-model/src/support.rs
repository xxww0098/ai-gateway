//! 工单与公告，两条面向用户的支持通道。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::compat;

/// `tickets` 的实体。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Ticket {
    pub id: Id,
    pub user_id: Id,
    pub title: String,
    #[sqlx(try_from = "compat::Text")]
    pub category: String,
    #[sqlx(try_from = "compat::Text")]
    pub priority: String,
    #[sqlx(try_from = "compat::Text")]
    pub status: String,
    pub assignee_id: Option<Id>,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

/// `ticket_replies` 的实体。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TicketReply {
    pub id: Id,
    pub ticket_id: Id,
    pub user_id: Id,
    #[sqlx(try_from = "compat::Bool")]
    pub is_admin: bool,
    pub content: String,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
}

/// `announcements` 的实体，运营发布、用户仪表盘可见的公告。
///
/// 落库（而不是留在进程内存）是有意的：管理员建的公告要能跨重启、跨副本可见。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Announcement {
    pub id: Id,
    pub title: String,
    #[sqlx(try_from = "compat::Text")]
    pub content: String,
    /// info | warning | …
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub kind: String,
    pub is_active: bool,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
}

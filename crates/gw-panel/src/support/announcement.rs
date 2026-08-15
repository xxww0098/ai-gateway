//! 公告：用户仪表盘上的只读列表 + 管理员 CRUD。
//!
//! 对应公告相关的 `Admin*AnnouncementHandler` + `UserAnnouncementsHandler`。
//!
//! # 两个视图的字段数刻意不同
//!
//! 用户侧只给 `{id, title, type}` —— 仪表盘上是一行标题，正文不在那里展开；
//! 管理员侧给全字段。把用户视图"补全"成管理员视图会让每次进仪表盘都拖着
//! 全部公告正文。

use axum::extract::{Path, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::identity::{bad_request, db_failure, not_found, parse_json_body};
use crate::paging::parse_id;
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 没写类型时的默认值。对应 `if annType == "" { annType = "info" }`。
const DEFAULT_TYPE: &str = "info";

/// 对应 `announcementRecord`（管理员视图）。
#[derive(Debug, Serialize)]
pub struct AnnouncementRecord {
    pub id: i64,
    pub title: String,
    pub content: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// 用户仪表盘上的一行。原实现直接写的 `gin.H{"id":…, "title":…, "type":…}`。
#[derive(Debug, Serialize)]
pub struct AnnouncementDigest {
    pub id: i64,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AnnouncementRow {
    id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    title: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    content: String,
    #[sqlx(rename = "type", try_from = "gw_model::compat::Text")]
    kind: String,
    #[sqlx(try_from = "gw_model::compat::Bool")]
    is_active: bool,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
}

const ANNOUNCEMENT_COLUMNS: &str = "id, title, content, type, is_active, created_at";

impl From<AnnouncementRow> for AnnouncementRecord {
    fn from(row: AnnouncementRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            content: row.content,
            kind: row.kind,
            is_active: row.is_active,
            created_at: row.created_at,
        }
    }
}

/// 对应 `createAnnouncementRequest`（新建与更新共用）。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SaveRequest {
    title: String,
    content: String,
    #[serde(rename = "type")]
    kind: String,
    is_active: bool,
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// `GET /user/announcements` —— 仪表盘上的活跃公告，**裸数组**。
///
/// 对应 `UserAnnouncementsHandler`。落库而不是写死在代码里是有意的：运营发的
/// 公告要能跨重启、跨副本可见。
pub async fn list_active(State(state): State<PanelState>, _user: AuthUser) -> Response {
    let rows: Result<Vec<(i64, String, String)>, _> = sqlx::query_as(
        "SELECT id, COALESCE(title,''), COALESCE(type,'') FROM announcements \
         WHERE is_active = TRUE ORDER BY id DESC",
    )
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(rows
            .into_iter()
            .map(|(id, title, kind)| AnnouncementDigest { id, title, kind })
            .collect::<Vec<_>>()),
        Err(error) => db_failure("list_announcements", &error, "获取公告失败，请稍后重试"),
    }
}

/// `GET /admin/announcements` —— 全部公告（含停用的），`{"items": [...]}`。
pub async fn admin_list(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    let rows: Result<Vec<AnnouncementRow>, _> = sqlx::query_as(&format!(
        "SELECT {ANNOUNCEMENT_COLUMNS} FROM announcements ORDER BY id DESC"
    ))
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(serde_json::json!({
            "items": rows.into_iter().map(AnnouncementRecord::from).collect::<Vec<_>>(),
        })),
        Err(error) => db_failure(
            "admin_list_announcements",
            &error,
            "获取公告失败，请稍后重试",
        ),
    }
}

/// `POST /admin/announcements` —— 新建。
///
/// 对应 `AdminCreateAnnouncementHandler`：标题与正文**都必须非空**，类型缺省
/// 为 `info`。
pub async fn admin_create(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: axum::body::Bytes,
) -> Response {
    let req: SaveRequest = match parse_json_body(&body, "公告内容无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    let title = req.title.trim();
    let content = req.content.trim();
    if title.is_empty() || content.is_empty() {
        return bad_request("公告内容无效");
    }
    let kind = if req.kind.trim().is_empty() {
        DEFAULT_TYPE
    } else {
        req.kind.trim()
    };

    let created: Result<AnnouncementRow, _> = sqlx::query_as(&format!(
        "INSERT INTO announcements (title, content, type, is_active, created_at) \
         VALUES ($1, $2, $3, $4, $5) RETURNING {ANNOUNCEMENT_COLUMNS}"
    ))
    .bind(title)
    .bind(content)
    .bind(kind)
    .bind(req.is_active)
    .bind(Utc::now())
    .fetch_one(&state.pg)
    .await;

    match created {
        Ok(row) => ok(AnnouncementRecord::from(row)),
        Err(error) => db_failure("create_announcement", &error, "创建公告失败，请稍后重试"),
    }
}

/// `PUT /admin/announcements/{id}` —— 更新。
///
/// 对应 `AdminUpdateAnnouncementHandler`。两处与新建**不同**，别顺手统一：
///
/// * 标题/正文**不做非空校验**（原实现只在 create 上校验），所以清空是允许的；
/// * `type` 为空表示"保持原值"，而不是回落到 `info`。
pub async fn admin_update(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的 ID");
    };
    let req: SaveRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    // 空 type 保持原值：`COALESCE(NULLIF($4, ''), type)`。
    let kind = req.kind.trim();

    let updated: Result<Option<AnnouncementRow>, _> = sqlx::query_as(&format!(
        "UPDATE announcements SET title = $2, content = $3, \
             type = COALESCE(NULLIF($4, ''), type), is_active = $5 \
         WHERE id = $1 RETURNING {ANNOUNCEMENT_COLUMNS}"
    ))
    .bind(id)
    .bind(req.title.trim())
    .bind(req.content.trim())
    .bind(kind)
    .bind(req.is_active)
    .fetch_optional(&state.pg)
    .await;

    match updated {
        Ok(Some(row)) => ok(AnnouncementRecord::from(row)),
        Ok(None) => not_found("未找到该公告"),
        Err(error) => db_failure("update_announcement", &error, "更新公告失败，请稍后重试"),
    }
}

/// `DELETE /admin/announcements/{id}`。
///
/// 对应 `AdminDeleteAnnouncementHandler`：**检查影响行数**，删不存在的 id → 404。
pub async fn admin_delete(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的 ID");
    };
    match sqlx::query("DELETE FROM announcements WHERE id = $1")
        .bind(id)
        .execute(&state.pg)
        .await
    {
        Ok(done) if done.rows_affected() == 0 => not_found("未找到该公告"),
        Ok(_) => ok(serde_json::json!({ "deleted": true })),
        Err(error) => db_failure("delete_announcement", &error, "删除公告失败，请稍后重试"),
    }
}

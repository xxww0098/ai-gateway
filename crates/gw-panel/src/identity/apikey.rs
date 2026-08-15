//! API Key 的增删查与改绑分组。
//!
//! 对应既有实现的 `apikey`（`GenerateAPIKey`）+ `handler_user` 的四个
//! key handler + `handler_admin_expanded` 的 `AdminUsersAPIKeysHandler`。
//!
//! 用户自己的 key 路由和管理员查看某用户 key 的路由住在同一个文件里 —— 它们读的
//! 是同一张表、同一套语义，按角色劈开只会让「API Key」这个功能横跨两个目录
//! （规则 1.6）。
//!
//! # 明文只出现一次
//!
//! 库里只有 SHA-256 摘要和前缀。创建响应是明文唯一一次离开进程的机会；列表接口
//! 返回的 `key` 是 `<prefix>****`，不是密钥。别为了「方便」在列表里回明文。

use axum::extract::{Path, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use gw_authcore::{api_key_prefix, hash_api_key, new_api_key};
use gw_infra::Db;

use super::{bad_request, db_failure, forbidden, internal, not_found, parse_json_body};
use crate::paging::{Page, parse_id};
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 列表里遮蔽明文用的后缀。对应 `key.KeyPrefix + "****"`。
const MASK_SUFFIX: &str = "****";

/// 被"删除"的 key 落到这个状态；行本身保留，因为用量日志还引用着它的 id。
const STATUS_REVOKED: &str = "revoked";

/// 对应 `apiKeyListItem`。
///
/// `group_id` / `last_used_at` 带 `omitempty`：旧实现里它们是指针，为空时**整个键消失**，
/// 不是 `null`。前端 `ApiKey` 把两者都声明成可选，正是照这个来的。
#[derive(Debug, Serialize)]
pub struct ApiKeyListItem {
    pub id: i64,
    pub name: String,
    /// 遮蔽后的展示值，不是密钥。
    pub key: String,
    pub key_prefix: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// 对应 `apiKeyCreateResponse` —— 唯一一次带明文的响应。
#[derive(Debug, Serialize)]
pub struct ApiKeyCreated {
    pub id: i64,
    pub name: String,
    /// 明文。只在这里出现一次，之后无法再取回。
    pub key: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
}

/// 对应 `apiKeyCreateRequest`。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CreateRequest {
    name: String,
}

/// 对应 `rebindGroupRequest`。`group_id: null` 是**解绑**，不是"没传"。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RebindRequest {
    group_id: Option<i64>,
}

/// 管理员视图的一行。字段与旧实现的 `gin.H` 逐个对齐 —— 注意它用的是 `prefix`
/// 而不是用户侧的 `key_prefix`，`quota` / `quota_used` 是恒为 0 的占位列。
#[derive(Debug, Serialize)]
pub struct AdminApiKeyItem {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub status: String,
    pub quota: i64,
    pub quota_used: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct ApiKeyRow {
    id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    name: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    key_prefix: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    group_id: Option<i64>,
    last_used_at: Option<DateTime<Utc>>,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
}

/// 遮蔽后的展示值。对应 `key.KeyPrefix + "****"`。
#[must_use]
pub fn masked(key_prefix: &str) -> String {
    format!("{key_prefix}{MASK_SUFFIX}")
}

/// 新建一个 API Key 并落库，返回 `(明文, 行)`。
///
/// Ports `PanelRouter.GenerateAPIKey`。明文只在返回值里出现，库里存的是
/// `hash_api_key` 的摘要和 `api_key_prefix` 的前缀。
///
/// # Errors
/// 生成随机串失败或插入失败。
pub async fn generate_api_key(
    pg: &Db,
    user_id: i64,
    name: &str,
    group_id: Option<i64>,
) -> Result<(String, ApiKeyRowPublic), ApiKeyError> {
    let plaintext = new_api_key().map_err(|_| ApiKeyError::Generate)?;
    let prefix = api_key_prefix(&plaintext).to_owned();
    let now = Utc::now();
    let row: (i64, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO api_keys \
             (user_id, key_hash, key_prefix, name, status, group_id, last_used_at, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'active', $5, NULL, $6, $6) \
         RETURNING id, created_at",
    )
    .bind(user_id)
    .bind(hash_api_key(&plaintext))
    .bind(&prefix)
    .bind(name)
    .bind(group_id)
    .bind(now)
    .fetch_one(pg)
    .await?;

    Ok((
        plaintext,
        ApiKeyRowPublic {
            id: row.0,
            name: name.to_owned(),
            key_prefix: prefix,
            created_at: row.1,
        },
    ))
}

/// [`generate_api_key`] 返回的那一小撮字段（响应真正需要的）。
#[derive(Debug, Clone)]
pub struct ApiKeyRowPublic {
    pub id: i64,
    pub name: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
}

/// [`generate_api_key`] 的失败原因。
#[derive(Debug, thiserror::Error)]
pub enum ApiKeyError {
    /// 系统随机源不可用。
    #[error("failed to generate api key material")]
    Generate,
    /// 插入失败。
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// `GET /user/api-keys` —— 当前用户未撤销的 key。
///
/// Ports `ListAPIKeysHandler`。**信封是假分页**：旧实现一次取全部，然后填
/// `page=1, page_size=len(items), total=len(items), total_pages=1`。照抄，
/// 前端读的就是这五个键。
pub async fn list_own(State(state): State<PanelState>, user: AuthUser) -> Response {
    let rows: Result<Vec<ApiKeyRow>, _> = sqlx::query_as(
        "SELECT id, name, key_prefix, status, group_id, last_used_at, created_at \
         FROM api_keys WHERE user_id = $1 AND status <> $2 ORDER BY created_at DESC",
    )
    .bind(user.user_id)
    .bind(STATUS_REVOKED)
    .fetch_all(&state.pg)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            return db_failure("list_api_keys", &error, "获取 API Key 列表失败，请稍后重试");
        }
    };

    let items: Vec<ApiKeyListItem> = rows
        .into_iter()
        .map(|r| ApiKeyListItem {
            id: r.id,
            name: r.name,
            key: masked(&r.key_prefix),
            key_prefix: r.key_prefix,
            status: r.status,
            group_id: r.group_id,
            last_used_at: r.last_used_at,
            created_at: r.created_at,
        })
        .collect();

    let count = i64::try_from(items.len()).unwrap_or(i64::MAX);
    ok(Page {
        items,
        page: 1,
        page_size: count,
        total: count,
        total_pages: 1,
    })
}

/// `POST /user/api-keys` —— 建 key，明文只在这次响应里出现。
///
/// Ports `CreateAPIKeyHandler`。新 key **不绑分组**（`group_id = NULL`），要绑得
/// 走改绑接口，那里有权限校验。
pub async fn create_own(
    State(state): State<PanelState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> Response {
    let req: CreateRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    let name = req.name.trim();
    if name.is_empty() {
        return bad_request("名称不能为空");
    }

    match generate_api_key(&state.pg, user.user_id, name, None).await {
        Ok((plaintext, row)) => ok(ApiKeyCreated {
            id: row.id,
            name: row.name,
            key: plaintext,
            key_prefix: row.key_prefix,
            created_at: row.created_at,
        }),
        Err(error) => {
            tracing::warn!(event = "api_key_create_failed", user_id = user.user_id, error = %error);
            internal("创建 API Key 失败，请稍后重试")
        }
    }
}

/// `DELETE /user/api-keys/{id}` —— 软撤销。
///
/// Ports `DeleteAPIKeyHandler`。行不删，只把 `status` 改成 `revoked`，因为
/// `usage_logs.api_key_id` 还指着它。`user_id` 进 WHERE：越权删别人的 key 会得到
/// 404 而不是 403，两者对外不可区分（不给存在性预言机）。
pub async fn delete_own(
    State(state): State<PanelState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的 API Key ID");
    };

    let result = sqlx::query(
        "UPDATE api_keys SET status = $3, updated_at = $4 WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.user_id)
    .bind(STATUS_REVOKED)
    .bind(Utc::now())
    .execute(&state.pg)
    .await;

    match result {
        Ok(done) if done.rows_affected() == 0 => not_found("未找到该 API Key"),
        Ok(_) => ok(serde_json::json!({ "revoked": true })),
        Err(error) => db_failure("delete_api_key", &error, "删除 API Key 失败，请稍后重试"),
    }
}

/// `PATCH /user/api-keys/{id}/group` —— 改绑分组。
///
/// Ports `RebindAPIKeyGroupHandler`。三处顺序是**安全属性**，别重排：
///
/// 1. key 必须属于调用者（否则 404）；
/// 2. 目标分组必须存在（否则 400），且调用者当前**持有它的权益**（否则 403
///    `group_not_entitled`）；
/// 3. 只有提交成功之后才清 L1 缓存 —— 403 路径**绝不能**碰缓存（Property 9），
///    否则一次被拒的改绑会顺手把别人的热缓存打掉。
pub async fn rebind_group(
    State(state): State<PanelState>,
    user: AuthUser,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的 API Key ID");
    };
    let req: RebindRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };

    let key: Option<(i64, String)> = match sqlx::query_as(
        "SELECT id, key_hash FROM api_keys WHERE id = $1 AND user_id = $2 LIMIT 1",
    )
    .bind(id)
    .bind(user.user_id)
    .fetch_optional(&state.pg)
    .await
    {
        Ok(row) => row,
        Err(error) => {
            return db_failure("rebind_load_key", &error, "加载 API Key 失败，请稍后重试");
        }
    };
    let Some((key_id, key_hash)) = key else {
        return not_found("未找到该 API Key");
    };

    if let Some(group_id) = req.group_id {
        let exists: Result<Option<i64>, _> =
            sqlx::query_scalar("SELECT id FROM groups WHERE id = $1 LIMIT 1")
                .bind(group_id)
                .fetch_optional(&state.pg)
                .await;
        match exists {
            Ok(None) => return bad_request("未找到该分组"),
            Ok(Some(_)) => {}
            Err(error) => {
                return db_failure("rebind_load_group", &error, "校验分组失败，请稍后重试");
            }
        }

        match super::entitlement::user_holds_entitlement(&state.pg, user.user_id, group_id).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!(
                    event = "unentitled_group_bind_rejected",
                    user_id = user.user_id,
                    group_id = group_id,
                    api_key_id = key_id,
                );
                // 文案就是这个下划线机器码，不是中文 —— 前端按它分支。
                return forbidden("group_not_entitled");
            }
            Err(error) => {
                return db_failure("rebind_entitlement", &error, "校验分组权限失败，请稍后重试");
            }
        }
    }

    if let Err(error) =
        sqlx::query("UPDATE api_keys SET group_id = $2, updated_at = $3 WHERE id = $1")
            .bind(key_id)
            .bind(req.group_id)
            .bind(Utc::now())
            .execute(&state.pg)
            .await
    {
        return db_failure("rebind_update", &error, "更新 API Key 分组失败，请稍后重试");
    }

    // 提交成功之后才清缓存，好让下一次 /v1/* 从真相源重新解析分组绑定和倍率。
    state.api_key_cache.delete(&key_hash);

    ok(serde_json::json!({ "id": key_id, "group_id": req.group_id }))
}

/// `GET /admin/users/{id}/api-keys` —— 管理员看某个用户的全部 key（含已撤销）。
///
/// Ports `AdminUsersAPIKeysHandler`。响应是**裸数组**，不是分页信封。
pub async fn admin_list_for_user(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    let Some(user_id) = parse_id(&id) else {
        return bad_request("无效的用户 ID");
    };

    let rows: Result<Vec<ApiKeyRow>, _> = sqlx::query_as(
        "SELECT id, name, key_prefix, status, group_id, last_used_at, created_at \
         FROM api_keys WHERE user_id = $1 ORDER BY id DESC",
    )
    .bind(user_id)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(rows
            .into_iter()
            .map(|r| AdminApiKeyItem {
                id: r.id,
                name: r.name,
                prefix: r.key_prefix,
                status: r.status,
                quota: 0,
                quota_used: 0,
                created_at: r.created_at,
            })
            .collect::<Vec<_>>()),
        Err(error) => db_failure(
            "admin_list_api_keys",
            &error,
            "获取 API Key 列表失败，请稍后重试",
        ),
    }
}

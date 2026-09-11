//! `POST /api/service/users/{user_id}/keys` —— 发 / 轮换 API Key。
//!
//! 契约 §3.2：单事务内先把该 user 下**同名且 active** 的行置 `revoked`，再插入新行。
//! `rotated` 表示本次是否撤销了旧 key，所以同一个名字重发天然可重放 —— 超时重试
//! 不会留下两个 active key。
//!
//! # 明文只出现一次
//!
//! 库里只有 `key_hash` 与 `key_prefix`（复用 `gw-authcore` 的
//! `new_api_key` / `api_key_prefix` / `hash_api_key`）。响应里的 `key` 是明文唯一
//! 一次离开进程的机会；没有任何端点能再取回它。
//!
//! # 撤销之后要清 L1 缓存
//!
//! `ApiKeyCache` 会把已解析的 key 缓存 5 分钟。旧 key 刚被撤销、缓存还没过期时，
//! 它仍能从 `/v1/*` 与面板通过验证 —— 所以提交成功后逐个 `delete`。顺序不能反：
//! 事务回滚时清缓存只会白打一次热路径。

use axum::extract::{Path, State};
use axum::response::Response;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use gw_authcore::{api_key_prefix, hash_api_key, new_api_key};

use super::{ServiceCaller, bad_request, db_failure, internal, not_found, parse_json_body, user_exists};
use crate::PanelState;
use crate::ok;
use crate::paging::parse_id;

/// 名字长度上限（契约 §3.2：1–64 字符）。
const MAX_NAME_CHARS: usize = 64;

/// 被轮换掉的 key 落到这个状态；行本身保留，因为 `usage_logs.api_key_id` 还指着它。
const STATUS_REVOKED: &str = "revoked";

/// 唯一在用的状态值。
const STATUS_ACTIVE: &str = "active";

/// 契约 §3.2 的请求体。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RotateRequest {
    name: String,
}

/// 契约 §3.2 的 `data`。
#[derive(Debug, Serialize)]
pub struct RotatedKey {
    /// 新插入的 `api_keys.id`。
    pub key_id: i64,
    /// 明文。只在这里出现一次。
    pub key: String,
    /// 展示用前缀（`agw-` + 8 hex）。
    pub key_prefix: String,
    /// 请求里的名字（trim 之后）。
    pub name: String,
    /// 本次是否撤销了同名的旧 active key。
    pub rotated: bool,
}

/// `POST /api/service/users/{user_id}/keys`。
pub async fn rotate(
    State(state): State<PanelState>,
    _caller: ServiceCaller,
    Path(user_id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(user_id) = parse_id(&user_id) else {
        return bad_request("无效的用户 ID");
    };
    let req: RotateRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return bad_request("名称长度必须是 1-64 个字符");
    }

    // 未知 user 是 404/4004，而不是"给它建一个 key"：没有 users 行时插入
    // api_keys 只会留下一张谁也认领不了的凭证。
    match user_exists(&state.pg, user_id).await {
        Ok(true) => {}
        Ok(false) => return not_found("用户不存在"),
        Err(error) => {
            return db_failure("service_key_user_lookup", &error, "创建 API Key 失败，请稍后重试");
        }
    }

    let plaintext = match new_api_key() {
        Ok(plaintext) => plaintext,
        Err(error) => {
            tracing::warn!(event = "service_key_generate_failed", error = %error);
            return internal("创建 API Key 失败，请稍后重试");
        }
    };
    let key_prefix = api_key_prefix(&plaintext).to_owned();
    let key_hash = hash_api_key(&plaintext);
    let now = Utc::now();

    let mut tx = match state.pg.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            return db_failure("service_key_begin", &error, "创建 API Key 失败，请稍后重试");
        }
    };

    let revoked: Result<Vec<(String,)>, _> = sqlx::query_as(
        "UPDATE api_keys SET status = $4, updated_at = $3 \
         WHERE user_id = $1 AND name = $2 AND status = $5 \
         RETURNING key_hash",
    )
    .bind(user_id)
    .bind(name)
    .bind(now)
    .bind(STATUS_REVOKED)
    .bind(STATUS_ACTIVE)
    .fetch_all(&mut *tx)
    .await;
    let revoked = match revoked {
        Ok(rows) => rows,
        Err(error) => {
            let _ = tx.rollback().await;
            return db_failure("service_key_revoke", &error, "创建 API Key 失败，请稍后重试");
        }
    };

    let inserted: Result<(i64,), _> = sqlx::query_as(
        "INSERT INTO api_keys \
             (user_id, key_hash, key_prefix, name, status, group_id, last_used_at, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'active', NULL, NULL, $5, $5) \
         RETURNING id",
    )
    .bind(user_id)
    .bind(&key_hash)
    .bind(&key_prefix)
    .bind(name)
    .bind(now)
    .fetch_one(&mut *tx)
    .await;
    let key_id = match inserted {
        Ok((key_id,)) => key_id,
        Err(error) => {
            let _ = tx.rollback().await;
            return db_failure("service_key_insert", &error, "创建 API Key 失败，请稍后重试");
        }
    };

    if let Err(error) = tx.commit().await {
        return db_failure("service_key_commit", &error, "创建 API Key 失败，请稍后重试");
    }

    // 提交之后才清缓存：让下一次 /v1/* 从真相源重新解析，旧 key 立即失效。
    for (revoked_hash,) in &revoked {
        state.api_key_cache.delete(revoked_hash);
    }

    ok(RotatedKey {
        key_id,
        key: plaintext,
        key_prefix,
        name: name.to_owned(),
        rotated: !revoked.is_empty(),
    })
}

//! 操作审计的**写**一半：把一条 [`crate::audit::OperationEntry`] 落进 `operation_logs`。
//!
//! 对应既有实现的 `oplog` 的 `recordOperation`。canonical 编码与 HMAC 在
//! [`crate::audit`]（协调者独占，两个 worker 共用）；**读**（`/admin/audit-logs`
//! 聚合、验签）归 `ops` 域。
//!
//! 语义与旧实现一致的两点，别"改进"：
//!
//! * **失败只记日志、绝不影响业务**。审计写不进去时，触发它的那次注册/充值/确认
//!   仍然成功 —— 审计是旁路，不是事务的一部分（旧实现注释原话："the audit trail must
//!   never block the business operation that triggered it"）。
//! * **`created_at` 截到秒**。哈希覆盖了这个时间戳，而亚秒精度在不同驱动上会被
//!   改写，截断后才能往返一致地验签。

use axum::extract::{FromRequestParts, MatchedPath};
use axum::http::request::Parts;
use chrono::{SubsecRound, Utc};
use serde_json::Value;
use std::convert::Infallible;
use std::net::SocketAddr;

use crate::PanelState;
use crate::audit::{Actor, OperationEntry, RequestMeta, SOURCE_PANEL, entry_hash};

#[cfg(test)]
mod tests;

/// 旧实现用 `c.GetHeader("X-Trace-ID")` 透传的链路 id 头。
pub const TRACE_ID_HEADER: &str = "X-Trace-ID";

/// 从请求头里抽出 `RequestMeta`。
///
/// 旧实现直接从 `*gin.Context` 上读（`c.Request.Method` / `c.FullPath()` /
/// `c.ClientIP()` / `traceIDFromGin`）；axum 的 handler 拿不到 `Context`，所以做成
/// 提取器，需要写审计的 handler 显式声明它。
///
/// 提取**永不失败**：审计元数据缺失不该让业务请求 400。
#[derive(Debug, Clone)]
pub struct ReqMeta(pub RequestMeta);

impl<S: Send + Sync> FromRequestParts<S> for ReqMeta {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // 旧实现的 c.FullPath() 是**匹配到的路由模式**（`/user/api-keys/:id`），不是
        // 具体 URL —— 否则每个 id 都会变成一个新的审计 path 基数。axum 的等价物
        // 是 MatchedPath；没有匹配到路由时退回原始 path，与旧实现的 fallback 一致。
        let path = parts
            .extensions
            .get::<MatchedPath>()
            .map(|m| m.as_str().to_owned())
            .unwrap_or_else(|| parts.uri.path().to_owned());

        Ok(Self(RequestMeta {
            method: parts.method.as_str().to_owned(),
            path,
            ip_address: client_ip(parts),
            request_id: trace_id(parts),
        }))
    }
}

/// 调用方的 IP。
///
/// 旧实现用 gin 的 `c.ClientIP()`，它在配置了受信代理时读 `X-Forwarded-For` /
/// `X-Real-IP`，否则用 TCP 对端地址。这里按同样的优先级来：转发头优先，其次是
/// `ConnectInfo`（只有当组合根用 `into_make_service_with_connect_info` 起服务时
/// 才有），最后是空串 —— 旧实现在拿不到时给的也是空串，而不是某个假地址。
#[must_use]
pub fn client_ip(parts: &Parts) -> String {
    if let Some(xff) = parts.headers.get("x-forwarded-for")
        && let Ok(raw) = xff.to_str()
        && let Some(first) = raw.split(',').next()
        && !first.trim().is_empty()
    {
        return first.trim().to_owned();
    }
    if let Some(real) = parts.headers.get("x-real-ip")
        && let Ok(raw) = real.to_str()
        && !raw.trim().is_empty()
    {
        return raw.trim().to_owned();
    }
    parts
        .extensions
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .unwrap_or_default()
}

/// 本次请求的链路 id。Ports `traceIDFromGin`：请求头优先，缺失时新生成一个。
#[must_use]
pub fn trace_id(parts: &Parts) -> String {
    parts
        .headers
        .get(TRACE_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| uuid::Uuid::new_v4().to_string(), ToOwned::to_owned)
}

/// `operation_logs` 的插入语句。列顺序与 `gw_model::OperationLog` 一致。
const INSERT_OPERATION_LOG: &str = "INSERT INTO operation_logs \
     (source, actor_id, actor_email, actor_role, action, target, method, path, \
      status_code, ip_address, request_id, metadata, created_at, entry_hash) \
     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)";

/// 用 Postgres 自己把 `value` 归一化成它存进 `jsonb` 后 `::text` 会返回的字节。
///
/// 审计哈希必须覆盖这串字节，才能和 verify 侧读回的 `metadata::text` 对得上 ——
/// serde 的紧凑序列化（键序、`":"` 无空格）跟 Postgres 的 jsonb 渲染不一致。
async fn normalize_jsonb(pool: &sqlx::PgPool, value: &Value) -> Result<Vec<u8>, sqlx::Error> {
    let text: String = sqlx::query_scalar("SELECT $1::jsonb::text")
        .bind(value)
        .fetch_one(pool)
        .await?;
    Ok(text.into_bytes())
}

/// 写一条面板操作审计。Ports `PanelRouter.recordOperation`。
///
/// `actor` 传 `None` 表示尚未认证的事件（注册、登录失败）—— 旧实现在那里把
/// `ActorID` 留在 0、`ActorEmail` 留空，这里同样。
///
/// 本函数吞掉所有错误（只 `warn!`），调用方不需要也不应该处理返回值。
pub async fn record(
    state: &PanelState,
    meta: &RequestMeta,
    actor: Option<&Actor>,
    action: &str,
    target: &str,
    status_code: i32,
    extras: Option<Value>,
) {
    // metadata 落进 `jsonb` 后，Postgres 会重排键、重渲染空白；审计哈希覆盖的是
    // 「列里最终存的字节」—— verify 读的正是 `metadata::text`。所以这里先让 Postgres
    // 归一化一遍，再对归一化后的字节做 HMAC，否则任何带 metadata 的行经过一次 jsonb
    // 往返就验不过（既有实现的 verify 同样读 `::text`，这样才能跨实现对上）。
    let metadata = match extras.as_ref() {
        Some(value) => match normalize_jsonb(&state.pg, value).await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    event = "operation_log_write_failed",
                    action = action,
                    target = target,
                    error = %err,
                    "record_operation_metadata_normalize_failed"
                );
                return;
            }
        },
        None => Vec::new(),
    };

    let entry = OperationEntry {
        source: SOURCE_PANEL.to_owned(),
        actor_id: actor.map_or(0, |a| a.user_id),
        actor_email: actor.map(|a| a.email.clone()).unwrap_or_default(),
        actor_role: actor.map(|a| a.role.clone()).unwrap_or_default(),
        action: action.to_owned(),
        target: target.to_owned(),
        method: meta.method.clone(),
        path: meta.path.clone(),
        status_code: i64::from(status_code),
        ip_address: meta.ip_address.clone(),
        request_id: meta.request_id.clone(),
        metadata,
        created_at: Utc::now().trunc_subsecs(0),
    };

    let hash = entry_hash(state.audit_hmac_key.as_deref().map(Vec::as_slice), &entry);

    let result = sqlx::query(INSERT_OPERATION_LOG)
        .bind(&entry.source)
        .bind(entry.actor_id)
        .bind(&entry.actor_email)
        .bind(&entry.actor_role)
        .bind(&entry.action)
        .bind(&entry.target)
        .bind(&entry.method)
        .bind(&entry.path)
        .bind(entry.status_code)
        .bind(&entry.ip_address)
        .bind(&entry.request_id)
        .bind(extras)
        .bind(entry.created_at)
        .bind(&hash)
        .execute(&state.pg)
        .await;

    if let Err(err) = result {
        tracing::warn!(
            event = "operation_log_write_failed",
            action = action,
            target = target,
            error = %err,
            "record_operation_failed"
        );
    }
}

//! 用户资料（自己看）。
//!
//! 对应既有实现的 `handler_user` 的 `UserProfileHandler` 与 `authUserFromModel`。
//! 管理员侧的「用户管理」已下线：密钥与余额都是用户自助打理，登出后角色、
//! 状态、余额的变更也没有了管理入口。

use axum::extract::State;
use axum::response::Response;
use chrono::{DateTime, Utc};

use super::auth::{AuthUserPayload, legacy_rfc3339};
use super::{db_failure, internal, not_found};
use crate::{AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, sqlx::FromRow)]
struct AdminUserRow {
    id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    email: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    role: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    balance: f64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
}

/// 老名字沿用旧实现：这组列就是当年管理员负载（也是现在 profile）读的那组。
const ADMIN_USER_COLUMNS: &str = "id, email, role, balance, status, created_at";

/// 对应 `nullableString` —— trim 后为空就变成 JSON `null`。
#[must_use]
pub fn nullable_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// `GET /user/profile` —— 自己的资料 + 可用余额。
///
/// Ports `UserProfileHandler`。`available_balance` 走账本（**已扣掉在途预扣**），
/// 与 `user.balance` 那个持久化列不是一回事：前者是"现在还能花多少"。
pub async fn profile(State(state): State<PanelState>, user: AuthUser) -> Response {
    let row: Result<Option<AdminUserRow>, _> = sqlx::query_as(&format!(
        "SELECT {ADMIN_USER_COLUMNS} FROM users WHERE id = $1 LIMIT 1"
    ))
    .bind(user.user_id)
    .fetch_optional(&state.pg)
    .await;

    let row = match row {
        Ok(Some(row)) => row,
        // 旧实现的 handleRecordError：查不到这行是 404「未找到该用户」。
        Ok(None) => return not_found("未找到该用户"),
        Err(error) => {
            return db_failure("load_profile", &error, "加载用户信息失败，请稍后重试");
        }
    };

    let available = match state.ledger.get_balance(user.user_id).await {
        Ok(balance) => balance,
        Err(error) => {
            tracing::warn!(event = "profile_balance_failed", user_id = user.user_id, error = %error);
            return internal("加载余额失败，请稍后重试");
        }
    };

    ok(serde_json::json!({
        "user": auth_payload(&row),
        "available_balance": available,
    }))
}

/// Ports `authUserFromModel` —— 与登录/注册返回的是同一个形状（秒精度字符串时间）。
fn auth_payload(row: &AdminUserRow) -> AuthUserPayload {
    AuthUserPayload {
        id: row.id,
        email: row.email.clone(),
        role: row.role.clone(),
        balance: row.balance,
        status: row.status.clone(),
        created_at: legacy_rfc3339(row.created_at),
    }
}

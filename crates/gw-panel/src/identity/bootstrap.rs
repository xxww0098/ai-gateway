//! 一次性的 `super_admin` 引导。
//!
//! 授权始终由库里的 `users.role` 决定；这里做的提权同时被两道闸门夹住：
//!
//! 1. 目标邮箱来自**服务端配置**（`auth.bootstrap_admin_email`），永远不取自请求；
//! 2. 系统里**一个 active `super_admin` 都没有**时才动手。
//!
//! 已有 `admin` 不挡住这条路径（那是救砖）。只要出现过 `super_admin`，引导永久失效。

use gw_infra::Db;
use gw_role::Role;

use super::USER_STATUS_ACTIVE;
use crate::PanelState;

#[cfg(test)]
mod tests;

/// 这个刚注册的邮箱是不是配置里指定的那个引导账号。
///
/// `configured` 必须已经 trim + 转小写。空配置必须返回 false。
#[must_use]
pub fn is_bootstrap_target(configured: &str, user_email: &str) -> bool {
    match configured {
        "" => false,
        _ => user_email.trim().eq_ignore_ascii_case(configured),
    }
}

/// 系统里是否已经存在至少一个 active `super_admin`。
///
/// # Errors
/// 查询失败时原样上抛。
pub async fn any_active_super_admin_exists(pg: &Db) -> Result<bool, sqlx::Error> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM users WHERE role = $1 AND status = $2")
            .bind(Role::SuperAdmin.as_str())
            .bind(USER_STATUS_ACTIVE)
            .fetch_one(pg)
            .await?;
    Ok(count > 0)
}

/// 注册路径上的引导。命中配置邮箱且还没有 `super_admin` 时提权。
pub async fn maybe_bootstrap_admin(state: &PanelState, user_id: i64, user_email: &str) -> bool {
    let configured = state.cfg.auth.bootstrap_admin_email.trim().to_lowercase();
    match is_bootstrap_target(&configured, user_email) {
        false => false,
        true => match any_active_super_admin_exists(&state.pg).await {
            Ok(false) => promote_super_admin(&state.pg, user_id, &configured).await,
            _ => false,
        },
    }
}

async fn promote_super_admin(pg: &Db, user_id: i64, email: &str) -> bool {
    match sqlx::query("UPDATE users SET role = $1 WHERE id = $2")
        .bind(Role::SuperAdmin.as_str())
        .bind(user_id)
        .execute(pg)
        .await
    {
        Ok(_) => {
            tracing::warn!(
                user_id = user_id,
                email = %email,
                "bootstrap super_admin: promoted registering user"
            );
            true
        }
        Err(error) => {
            tracing::warn!(
                user_id = user_id,
                error = %error,
                "bootstrap super_admin: failed to promote registering user"
            );
            false
        }
    }
}

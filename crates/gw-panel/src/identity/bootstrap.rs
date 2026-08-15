//! 一次性的管理员引导。
//!
//! 对应既有实现的 `bootstrap` 模块。
//!
//! # 为什么这条路径不是提权漏洞
//!
//! 授权始终由库里的 `users.role` 决定；这里做的提权同时被两道闸门夹住：
//!
//! 1. 目标邮箱来自**服务端配置**（`auth.bootstrap_admin_email`），永远不取自请求；
//! 2. 系统里**一个 active 管理员都没有**时才动手。
//!
//! 只要出现过任何管理员，这条路径就永久失效 —— 它无法在一个已经有人管的系统上
//! 抬升任何人。安全回归测试 `TestRegisterAdminEmailDoesNotGrantAdmin` 盯的就是
//! 第二条：拿配置里那个邮箱去注册，在已有管理员时不给管理员。

use gw_infra::Db;

use super::USER_STATUS_ACTIVE;
use crate::PanelState;

#[cfg(test)]
mod tests;

/// 这个刚注册的邮箱是不是配置里指定的那个引导账号。
///
/// Ports `maybeBootstrapAdmin` 开头那两行判断。抽出来是因为它是整条路径上唯一
/// 不碰数据库的一步，也是最容易写错的一步：**空配置必须返回 false**（否则一个
/// 没配置引导邮箱的部署会把每个注册者都当成候选），大小写和首尾空白都要吃掉
/// （旧实现用 `EqualFold` + `TrimSpace`）。
///
/// `configured` 必须已经 trim + 转小写。
#[must_use]
pub fn is_bootstrap_target(configured: &str, user_email: &str) -> bool {
    !configured.is_empty() && user_email.trim().eq_ignore_ascii_case(configured)
}

/// 系统里是否已经存在至少一个 active 管理员。Ports `anyActiveAdminExists`。
///
/// # Errors
/// 查询失败时原样上抛，调用方各自决定是「放弃引导」还是「启动失败」。
pub async fn any_active_admin_exists(pg: &Db) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM users WHERE role = 'admin' AND status = $1",
    )
    .bind(USER_STATUS_ACTIVE)
    .fetch_one(pg)
    .await?;
    Ok(count > 0)
}

/// 启动时的引导。Ports `EnsureBootstrapAdmin`。
///
/// 组合根（`gw-server`）在迁移之后、开始接请求之前调一次。空邮箱是 no-op，
/// 配置里没写这一项的部署不受影响。
///
/// # Errors
/// 只有数据库错误会上抛；「用户还没注册」不是错误 —— 那正是等着用户去注册的
/// 常见情形，旧实现在那里打一条 warn 然后返回 nil，这里同样。
pub async fn ensure_bootstrap_admin(pg: &Db, email: &str) -> Result<(), sqlx::Error> {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return Ok(());
    }
    if any_active_admin_exists(pg).await? {
        // 引导已完成；绝不覆盖库里既有的角色分配。
        return Ok(());
    }

    let existing: Option<(i64, String)> =
        sqlx::query_as("SELECT id, COALESCE(role,'') FROM users WHERE email = $1 LIMIT 1")
            .bind(&email)
            .fetch_optional(pg)
            .await?;

    let Some((user_id, role)) = existing else {
        tracing::warn!(
            bootstrap_admin_email = %email,
            "bootstrap admin: no admin exists yet; register this email to gain admin access"
        );
        return Ok(());
    };
    if role.trim().eq_ignore_ascii_case("admin") {
        return Ok(());
    }

    sqlx::query("UPDATE users SET role = 'admin' WHERE id = $1")
        .bind(user_id)
        .execute(pg)
        .await?;
    tracing::warn!(
        user_id = user_id,
        email = %email,
        "bootstrap admin: promoted existing user to admin (no admin existed)"
    );
    Ok(())
}

/// 注册路径上的引导。Ports `PanelRouter.maybeBootstrapAdmin`。
///
/// 返回**是否**提升了这个刚注册的账号，让调用方就地把响应里的 `role` 改掉，
/// 用户不用重启服务、也不用重新登录就能看到管理员界面。
///
/// 尽力而为：任何失败只打日志，绝不让注册本身失败 —— 引导是附加价值，不是注册
/// 的前置条件。
pub async fn maybe_bootstrap_admin(state: &PanelState, user_id: i64, user_email: &str) -> bool {
    let configured = state.cfg.auth.bootstrap_admin_email.trim().to_lowercase();
    if !is_bootstrap_target(&configured, user_email) {
        return false;
    }

    match any_active_admin_exists(&state.pg).await {
        Ok(false) => {}
        // 出错或者已经有管理员 —— 两种情况都什么都不做，与旧实现的
        // `if err != nil || adminExists { return }` 一致。
        _ => return false,
    }

    if let Err(error) = sqlx::query("UPDATE users SET role = 'admin' WHERE id = $1")
        .bind(user_id)
        .execute(&state.pg)
        .await
    {
        tracing::warn!(
            user_id = user_id,
            error = %error,
            "bootstrap admin: failed to promote registering user"
        );
        return false;
    }
    tracing::warn!(
        user_id = user_id,
        email = %configured,
        "bootstrap admin: promoted registering user to admin (no admin existed)"
    );
    true
}

//! 运维域：统一审计流、仪表盘、模型目录。
//!
//! Owner: worker `panel-upstream`。统一审计流、仪表盘、模型目录。
//!
//! | 模块 | 对应 handler |
//! | --- | --- |
//! | [`audit_log`] | `AdminListAuditLogsHandler` + 三个 `*LogToEntry` + `VerifyAuditLog` |
//! | [`dashboard`] | `AdminDashboardHandler` |
//! | [`catalog`] | `AdminModelCatalog*Handler` 四个 |
//!
//! # 审计日志的「写」不在这里
//!
//! `recordOperation` 是横切的（`identity` 写 8 处、`commerce` 更多），所以哈希与
//! canonical 形式住在 [`crate::audit`]，本域只负责**读**与**验**。这不是分层，
//! 是入度：把写函数放进 `ops` 会让另外两个域反向依赖运维域。
//!
//! # `/admin/probe` 说明
//!
//! 任务书里提到的 `/admin/probe` 在生产代码里**不存在** —— 它只出现在某条安全
//! 回归测试里，由那个测试自己 `authed.GET` 注册，用来验证「注册时自称 admin 邮箱的
//! 用户拿不到管理员权限」。对应的 Rust 断言属于 [`crate::auth`] 的 `AdminUser`
//! 守卫，不需要一条生产路由。存活/就绪探针是 `gw_server::health` 的
//! `/api/health/ready`，也不在面板里。

pub mod audit_log;
pub mod catalog;
pub mod dashboard;

use axum::Router;
use axum::routing::{get, post};

use crate::PanelState;

/// Routes owned by this domain. Paths are relative to the panel mount point.
pub fn router() -> Router<PanelState> {
    Router::new()
        .route("/admin/audit-logs", get(audit_log::list_audit_logs))
        .route("/admin/dashboard", get(dashboard::dashboard))
        .route(
            "/admin/model-catalog/models-url",
            get(catalog::get_models_url).put(catalog::put_models_url),
        )
        .route(
            "/admin/model-catalog/ensure-openai-channel",
            post(catalog::ensure_channel),
        )
        .route(
            "/admin/model-catalog/openai-visibility",
            post(catalog::set_visibility),
        )
}

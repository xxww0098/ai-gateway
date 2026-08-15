//! 计费域：定价管理（分组/模型四列单价）、用量查询与趋势统计。
//!
//! Owner: worker `panel-upstream`。定价管理与用量查询/统计两条面。
//!
//! # 这个域为什么同时装「改价」和「看账」
//!
//! 规则 1.6：删掉一个功能应该等于删掉一个文件夹。四列单价是**输入**，用量日志与
//! 聚合是同一套单价产生的**输出** —— 改价后聚合里的钱立刻不一样。把它们劈成
//! `pricing/` 和 `usage/` 会让「为什么改了价旧账不变」这类问题横跨两个目录。
//!
//! | 模块 | 对应 handler |
//! | --- | --- |
//! | [`prices`] | `AdminList/Upsert/DeletePricingGroupHandler` + `AdminUpsertPricingModelHandler` + `ModelsHandler` |
//! | [`usage`] | `AdminUsageLogsHandler` + `AdminUsageTrendHandler` + `AdminUsageModelsHandler` |
//! | [`usage::user`] | `UsageHandler` + `UsageDetailHandler` + `UsageStatsHandler` + `UserUsageTrendHandler` + `UserUsageModelsHandler` |
//!
//! 用户自助的那几条（`/user/usage/**`、`/user/models`）按**业务域属于这里** ——
//! 它们读的是本域写的那张价目表和那套四列口径。规则 1.6 说的是按域切、不按角色切，
//! 所以它们不进 `identity`。
//!
//! # 本域唯一的跨进程不变量
//!
//! `POST /admin/pricing/models` 必须让 **Calculator 正在读的那一个**
//! `ModelPriceCache` 失效。各建一个 cache 会让管理员改价永远到不了计费。实例由
//! [`crate::PanelState::price_cache`] 注入，本域不得自建。

pub mod prices;
pub mod usage;

use axum::Router;
use axum::routing::{delete, get, post};

use crate::PanelState;

/// Routes owned by this domain.
///
/// Paths are relative to the panel mount point, matching the other domains.
pub fn router() -> Router<PanelState> {
    Router::new()
        // ── pricing ──
        .route(
            "/admin/pricing/groups",
            get(prices::list_groups).post(prices::upsert_group),
        )
        .route("/admin/pricing/groups/{name}", delete(prices::delete_group))
        .route("/admin/pricing/models", post(prices::upsert_model_price))
        // ── usage（管理员） ──
        .route("/admin/usage-logs", get(usage::list_usage_logs))
        .route("/admin/usage/trend", get(usage::usage_trend))
        .route("/admin/usage/models", get(usage::usage_models))
        // ── usage（用户自助） ──
        //
        // 与上面三条同域、同表、同口径，只多一个 `user_id` 约束。按角色劈到
        // `identity` 会让「用量口径」这一个概念横跨两个目录（规则 1.6）。
        .route("/user/usage", get(usage::user::list_usage))
        .route("/user/usage/detail", get(usage::user::usage_detail))
        .route("/user/usage/stats", get(usage::user::usage_stats))
        .route("/user/usage/trend", get(usage::user::usage_trend))
        .route("/user/usage/models", get(usage::user::usage_models))
        // ── 价目表（用户自助） ──
        //
        // `POST /admin/pricing/models` 写的就是这张表；写和读住在一起。
        .route("/user/models", get(prices::user_models))
}

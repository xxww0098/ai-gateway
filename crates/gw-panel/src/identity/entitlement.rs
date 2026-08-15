//! 「这个用户现在能不能绑到这个分组上」这一个谓词。
//!
//! 对应既有实现的 `entitlements` 的 `UserHoldsEntitlement`。
//!
//! # 语义（照抄旧实现的注释，一条都不要改）
//!
//! * id 为 0 或未知 → `false`，**不是错误**。传零值的调用方想表达的是「没有权限」。
//! * 分组行读不出来 → `false`。管理员可以合法地删掉分组，而租户手上还留着指向它
//!   的陈旧引用；把这当成「不可绑定」而不是抛错。
//! * `groups.rate_multiplier == 1.0` → `true`。这是基线分组，每个 active 用户都
//!   隐含持有它。
//! * 否则 → 存在一行 `subscriptions`，`user_id`/`group_id` 匹配、`status='active'`、
//!   且 `expires_at > now()`（UTC）。
//!
//! 旧实现的注释特意写明：这份谓词与既有 SDK 里的
//! `accessControlsGroupEntitled` 是**两份独立实现**，改一边必须改另一边。Rust 侧的
//! 另一半在 `gw_proxy::access`（`group_entitled`）。

use chrono::Utc;
use gw_infra::Db;

/// `subscriptions.status` 的放行值。
///
/// 与 `users.status` 的 `active` 是**两个不同的概念**，只是恰好同名，所以各自
/// 有各自的常量 —— 合并成一个会让「哪天订阅状态改叫 running」变成一次跨域事故。
const SUBSCRIPTION_STATUS_ACTIVE: &str = "active";

/// 基线分组的倍率。等于它就等于「人人可绑」。
///
/// 浮点等值比较在这里是**对的**：旧实现写的就是 `grp.RateMultiplier == 1.0`，而
/// `groups.rate_multiplier` 的值来自管理员输入的十进制字面量（`1`、`0.95`），
/// 不是算出来的，所以 1.0 能被精确表示、也能被精确比出来。
pub const BASELINE_RATE_MULTIPLIER: f64 = 1.0;

/// 用户当前是否被授权绑定到该分组。Ports `UserHoldsEntitlement`。
///
/// # Errors
/// 只有订阅存在性查询的数据库错误会上抛（与旧实现同）；分组行读不到会被归一成
/// `Ok(false)`。
pub async fn user_holds_entitlement(
    pg: &Db,
    user_id: i64,
    group_id: i64,
) -> Result<bool, sqlx::Error> {
    if user_id == 0 || group_id == 0 {
        return Ok(false);
    }

    // 先看分组本身：不存在 = 不可绑（这样 API Key 永远不会被改绑到一个已消失的
    // 分组上）。查询错误在旧实现那边是上抛的，这里也上抛。
    let multiplier: Option<f64> = sqlx::query_scalar(
        "SELECT COALESCE(rate_multiplier, 0)::float8 FROM groups WHERE id = $1 LIMIT 1",
    )
    .bind(group_id)
    .fetch_optional(pg)
    .await?;

    let Some(multiplier) = multiplier else {
        return Ok(false);
    };
    if multiplier == BASELINE_RATE_MULTIPLIER {
        return Ok(true);
    }

    // 非基线分组 → 必须有一份未过期的 active 订阅。
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM (\
             SELECT 1 FROM subscriptions \
             WHERE user_id = $1 AND group_id = $2 AND status = $3 AND expires_at > $4 \
             LIMIT 1\
         ) AS hit",
    )
    .bind(user_id)
    .bind(group_id)
    .bind(SUBSCRIPTION_STATUS_ACTIVE)
    .bind(Utc::now())
    .fetch_one(pg)
    .await?;

    Ok(count > 0)
}

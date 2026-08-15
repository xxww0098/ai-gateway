//! 订阅：套餐清单、我的订阅、购买（含补偿），以及管理员侧的订阅 CRUD。
//!
//! 对应 `handler_subscription` + `handler_admin_expanded` 的
//! `AdminSubscriptions*Handler`。
//!
//! # 购买是本 crate 里唯一会「先扣钱、再建行」的地方
//!
//! 两步之间没有共同事务可用 —— 扣款走账本（自带事务 + Redis 缓存失效），建行走
//! 面板的连接。所以第二步失败时必须**立刻补偿**，这就是 [`purchase`] 里那条
//! `subscription_purchase:<pkgID>:compensate:<debitRef>` 的 Credit 的全部理由。
//! 补偿本身也可能失败：那时用户确实被扣了钱，日志打 `error` 级并等人工核销，
//! 但**响应仍然是 500**，绝不能让调用方以为购买成功了。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, Days, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use gw_infra::Db;
use gw_ledger::{Ledger, LedgerError};
use gw_model::{next_daily_reset_after, next_monthly_reset_after, next_weekly_reset_after};

use super::{SUBSCRIPTION_STATUS_ACTIVE, raw_error};
use crate::identity::{bad_request, db_failure, internal, not_found, parse_json_body};
use crate::paging::{ListPage, offset, page_params, parse_id};
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 管理员订阅列表的默认页大小（既有实现默认 20）。
const ADMIN_SUBSCRIPTIONS_DEFAULT_PAGE_SIZE: i64 = 20;

/// 管理员建订阅时，套餐没写有效期就用这个（旧实现 `days <= 0` 时补成 30）。
const FALLBACK_VALIDITY_DAYS: i64 = 30;

/// 恢复一份已过期订阅时顺延的天数（旧实现固定顺延 30 天）。
const REACTIVATE_EXTENSION_DAYS: u64 = 30;

/// 购买时套餐有效期的下限（旧实现 `days < 1` 时补成 1）。
const MIN_PURCHASE_VALIDITY_DAYS: i64 = 1;

/// 套餐没名字时兜底的展示名（旧实现名字为空时补成 "Plan"）。
const FALLBACK_GROUP_NAME: &str = "Plan";

const STATUS_REVOKED: &str = "revoked";

/// 对应 `subscriptionPackageItem`。
///
/// **`id` 是 `group_id`，不是 `subscription_packages.id`** —— 前端拿它去
/// `POST /user/subscriptions/purchase {"group_id": …}`。这条最容易在移植时搞反。
/// `description` 与三个 `*_limit_usd` 带 `omitempty`（为空时整个键消失）。
#[derive(Debug, Serialize)]
pub struct PackageItem {
    pub id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub rate_multiplier: f64,
    pub default_validity_days: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_limit_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_limit_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_limit_usd: Option<f64>,
    pub subscription_price_usd: f64,
}

/// 对应 `subscriptionItem`（用户自己的订阅）。三个 limit 同样是 `omitempty`。
#[derive(Debug, Serialize)]
pub struct SubscriptionItem {
    pub id: i64,
    pub group_id: i64,
    pub group_name: String,
    pub status: String,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub daily_usage_usd: f64,
    pub weekly_usage_usd: f64,
    pub monthly_usage_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_limit_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_limit_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_limit_usd: Option<f64>,
}

/// 对应 `subscriptionAdminPayload`。
///
/// 三个 limit 在这里**没有** `omitempty`（旧实现里键恒在、值可为 null），
/// 与上面的用户视图刚好相反 —— 别统一。`price_paid` 的键名也和列名
/// (`price_paid_usd`) 不同，照抄。
#[derive(Debug, Serialize)]
pub struct AdminSubscriptionPayload {
    pub id: i64,
    pub user_id: i64,
    pub group_id: i64,
    pub email: String,
    pub username: Option<String>,
    pub group_name: String,
    pub status: String,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub daily_usage_usd: f64,
    pub weekly_usage_usd: f64,
    pub monthly_usage_usd: f64,
    pub daily_limit_usd: Option<f64>,
    pub weekly_limit_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
    pub created_at: DateTime<Utc>,
    pub funding_source: String,
    pub funding_reference: String,
    pub price_paid: f64,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PackageRow {
    id: i64,
    group_id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    name: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    description: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    rate_multiplier: f64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    default_validity_days: i64,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    daily_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    weekly_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    monthly_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::Money")]
    subscription_price_usd: f64,
}

const PACKAGE_COLUMNS: &str = "id, group_id, name, description, rate_multiplier, \
     default_validity_days, daily_limit_usd, weekly_limit_usd, monthly_limit_usd, \
     subscription_price_usd";

#[derive(Debug, Clone, sqlx::FromRow)]
struct SubscriptionRow {
    id: i64,
    user_id: i64,
    group_id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    group_name: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    starts_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    #[sqlx(try_from = "gw_model::compat::Money")]
    daily_usage_usd: f64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    weekly_usage_usd: f64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    monthly_usage_usd: f64,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    daily_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    weekly_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    monthly_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::Text")]
    funding_source: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    funding_reference: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    price_paid_usd: f64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    notes: String,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
}

const SUBSCRIPTION_COLUMNS: &str = "id, user_id, group_id, group_name, status, starts_at, \
     expires_at, daily_usage_usd, weekly_usage_usd, monthly_usage_usd, daily_limit_usd, \
     weekly_limit_usd, monthly_limit_usd, funding_source, funding_reference, price_paid_usd, \
     notes, created_at";

impl From<SubscriptionRow> for SubscriptionItem {
    fn from(row: SubscriptionRow) -> Self {
        Self {
            id: row.id,
            group_id: row.group_id,
            group_name: row.group_name,
            status: row.status,
            starts_at: row.starts_at,
            expires_at: row.expires_at,
            daily_usage_usd: row.daily_usage_usd,
            weekly_usage_usd: row.weekly_usage_usd,
            monthly_usage_usd: row.monthly_usage_usd,
            daily_limit_usd: row.daily_limit_usd,
            weekly_limit_usd: row.weekly_limit_usd,
            monthly_limit_usd: row.monthly_limit_usd,
        }
    }
}

/// 对应 `purchaseSubscriptionRequest`。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PurchaseRequest {
    group_id: i64,
}

/// 对应 `AdminSubscriptionsCreateHandler` 的请求体。
///
/// ⚠️ **`user_id` / `group_id` 这两个键在旧实现里绑不上**：旧实现的字段是
/// `UserID, GroupID uint`，没有 json tag，而 `encoding/json` 只做字段名的
/// 大小写不敏感匹配、不拆下划线。前端 `AdminSubscriptionAssignDialog` 发的正是
/// `user_id` / `group_id`，于是两者恒为 0，handler 走 `400 订阅信息格式无效`。
/// 也就是说**这个端点在旧实现里目前是坏的**。
///
/// 这里逐字复刻该绑定语义（`UserID` / `userid` 认，`user_id` 不认），因为改成能用
/// 会静默地打开一条旧实现从未真正执行过的写路径。要修请在上游显式决定，只需给下面
/// 三个字段各加一个 `alias`。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AdminCreateRequest {
    #[serde(rename = "UserID", alias = "userid")]
    user_id: i64,
    #[serde(rename = "GroupID", alias = "groupid")]
    group_id: i64,
    validity_days: i64,
    #[serde(rename = "Notes", alias = "notes")]
    notes: String,
    #[serde(rename = "FundingSource", alias = "fundingsource")]
    funding_source: String,
    #[serde(rename = "FundingReference", alias = "fundingreference")]
    funding_reference: String,
    price_paid_usd: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ExtendRequest {
    days: i64,
}

/// [`purchase_subscription`] 的失败原因。
#[derive(Debug, thiserror::Error)]
pub enum PurchaseError {
    /// 余额不足。**什么都没写** —— 账本在这条路径上不落 `balance_logs`。
    #[error("insufficient balance")]
    InsufficientBalance,
    /// 扣款本身失败（非余额原因）。同样什么都没写。
    #[error("debit failed: {0}")]
    Debit(String),
    /// 扣款成功但建订阅失败。`compensated` 说明补偿 Credit 是否也成功了 ——
    /// 为 `false` 时用户**确实被扣了钱而没拿到订阅**，必须人工核销。
    #[error("subscription create failed (compensated: {compensated})")]
    CreateFailed {
        /// 补偿 Credit 是否成功。
        compensated: bool,
        /// 这次扣款的 reference，人工核销时按它配对。
        debit_reference: String,
    },
}

/// 扣款 → 建订阅 → 失败即补偿。**这是本 crate 唯一会先动钱再写业务行的地方。**
///
/// `create` 是一个返回新订阅 id 的闭包，而不是写死的 INSERT：
///
/// * 参数收窄到 `(&Db, &Ledger)`，一个只有 Postgres 的测试就能跑完整条路径；
/// * 更关键的是**补偿分支可测** —— 把一个必然失败的闭包传进来，就能验证
///   "扣款被原额退回，且 reference 里嵌着原始扣款串"，而不必去人为制造一次
///   数据库故障。
///
/// 补偿失败**不会**被吞：它进 `CreateFailed { compensated: false }`，调用方据此
/// 打 error 级日志。响应无论如何都是失败 —— 绝不能让调用方以为购买成功了。
///
/// # Errors
/// 见 [`PurchaseError`]。
pub async fn purchase_subscription<F, Fut>(
    ledger: &Ledger,
    user_id: i64,
    package_id: i64,
    price: f64,
    create: F,
) -> Result<i64, PurchaseError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<i64, sqlx::Error>>,
{
    // nonce 让同一个用户重复购买同一套餐的两次扣款可区分，补偿才不会配错。
    let debit_reference = format!(
        "subscription_purchase:{package_id}:{}",
        uuid::Uuid::new_v4()
    );

    ledger
        .debit(user_id, price, &debit_reference)
        .await
        .map_err(|error| match error {
            LedgerError::InsufficientBalance => PurchaseError::InsufficientBalance,
            other => PurchaseError::Debit(other.to_string()),
        })?;

    match create().await {
        Ok(id) => Ok(id),
        Err(error) => {
            tracing::warn!(
                event = "subscription_create_failed",
                user_id = user_id,
                package_id = package_id,
                debit_ref = %debit_reference,
                error = %error,
            );
            let compensate_reference = compensation_reference(package_id, &debit_reference);
            let compensated = ledger
                .credit(user_id, price, &compensate_reference)
                .await
                .is_ok();
            Err(PurchaseError::CreateFailed {
                compensated,
                debit_reference,
            })
        }
    }
}

/// 补偿 Credit 的 reference。
///
/// `subscription_purchase:<pkgID>:compensate:<debitRef>` —— 前缀让运维能用
/// `Reference LIKE 'subscription_purchase:<pkg>:%'` 一次捞出扣款与补偿这一对，
/// 嵌入的完整扣款串则说明"退的是哪一次"。
#[must_use]
pub fn compensation_reference(package_id: i64, debit_reference: &str) -> String {
    format!("subscription_purchase:{package_id}:compensate:{debit_reference}")
}

// ── 用户侧 ───────────────────────────────────────────────────────────────────

/// `GET /user/subscription-packages` —— 在售套餐，裸数组。
///
/// Ports `ListSubscriptionPackagesHandler`。**不需要登录态之外的任何校验**，
/// 旧实现这里连 `requireBillingCtx` 都没调。
pub async fn list_packages(State(state): State<PanelState>) -> Response {
    let rows: Result<Vec<PackageRow>, _> = sqlx::query_as(&format!(
        "SELECT {PACKAGE_COLUMNS} FROM subscription_packages WHERE enabled = TRUE ORDER BY id ASC"
    ))
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(rows
            .into_iter()
            .map(|p| PackageItem {
                // 这里给的是 group_id，前端拿它当购买入参。
                id: p.group_id,
                name: p.name,
                description: p.description,
                rate_multiplier: p.rate_multiplier,
                default_validity_days: p.default_validity_days,
                daily_limit_usd: p.daily_limit_usd,
                weekly_limit_usd: p.weekly_limit_usd,
                monthly_limit_usd: p.monthly_limit_usd,
                subscription_price_usd: p.subscription_price_usd,
            })
            .collect::<Vec<_>>()),
        Err(error) => db_failure("list_packages", &error, "获取订阅套餐失败，请稍后重试"),
    }
}

/// `GET /user/subscriptions` —— 我的订阅，裸数组。Ports `ListSubscriptionsHandler`。
pub async fn list_own(State(state): State<PanelState>, user: AuthUser) -> Response {
    let rows: Result<Vec<SubscriptionRow>, _> = sqlx::query_as(&format!(
        "SELECT {SUBSCRIPTION_COLUMNS} FROM subscriptions WHERE user_id = $1 \
         ORDER BY created_at DESC"
    ))
    .bind(user.user_id)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(rows
            .into_iter()
            .map(SubscriptionItem::from)
            .collect::<Vec<_>>()),
        Err(error) => db_failure("list_subscriptions", &error, "获取订阅失败，请稍后重试"),
    }
}

/// `POST /user/subscriptions/purchase` —— 扣款 → 建订阅，失败即补偿。
///
/// Ports `PurchaseSubscriptionHandler`（Requirement 5）。顺序不可调换：
///
/// 1. **欠款前置检查**（Requirement 2.5）。有未清偿欠款就 402
///    `{"error":"outstanding_debt"}`，**任何写之前**。查询本身出错也当作「有欠款」
///    处理（fail closed）—— 一次抖动不该让欠款用户冲过闸门。
/// 2. 载入套餐；价格 ≤ 0 是配置错误，400。
/// 3. 造一个带 nonce 的扣款 reference：`subscription_purchase:<pkgID>:<uuid>`。
///    前缀是运营用 `Reference LIKE 'subscription_purchase:<id>:%'` 配对补偿的契约。
/// 4. `Debit`。余额不足 → 400 `{"error":"insufficient balance"}`，且**一行都没写**
///    （账本在余额不足时不写 BalanceLog，Requirement 5.5 靠这一点成立）。
/// 5. `INSERT subscriptions`。失败 → 立刻以
///    `subscription_purchase:<pkgID>:compensate:<debitRef>` 发补偿 Credit，
///    响应 500 `{"error":"subscription create failed"}`。
/// 6. 成功 → `{"subscription_id": …, "balance": …}`。
pub async fn purchase(
    State(state): State<PanelState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> Response {
    let req: PurchaseRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if req.group_id == 0 {
        return bad_request("请求格式无效");
    }

    // (1) 欠款闸门。
    match state.ledger.has_unresolved_shortfall(user.user_id).await {
        Ok(false) => {}
        Ok(true) => {
            tracing::warn!(
                event = "subscription_purchase_debt_block",
                user_id = user.user_id,
                group_id = req.group_id,
            );
            return raw_error(StatusCode::PAYMENT_REQUIRED, "outstanding_debt");
        }
        Err(error) => {
            tracing::warn!(
                event = "subscription_purchase_shortfall_lookup_failed",
                user_id = user.user_id,
                error = %error,
            );
            return raw_error(StatusCode::PAYMENT_REQUIRED, "outstanding_debt");
        }
    }

    // (2) 套餐。
    let pkg: Result<Option<PackageRow>, _> = sqlx::query_as(&format!(
        "SELECT {PACKAGE_COLUMNS} FROM subscription_packages \
         WHERE group_id = $1 AND enabled = TRUE LIMIT 1"
    ))
    .bind(req.group_id)
    .fetch_optional(&state.pg)
    .await;
    let pkg = match pkg {
        Ok(Some(pkg)) => pkg,
        // 旧实现在这里不区分「查询失败」和「没有这个套餐」，一律 404。
        Ok(None) | Err(_) => return not_found("未找到该订阅套餐"),
    };

    let price = pkg.subscription_price_usd;
    if price <= 0.0 {
        return bad_request("订阅套餐价格无效");
    }

    // (3)-(5) 扣款 → 建订阅 → 失败即补偿，全在这一个调用里。
    let now = Utc::now();
    let days = pkg.default_validity_days.max(MIN_PURCHASE_VALIDITY_DAYS);
    let group_name = if pkg.name.is_empty() {
        FALLBACK_GROUP_NAME.to_owned()
    } else {
        pkg.name.clone()
    };
    let pg = state.pg.clone();
    let outcome =
        purchase_subscription(&state.ledger, user.user_id, pkg.id, price, || async move {
            insert_subscription(
                &pg,
                &NewSubscription {
                    user_id: user.user_id,
                    package_id: pkg.id,
                    group_id: pkg.group_id,
                    group_name: &group_name,
                    starts_at: now,
                    expires_at: add_days(now, days),
                    daily_limit_usd: pkg.daily_limit_usd,
                    weekly_limit_usd: pkg.weekly_limit_usd,
                    monthly_limit_usd: pkg.monthly_limit_usd,
                    funding_source: "",
                    funding_reference: "",
                    price_paid_usd: 0.0,
                    notes: "",
                },
            )
            .await
            .map(|row| row.id)
        })
        .await;

    let subscription_id = match outcome {
        Ok(id) => id,
        Err(PurchaseError::InsufficientBalance) => {
            return raw_error(StatusCode::BAD_REQUEST, "insufficient balance");
        }
        Err(PurchaseError::Debit(error)) => {
            tracing::warn!(event = "subscription_debit_failed", user_id = user.user_id, error = %error);
            return internal("购买订阅失败，请稍后重试");
        }
        Err(PurchaseError::CreateFailed {
            compensated,
            debit_reference,
        }) => {
            if !compensated {
                // 扣款还挂在账上，用户被真金白银地收了钱。运维必须人工发一笔
                // shortfall_resolve 的 Credit —— 这条日志是唯一的线索，别降级。
                tracing::error!(
                    event = "subscription_compensation_failed",
                    user_id = user.user_id,
                    package_id = pkg.id,
                    debit_ref = %debit_reference,
                    compensate_ref = %compensation_reference(pkg.id, &debit_reference),
                );
            }
            return raw_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "subscription create failed",
            );
        }
    };

    // (6) 成功。
    match state.ledger.get_balance(user.user_id).await {
        Ok(balance) => ok(serde_json::json!({
            "subscription_id": subscription_id,
            "balance": balance,
        })),
        Err(error) => {
            tracing::warn!(event = "purchase_balance_failed", user_id = user.user_id, error = %error);
            internal("加载余额失败，请稍后重试")
        }
    }
}

// ── 管理员侧 ─────────────────────────────────────────────────────────────────

/// `GET /admin/subscriptions` —— 全量分页。Ports `AdminSubscriptionsListHandler`。
pub async fn admin_list(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (page, page_size) = page_params(
        params.get("page").map(String::as_str),
        params.get("page_size").map(String::as_str),
        ADMIN_SUBSCRIPTIONS_DEFAULT_PAGE_SIZE,
    );

    let total: Result<i64, _> = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM subscriptions")
        .fetch_one(&state.pg)
        .await;
    let total = match total {
        Ok(total) => total,
        Err(error) => return db_failure("count_subscriptions", &error, "统计订阅失败，请稍后重试"),
    };

    let rows: Result<Vec<SubscriptionRow>, _> = sqlx::query_as(&format!(
        "SELECT {SUBSCRIPTION_COLUMNS} FROM subscriptions ORDER BY id DESC LIMIT $1 OFFSET $2"
    ))
    .bind(page_size)
    .bind(offset(page, page_size))
    .fetch_all(&state.pg)
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => return db_failure("list_subscriptions", &error, "获取订阅失败，请稍后重试"),
    };

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(admin_payload(&state, row).await);
    }
    ok(ListPage::new(items, total, page, page_size))
}

/// `POST /admin/subscriptions` —— 管理员直接分配一份订阅（不走扣款）。
///
/// Ports `AdminSubscriptionsCreateHandler`。绑定语义见 [`AdminCreateRequest`] 的
/// 注释：这个端点在旧实现里因为 json 字段名不匹配而恒返回 400，这里保持一致。
pub async fn admin_create(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: axum::body::Bytes,
) -> Response {
    let req: AdminCreateRequest = match parse_json_body(&body, "订阅信息格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if req.user_id == 0 || req.group_id == 0 {
        return bad_request("订阅信息格式无效");
    }

    // 旧实现 `Where("group_id = ? OR id = ?", req.GroupID, req.GroupID)` —— 参数
    // 既可能是 group_id 也可能是套餐主键，两边都试。
    let pkg: Result<Option<PackageRow>, _> = sqlx::query_as(&format!(
        "SELECT {PACKAGE_COLUMNS} FROM subscription_packages \
         WHERE group_id = $1 OR id = $1 ORDER BY id ASC LIMIT 1"
    ))
    .bind(req.group_id)
    .fetch_optional(&state.pg)
    .await;
    let pkg = match pkg {
        Ok(Some(pkg)) => pkg,
        Ok(None) | Err(_) => return not_found("未找到该分组"),
    };

    let days = match (req.validity_days, pkg.default_validity_days) {
        (d, _) if d > 0 => d,
        (_, d) if d > 0 => d,
        _ => FALLBACK_VALIDITY_DAYS,
    };
    let now = Utc::now();
    let created = insert_subscription(
        &state.pg,
        &NewSubscription {
            user_id: req.user_id,
            package_id: pkg.id,
            group_id: pkg.group_id,
            group_name: &pkg.name,
            starts_at: now,
            expires_at: add_days(now, days),
            daily_limit_usd: pkg.daily_limit_usd,
            weekly_limit_usd: pkg.weekly_limit_usd,
            monthly_limit_usd: pkg.monthly_limit_usd,
            funding_source: &req.funding_source,
            funding_reference: &req.funding_reference,
            price_paid_usd: req.price_paid_usd,
            notes: &req.notes,
        },
    )
    .await;

    match created {
        Ok(row) => ok(admin_payload(&state, row).await),
        Err(error) => db_failure(
            "admin_create_subscription",
            &error,
            "创建订阅失败，请稍后重试",
        ),
    }
}

/// `PUT /admin/subscriptions/{id}/extend`。Ports `AdminSubscriptionsExtendHandler`。
pub async fn admin_extend(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的订阅 ID");
    };
    let req: ExtendRequest = match parse_json_body(&body, "天数无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if req.days <= 0 {
        return bad_request("天数无效");
    }

    // 从**当前的 expires_at** 往后加，而不是从现在 —— 续期不该吞掉剩余时长。
    let updated: Result<Option<SubscriptionRow>, _> = sqlx::query_as(&format!(
        "UPDATE subscriptions SET expires_at = expires_at + make_interval(days => $2), \
             updated_at = $3 WHERE id = $1 RETURNING {SUBSCRIPTION_COLUMNS}"
    ))
    .bind(id)
    .bind(i32::try_from(req.days).unwrap_or(i32::MAX))
    .bind(Utc::now())
    .fetch_optional(&state.pg)
    .await;

    respond_with_subscription(&state, updated, "续期订阅失败，请稍后重试").await
}

/// `DELETE /admin/subscriptions/{id}` —— 撤销（`status = 'revoked'`）。
///
/// Ports `AdminSubscriptionsRevokeHandler`。**不动余额、不发退款**：撤销只是把
/// 权益关掉，退款是 [`super::refund`] 那条独立的审批流。
pub async fn admin_revoke(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    set_status(&state, &id, STATUS_REVOKED, "撤销订阅失败，请稍后重试").await
}

/// `POST|PUT /admin/subscriptions/{id}/reactivate` —— 恢复。
///
/// Ports `AdminSubscriptionsReactivateHandler`：状态回 `active`；**如果已经过期**，
/// 顺带把到期时间推到 30 天后，否则恢复出来的是一份立刻又过期的订阅。
pub async fn admin_reactivate(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的订阅 ID");
    };
    let now = Utc::now();
    let extended = add_days(now, i64::try_from(REACTIVATE_EXTENSION_DAYS).unwrap_or(30));

    let updated: Result<Option<SubscriptionRow>, _> = sqlx::query_as(&format!(
        "UPDATE subscriptions SET status = $2, \
             expires_at = CASE WHEN expires_at < $3 THEN $4 ELSE expires_at END, \
             updated_at = $3 \
         WHERE id = $1 RETURNING {SUBSCRIPTION_COLUMNS}"
    ))
    .bind(id)
    .bind(SUBSCRIPTION_STATUS_ACTIVE)
    .bind(now)
    .bind(extended)
    .fetch_optional(&state.pg)
    .await;

    respond_with_subscription(&state, updated, "恢复订阅失败，请稍后重试").await
}

/// `POST /admin/subscriptions/{id}/reset-quota` —— 三个用量计数器归零。
///
/// Ports `AdminSubscriptionsResetQuotaHandler`。**只清 usage，不动 reset_at**：
/// 下一次自然重置的时刻由 `UsagePlugin` 维护，在这里顺手改会打乱周期。
pub async fn admin_reset_quota(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的订阅 ID");
    };
    let updated: Result<Option<SubscriptionRow>, _> = sqlx::query_as(&format!(
        "UPDATE subscriptions SET daily_usage_usd = 0, weekly_usage_usd = 0, \
             monthly_usage_usd = 0, updated_at = $2 \
         WHERE id = $1 RETURNING {SUBSCRIPTION_COLUMNS}"
    ))
    .bind(id)
    .bind(Utc::now())
    .fetch_optional(&state.pg)
    .await;

    respond_with_subscription(&state, updated, "重置配额失败，请稍后重试").await
}

// ── 内部 ─────────────────────────────────────────────────────────────────────

/// 新订阅行的全部入参。写成结构体是因为它有十二个字段，位置参数没人读得懂。
struct NewSubscription<'a> {
    user_id: i64,
    package_id: i64,
    group_id: i64,
    group_name: &'a str,
    starts_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    daily_limit_usd: Option<f64>,
    weekly_limit_usd: Option<f64>,
    monthly_limit_usd: Option<f64>,
    funding_source: &'a str,
    funding_reference: &'a str,
    price_paid_usd: f64,
    notes: &'a str,
}

/// 插入一行订阅，三个 `*_reset_at` 用 `gw_model` 的重置时刻算法初始化。
///
/// 三个重置时刻**必须**用 `next_*_reset_after`，不能填 `now`：它们是「严格晚于
/// 现在的下一个边界」，填成 now 会让配额在第一次结算时立刻被判定为该重置。
async fn insert_subscription(
    pg: &Db,
    new: &NewSubscription<'_>,
) -> Result<SubscriptionRow, sqlx::Error> {
    let now = new.starts_at;
    sqlx::query_as(&format!(
        "INSERT INTO subscriptions \
             (user_id, package_id, group_id, group_name, status, starts_at, expires_at, \
              daily_usage_usd, daily_reset_at, weekly_usage_usd, weekly_reset_at, \
              monthly_usage_usd, monthly_reset_at, daily_limit_usd, weekly_limit_usd, \
              monthly_limit_usd, funding_source, funding_reference, price_paid_usd, notes, \
              created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8, 0, $9, 0, $10, $11, $12, $13, $14, $15, \
                 $16, $17, $18, $18) \
         RETURNING {SUBSCRIPTION_COLUMNS}"
    ))
    .bind(new.user_id)
    .bind(new.package_id)
    .bind(new.group_id)
    .bind(new.group_name)
    .bind(SUBSCRIPTION_STATUS_ACTIVE)
    .bind(new.starts_at)
    .bind(new.expires_at)
    .bind(next_daily_reset_after(now))
    .bind(next_weekly_reset_after(now))
    .bind(next_monthly_reset_after(now))
    .bind(new.daily_limit_usd)
    .bind(new.weekly_limit_usd)
    .bind(new.monthly_limit_usd)
    .bind(new.funding_source)
    .bind(new.funding_reference)
    .bind(new.price_paid_usd)
    .bind(new.notes)
    .bind(now)
    .fetch_one(pg)
    .await
}

async fn set_status(
    state: &PanelState,
    raw_id: &str,
    status: &str,
    failure_message: &str,
) -> Response {
    let Some(id) = parse_id(raw_id) else {
        return bad_request("无效的订阅 ID");
    };
    let updated: Result<Option<SubscriptionRow>, _> = sqlx::query_as(&format!(
        "UPDATE subscriptions SET status = $2, updated_at = $3 WHERE id = $1 \
         RETURNING {SUBSCRIPTION_COLUMNS}"
    ))
    .bind(id)
    .bind(status)
    .bind(Utc::now())
    .fetch_optional(&state.pg)
    .await;
    respond_with_subscription(state, updated, failure_message).await
}

async fn respond_with_subscription(
    state: &PanelState,
    updated: Result<Option<SubscriptionRow>, sqlx::Error>,
    failure_message: &str,
) -> Response {
    match updated {
        Ok(Some(row)) => ok(admin_payload(state, row).await),
        // 旧实现先 `loadSubscription`（读不到 → 404「未找到该订阅」）再写；这里靠
        // RETURNING 的空结果得到同一个答案，少一次往返。
        Ok(None) => not_found("未找到该订阅"),
        Err(error) => db_failure("update_subscription", &error, failure_message),
    }
}

/// 补上订阅所属用户的 email / username。
///
/// 旧实现 `_ = pr.DB.First(&u, s.UserID).Error` —— 读不到就留空，**不报错**。
/// 这里同样：用户行缺失不该让整个列表 500。
async fn admin_payload(state: &PanelState, row: SubscriptionRow) -> AdminSubscriptionPayload {
    let owner: Option<(String, String)> = sqlx::query_as(
        "SELECT COALESCE(email,''), COALESCE(username,'') FROM users WHERE id = $1 LIMIT 1",
    )
    .bind(row.user_id)
    .fetch_optional(&state.pg)
    .await
    .unwrap_or(None);
    let (email, username) = owner.unwrap_or_default();

    AdminSubscriptionPayload {
        id: row.id,
        user_id: row.user_id,
        group_id: row.group_id,
        email,
        username: crate::identity::users::nullable_string(&username),
        group_name: row.group_name,
        status: row.status,
        starts_at: row.starts_at,
        expires_at: row.expires_at,
        daily_usage_usd: row.daily_usage_usd,
        weekly_usage_usd: row.weekly_usage_usd,
        monthly_usage_usd: row.monthly_usage_usd,
        daily_limit_usd: row.daily_limit_usd,
        weekly_limit_usd: row.weekly_limit_usd,
        monthly_limit_usd: row.monthly_limit_usd,
        created_at: row.created_at,
        funding_source: row.funding_source,
        funding_reference: row.funding_reference,
        price_paid: row.price_paid_usd,
        notes: crate::identity::users::nullable_string(&row.notes),
    }
}

/// 对应旧实现的 `t.AddDate(0, 0, days)`。天数为负或溢出时原样返回，绝不 panic。
fn add_days(from: DateTime<Utc>, days: i64) -> DateTime<Utc> {
    u64::try_from(days)
        .ok()
        .and_then(|d| from.checked_add_days(Days::new(d)))
        .unwrap_or(from)
}

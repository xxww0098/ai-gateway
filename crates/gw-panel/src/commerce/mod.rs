//! 交易域：订阅套餐与购买、订单与人工确认、支付渠道、退款申请与审批、兑换码、
//! 余额流水。
//!
//! Owner: worker `panel-identity`。订阅套餐与购买、订单与人工确认、支付渠道、
//! 退款申请与审批、兑换码、余额流水。
//!
//! # 这个域的一条主线：钱只能动一次
//!
//! 五条独立的入账/出账路径，每一条都靠**条件 UPDATE**（而不是先读后写）来保证幂等，
//! 因此在多副本下也成立：
//!
//! | 路径 | 幂等靠什么 |
//! | --- | --- |
//! | 订单确认 / Stripe 回调 | `UPDATE … WHERE status='pending'`，只有抢到的那个入账 |
//! | 兑换码 | `UPDATE … WHERE status='unused'`，`rows_affected==0` 即已被人用掉 |
//! | 退款处置 | `UPDATE … WHERE status='pending'`，第二次审批得到 409 |
//! | 订阅购买 | 先 `Debit`；建行失败立刻发补偿 `Credit` |
//! | 管理员充值 | 在 `identity::users`，事务内改列 + 写流水 |
//!
//! **补偿逻辑不是可选项**：`Debit` 成功而 `INSERT subscriptions` 失败时，必须立刻以
//! `subscription_purchase:<pkgID>:compensate:<debitRef>` 为 reference 退回同额，
//! 否则用户被扣了钱却没拿到订阅。见 [`subscription::purchase`]。

pub mod balance;
pub mod payment;
pub mod redeem;
pub mod refund;
pub mod stripe;
pub mod subscription;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};

use crate::PanelState;

/// `subscriptions.status` / `payment_orders.status` 等状态机里的放行值。
/// 各自的含义不同，所以分开声明（规则 1.9 说的是"一个概念一处"，不是"同名合并"）。
pub(crate) const SUBSCRIPTION_STATUS_ACTIVE: &str = "active";

/// 对应 `c.AbortWithStatusJSON(status, gin.H{"error": …})`。
///
/// **这不是统一信封**。计费硬失败（欠款、余额不足、订阅创建失败）走的是
/// 裸 `{"error": "..."}`，没有 `code` / `message` / `data`，前端的
/// `extractErrorMessage` 认得这个形状。别顺手改成 [`crate::err`]。
pub(crate) fn raw_error(status: StatusCode, error: &'static str) -> Response {
    (status, axum::Json(serde_json::json!({ "error": error }))).into_response()
}

/// 交易域的路由表（挂在 `/api/panel` 下）。
///
/// 用户路由与管理员路由住在一起：退款既有 `POST /refund/apply` 也有
/// `PUT /admin/refund/{id}/approve`，把它们拆到两个目录就等于让"退款"这个功能
/// 横跨两处（规则 1.6）。
pub fn router() -> Router<PanelState> {
    Router::new()
        // ── 订阅 ──
        .route(
            "/user/subscription-packages",
            get(subscription::list_packages),
        )
        .route("/user/subscriptions", get(subscription::list_own))
        .route("/user/subscriptions/purchase", post(subscription::purchase))
        .route(
            "/admin/subscriptions",
            get(subscription::admin_list).post(subscription::admin_create),
        )
        .route(
            "/admin/subscriptions/{id}",
            delete(subscription::admin_revoke),
        )
        .route(
            "/admin/subscriptions/{id}/extend",
            put(subscription::admin_extend),
        )
        // 同时注册了 POST 和 PUT —— 前端两条都调（见 api.ts 的
        // reactivateSubscription / reactivateSubscriptionFallback）。
        .route(
            "/admin/subscriptions/{id}/reactivate",
            post(subscription::admin_reactivate).put(subscription::admin_reactivate),
        )
        .route(
            "/admin/subscriptions/{id}/reset-quota",
            post(subscription::admin_reset_quota),
        )
        // ── 订单与支付 ──
        .route("/user/orders", get(payment::list_own))
        .route("/admin/orders", get(payment::admin_list_orders))
        .route("/admin/orders/{id}/confirm", put(payment::admin_confirm))
        .route("/payment/stripe/config", get(payment::stripe_config))
        .route("/payment/stripe/create", post(payment::stripe_create))
        .route("/payment/alipay/create", post(payment::alipay_create))
        .route("/payment/alipay/status", get(payment::alipay_status))
        .route("/payment/wechat/create", post(payment::wechat_create))
        .route("/payment/wechat/status", get(payment::wechat_status))
        // ── 退款 ──
        .route("/refund/list", get(refund::list_own))
        .route("/refund/apply", post(refund::apply))
        .route("/admin/refunds", get(refund::admin_list))
        .route("/admin/refund/{id}/approve", put(refund::admin_approve))
        .route("/admin/refund/{id}/reject", put(refund::admin_reject))
        // ── 兑换码 ──
        .route("/user/redeem", post(redeem::redeem))
        .route(
            "/admin/redeem-codes",
            get(redeem::admin_list).post(redeem::admin_create),
        )
        .route("/admin/redeem-codes/{id}", delete(redeem::admin_delete))
        // ── 余额流水 ──
        .route("/user/balance-history", get(balance::history_own))
        .route(
            "/admin/users/{id}/balance-history",
            get(balance::admin_history),
        )
}

/// 必须挂在**应用根**（不是 `/api/panel`）、且**不经过鉴权**的路由。
///
/// 目前只有 Stripe 回调一条：注册在 `/api/payment/stripe/webhook`，在
/// panel 组之外 —— Stripe 不带 bearer token，签名就是它的身份证明。
///
/// 单独开一个函数而不是塞进 [`router`]，是因为路径前缀和鉴权层都不一样；组合根
/// （`gw-server`）需要显式地把它 merge 到根 Router 上。
pub fn public_router() -> Router<PanelState> {
    Router::new().route("/api/payment/stripe/webhook", post(stripe::webhook))
}

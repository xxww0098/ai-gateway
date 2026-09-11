//! `/admin/*` 上挡着的必须是 `AdminUser`，不是 `AuthUser`。
//!
//! 这一档是唯一走 axum router 的测试：其余测试直调落库函数，提取器根本不在
//! 它们的路径上。把某个 `admin_*` handler 的 `AdminUser` 换成 `AuthUser`，
//! 只有这里会红。
//!
//! 覆盖两个端点是刻意的：架子一次性服务整个 `/admin/*` 面，而不是只服务
//! 最新加的那个。欠款注销尤其不能漏 —— 它决定公司吃不吃掉这笔没收回来的服务
//! 成本，让用户自助等于让用户给自己免单。

use axum::http::{Method, StatusCode};
use chrono::{Days, Utc};

use crate::common::http::{call, panel_state, token_for};
use crate::common::{
    balance_of, fresh_db, seed_group, seed_refund, seed_subscription, seed_user, seed_user_with,
};
use gw_panel::PanelState;

/// 造一条**真**欠款：余额不够时 `settle` 按 `min(balance, actual)` 扣，扣不动的
/// 部分自己写成那条标记行。不手写 metadata —— 让生产路径去写，测试才不是把实现
/// 抄进断言。返回标记行的 `balance_logs.id`。
async fn seed_shortfall(state: &PanelState, user_id: i64, cost_usd: f64) -> i64 {
    state
        .ledger
        .settle(user_id, "req-shortfall", cost_usd)
        .await
        .expect("settle 产生欠款");

    let (rows, total) = state
        .ledger
        .list_unresolved_shortfalls(10, 0)
        .await
        .expect("list shortfalls");
    assert_eq!(total, 1, "铺数据没铺出恰好一条未清偿欠款");
    rows[0].log_id
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_plain_user_cannot_write_off_their_own_shortfall() {
    let pool = fresh_db("admin_guard_write_off_denied").await;
    let state = panel_state(&pool);
    let user = seed_user(&pool, "debtor@example.com", 1.0).await;
    let log_id = seed_shortfall(&state, user, 5.0).await;

    let status = call(
        &state,
        Method::POST,
        &format!("/admin/shortfalls/{log_id}/write-off"),
        &token_for(user, "debtor@example.com"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "普通用户注销掉自己的欠款 = 自助免单；这里放行说明 handler 上挂的不是 AdminUser"
    );
    assert!(
        state
            .ledger
            .has_unresolved_shortfall(user)
            .await
            .expect("recheck"),
        "被拒之后欠款必须还锁着门"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_admin_writes_the_shortfall_off_once_and_the_balance_never_moves() {
    let pool = fresh_db("admin_guard_write_off_allowed").await;
    let state = panel_state(&pool);
    let user = seed_user(&pool, "debtor@example.com", 1.0).await;
    let log_id = seed_shortfall(&state, user, 5.0).await;
    // 注销后余额必须一分不动 —— 给一个非零余额，"没变"才不是"恰好都是 0"。
    state
        .ledger
        .credit(user, 7.0, "topup-after-debt")
        .await
        .expect("credit");
    let before = balance_of(&pool, user).await;

    let admin = seed_user_with(&pool, "boss@example.com", 0.0, "admin", "active").await;
    let token = token_for(admin, "boss@example.com");
    let uri = format!("/admin/shortfalls/{log_id}/write-off");

    assert_eq!(
        call(&state, Method::POST, &uri, &token).await,
        StatusCode::OK
    );
    assert!(
        !state
            .ledger
            .has_unresolved_shortfall(user)
            .await
            .expect("recheck"),
        "注销之后门必须解开"
    );
    assert!(
        (balance_of(&pool, user).await - before).abs() < 1e-9,
        "零额注销：公司吃掉这笔成本，绝不额外送用户等额余额"
    );

    assert_eq!(
        call(&state, Method::POST, &uri, &token).await,
        StatusCode::CONFLICT,
        "同一条欠款注销两次必须撞唯一索引"
    );
    assert!(
        (balance_of(&pool, user).await - before).abs() < 1e-9,
        "被拒的第二次同样不许动余额"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_plain_user_cannot_approve_a_refund() {
    // 同一个架子上的第二个端点：证明它钉的是整个 /admin/* 面。
    let pool = fresh_db("admin_guard_refund_denied").await;
    let state = panel_state(&pool);
    let user = seed_user(&pool, "applicant@example.com", 0.0).await;
    let group = seed_group(&pool, "g-refund", 1.0).await;
    let expires = Utc::now().checked_add_days(Days::new(30)).expect("date");
    let sub = seed_subscription(&pool, user, group, "active", expires).await;
    let refund = seed_refund(&pool, user, sub).await;

    let status = call(
        &state,
        Method::PUT,
        &format!("/admin/refund/{refund}/approve"),
        &token_for(user, "applicant@example.com"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "申请人自己批自己的退款 —— handler 上挂的必须是 AdminUser"
    );
    let processed: Option<i64> =
        sqlx::query_scalar("SELECT processed_by FROM refunds WHERE id = $1")
            .bind(refund)
            .fetch_one(&pool)
            .await
            .expect("read refund");
    assert!(processed.is_none(), "被拒的审批不许留下处置痕迹");
}

/// `commerce::router()` 里**每一条** `/admin/*` 的守卫。
///
/// 上面三档钉的是具体业务后果，这一张表钉的是覆盖面：新加一条 `/admin/*` 路由却
/// 忘了挂 `AdminUser` 时，这里会红。表是手写的 —— axum 的 `Router` 不暴露路由表，
/// 没法自动枚举，所以加路由时也要在这里加一行（漏加只会少测一条，不会假绿）。
///
/// 两个方向都断言：普通用户必须 403，管理员必须**不是** 403。只断言前者的话，
/// 一条因为别的原因恒 403 的路由会假装自己被守卫保护着。
///
/// 路径参数一律用 1：`AdminUser` 排在 `Path`/`Query`/body 之前，请求根本走不到
/// 解析那一步，所以这些 id 存不存在都无所谓。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn every_admin_route_is_guarded() {
    const ADMIN_ROUTES: &[(Method, &str)] = &[
        (Method::GET, "/admin/subscriptions"),
        (Method::POST, "/admin/subscriptions"),
        (Method::DELETE, "/admin/subscriptions/1"),
        (Method::PUT, "/admin/subscriptions/1/extend"),
        (Method::POST, "/admin/subscriptions/1/reactivate"),
        (Method::PUT, "/admin/subscriptions/1/reactivate"),
        (Method::POST, "/admin/subscriptions/1/reset-quota"),
        (Method::GET, "/admin/orders"),
        (Method::PUT, "/admin/orders/1/confirm"),
        (Method::GET, "/admin/payment-config"),
        (Method::PUT, "/admin/payment-config"),
        (Method::GET, "/admin/refunds"),
        (Method::PUT, "/admin/refund/1/approve"),
        (Method::PUT, "/admin/refund/1/reject"),
        (Method::GET, "/admin/redeem-codes"),
        (Method::POST, "/admin/redeem-codes"),
        (Method::DELETE, "/admin/redeem-codes/1"),
        (Method::GET, "/admin/shortfalls"),
        (Method::POST, "/admin/shortfalls/1/write-off"),
    ];

    let pool = fresh_db("admin_guard_every_route").await;
    let state = panel_state(&pool);
    let plain = seed_user(&pool, "plain@example.com", 0.0).await;
    let plain_token = token_for(plain, "plain@example.com");
    let admin = seed_user_with(&pool, "root@example.com", 0.0, "admin", "active").await;
    let admin_token = token_for(admin, "root@example.com");

    for (method, uri) in ADMIN_ROUTES {
        assert_eq!(
            call(&state, method.clone(), uri, &plain_token).await,
            StatusCode::FORBIDDEN,
            "{method} {uri} 放行了普通用户 —— handler 上挂的不是 AdminUser"
        );
        assert_ne!(
            call(&state, method.clone(), uri, &admin_token).await,
            StatusCode::FORBIDDEN,
            "{method} {uri} 把管理员也挡了 —— 上面那条 403 不是守卫给的，这条路由并没有被真正测到"
        );
    }
}

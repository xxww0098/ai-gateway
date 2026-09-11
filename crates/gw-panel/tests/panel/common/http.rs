//! HTTP 层的最小架子：起真 router，用 `oneshot` 打真请求。
//!
//! 为什么必须走 router：这个 crate 其余的测试全是直调落库函数
//! （`apply_disposition`、`purchase`…），提取器根本不在路径上。把任意 `admin_*`
//! handler 的 [`AdminUser`](gw_panel::AdminUser) 换成
//! [`AuthUser`](gw_panel::AuthUser)，那些测试一条都不会红 —— 而普通用户从此
//! 能自助调用管理员操作。只有从 header 打进去才钉得住这条不变量。
//!
//! 只挂 [`gw_panel::commerce::router`]，所以 URI 是**相对面板前缀**的
//! （`/admin/…`，不是 `/api/panel/admin/…`）：被测的是 handler 上的提取器，
//! 不是 `gw_panel::router` 的 `nest` 前缀。

use std::sync::Arc;

use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Method, Request, StatusCode};
use sqlx::PgPool;
use tower::ServiceExt as _;

use gw_panel::PanelState;

/// 签发与校验共用的测试密钥。值本身无意义，两头一致就行。
const JWT_SECRET: &str = "panel-http-harness-secret";

/// 签发的 token 有效期（小时）。测试跑不了这么久，过期不是这里要测的东西。
const TOKEN_EXPIRY_HOURS: i64 = 1;

/// 组一个够跑 `commerce` 路由的 [`PanelState`]。
///
/// 只有鉴权提取器和被测 handler 真正会碰的字段是「活」的：`pg`、
/// `cfg.auth.jwt.secret`、`ledger`、两个 L1 cache。价格缓存给空的、审计 HMAC 与
/// Stripe secret 给 `None` —— 被测路径一个都不读。
pub(crate) fn panel_state(pool: &PgPool) -> PanelState {
    let cfg = gw_config::Config {
        auth: gw_config::AuthConfig {
            jwt: gw_config::JwtConfig {
                secret: JWT_SECRET.to_owned(),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };

    let price_cache = Arc::new(gw_pricing::ModelPriceCache::empty());
    PanelState {
        pg: pool.clone(),
        cfg: Arc::new(cfg),
        calc: Arc::new(gw_pricing::Calculator::new(
            Some(Arc::clone(&price_cache)),
            0.0,
        )),
        price_cache,
        ledger: super::ledger_without_redis(pool),
        auth_store: Arc::new(
            gw_authcore::PostgresAuthStore::new(pool.clone(), "").expect("build auth store"),
        ),
        user_status_cache: Arc::new(gw_infra::UserStatusCache::new()),
        api_key_cache: Arc::new(gw_infra::ApiKeyCache::new()),
        audit_hmac_key: None,
        stripe_webhook_secret: None,
    }
}

/// 给 `user_id` 签一个面板 JWT。
///
/// token version 用 0：`user_token_versions` 里没有行就是 0，没登出过的用户
/// 拿到的正是这个值。
pub(crate) fn token_for(user_id: i64, email: &str) -> String {
    gw_authcore::generate_jwt_with_version(user_id, email, JWT_SECRET, TOKEN_EXPIRY_HOURS, 0)
        .expect("sign jwt")
}

/// 带 bearer token 打一次请求，返回状态码。
///
/// 每次现挂一个 router：路由表是无状态的，两个 L1 cache 在 `state` 里，跨调用
/// 仍然是同一份（缓存串味本身也是这个架子该暴露的东西）。
pub(crate) async fn call(state: &PanelState, method: Method, uri: &str, token: &str) -> StatusCode {
    gw_panel::commerce::router()
        .with_state(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router responds")
        .status()
}

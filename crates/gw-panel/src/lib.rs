//! `/api/panel/**` — the operations surface consumed by the existing React
//! frontend. JSON shapes are a hard contract: the frontend is NOT changing.
//!
//! Modules are cut by BUSINESS DOMAIN, not by admin/user role (rule 1.6).
//! The test is: deleting a feature should mean deleting one folder. "Refunds"
//! has both a user route and an admin route — they live together in `commerce`.
//!
//! OWNERSHIP:
//!   worker `panel-identity` -> identity/, commerce/, support/
//!   worker `panel-upstream` -> upstream/, ops/, billing/
//! Everything in THIS file is coordinator-owned shared ground: both workers
//! consume it, so neither may fork it (rule 1.9 — one concept, one declaration).

#![deny(clippy::todo, clippy::unimplemented)]

pub mod audit;
pub mod auth;
pub mod billing;
pub mod commerce;
pub mod identity;
pub mod ops;
// Promoted out of `identity/` (rule 1.8): `billing` and `ops` both use it, and a
// domain depending on a sibling domain is the edge that makes crates unsplittable.
// Shared vocabulary belongs at the root, not inside whichever domain wrote it first.
pub mod paging;
pub mod support;
pub mod upstream;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Everything a panel handler needs, cloned per request.
///
/// 承载 `api.PanelRouter` 原来的那些字段（见 `NewPanelRouter` 及其构造后的
/// 赋值）。
#[derive(Clone)]
pub struct PanelState {
    pub pg: gw_infra::Db,
    pub redis: gw_infra::Redis,
    pub cfg: Arc<gw_config::Config>,

    /// MUST be the same instance the `Calculator` reads from. 这里显式要求：
    /// 管理员改价会让这个 cache 失效，而另建第二个 cache 会让 Calculator 一直
    /// 读到过期价格。
    pub price_cache: Arc<gw_pricing::ModelPriceCache>,
    pub calc: Arc<gw_pricing::Calculator>,
    pub ledger: Arc<gw_ledger::Ledger>,

    /// Upstream provider credentials, for the `upstream` domain.
    pub auth_store: Arc<dyn gw_authcore::AuthStore>,

    /// Shared with `gw_proxy`'s access layer so a status flip on one surface is
    /// visible on the other —— 同一个实例同时接进两头。
    pub user_status_cache: Arc<gw_infra::UserStatusCache>,
    pub api_key_cache: Arc<gw_infra::ApiKeyCache>,

    /// Derived from the credential encryption secret; `None` disables the
    /// tamper-evident `OperationLog.EntryHash`. 对应 `DeriveAuditKey`。
    pub audit_hmac_key: Option<Arc<Vec<u8>>>,
    pub stripe_webhook_secret: Option<Arc<String>>,
}

/// The unified envelope every panel route returns.
///
/// 严格对应 `api.APIResponse`：三个键，`data` 缺省时整个键消失 —— 对应
/// `json:"data,omitempty"` 标签，所以 `Success(c, nil)` 发出的是
/// `{"code":0,"message":"ok"}` 且没有 `data` 键。
/// The frontend's `unwrap` depends on this shape; do not "improve" it.
#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse<T> {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

/// `200 {"code":0,"message":"ok","data":…}`. 对应 `api.Success`。
pub fn ok<T: Serialize>(data: T) -> Response {
    (
        StatusCode::OK,
        axum::Json(ApiResponse {
            code: 0,
            message: "ok".to_owned(),
            data: Some(data),
        }),
    )
        .into_response()
}

/// `200 {"code":0,"message":"ok"}` with no `data` key. 对应 `Success(c, nil)`。
pub fn ok_empty() -> Response {
    (
        StatusCode::OK,
        axum::Json(ApiResponse::<()> {
            code: 0,
            message: "ok".to_owned(),
            data: None,
        }),
    )
        .into_response()
}

/// 对应 `api.Error`. `code` 是*业务*码，不是 HTTP 状态：400 下用
/// 4000/4001/4002，404 下用 4040，500 下用 5000/5003/5004/5005/5006，503 下用
/// 5031。要照抄精确的 (status, code) 对，别自己造。
pub fn err(status: StatusCode, code: i32, message: impl Into<String>) -> Response {
    (
        status,
        axum::Json(ApiResponse::<()> {
            code,
            message: message.into(),
            data: None,
        }),
    )
        .into_response()
}

/// Business error codes. 对应业务常量里那份及中间件本地那份
/// `middlewareErrorUnauthorized`。These are NOT HTTP statuses ——
/// copy the exact (status, code) pair from the 对应的 handler。
pub mod codes {
    /// 业务码 4000（「坏请求」）。
    pub const BAD_REQUEST: i32 = 4000;
    /// 业务码 4001（「未授权」）—— 也是 `requireAdmin` 以 HTTP 403 返回的码。
    pub const UNAUTHORIZED: i32 = 4001;
    /// 业务码 5000（「内部错误」）。
    pub const INTERNAL: i32 = 5000;
    /// 业务码 1001 —— 鉴权中间件自己的 401 码，特意与 `UNAUTHORIZED` 区分。
    pub const MIDDLEWARE_UNAUTHORIZED: i32 = 1001;
}

/// A caller authenticated on the panel surface.
///
/// 对应 `AuthMiddleware`。注意这个面板接受**两种**凭证，不止 JWT：
/// `agw-` 前缀的 API key（经 `APIKeyCache` 再落到 DB）或一个面板
/// JWT。两条路径之后都走共享的 `UserStatusCache`
/// recheck so a suspension seen on `/v1/*` is honored here immediately, and the
/// JWT path additionally rechecks `token_version` for global logout.
///
/// Every failure returns the same opaque body — do not leak which check failed.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: i64,
    pub email: String,
    pub role: String,
    /// Set only on the API-key path; `None` for JWT callers.
    pub api_key_id: Option<i64>,
    pub group_id: Option<i64>,
    /// The caller's group rate multiplier, defaulting to 1.0.
    pub rate_multiplier: f64,
}

impl AuthUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

/// An authenticated caller that additionally passed the admin check.
///
/// 对应 `requireAdmin`：`users.role = 'admin'`
/// AND `status = 'active'`, else HTTP 403 with code
/// [`codes::UNAUTHORIZED`] and message `需要管理员权限`.
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthUser);

/// The rejection both extractors emit. Kept as one type so the two failure
/// bodies can never drift apart.
pub struct AuthRejection(Response);

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        self.0
    }
}

impl AuthRejection {
    /// `401` with the middleware's own code. 当 header 缺失或格式错误时使用
    /// exactly this message.
    pub fn missing_bearer() -> Self {
        Self(err(
            StatusCode::UNAUTHORIZED,
            codes::MIDDLEWARE_UNAUTHORIZED,
            "missing or invalid authorization bearer token",
        ))
    }

    /// `401` for every credential that was present but did not check out —
    /// bad key, bad JWT, suspended user, revoked token version. Deliberately
    /// indistinguishable from one another.
    pub fn invalid_credentials() -> Self {
        Self(err(
            StatusCode::UNAUTHORIZED,
            codes::MIDDLEWARE_UNAUTHORIZED,
            "invalid_credentials",
        ))
    }

    /// `403` from the admin guard.
    pub fn not_admin() -> Self {
        Self(err(
            StatusCode::FORBIDDEN,
            codes::UNAUTHORIZED,
            "需要管理员权限",
        ))
    }

    /// `500` when the admin lookup itself failed.
    pub fn admin_check_failed() -> Self {
        Self(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            codes::INTERNAL,
            "校验管理员权限失败，请稍后重试",
        ))
    }
}

/// Pulls the raw bearer token out of the `Authorization` header.
///
/// 对应鉴权中间件里对 header 的解析。
pub fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?;
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

/// Mounts every domain router under `/api/panel`.
///
/// Each domain owns both its user-facing and admin-facing routes, so this stays
/// a flat merge — no admin/user split at this level (rule 1.6).
pub fn router(state: PanelState) -> Router {
    // Every domain declares its paths RELATIVE to the panel prefix
    // (`/auth/register`, `/user/profile`, `upstream::BASE` = `/admin/sdk-management`),
    // so the prefix is applied once here. Merging them onto a bare `Router`
    // instead would serve `/auth/register` at the root and 404 everything the
    // frontend asks for — the zero-frontend-change constraint lives or dies on
    // this `nest`.
    let panel = Router::new()
        .merge(identity::router())
        .merge(commerce::router())
        .merge(support::router())
        .merge(billing::router())
        .merge(upstream::router())
        .merge(ops::router());

    Router::new()
        .nest("/api/panel", panel)
        // Stripe webhook 注册在 panel 组**之外**、没有 auth 中间件 ——
        // `/api/payment/stripe/webhook` 靠它的 HMAC 签名认证。它带自己的绝对
        // 路径，所以必须在 **ROOT** 合并，绝不放进上面的 `nest`：嵌套会把
        // 它服务在 `/api/panel/api/payment/...`，Stripe 的回调就会静默中断。
        .merge(commerce::public_router())
        .with_state(state)
}

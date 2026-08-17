//! 身份域：注册/登录/登出、用户 CRUD、API Key、分组、面板鉴权中间件。
//!
//! Owner: worker `panel-identity`。注册/登录/登出、用户 CRUD、API Key、分组、
//! 面板鉴权中间件。
//!
//! # 这个域为什么装得下「用户」和「管理员」两侧
//!
//! 规则 1.6：删掉一个功能应该等于删掉一个文件夹。「API Key」既有
//! `GET /user/api-keys` 也有 `GET /admin/users/{id}/api-keys`，两条路由读的是同一张
//! 表、同一套语义，所以它们住在同一个 [`apikey`] 里，而不是被 admin/user 的角色边界
//! 劈成两半。分组、用户资料同理。
//!
//! # 子模块
//!
//! | 模块 | 管什么 |
//! | --- | --- |
//! | [`auth`] | 鉴权（`AuthMiddleware` 的鉴权部分） |
//! | [`apikey`] | `GenerateAPIKey` + 各 API Key handler |
//! | [`groups`] | available-groups + 分组 CRUD |
//! | [`users`] | profile + 用户 CRUD/deposit |
//! | [`entitlement`] | `UserHoldsEntitlement` 谓词 |
//! | [`bootstrap`] | 管理员引导 |
//! | [`oplog`] | `recordOperation` 的**写**一半（哈希与 canonical 在 [`crate::audit`]） |
//! | [`paging`] | `queryInt` / `parseUintParam` 与分页信封 |
//!
//! [`oplog`] 和 [`paging`] 是面板级的公共词汇，`commerce` / `support` 也在用。
//! 它们暂住这里是因为 `lib.rs` 归协调者独占；真正的归宿是 crate 根，等协调者上收。

pub mod apikey;
pub mod auth;
pub mod bootstrap;
pub mod dsh_oauth;
pub mod entitlement;
pub mod groups;
pub mod oplog;
pub mod users;

use axum::Router;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{delete, get, patch, post, put};
use serde::de::DeserializeOwned;

use crate::{PanelState, codes, err};

// ── 业务错误码（crate::codes 之外的那几个） ───────────────────────────────────
//
// `crate::codes` 已经收了 BAD_REQUEST / UNAUTHORIZED / INTERNAL /
// MIDDLEWARE_UNAUTHORIZED。还有三个它没收的，加上中间件那个限流码，
// 补在这里 —— **不重复声明已经有家的那几个**（规则 1.9）。
// 业务码不等于 HTTP 状态码：照对应 handler 里的那一对，别自创。

/// 对应 `apiErrorForbidden`。
pub const ERR_FORBIDDEN: i32 = 4003;
/// 对应 `apiErrorNotFound`。
pub const ERR_NOT_FOUND: i32 = 4004;
/// 对应 `apiErrorConflict`。
pub const ERR_CONFLICT: i32 = 4009;
/// 对应 `middlewareErrorRateLimit`。
pub const ERR_MW_RATE_LIMIT: i32 = 3001;

/// 对应 `userStatusActive` —— 唯一放行的 `users.status` 值。
pub const USER_STATUS_ACTIVE: &str = "active";

/// 对应 `initialRegisterCredit` —— 注册即赠的额度。
pub const INITIAL_REGISTER_CREDIT: f64 = 1.0;

/// 取第一个非空（trim 后）的字符串。对应 `firstNonEmpty`。
#[must_use]
pub fn first_non_empty<'a>(values: &[&'a str]) -> &'a str {
    for v in values {
        if !v.trim().is_empty() {
            return v.trim();
        }
    }
    ""
}

/// 金额是否严格为正。
///
/// 存在的理由只有一个：**NaN**。`amount <= 0.0` 对 NaN 是 `false`，于是一个 NaN
/// 金额会被当成合法输入放行，随后 `balance + NaN` 把余额彻底毁掉。正确的写法是
/// 「不满足 > 0 就拒」，而这个函数把那层否定收进来，免得每个调用点都写成
/// `!(x > 0.0)`（那个写法本身是对的，但读的人十有八九会"顺手改成" `x <= 0.0`）。
#[must_use]
pub fn is_positive_amount(amount: f64) -> bool {
    amount > 0.0
}

// ── 常用错误响应 ──────────────────────────────────────────────────────────────
//
// 只是把「HTTP 状态 + 业务码」这一对固定下来，省得每个 handler 各配各的。
// 文案仍然由调用方逐字给出 —— 同一个状态码配着十几种中文提示，
// 前端把它直接弹给用户，所以不能在这里统一。

/// `400` + [`codes::BAD_REQUEST`]。
pub fn bad_request(message: &str) -> Response {
    err(StatusCode::BAD_REQUEST, codes::BAD_REQUEST, message)
}

/// `404` + [`ERR_NOT_FOUND`]。
pub fn not_found(message: &str) -> Response {
    err(StatusCode::NOT_FOUND, ERR_NOT_FOUND, message)
}

/// `409` + [`ERR_CONFLICT`]。
pub fn conflict(message: &str) -> Response {
    err(StatusCode::CONFLICT, ERR_CONFLICT, message)
}

/// `403` + [`ERR_FORBIDDEN`]。
pub fn forbidden(message: &str) -> Response {
    err(StatusCode::FORBIDDEN, ERR_FORBIDDEN, message)
}

/// `500` + [`codes::INTERNAL`]。
pub fn internal(message: &str) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, codes::INTERNAL, message)
}

/// 记一条数据库失败日志，然后返回对应的 500 文案。
///
/// handler 一律把底层错误咽掉、只给用户一句中文；错误本身进服务端日志。
/// 保持这个分工：**别把 `sqlx::Error` 的内容放进响应体**，那是信息泄露。
pub fn db_failure(context: &str, error: &sqlx::Error, message: &str) -> Response {
    tracing::warn!(event = "panel_db_error", context = context, error = %error);
    internal(message)
}

/// 解析请求体，语义对齐 gin 的 `ShouldBindJSON`。
///
/// 三处必须一致，否则会在边界上不一致：
///
/// * **缺字段不是错误**。`json.Unmarshal` 把缺失字段留成零值，校验在之后的
///   业务判断里做（所以 `{}` 得到的是「请输入邮箱和密码」而不是「请求格式无效」）。
///   Rust 侧靠目标结构体上的 `#[serde(default)]` 复刻，这里只负责报告真正的语法/
///   类型错误。
/// * **不看 `Content-Type`**。gin 的 `ShouldBindJSON` 直接按 JSON 解，不校验头；
///   用 `axum::Json` 会因为缺 `application/json` 而 415，那是多余的行为。
/// * **空 body 是错误**。拿到 `EOF`，返回「请求格式无效」。
///
/// # Errors
/// 返回构造好的 `400` 响应，调用方直接 `?` 出去即可。
// `Response` 作为错误类型确实"大"（128 字节），但它正是 axum handler 的返回类型：
// 装箱只会在每个调用点多一次解包，换不来任何东西。
#[allow(clippy::result_large_err)]
pub fn parse_json_body<T: DeserializeOwned>(body: &[u8], message: &str) -> Result<T, Response> {
    serde_json::from_slice(body).map_err(|_| bad_request(message))
}

/// 身份域的路由表。
///
/// 对齐注册路由（`RegisterAuthRoutes` / `RegisterUserRoutes` /
/// `RegisterAdminRoutes`）中属于本域的那些行。`/auth/register` 与 `/auth/login`
/// 提取器（未登录即可访问），其余全部带；管理员路由带 [`crate::AdminUser`]。
///
/// 注意 axum 0.8 的路径参数是 `{id}` 而不是 `:id`。
pub fn router() -> Router<PanelState> {
    Router::new()
        // ── auth（无需登录，按 IP 限流） ──
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .merge(dsh_oauth::router())
        // ── 用户自助 ──
        .route("/user/profile", get(users::profile))
        .route(
            "/user/api-keys",
            get(apikey::list_own).post(apikey::create_own),
        )
        .route("/user/api-keys/{id}", delete(apikey::delete_own))
        .route("/user/api-keys/{id}/group", patch(apikey::rebind_group))
        .route("/user/available-groups", get(groups::available))
        // ── 管理员：用户 ──
        .route(
            "/admin/users",
            get(users::admin_list).post(users::admin_create),
        )
        .route(
            "/admin/users/{id}",
            put(users::admin_update).delete(users::admin_delete),
        )
        .route("/admin/users/{id}/deposit", post(users::admin_deposit))
        .route(
            "/admin/users/{id}/api-keys",
            get(apikey::admin_list_for_user),
        )
        // ── 管理员：分组（即 subscription_packages） ──
        .route(
            "/admin/groups",
            get(groups::admin_list).post(groups::admin_create),
        )
        .route(
            "/admin/groups/{id}",
            put(groups::admin_update).delete(groups::admin_delete),
        )
        .layer(axum::middleware::from_fn(auth::trace_id_layer))
}

/// Consent page for DeepSeek Harness device-code login. Mounted at the app root
/// (not under `/api/panel`) so the verification URL stays short.
pub fn public_router() -> Router<PanelState> {
    dsh_oauth::public_router()
}

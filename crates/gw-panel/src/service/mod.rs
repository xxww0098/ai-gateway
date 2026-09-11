//! 服务面 `/api/service` —— ozon-pod 集成契约（`docs/service-api.md`）的实现。
//!
//! # 这是什么
//!
//! ozon-pod 为每个 Workspace 建一个 AGW 账号、拿一个 API Key、读余额与消费明细，
//! 并把用户在 ozon-pod 支付得到的钱幂等入账。这五件事都在这里；`/v1/*` 代理与结算
//! 时序**不在**本域，也不受本域影响。
//!
//! # 三条不可违反的规则
//!
//! * **挂载是有条件的**：`service.token` 为空时 `/api/service/**` 整组不存在
//!   （404），而不是"不带凭证也放行"。装配在 [`crate::router`]。
//! * **鉴权是常量时间比较**：每个 handler 都带 [`ServiceCaller`] 提取器；缺凭证与
//!   凭证错误都返回同一个 `401 + 1001`，不泄露是哪一步失败的。
//! * **金额只以 decimal 字符串出网**：8 位小数，见 [`amount_str`]。库里的 `numeric`
//!   在 SQL 里显式 `::float8` 读出来，响应里再格式化，绝不让调用方看到浮点字面量。
//!
//! # 幂等
//!
//! 三个写操作各有各的幂等身份，都是"重放安全"而不是"只许调用一次"：
//!
//! | 端点 | 幂等身份 | 重复时 |
//! | --- | --- | --- |
//! | `POST /users` | `users.email` 唯一索引 | `created:false` + 既有 `user_id` |
//! | `POST /users/{id}/keys` | `(user_id, name, active)` | 先撤销同名的 active key 再插入 |
//! | `POST /users/{id}/credits` | `balance_logs.reference = service_credit:<key>` | `applied:false, duplicate:true` |
//!
//! # 子模块
//!
//! | 模块 | 端点 |
//! | --- | --- |
//! | [`users`] | `POST /users` |
//! | [`keys`] | `POST /users/{user_id}/keys` |
//! | [`balance`] | `GET /users/{user_id}/balance` |
//! | [`usage`] | `GET /users/{user_id}/usage`、`.../usage/by-idempotency-key/{key}` |
//! | [`credits`] | `POST /users/{user_id}/credits` |
//! | [`models`] | `GET /models` |

pub mod balance;
pub mod credits;
pub mod keys;
pub mod models;
pub mod usage;
pub mod users;

#[cfg(test)]
mod tests;

use axum::Router;
use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::Response;
use axum::routing::{get, post};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::de::DeserializeOwned;

use gw_infra::Db;

use crate::identity::{ERR_CONFLICT, ERR_NOT_FOUND};
use crate::{PanelState, bearer_token, codes, err};

/// 通过服务面鉴权的调用方。
///
/// 它只证明"请求带着正确的共享 token"，**没有**用户身份：服务面的每个端点都作用
/// 于 URL 里的 `user_id`，鉴权只发生在 ozon-pod 与网关之间。把它写进 handler 的
/// 参数表，就等价于给这条路由挂上了鉴权中间件。
#[derive(Debug, Clone, Copy)]
pub struct ServiceCaller;

impl FromRequestParts<PanelState> for ServiceCaller {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &PanelState,
    ) -> Result<Self, Self::Rejection> {
        let Some(token) = bearer_token(&parts.headers) else {
            return Err(unauthorized());
        };
        // 请求头两侧的空白由共享的 `bearer_token` 统一裁掉，配置侧必须按同一规则
        // 裁剪，否则一个尾部带空格的 token 永远匹配不上（写它的人也看不出来）。
        // token **内部**的空白仍是秘密的一部分。
        let expected = state.cfg.service.token.trim().as_bytes();
        // 空 token 永远不通过：路由本来就不该挂载，这里是纵深防御。
        if expected.is_empty() || !constant_time_eq(token.as_bytes(), expected) {
            return Err(unauthorized());
        }
        Ok(ServiceCaller)
    }
}

/// 常量时间的字节比较。
///
/// 长度不等立刻返回 `false` —— 长度不是秘密（攻击者能随便试），而先按长度短路
/// 才能保证下面的循环只在"长度相同"这一种情况下跑。相同长度时逐字节累积差异，
/// 不因第一个不等的字节提前返回，所以比较耗时与"猜对了几个前缀字节"无关。
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

/// 缺凭证 / 凭证不匹配统一的响应：`401` + `1001`。
///
/// 两种情况刻意不可区分（契约 §2）：调用方只需要知道"没通过"，不需要知道是
/// 哪一步没通过。
#[must_use]
pub fn unauthorized() -> Response {
    err(
        StatusCode::UNAUTHORIZED,
        codes::MIDDLEWARE_UNAUTHORIZED,
        "unauthorized",
    )
}

/// `400` + `4000`。
#[must_use]
pub fn bad_request(message: &str) -> Response {
    err(StatusCode::BAD_REQUEST, codes::BAD_REQUEST, message)
}

/// `404` + `4004`。
#[must_use]
pub fn not_found(message: &str) -> Response {
    err(StatusCode::NOT_FOUND, ERR_NOT_FOUND, message)
}

/// `409` + `4009`。
#[must_use]
pub fn conflict(message: &str) -> Response {
    err(StatusCode::CONFLICT, ERR_CONFLICT, message)
}

/// `500` + `5000`。
#[must_use]
pub fn internal(message: &str) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, codes::INTERNAL, message)
}

/// 记一条数据库失败日志，再返回 500。
///
/// 与面板同一条分工：底层错误进日志，响应体只给一句中文，绝不把 `sqlx::Error`
/// 的内容透出去。
#[must_use]
pub fn db_failure(context: &str, error: &sqlx::Error, message: &str) -> Response {
    tracing::warn!(event = "service_db_error", context = context, error = %error);
    internal(message)
}

/// 解析 JSON 请求体；缺字段走 `#[serde(default)]`，语法/类型错误才是 400。
///
/// # Errors
/// 返回构造好的 `400` 响应，调用方直接 `?` 出去。
#[allow(clippy::result_large_err)]
pub fn parse_json_body<T: DeserializeOwned>(body: &[u8], message: &str) -> Result<T, Response> {
    serde_json::from_slice(body).map_err(|_| bad_request(message))
}

/// 金额的线上格式：8 位小数的 decimal 字符串（契约 §2），如 `"10.00000000"`。
///
/// 负零（`-0.0`，以及四舍五入到 8 位后变成负零的极小负数）归一成 `"0.00000000"`：
/// 展示层的负零没有意义，而 `format!` 会把它渲染成 `"-0.00000000"`。
#[must_use]
pub fn amount_str(value: f64) -> String {
    let rendered = format!("{value:.8}");
    if rendered == "-0.00000000" {
        "0.00000000".to_owned()
    } else {
        rendered
    }
}

/// 时间的线上格式：秒精度 RFC3339 UTC（契约 §2 的 `2026-09-11T10:00:00Z`）。
///
/// 刻意丢掉亚秒位：契约给的是这个形状，而用量列表的顺序由 `(created_at, id)`
/// 决定，不靠小数秒区分同一秒内的两行。
#[must_use]
pub fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// `user_id` 是否存在。
///
/// `/users/{user_id}/...` 的每个端点都要先过这一关：未知 user 是 `404 + 4004`，
/// 而不是"空列表"或"0 余额" —— 后者会把一个拼错的 id 伪装成正常结果。
///
/// # Errors
/// 查询失败。
pub async fn user_exists(pg: &Db, user_id: i64) -> Result<bool, sqlx::Error> {
    let found: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1 LIMIT 1")
        .bind(user_id)
        .fetch_optional(pg)
        .await?;
    Ok(found.is_some())
}

/// 服务面路由表。
///
/// 路径都相对于 `/api/service` 前缀 —— 前缀由 [`crate::router`] 在 `service.token`
/// 非空时挂载（契约 §2）。这里绝不写绝对路径，否则改前缀会漏掉一半路由。
pub fn router() -> Router<PanelState> {
    Router::new()
        .route("/users", post(users::provision))
        .route("/users/{user_id}/keys", post(keys::rotate))
        .route("/users/{user_id}/balance", get(balance::show))
        .route("/users/{user_id}/usage", get(usage::list))
        .route(
            "/users/{user_id}/usage/by-idempotency-key/{key}",
            get(usage::by_idempotency_key),
        )
        .route("/users/{user_id}/credits", post(credits::create))
        .route("/models", get(models::list))
}

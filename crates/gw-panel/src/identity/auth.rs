//! 面板鉴权：注册 / 登录 / 全端登出，以及 [`AuthUser`] / [`AdminUser`] 两个提取器。
//!
//! 对应既有实现的 `handler_auth` + `middleware` 的鉴权部分。
//!
//! # Gin 的中间件在 axum 里变成了提取器
//!
//! 旧实现把 `AuthMiddleware()` 挂在 `/api/panel` 组上，往 `*gin.Context` 里塞一个
//! `BillingCtx`，handler 再用 `requireBillingCtx` 取出来。axum 里同样的东西是
//! `FromRequestParts`：handler 的参数表里写上 `AuthUser`，就等价于旧实现的
//! "middleware + requireBillingCtx" 两步，而且**忘记写会编译不过**，比旧实现的
//! 「忘了挂中间件」安全一档。
//!
//! [`crate::AuthUser`] / [`crate::AdminUser`] 的类型在 crate 根、`FromRequestParts`
//! 实现在 [`crate::auth`]（归 `panel-upstream`）。本文件只消费它们，**不重复实现**
//! （规则 1.9：一个概念只声明一处）。这里剩下的是身份域自己的三个 handler、
//! 未认证端点的按 IP 限流，以及链路 id 中间件。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use axum::body::Bytes;
use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use gw_authcore::{TokenVersionStore, generate_jwt_with_version};

use super::oplog::{ReqMeta, TRACE_ID_HEADER};
use super::{
    ERR_MW_RATE_LIMIT, INITIAL_REGISTER_CREDIT, USER_STATUS_ACTIVE, bad_request, conflict,
    db_failure, internal, parse_json_body,
};
use crate::audit::Actor;
use crate::{AuthUser, PanelState, codes, err, ok};

#[cfg(test)]
mod tests;

/// 对应 `authRateLimitPerMin` —— 未认证的 auth 端点上，每 IP 每分钟的硬上限。
/// 比面板通用限流低一个数量级，因为这里是撞库和刷注册的入口。
pub const AUTH_RATE_LIMIT_PER_MIN: i64 = 15;

// ── 请求 / 响应体 ────────────────────────────────────────────────────────────

/// 对应 `authRequest`。`#[serde(default)]` 复刻旧实现「缺字段 = 零值，不是解析错误」。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AuthRequest {
    email: String,
    password: String,
}

/// 对应 `authUserResponse`。
///
/// `created_at` 是**字符串**而不是时间戳对象：旧实现用
/// `CreatedAt.Format("2006-01-02T15:04:05Z07:00")` 手工格式化成秒精度的 RFC3339，
/// 与其他 handler 直接输出 `time.Time`（纳秒精度）不同。前端 `AuthUser.created_at`
/// 声明的就是 `string`，所以这里不能"顺手统一"成 `DateTime<Utc>`。
#[derive(Debug, Serialize)]
pub struct AuthUserPayload {
    pub id: i64,
    pub email: String,
    pub role: String,
    pub balance: f64,
    pub status: String,
    pub created_at: String,
}

/// 一行 `users`，只取响应需要的列。
#[derive(Debug, sqlx::FromRow)]
struct AuthUserRow {
    id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    email: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    role: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    balance: f64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
}

impl AuthUserRow {
    fn into_payload(self) -> AuthUserPayload {
        AuthUserPayload {
            id: self.id,
            email: self.email,
            role: self.role,
            balance: self.balance,
            status: self.status,
            created_at: legacy_rfc3339(self.created_at),
        }
    }
}

/// 对应 `"2006-01-02T15:04:05Z07:00"`，即秒精度的 RFC3339。
///
/// UTC 时区在旧实现里渲染成 `Z`；chrono 的 `SecondsFormat::Secs` + `use_z = true`
/// 产出同一串。
#[must_use]
pub fn legacy_rfc3339(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ── 未认证端点的按 IP 限流 ────────────────────────────────────────────────────

#[derive(Debug)]
struct RateBucket {
    /// 当前分钟窗口（Unix 分钟数）。
    window: i64,
    used: i64,
    capacity: i64,
}

/// 进程内的限流桶表。对应旧实现的 `var userLimiters sync.Map`。
///
/// 和旧实现一样是**单进程**的：多副本部署必须在前面放一层共享（Redis）限流，
/// 否则上限会被副本数乘开。旧实现的注释写了同一件事。
static AUTH_LIMITERS: LazyLock<Mutex<HashMap<String, RateBucket>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 超过这个规模就顺手清一次过期桶。
///
/// 旧实现用一个每 5 分钟醒一次的 goroutine 做同样的事。这里换成按需清理：没有常驻
/// 任务要托管，也就没有"进程退出时这个 goroutine 去哪了"的问题；代价是清理时机
/// 由流量决定，而不是时钟。桶只在被撞过的 IP 上存在，所以规模天然有界。
const LIMITER_SWEEP_THRESHOLD: usize = 1024;

fn unix_minute(now: DateTime<Utc>) -> i64 {
    now.timestamp().div_euclid(60)
}

/// 对应旧实现的 `allowRequest`。`capacity <= 0` 一律拒绝（与旧实现同）。
fn allow_request_at(identity: &str, capacity: i64, now: DateTime<Utc>) -> bool {
    if capacity <= 0 {
        return false;
    }
    let identity = if identity.is_empty() {
        "anonymous"
    } else {
        identity
    };
    let minute = unix_minute(now);

    let mut table = match AUTH_LIMITERS.lock() {
        Ok(guard) => guard,
        // 一个 panic 过的持有者不该把整个登录入口锁死。旧实现的 sync.Map 没有中毒
        // 概念，这里取回内层数据继续用，行为与旧实现一致。
        Err(poisoned) => poisoned.into_inner(),
    };

    if table.len() > LIMITER_SWEEP_THRESHOLD {
        table.retain(|_, b| b.window >= minute);
    }

    let bucket = table.entry(identity.to_owned()).or_insert(RateBucket {
        window: minute,
        used: 0,
        capacity,
    });
    if bucket.window != minute {
        bucket.window = minute;
        bucket.used = 0;
        bucket.capacity = capacity;
    }
    if bucket.used >= bucket.capacity {
        return false;
    }
    bucket.used += 1;
    true
}

/// 对应旧实现的 `AuthRateLimitMiddleware`：身份前缀 `auth:` + 调用方 IP。
///
/// 前缀是有意的：旧实现的注释说它保证这些桶「不会和将来任何限流面撞车」。
fn allow_auth_attempt(client_ip: &str) -> bool {
    let identity = format!("auth:{client_ip}");
    allow_request_at(&identity, AUTH_RATE_LIMIT_PER_MIN, Utc::now())
}

fn rate_limited() -> Response {
    err(
        StatusCode::TOO_MANY_REQUESTS,
        ERR_MW_RATE_LIMIT,
        "too many authentication attempts; please retry shortly",
    )
}

// ── 链路 id ──────────────────────────────────────────────────────────────────

/// 对应旧实现的 `TraceIDMiddleware`：透传或生成 `X-Trace-ID`，并回写到响应头。
///
/// 幂等 —— 请求头已有值就原样沿用，所以即便被套两层也只会有一个 id。
pub async fn trace_id_layer(mut request: Request, next: Next) -> Response {
    let existing = request
        .headers()
        .get(TRACE_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let trace_id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if let Ok(value) = HeaderValue::from_str(&trace_id) {
        request.headers_mut().insert(TRACE_ID_HEADER, value.clone());
        let mut response = next.run(request).await;
        response.headers_mut().insert(TRACE_ID_HEADER, value);
        return response;
    }
    next.run(request).await
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// `POST /auth/register` —— 建号、送初始额度、签发 JWT。
///
/// Ports `RegisterHandler`（含 `AuthRateLimitMiddleware`）。
pub async fn register(
    State(state): State<PanelState>,
    ReqMeta(meta): ReqMeta,
    body: Bytes,
) -> Response {
    if !allow_auth_attempt(&meta.ip_address) {
        return rate_limited();
    }
    let req: AuthRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };

    let email = req.email.trim().to_lowercase();
    if !valid_auth_input(&email, &req.password) {
        return bad_request("请输入邮箱和密码（密码至少 8 位）");
    }

    let password = req.password.clone();
    let hash =
        match tokio::task::spawn_blocking(move || gw_authcore::hash_password(&password)).await {
            Ok(Ok(hash)) => hash,
            _ => return internal("密码处理失败，请稍后重试"),
        };

    let now = Utc::now();
    let created: Result<AuthUserRow, sqlx::Error> = sqlx::query_as(
        "INSERT INTO users (email, password_hash, role, username, balance, status, concurrency, created_at, updated_at) \
         VALUES ($1, $2, 'user', '', 0, $3, 1, $4, $4) \
         RETURNING id, email, role, balance, status, created_at",
    )
    .bind(&email)
    .bind(&hash)
    .bind(USER_STATUS_ACTIVE)
    .bind(now)
    .fetch_one(&state.pg)
    .await;

    let mut row = match created {
        Ok(row) => row,
        Err(error) => {
            let duplicate = error
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_unique_violation);
            let (status, reason, message) = if duplicate {
                (StatusCode::CONFLICT.as_u16(), "duplicate", "该邮箱已被注册")
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    "create_failed",
                    "创建用户失败，请稍后重试",
                )
            };
            super::oplog::record(
                &state,
                &meta,
                None,
                "auth.register",
                &format!("user:{email}"),
                i32::from(status),
                Some(serde_json::json!({ "reason": reason })),
            )
            .await;
            return if duplicate {
                conflict(message)
            } else {
                internal(message)
            };
        }
    };

    // 旧实现的 createUserWithInitialCredit：账本在位时先建行、再 Credit。**注意
    // Credit 失败时用户行已经建出来了** —— 旧实现也一样（错误一路冒到
    // "创建用户失败"），照抄，不要在这里补一个删除，那会引入一条旧实现没有的路径。
    if let Err(error) = state
        .ledger
        .credit(row.id, INITIAL_REGISTER_CREDIT, "initial_register_credit")
        .await
    {
        tracing::warn!(event = "initial_credit_failed", user_id = row.id, error = %error);
        super::oplog::record(
            &state,
            &meta,
            None,
            "auth.register",
            &format!("user:{email}"),
            i32::from(StatusCode::INTERNAL_SERVER_ERROR.as_u16()),
            Some(serde_json::json!({ "reason": "create_failed" })),
        )
        .await;
        return internal("创建用户失败，请稍后重试");
    }
    row.balance = INITIAL_REGISTER_CREDIT;

    // 一次性管理员引导：只有当这就是配置里那个邮箱、且系统里还没有任何管理员时
    // 才生效。就地改写 role，让本次响应和随后的路由立刻反映管理员身份。
    if super::bootstrap::maybe_bootstrap_admin(&state, row.id, &row.email).await {
        row.role = "admin".to_owned();
    }

    // 新账号还没有 user_token_versions 行，版本恒为 0。
    let token = match generate_jwt_with_version(
        row.id,
        &row.email,
        &state.cfg.auth.jwt.secret,
        i64::from(state.cfg.auth.jwt.expiry_hours),
        0,
    ) {
        Ok(token) => token,
        Err(error) => {
            tracing::warn!(event = "jwt_sign_failed", user_id = row.id, error = %error);
            return internal("登录令牌生成失败，请稍后重试");
        }
    };

    let actor = Actor {
        user_id: row.id,
        email: row.email.clone(),
        role: row.role.clone(),
    };
    let target = format!("user:{}", row.id);
    super::oplog::record(
        &state,
        &meta,
        Some(&actor),
        "auth.register",
        &target,
        i32::from(StatusCode::OK.as_u16()),
        None,
    )
    .await;

    ok(serde_json::json!({ "token": token, "user": row.into_payload() }))
}

/// `POST /auth/login` —— 校验口令、带上当前会话版本签发 JWT。
///
/// Ports `LoginHandler`（含 `AuthRateLimitMiddleware`）。
pub async fn login(
    State(state): State<PanelState>,
    ReqMeta(meta): ReqMeta,
    body: Bytes,
) -> Response {
    if !allow_auth_attempt(&meta.ip_address) {
        return rate_limited();
    }
    let req: AuthRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };

    let email = req.email.trim().to_lowercase();
    if !valid_auth_input(&email, &req.password) {
        return bad_request("请输入邮箱和密码（密码至少 8 位）");
    }

    // 旧实现的查询把 status 也放进 WHERE：非 active 的账号与"不存在"给出同一个
    // 401，不泄露账号是否被停用。
    let found: Result<Option<(AuthUserRow, String)>, sqlx::Error> =
        sqlx::query_as::<_, AuthUserRowWithHash>(
            "SELECT id, email, role, balance, status, created_at, password_hash \
         FROM users WHERE email = $1 AND status = $2 LIMIT 1",
        )
        .bind(&email)
        .bind(USER_STATUS_ACTIVE)
        .fetch_optional(&state.pg)
        .await
        .map(|opt| opt.map(AuthUserRowWithHash::split));

    let (row, password_hash) = match found {
        Ok(Some(pair)) => pair,
        Ok(None) => {
            super::oplog::record(
                &state,
                &meta,
                None,
                "auth.login",
                &format!("user:{email}"),
                i32::from(StatusCode::UNAUTHORIZED.as_u16()),
                Some(serde_json::json!({ "reason": "not_found" })),
            )
            .await;
            return err(
                StatusCode::UNAUTHORIZED,
                codes::UNAUTHORIZED,
                "邮箱或密码错误",
            );
        }
        Err(error) => {
            return db_failure("login_lookup", &error, "服务暂不可用，请稍后重试");
        }
    };

    let candidate = req.password.clone();
    let verified = tokio::task::spawn_blocking(move || {
        gw_authcore::verify_password(&candidate, &password_hash).unwrap_or(false)
    })
    .await
    .unwrap_or(false);

    let actor = Actor {
        user_id: row.id,
        email: row.email.clone(),
        role: row.role.clone(),
    };
    let target = format!("user:{}", row.id);

    if !verified {
        super::oplog::record(
            &state,
            &meta,
            Some(&actor),
            "auth.login",
            &target,
            i32::from(StatusCode::UNAUTHORIZED.as_u16()),
            Some(serde_json::json!({ "reason": "bad_password" })),
        )
        .await;
        return err(
            StatusCode::UNAUTHORIZED,
            codes::UNAUTHORIZED,
            "邮箱或密码错误",
        );
    }

    // 把当前会话版本嵌进 token，这样之后的全端登出/强制吊销能作废它。
    let version = match TokenVersionStore::new(state.pg.clone())
        .current(row.id)
        .await
    {
        Ok(version) => version,
        Err(error) => {
            tracing::warn!(event = "token_version_lookup_failed", user_id = row.id, error = %error);
            return internal("服务暂不可用，请稍后重试");
        }
    };
    let token = match generate_jwt_with_version(
        row.id,
        &row.email,
        &state.cfg.auth.jwt.secret,
        i64::from(state.cfg.auth.jwt.expiry_hours),
        version,
    ) {
        Ok(token) => token,
        Err(error) => {
            tracing::warn!(event = "jwt_sign_failed", user_id = row.id, error = %error);
            return internal("登录令牌生成失败，请稍后重试");
        }
    };

    let role = row.role.clone();
    super::oplog::record(
        &state,
        &meta,
        Some(&actor),
        "auth.login",
        &target,
        i32::from(StatusCode::OK.as_u16()),
        Some(serde_json::json!({ "role": role })),
    )
    .await;

    ok(serde_json::json!({ "token": token, "user": row.into_payload() }))
}

/// `POST /auth/logout` —— 递增会话版本，作废该用户手上的**每一个** JWT。
///
/// Ports `LogoutHandler`。这是"全端登出 / 令牌被偷"的杀死开关，不是清一下本地
/// 存储：递增之后，任何在此之前签发的 token 在下一次请求就会被拒。
pub async fn logout(
    State(state): State<PanelState>,
    ReqMeta(meta): ReqMeta,
    user: AuthUser,
) -> Response {
    if let Err(error) = TokenVersionStore::new(state.pg.clone())
        .bump(user.user_id)
        .await
    {
        tracing::warn!(event = "token_version_bump_failed", user_id = user.user_id, error = %error);
        return internal("退出登录失败，请稍后重试");
    }
    let actor = Actor {
        user_id: user.user_id,
        email: user.email.clone(),
        role: user.role.clone(),
    };
    super::oplog::record(
        &state,
        &meta,
        Some(&actor),
        "auth.logout",
        &format!("user:{}", user.user_id),
        i32::from(StatusCode::OK.as_u16()),
        None,
    )
    .await;
    ok(serde_json::json!({ "ok": true }))
}

/// Ports `validAuthInput`。
///
/// 刻意宽松：只要求有 `@` 和 8 个**字节**的口令。旧实现用的是 `len(password)`
/// （字节数，不是字符数），改成 `chars().count()` 会让一部分现有口令在 Rust 下
/// 通不过校验。
#[must_use]
pub fn valid_auth_input(email: &str, password: &str) -> bool {
    !email.is_empty() && email.contains('@') && password.len() >= 8
}

/// 登录查询多带一列口令哈希；拆成两半后哈希立刻丢给校验线程，不进响应体。
#[derive(Debug, sqlx::FromRow)]
struct AuthUserRowWithHash {
    id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    email: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    role: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    balance: f64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
    #[sqlx(try_from = "gw_model::compat::Text")]
    password_hash: String,
}

impl AuthUserRowWithHash {
    fn split(self) -> (AuthUserRow, String) {
        (
            AuthUserRow {
                id: self.id,
                email: self.email,
                role: self.role,
                balance: self.balance,
                status: self.status,
                created_at: self.created_at,
            },
            self.password_hash,
        )
    }
}

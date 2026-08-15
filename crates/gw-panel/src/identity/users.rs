//! 用户资料（自己看）与用户管理（管理员看），以及两级缓存的失效钩子。
//!
//! 对应既有实现的 `handler_user` 的 `UserProfileHandler` + `handler_admin_expanded`
//! 的 `AdminUsers*Handler` / `adminUserPayload` / `loadAdminUser`
//! + `middleware` 的 `invalidateUserCaches`。
//!
//! # 缓存失效的顺序是安全属性
//!
//! 管理员把用户停用/删除之后，必须**先提交、再清缓存**：提交失败时缓存不能清
//! （库里那行还是 active，清了只会让下一次请求白读一次库）；提交成功后必须清，
//! 否则一个陈旧的 "active" 条目会让已删除用户继续通过 `/v1/*` 和 `/api/panel/**`
//! 的鉴权，直到 TTL 到期。[`invalidate_user_caches`] 里的两步顺序同理。

use axum::extract::{Path, Query, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::auth::{AuthUserPayload, legacy_rfc3339};
use super::{
    USER_STATUS_ACTIVE, bad_request, db_failure, first_non_empty, internal, is_positive_amount,
    not_found, parse_json_body,
};
use crate::audit::Actor;
use crate::identity::oplog::ReqMeta;
use crate::paging::{ListPage, offset, page_params, parse_id};
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 管理员列表的默认页大小。对应 `queryInt(c, "page_size", 15, 1, 100)`。
const ADMIN_USERS_DEFAULT_PAGE_SIZE: i64 = 15;

/// 管理员删除用户时写入的状态。行不删，`usage_logs` 还引用着它。
const STATUS_DELETED: &str = "deleted";

/// `balance_logs.type`：管理员直接改余额留下的痕迹。
const BALANCE_LOG_ADMIN_ADJUSTMENT: &str = "admin_adjustment";
/// `balance_logs.type`：管理员充值。
const BALANCE_LOG_ADMIN_DEPOSIT: &str = "admin_deposit";

/// 对应 `adminUserPayload`。
///
/// `username` 为空串时是 **`null`**（旧实现的 `nullableString`），不是 `""`；
/// `role` / `status` 为空时兜底成 `user` / `active`。
#[derive(Debug, Serialize)]
pub struct AdminUserPayload {
    pub id: i64,
    pub email: String,
    pub username: Option<String>,
    pub role: String,
    pub balance: f64,
    pub status: String,
    pub concurrency: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AdminUserRow {
    id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    email: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    username: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    role: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    balance: f64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    #[sqlx(try_from = "gw_model::compat::Int")]
    concurrency: i64,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    updated_at: DateTime<Utc>,
}

const ADMIN_USER_COLUMNS: &str =
    "id, email, username, role, balance, status, concurrency, created_at, updated_at";

/// 对应 `nullableString` —— trim 后为空就变成 JSON `null`。
#[must_use]
pub fn nullable_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

impl From<AdminUserRow> for AdminUserPayload {
    fn from(row: AdminUserRow) -> Self {
        Self {
            id: row.id,
            username: nullable_string(&row.username),
            role: first_non_empty(&[&row.role, "user"]).to_owned(),
            status: first_non_empty(&[&row.status, USER_STATUS_ACTIVE]).to_owned(),
            email: row.email,
            balance: row.balance,
            concurrency: row.concurrency,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// 旧实现 `AdminUsersCreateHandler` 的请求体。
///
/// 旧实现那四个字符串字段没有 json tag，`encoding/json` 做大小写不敏感匹配 ——
/// 它们都是单个词，所以前端发的 `email` / `password` / `role` / `username`
/// 正好能绑上（不像 `saveAdminGroup` 里的 `DailyLimitUSD`，见 [`super::groups`]）。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CreateUserRequest {
    #[serde(alias = "Email")]
    email: String,
    #[serde(alias = "Password")]
    password: String,
    #[serde(alias = "Role")]
    role: String,
    #[serde(alias = "Username")]
    username: String,
    balance: f64,
}

/// 旧实现 `AdminUsersUpdateHandler` 的请求体。
///
/// `balance` 是 `Option` 而不是 `f64`：旧实现特意把它做成指针，这样一次只改角色/状态
/// 的编辑**不会把余额悄悄清零**。别"简化"掉这个 Option。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UpdateUserRequest {
    role: String,
    balance: Option<f64>,
    concurrency: i64,
    status: String,
    username: Option<String>,
    password: String,
}

/// 旧实现 `AdminUsersDepositHandler` 的请求体。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DepositRequest {
    amount: f64,
    note: String,
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// `GET /user/profile` —— 自己的资料 + 可用余额。
///
/// Ports `UserProfileHandler`。`available_balance` 走账本（**已扣掉在途预扣**），
/// 与 `user.balance` 那个持久化列不是一回事：前者是"现在还能花多少"。
pub async fn profile(State(state): State<PanelState>, user: AuthUser) -> Response {
    let row: Result<Option<AdminUserRow>, _> = sqlx::query_as(&format!(
        "SELECT {ADMIN_USER_COLUMNS} FROM users WHERE id = $1 LIMIT 1"
    ))
    .bind(user.user_id)
    .fetch_optional(&state.pg)
    .await;

    let row = match row {
        Ok(Some(row)) => row,
        // 旧实现的 handleRecordError：查不到这行是 404「未找到该用户」。
        Ok(None) => return not_found("未找到该用户"),
        Err(error) => {
            return db_failure("load_profile", &error, "加载用户信息失败，请稍后重试");
        }
    };

    let available = match state.ledger.get_balance(user.user_id).await {
        Ok(balance) => balance,
        Err(error) => {
            tracing::warn!(event = "profile_balance_failed", user_id = user.user_id, error = %error);
            return internal("加载余额失败，请稍后重试");
        }
    };

    ok(serde_json::json!({
        "user": auth_payload(&row),
        "available_balance": available,
    }))
}

/// Ports `authUserFromModel` —— 与登录/注册返回的是同一个形状（秒精度字符串时间）。
fn auth_payload(row: &AdminUserRow) -> AuthUserPayload {
    AuthUserPayload {
        id: row.id,
        email: row.email.clone(),
        role: row.role.clone(),
        balance: row.balance,
        status: row.status.clone(),
        created_at: legacy_rfc3339(row.created_at),
    }
}

/// `GET /admin/users` —— 分页 + 关键字/角色/状态过滤。Ports `AdminUsersListHandler`。
pub async fn admin_list(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (page, page_size) = page_params(
        params.get("page").map(String::as_str),
        params.get("page_size").map(String::as_str),
        ADMIN_USERS_DEFAULT_PAGE_SIZE,
    );
    let q = params.get("q").map(|s| s.trim()).unwrap_or_default();
    let role = params.get("role").map(|s| s.trim()).unwrap_or_default();
    let status = params.get("status").map(|s| s.trim()).unwrap_or_default();

    // 空串代表"不过滤"。把它编码成 SQL 里的 `($n = '' OR col = $n)`，这样三个
    // 可选过滤条件不需要动态拼 SQL —— 拼串是注入的温床，而这三个值全部来自
    // 查询参数。`q` 走 ILIKE，与旧实现一致（大小写不敏感的子串匹配）。
    let filter = "($1 = '' OR email ILIKE '%' || $1 || '%' OR username ILIKE '%' || $1 || '%') \
         AND ($2 = '' OR role = $2) AND ($3 = '' OR status = $3)";

    let total: Result<i64, _> = sqlx::query_scalar(&format!(
        "SELECT COUNT(*)::bigint FROM users WHERE {filter}"
    ))
    .bind(q)
    .bind(role)
    .bind(status)
    .fetch_one(&state.pg)
    .await;
    let total = match total {
        Ok(total) => total,
        Err(error) => return db_failure("count_users", &error, "统计用户失败，请稍后重试"),
    };

    let rows: Result<Vec<AdminUserRow>, _> = sqlx::query_as(&format!(
        "SELECT {ADMIN_USER_COLUMNS} FROM users WHERE {filter} ORDER BY id DESC LIMIT $4 OFFSET $5"
    ))
    .bind(q)
    .bind(role)
    .bind(status)
    .bind(page_size)
    .bind(offset(page, page_size))
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(ListPage::new(
            rows.into_iter().map(AdminUserPayload::from).collect(),
            total,
            page,
            page_size,
        )),
        Err(error) => db_failure("list_users", &error, "获取用户列表失败，请稍后重试"),
    }
}

/// `POST /admin/users` —— 管理员建号。Ports `AdminUsersCreateHandler`。
///
/// 注意这里的余额是**直接写列**，不走账本、也不写 `balance_logs` —— 旧实现就是这样，
/// 建号时给的初始余额没有流水。
pub async fn admin_create(
    State(state): State<PanelState>,
    admin: AdminUser,
    ReqMeta(meta): ReqMeta,
    body: axum::body::Bytes,
) -> Response {
    let req: CreateUserRequest = match parse_json_body(&body, "用户信息格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if req.email.trim().is_empty() || req.password.is_empty() {
        return bad_request("用户信息格式无效");
    }

    let password = req.password.clone();
    let hash =
        match tokio::task::spawn_blocking(move || gw_authcore::hash_password(&password)).await {
            Ok(Ok(hash)) => hash,
            _ => return internal("密码处理失败，请稍后重试"),
        };

    let role = first_non_empty(&[&req.role, "user"]).to_owned();
    let now = Utc::now();
    let created: Result<AdminUserRow, _> = sqlx::query_as(&format!(
        "INSERT INTO users \
             (email, password_hash, role, username, balance, status, concurrency, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, 1, $7, $7) RETURNING {ADMIN_USER_COLUMNS}"
    ))
    .bind(req.email.trim().to_lowercase())
    .bind(&hash)
    .bind(&role)
    .bind(req.username.trim())
    .bind(req.balance)
    .bind(USER_STATUS_ACTIVE)
    .bind(now)
    .fetch_one(&state.pg)
    .await;

    match created {
        Ok(row) => {
            let payload = AdminUserPayload::from(row);
            super::oplog::record(
                &state,
                &meta,
                Some(&actor_of(&admin)),
                "admin.user.create",
                &format!("user:{}", payload.id),
                200,
                Some(serde_json::json!({ "email": payload.email, "role": payload.role })),
            )
            .await;
            ok(payload)
        }
        Err(error) => db_failure("create_user", &error, "创建用户失败，请稍后重试"),
    }
}

/// `PUT /admin/users/{id}` —— 改角色/余额/并发/状态/用户名/口令。
///
/// Ports `AdminUsersUpdateHandler`。余额如果**显式**变了，同一个事务里补一条
/// `balance_logs`，让余额列和审计流水不会分叉。
pub async fn admin_update(
    State(state): State<PanelState>,
    admin: AdminUser,
    ReqMeta(meta): ReqMeta,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的用户 ID");
    };
    let req: UpdateUserRequest = match parse_json_body(&body, "用户信息格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };

    let existing: Result<Option<AdminUserRow>, _> = sqlx::query_as(&format!(
        "SELECT {ADMIN_USER_COLUMNS} FROM users WHERE id = $1 LIMIT 1"
    ))
    .bind(id)
    .fetch_optional(&state.pg)
    .await;
    let existing = match existing {
        Ok(Some(row)) => row,
        Ok(None) => return not_found("未找到该用户"),
        Err(error) => return db_failure("load_user", &error, "加载用户信息失败，请稍后重试"),
    };

    let role = first_non_empty(&[&req.role, &existing.role, "user"]).to_owned();
    let previous_status = existing.status.clone();
    let status = first_non_empty(&[&req.status, &existing.status, USER_STATUS_ACTIVE]).to_owned();
    let username = req
        .username
        .as_ref()
        .map_or_else(|| existing.username.clone(), |u| u.trim().to_owned());

    let (balance, balance_delta) = match req.balance {
        Some(next) if (next - existing.balance).abs() > 0.0 => {
            (next, Some(next - existing.balance))
        }
        Some(next) => (next, None),
        None => (existing.balance, None),
    };

    let password_hash = if req.password.trim().is_empty() {
        None
    } else {
        let password = req.password.clone();
        match tokio::task::spawn_blocking(move || gw_authcore::hash_password(&password)).await {
            Ok(Ok(hash)) => Some(hash),
            _ => return internal("密码处理失败，请稍后重试"),
        }
    };

    let updated = update_user_tx(
        &state,
        id,
        &UserUpdate {
            role: &role,
            status: &status,
            username: &username,
            balance,
            // 旧实现无条件把 Concurrency 写成请求里的值（缺字段就是 0）。
            concurrency: req.concurrency,
            password_hash: password_hash.as_deref(),
            balance_delta,
            actor_id: admin.0.user_id,
        },
    )
    .await;

    let updated = match updated {
        Ok(Some(row)) => row,
        Ok(None) => return not_found("未找到该用户"),
        Err(error) => return db_failure("update_user", &error, "更新用户失败，请稍后重试"),
    };

    // 提交成功之后才清缓存：状态被改离 active 时，让下一次鉴权重新读库。
    if previous_status != updated.status && updated.status != USER_STATUS_ACTIVE {
        invalidate_user_caches(&state, id).await;
    }
    if balance_delta.is_some() {
        // 旧实现没有这一步，于是管理员改完余额后 Redis 里的余额缓存要等 TTL 才刷新。
        // 这里顺手刷一次：对外契约不变，只是让改动立刻可见。
        let _ = state.ledger.refresh_balance_cache(id).await;
    }

    let payload = AdminUserPayload::from(updated);
    super::oplog::record(
        &state,
        &meta,
        Some(&actor_of(&admin)),
        "admin.user.update",
        &format!("user:{id}"),
        200,
        Some(serde_json::json!({
            "status": payload.status,
            "role": payload.role,
            "previous_status": previous_status,
        })),
    )
    .await;
    ok(payload)
}

/// [`update_user_tx`] 的入参，纯粹为了不写一个九参数的函数。
struct UserUpdate<'a> {
    role: &'a str,
    status: &'a str,
    username: &'a str,
    balance: f64,
    concurrency: i64,
    password_hash: Option<&'a str>,
    balance_delta: Option<f64>,
    actor_id: i64,
}

/// 用户行 + 可选的余额流水，一个事务里写完。
async fn update_user_tx(
    state: &PanelState,
    id: i64,
    update: &UserUpdate<'_>,
) -> Result<Option<AdminUserRow>, sqlx::Error> {
    let mut tx = state.pg.begin().await?;

    let row: Option<AdminUserRow> = sqlx::query_as(&format!(
        "UPDATE users SET role = $2, username = $3, balance = $4, status = $5, \
             concurrency = $6, password_hash = COALESCE($7, password_hash), updated_at = $8 \
         WHERE id = $1 RETURNING {ADMIN_USER_COLUMNS}"
    ))
    .bind(id)
    .bind(update.role)
    .bind(update.username)
    .bind(update.balance)
    .bind(update.status)
    .bind(update.concurrency)
    .bind(update.password_hash)
    .bind(Utc::now())
    .fetch_optional(&mut *tx)
    .await?;

    if row.is_none() {
        tx.rollback().await?;
        return Ok(None);
    }

    if let Some(delta) = update.balance_delta {
        let reference = format!("admin_balance_adjust:{}", update.actor_id);
        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(delta)
        .bind(BALANCE_LOG_ADMIN_ADJUSTMENT)
        .bind(&reference)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(row)
}

/// `DELETE /admin/users/{id}` —— 软删除（`status = 'deleted'`）+ 两级缓存失效。
///
/// Ports `AdminUsersDeleteHandler`。失败路径提前返回，**不清缓存** —— 库里那行
/// 还是 active，清了只是白费一次读。
pub async fn admin_delete(
    State(state): State<PanelState>,
    admin: AdminUser,
    ReqMeta(meta): ReqMeta,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的用户 ID");
    };

    if let Err(error) = sqlx::query("UPDATE users SET status = $2, updated_at = $3 WHERE id = $1")
        .bind(id)
        .bind(STATUS_DELETED)
        .bind(Utc::now())
        .execute(&state.pg)
        .await
    {
        return db_failure("delete_user", &error, "删除用户失败，请稍后重试");
    }

    invalidate_user_caches(&state, id).await;
    super::oplog::record(
        &state,
        &meta,
        Some(&actor_of(&admin)),
        "admin.user.delete",
        &format!("user:{id}"),
        200,
        None,
    )
    .await;
    ok(serde_json::json!({ "deleted": true }))
}

/// `POST /admin/users/{id}/deposit` —— 管理员充值。Ports `AdminUsersDepositHandler`。
///
/// **金额必须严格为正**。允许负数等于用裸 SQL 做扣款，绕开账本的正数校验和
/// 余额充足校验，能把余额打到负数 —— 旧实现的注释专门写了这条，别放宽。
pub async fn admin_deposit(
    State(state): State<PanelState>,
    admin: AdminUser,
    ReqMeta(meta): ReqMeta,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的用户 ID");
    };
    let req: DepositRequest = match parse_json_body(&body, "充值金额必须大于 0") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if !is_positive_amount(req.amount) {
        return bad_request("充值金额必须大于 0");
    }

    // 旧实现先 loadAdminUser（404）再充值。
    let exists: Result<Option<i64>, _> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pg)
        .await;
    match exists {
        Ok(None) => return not_found("未找到该用户"),
        Ok(Some(_)) => {}
        Err(error) => return db_failure("load_user", &error, "加载用户信息失败，请稍后重试"),
    }

    let note = req.note.trim().to_owned();
    if let Err(error) = deposit_tx(&state, id, req.amount, &note).await {
        return db_failure("deposit", &error, "充值失败，请稍后重试");
    }
    // 与 admin_update 同理：让改动立刻对 `/v1/*` 的余额判定可见。
    let _ = state.ledger.refresh_balance_cache(id).await;

    super::oplog::record(
        &state,
        &meta,
        Some(&actor_of(&admin)),
        "admin.user.deposit",
        &format!("user:{id}"),
        200,
        Some(serde_json::json!({ "amount": req.amount, "note": note })),
    )
    .await;
    ok(serde_json::json!({ "ok": true }))
}

async fn deposit_tx(
    state: &PanelState,
    id: i64,
    amount: f64,
    note: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = state.pg.begin().await?;
    sqlx::query("UPDATE users SET balance = balance + $2, updated_at = $3 WHERE id = $1")
        .bind(id)
        .bind(amount)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(amount)
    .bind(BALANCE_LOG_ADMIN_DEPOSIT)
    .bind(note)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// 清掉某个用户在两级 L1 缓存里的痕迹。Ports `PanelRouter.invalidateUserCaches`。
///
/// 顺序照抄旧实现：
///
/// 1. **无条件**先丢 `UserStatusCache` 条目，让 JWT 路径的状态复查落回库；
/// 2. 再枚举这个用户的**每一把** key（不加 `status='active'` 过滤 —— 已经被停用
///    的 key 同样可能还躺在缓存里），逐个从 `ApiKeyCache` 里删掉。
///
/// 第 2 步的数据库失败只记日志：第 1 步已经生效，至少状态复查这条路被强制回库了。
pub async fn invalidate_user_caches(state: &PanelState, user_id: i64) {
    state.user_status_cache.invalidate_user(user_id);

    let hashes: Result<Vec<String>, _> =
        sqlx::query_scalar("SELECT key_hash FROM api_keys WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&state.pg)
            .await;
    match hashes {
        Ok(hashes) => {
            for hash in hashes {
                state.api_key_cache.delete(&hash);
            }
        }
        Err(error) => {
            tracing::warn!(
                event = "invalidate_user_caches_db_failed",
                user_id = user_id,
                error = %error,
            );
        }
    }
}

fn actor_of(admin: &AdminUser) -> Actor {
    Actor {
        user_id: admin.0.user_id,
        email: admin.0.email.clone(),
        role: admin.0.role.clone(),
    }
}

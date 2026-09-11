//! `POST /api/service/users` —— 供给账号（按 email 幂等）。
//!
//! 契约 §3.1：email 先 trim + 小写；已存在同 email 返回既有 `user_id` 与
//! `created:false` 且**不动余额、不动状态**；已存在但 `status != active` 是
//! `409 + 4009`；新行的 `role=user`、`status=active`、`concurrency=0`
//! （0 = 沿用限流器的 `max_concurrent` 配置，见迁移 0016 —— 供给账号与面板
//! 注册账号同规则），`username` 取 `display_name`。
//!
//! # 供给账号没有可用口令
//!
//! 入库的是一条 32 字节 OS 随机口令的 bcrypt 哈希。明文既不返回也不落库（契约 §2）:
//! 这里生成的随机串只作为 `hash_password` 的入参存在一个栈帧的生命周期。
//!
//! # 不赠 `initial_register_credit`
//!
//! 注册金那条路径属于面板注册（`identity::auth::register`），服务面刻意不复用它。
//! `service.initial_credit` 默认 0：非 0 时才在同一事务里走一次 `credit_tx`，
//! reference 取 `service_credit:initial:{user_id}` —— 仍是契约 §4 的命名空间，
//! 于是「一次建号只赠一次」由 0015 的部分唯一索引兜底，`balance_logs` 也保持自洽
//! （`verify_balance_integrity` 不会因此报漂移）。

use axum::extract::State;
use axum::response::Response;
use chrono::Utc;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};

use gw_infra::Db;

use super::{ServiceCaller, bad_request, conflict, db_failure, internal, parse_json_body};
use crate::identity::USER_STATUS_ACTIVE;
use crate::{PanelState, ok};

/// 契约 §3.1 的请求体。`display_name` 可省，缺字段 = 空串（不是解析错误）。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProvisionRequest {
    email: String,
    display_name: String,
}

/// 契约 §3.1 的 `data`。
#[derive(Debug, Serialize)]
pub struct ProvisionedUser {
    /// `users.id`，后续所有服务面端点都用它。
    pub user_id: i64,
    /// trim + 小写之后的邮箱，也是这次的幂等身份。
    pub email: String,
    /// 本次调用是否真的插入了新行。
    pub created: bool,
    /// 既有账号的状态原样返回；新行恒为 `active`。
    pub status: String,
}

/// `POST /api/service/users`。
pub async fn provision(
    State(state): State<PanelState>,
    _caller: ServiceCaller,
    body: axum::body::Bytes,
) -> Response {
    let req: ProvisionRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    let email = req.email.trim().to_lowercase();
    if !email.contains('@') {
        return bad_request("邮箱格式无效");
    }

    // 先查一次：命中幂等路径时连 bcrypt 都不用算（cost 10 是几十毫秒）。
    match find_by_email(&state.pg, &email).await {
        Ok(Some((user_id, status))) => return existing_account(user_id, email, status),
        Ok(None) => {}
        Err(error) => return db_failure("service_find_user", &error, "创建用户失败，请稍后重试"),
    }

    let password = match random_password() {
        Ok(password) => password,
        Err(error) => {
            tracing::warn!(event = "service_password_random_failed", error = %error);
            return internal("创建用户失败，请稍后重试");
        }
    };
    // bcrypt 是 CPU 密集的同步调用，放阻塞线程池，别占住 tokio worker。
    let hash =
        match tokio::task::spawn_blocking(move || gw_authcore::hash_password(&password)).await {
            Ok(Ok(hash)) => hash,
            _ => return internal("创建用户失败，请稍后重试"),
        };

    let initial_credit = state.cfg.service.effective_initial_credit();
    let display_name = req.display_name.trim().to_owned();
    let now = Utc::now();

    let mut tx = match state.pg.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            return db_failure("service_provision_begin", &error, "创建用户失败，请稍后重试");
        }
    };

    // `ON CONFLICT (email) DO NOTHING` 走 `idx_users_email`：两个并发的首次供给
    // 请求里只有一个能插入，另一个拿到 None 后按"已存在"回答，不会留下两个账号。
    let inserted: Result<Option<(i64, String)>, _> = sqlx::query_as(
        "INSERT INTO users \
             (email, password_hash, role, username, balance, status, concurrency, created_at, updated_at) \
         VALUES ($1, $2, 'user', $3, 0, 'active', 0, $4, $4) \
         ON CONFLICT (email) DO NOTHING \
         RETURNING id, COALESCE(status, '')",
    )
    .bind(&email)
    .bind(&hash)
    .bind(&display_name)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await;

    let mut credited = None;
    let user_id = match inserted {
        Ok(Some((user_id, _status))) => {
            if initial_credit > 0.0 {
                // 有意留在 `service_credit:` 命名空间里：索引把"只赠一次"钉住。
                let reference = gw_ledger::service_credit_reference(&format!("initial:{user_id}"));
                match state
                    .ledger
                    .credit_tx(&mut tx, user_id, initial_credit, &reference)
                    .await
                {
                    Ok(gw_ledger::BalanceChange::Applied {
                        balance_after,
                        balance_version,
                    }) => credited = Some((balance_after, balance_version)),
                    // 刚插入的账号不可能已有流水；真出现也只能说明有人抢了同一个
                    // reference，此时不动余额才是对的。
                    Ok(gw_ledger::BalanceChange::AlreadyApplied) => {}
                    Err(error) => {
                        let _ = tx.rollback().await;
                        tracing::warn!(event = "service_initial_credit_failed", %error);
                        return internal("创建用户失败，请稍后重试");
                    }
                }
            }
            user_id
        }
        Ok(None) => {
            // 并发竞态：查完到插入之间另一个请求建好了同一个 email。
            let _ = tx.rollback().await;
            return match find_by_email(&state.pg, &email).await {
                Ok(Some((user_id, status))) => existing_account(user_id, email, status),
                Ok(None) => internal("创建用户失败，请稍后重试"),
                Err(error) => {
                    db_failure("service_find_user", &error, "创建用户失败，请稍后重试")
                }
            };
        }
        Err(error) => {
            let _ = tx.rollback().await;
            return db_failure("service_insert_user", &error, "创建用户失败，请稍后重试");
        }
    };

    if let Err(error) = tx.commit().await {
        return db_failure("service_provision_commit", &error, "创建用户失败，请稍后重试");
    }
    if let Some((balance_after, balance_version)) = credited {
        let _ = state
            .ledger
            .publish_balance(user_id, balance_after, balance_version)
            .await;
    }

    ok(ProvisionedUser {
        user_id,
        email,
        created: true,
        status: USER_STATUS_ACTIVE.to_owned(),
    })
}

/// 既有账号的响应。`status != active` 是 409/4009，不是"顺手把它改回 active"
/// （契约 §3.1：不改余额、不改状态）。
fn existing_account(user_id: i64, email: String, status: String) -> Response {
    if status != USER_STATUS_ACTIVE {
        return conflict("该邮箱已存在且状态不是 active");
    }
    ok(ProvisionedUser {
        user_id,
        email,
        created: false,
        status,
    })
}

/// 按规范化后的 email 精确匹配 `(id, status)`。
///
/// 精确匹配而不是 `lower(email)`：写入路径已经统一小写（面板注册与服务面都是），
/// 而 `idx_users_email` 是精确唯一索引 —— 用函数包住列会让索引失效，且两处对
/// "同一个 email"的判定会分叉。
///
/// # Errors
/// 查询失败。
async fn find_by_email(pg: &Db, email: &str) -> Result<Option<(i64, String)>, sqlx::Error> {
    sqlx::query_as("SELECT id, COALESCE(status, '') FROM users WHERE email = $1 LIMIT 1")
        .bind(email)
        .fetch_optional(pg)
        .await
}

/// 32 字节 OS 随机的十六进制口令。
///
/// 长度 64 字节，落在 bcrypt 的 72 字节上限内。返回值立刻被哈希消费，绝不外传。
///
/// # Errors
/// 系统随机源不可用。
fn random_password() -> Result<String, rand::Error> {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.try_fill_bytes(&mut bytes)?;
    Ok(hex::encode(bytes))
}

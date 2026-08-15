//! 兑换码：生成、列表、删除，以及用户兑换。
//!
//! 对应 `generateRedeemCode` / `UserRedeemHandler` / `AdminListRedeemCodes` /
//! `AdminCreateRedeemCodes` / `AdminDeleteRedeemCode`。
//!
//! # 兑换是「先认领、再入账、失败回滚」
//!
//! ```text
//! UPDATE redeem_codes SET status='used' … WHERE id=? AND status='unused'
//!   rows_affected == 0 -> 已被别人用掉，400
//!   rows_affected == 1 -> 我抢到了 -> Credit
//!        Credit 失败 -> 把认领**退回去**（并且只退回我自己那一次）
//! ```
//!
//! 条件 UPDATE 是跨副本安全的；先读后写不是。回滚那句 `AND used_by_id = $2` 也
//! 是必要的：没有它，一次失败的兑换会把别人刚刚成功的认领擦掉。
//!
//! # 码必须不可猜
//!
//! `generate_redeem_code` 取 16 字节 CSPRNG 再 base32 —— 顺序生成的码等于把余额
//! 送人（安全回归测试 `TestRedeemCodesAreRandomAndSequentialGuessFails` 盯这条）。

use axum::extract::{Path, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};

use gw_infra::Db;

use crate::identity::{
    bad_request, db_failure, internal, is_positive_amount, not_found, parse_json_body,
};
use crate::paging::parse_id;
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 一次最多生成多少张。Ports `req.Count > 100` 的上限。
const MAX_BATCH: i64 = 100;

/// 随机源取多少字节。16 字节 = 128 bit，穷举不可行。
const CODE_ENTROPY_BYTES: usize = 16;

/// 码的可见前缀，纯粹是给人看的。
const CODE_PREFIX: &str = "RDM-";

const STATUS_UNUSED: &str = "unused";
const STATUS_USED: &str = "used";

/// 对应 `redeemCodeRecord`。`used_at` / `used_by` 带 `omitempty`（未使用时键消失）。
#[derive(Debug, Serialize)]
pub struct RedeemCodeRecord {
    pub id: i64,
    pub code: String,
    pub amount: f64,
    pub status: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_by: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct RedeemCodeRow {
    id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    code: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    amount: f64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    used_at: Option<DateTime<Utc>>,
    used_by: Option<String>,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
}

const REDEEM_COLUMNS: &str = "id, code, amount, status, used_at, used_by, created_at";

impl From<RedeemCodeRow> for RedeemCodeRecord {
    fn from(row: RedeemCodeRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            amount: row.amount,
            status: row.status,
            created_at: row.created_at,
            used_at: row.used_at,
            used_by: row.used_by,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedeemRequest {
    code: String,
}

/// 对应 `createRedeemCodeRequest`。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CreateRequest {
    count: i64,
    amount: f64,
}

/// RFC 4648 的标准 base32 字母表。**全大写是这里的硬要求**：兑换时用户输入会先
/// 过 `to_uppercase()`，字母表里一旦出现小写字符，编码就不再是单射的 ——
/// 两串不同的随机字节会映射到同一个码，撞上 `redeem_codes.code` 的唯一索引。
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// 无填充的标准 base32。对齐旧实现采用的
/// 标准 base32 无填充编码，所以生成出来的码与既有实现
/// 逐字符同形（16 字节 → 26 个字符）。
fn base32_no_pad(raw: &[u8]) -> String {
    let mut out = String::with_capacity(raw.len().div_ceil(5) * 8);
    // 最多攒 8 + 4 = 12 位，u32 绰绰有余。
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in raw {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(
                BASE32_ALPHABET[((buffer >> bits) & 0x1F) as usize],
            ));
        }
    }
    if bits > 0 {
        // 末尾不足 5 位的部分左移补零 —— 无填充编码就是这样收尾的。
        out.push(char::from(
            BASE32_ALPHABET[((buffer << (5 - bits)) & 0x1F) as usize],
        ));
    }
    out
}

/// 生成一张不可猜的兑换码。Ports `generateRedeemCode`。
///
/// 16 字节 CSPRNG + 无填充 base32，与既有实现同形。**随机源失败时报错，绝不退化**成
/// 计数器或时间戳 —— 可预测的兑换码等于把余额送人。
///
/// # Errors
/// 系统随机源不可用。
pub fn generate_redeem_code() -> Result<String, RedeemError> {
    let mut raw = [0_u8; CODE_ENTROPY_BYTES];
    rand::rngs::OsRng
        .try_fill_bytes(&mut raw)
        .map_err(|_| RedeemError::Entropy)?;
    Ok(format!("{CODE_PREFIX}{}", base32_no_pad(&raw)))
}

/// 生成兑换码时的失败原因。
#[derive(Debug, thiserror::Error)]
pub enum RedeemError {
    /// 系统随机源不可用 —— 绝不退化成可预测的码。
    #[error("system entropy unavailable")]
    Entropy,
}

/// 认领一张兑换码的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// 这次调用抢到了 —— **且只有它**应当去入账。
    Won,
    /// 已经被别人（或此前的自己）用掉了。
    AlreadyUsed,
}

/// 原子地把一张 `unused` 兑换码翻成 `used`。
///
/// 参数是 `&Db` 而不是 `&PanelState`，理由与 [`super::payment::settle_payment_order`]
/// 相同：这是"钱只能动一次"那条不变量的载体，值得被一个只需要 Postgres 的测试
/// 反复并发地撞。
///
/// # Errors
/// 更新失败。
pub async fn claim_code(
    pg: &Db,
    code_id: i64,
    user_id: i64,
    used_by: &str,
) -> Result<Claim, sqlx::Error> {
    let claimed = sqlx::query(
        "UPDATE redeem_codes SET status = $3, used_at = $4, used_by = $5, used_by_id = $6 \
         WHERE id = $1 AND status = $2",
    )
    .bind(code_id)
    .bind(STATUS_UNUSED)
    .bind(STATUS_USED)
    .bind(Utc::now())
    .bind(used_by)
    .bind(user_id)
    .execute(pg)
    .await?;

    if claimed.rows_affected() == 0 {
        Ok(Claim::AlreadyUsed)
    } else {
        Ok(Claim::Won)
    }
}

/// 撤回**自己**那一次认领，让码回到可兑状态。
///
/// `used_by_id` 进 WHERE 是必需的：没有它，一次失败的兑换会把并发者刚刚成功的
/// 认领擦掉，同一张码就被两个人各兑一次。
///
/// # Errors
/// 更新失败。
pub async fn release_claim(pg: &Db, code_id: i64, user_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE redeem_codes SET status = $2, used_at = NULL, used_by = NULL, used_by_id = NULL \
         WHERE id = $1 AND status = $3 AND used_by_id = $4",
    )
    .bind(code_id)
    .bind(STATUS_UNUSED)
    .bind(STATUS_USED)
    .bind(user_id)
    .execute(pg)
    .await?;
    Ok(())
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// `POST /user/redeem` —— 兑换。
///
/// 对应 `UserRedeemHandler`。码先转大写再查，
/// 所以用户粘贴小写也能兑。
pub async fn redeem(
    State(state): State<PanelState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> Response {
    let req: RedeemRequest = match parse_json_body(&body, "无效的兑换码") {
        Ok(req) => req,
        Err(response) => return response,
    };
    let code = req.code.trim().to_uppercase();
    if code.is_empty() {
        return bad_request("无效的兑换码");
    }

    // 先查一次，把「没有这张码」（404）和「已经被用掉」（400）分开 —— 两者对
    // 用户是不同的提示。
    let found: Result<Option<(i64, f64)>, _> = sqlx::query_as(
        "SELECT id, COALESCE(amount,0)::float8 FROM redeem_codes WHERE code = $1 LIMIT 1",
    )
    .bind(&code)
    .fetch_optional(&state.pg)
    .await;
    let (id, amount) = match found {
        Ok(Some(pair)) => pair,
        Ok(None) => return not_found("未找到该兑换码"),
        Err(error) => return db_failure("load_redeem_code", &error, "查询兑换码失败，请稍后重试"),
    };

    // 旧实现的 usedBy 取 BillingCtx.Email；中间件从不填它，于是实际落库的是用户 id
    // 的十进制串。这里 email 是从 users 行读出来的可信值，所以正常情况下落的是
    // 邮箱 —— 保留旧实现的兜底顺序，让 email 缺失时仍然退回 id。
    let used_by = if user.email.trim().is_empty() {
        user.user_id.to_string()
    } else {
        user.email.clone()
    };

    // 原子认领：只有把 unused 翻成 used 的那一次调用会去入账。
    match claim_code(&state.pg, id, user.user_id, &used_by).await {
        Ok(Claim::Won) => {}
        Ok(Claim::AlreadyUsed) => return bad_request("兑换码已被使用"),
        Err(error) => return db_failure("claim_redeem_code", &error, "兑换失败，请稍后重试"),
    }

    if let Err(error) = state
        .ledger
        .credit(user.user_id, amount, &format!("redeem:{code}"))
        .await
    {
        // 入账失败 → 把认领退回去，让这张码还能再兑。
        let _ = release_claim(&state.pg, id, user.user_id).await;
        tracing::warn!(event = "redeem_credit_failed", user_id = user.user_id, error = %error);
        return internal("兑换失败，请稍后重试");
    }

    ok(serde_json::json!({ "amount": amount }))
}

/// `GET /admin/redeem-codes` —— 全部兑换码，`{"items": [...]}`，无分页。
pub async fn admin_list(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    let rows: Result<Vec<RedeemCodeRow>, _> = sqlx::query_as(&format!(
        "SELECT {REDEEM_COLUMNS} FROM redeem_codes ORDER BY id DESC"
    ))
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(serde_json::json!({
            "items": rows.into_iter().map(RedeemCodeRecord::from).collect::<Vec<_>>(),
        })),
        Err(error) => db_failure("list_redeem_codes", &error, "获取兑换码失败，请稍后重试"),
    }
}

/// `POST /admin/redeem-codes` —— 批量生成。
///
/// Ports `AdminCreateRedeemCodesHandler`。`count ∈ [1, 100]` 且 `amount > 0`，
/// 越界一律 400「请求格式无效」。响应只回条数，**不回码本身** —— 码要从列表接口
/// 单独取。
pub async fn admin_create(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: axum::body::Bytes,
) -> Response {
    let req: CreateRequest = match parse_json_body(&body, "请求格式无效") {
        Ok(req) => req,
        Err(response) => return response,
    };
    if req.count <= 0 || req.count > MAX_BATCH || !is_positive_amount(req.amount) {
        return bad_request("请求格式无效");
    }

    let now = Utc::now();
    let mut codes = Vec::with_capacity(usize::try_from(req.count).unwrap_or(0));
    for _ in 0..req.count {
        match generate_redeem_code() {
            Ok(code) => codes.push(code),
            Err(error) => {
                tracing::warn!(event = "redeem_code_generate_failed", error = %error);
                return internal("生成兑换码失败，请稍后重试");
            }
        }
    }

    // 一条 INSERT … SELECT unnest(...) 把整批写进去：批量生成常常是几十张，
    // 逐条往返没有意义，而且一条语句天然是一个事务（要么全成要么全不成）。
    let inserted = sqlx::query(
        "INSERT INTO redeem_codes (code, amount, status, created_at) \
         SELECT c, $2, $3, $4 FROM unnest($1::text[]) AS c",
    )
    .bind(&codes)
    .bind(req.amount)
    .bind(STATUS_UNUSED)
    .bind(now)
    .execute(&state.pg)
    .await;

    match inserted {
        Ok(done) => ok(serde_json::json!({ "created": done.rows_affected() })),
        Err(error) => db_failure("create_redeem_codes", &error, "创建兑换码失败，请稍后重试"),
    }
}

/// `DELETE /admin/redeem-codes/{id}`。
///
/// Ports `AdminDeleteRedeemCodeHandler`。这里 **检查影响行数**（删不存在的 id →
/// 404），与 `/admin/groups/{id}` 的删除**不同** —— 旧实现两处就是不一样，照抄。
pub async fn admin_delete(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的 ID");
    };
    match sqlx::query("DELETE FROM redeem_codes WHERE id = $1")
        .bind(id)
        .execute(&state.pg)
        .await
    {
        Ok(done) if done.rows_affected() == 0 => not_found("未找到该兑换码"),
        Ok(_) => ok(serde_json::json!({ "deleted": true })),
        Err(error) => db_failure("delete_redeem_code", &error, "删除兑换码失败，请稍后重试"),
    }
}

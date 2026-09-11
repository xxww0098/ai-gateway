//! 充值订单：三个渠道的下单/查单、管理员列表与人工确认，以及**唯一的入账路径**。
//!
//! 对应 `settlePaymentOrder` / `AdminListOrders` / `AdminConfirmOrder` / `Payment*` 那几段。
//!
//! # 只有一条入账路径，而且它是幂等的
//!
//! [`settle_payment_order`] 是订单变成余额的**唯一**出口，管理员人工确认和 Stripe
//! 回调（[`super::stripe`]）都走它。幂等靠**单一事务**：锁住订单行后，pending 和
//! paid 两种状态都走 [`Ledger::credit_tx`]（事务内按 reference 预查 +
//! `0010` 的分区唯一索引把重复入账挡在库里），只有 pending 在入账后翻 `paid`。
//! 已 paid 但缺流水的历史行由重试补入恰好一次；不存在 `paid→pending` 的补偿 ——
//! 那会制造 "credit 已提交、状态被改回 pending、重试再 credit" 的窗口。任何失败
//! 都回滚整个事务，订单保持原状；暂时性失败可重试，用户/金额冲突需先对账。
//!
//! # 三个渠道目前都是本地模拟
//!
//! 原实现没有接真实支付网关：`/payment/*/create` 只落一行 `pending` 订单并回一个
//! 形如 `pi_000123` / `ai-gateway://…` 的假凭据。真正会动钱的只有管理员确认和
//! （配了签名密钥时的）Stripe 回调。这里保持原样 —— 补真实网关是另一件事。

use axum::extract::{Path, Query, State};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use gw_infra::Db;
use gw_ledger::{BalanceChange, Ledger};

use crate::audit::Actor;
use crate::identity::oplog::ReqMeta;
use crate::identity::{
    bad_request, conflict, db_failure, is_positive_amount, not_found, parse_json_body,
};
use crate::paging::{ListPage, offset, page_params, parse_id};
use crate::{AdminUser, AuthUser, PanelState, ok};

#[cfg(test)]
mod tests;

/// 管理员订单列表的默认页大小（既有实现默认 15）。
const ADMIN_ORDERS_DEFAULT_PAGE_SIZE: i64 = 15;

/// 人民币渠道的换算率。旧实现写死 `amount*7.2`，这里同样是一个展示用的固定值，
/// 不是行情 —— 真接支付网关时它必须换成网关返回的金额。
const CNY_PER_USD: f64 = 7.2;

const STATUS_PENDING: &str = "pending";
const STATUS_PAID: &str = "paid";

const PROVIDER_STRIPE: &str = "stripe";
const PROVIDER_ALIPAY: &str = "alipay";
const PROVIDER_WECHAT: &str = "wechat";

/// 对应 `paymentOrderRecord`。
#[derive(Debug, Serialize)]
pub struct OrderRecord {
    pub id: i64,
    pub user_id: i64,
    pub provider: String,
    pub amount_usd: f64,
    pub amount_local: f64,
    pub currency: String,
    pub status: String,
    pub transaction_id: Option<String>,
    pub metadata: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct OrderRow {
    id: i64,
    user_id: i64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    provider: String,
    #[sqlx(try_from = "gw_model::compat::Money")]
    amount_usd: f64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    amount_local: f64,
    #[sqlx(try_from = "gw_model::compat::Text")]
    currency: String,
    #[sqlx(try_from = "gw_model::compat::Text")]
    status: String,
    transaction_id: Option<String>,
    metadata: Option<String>,
    paid_at: Option<DateTime<Utc>>,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    created_at: DateTime<Utc>,
    #[sqlx(try_from = "gw_model::compat::Ts")]
    updated_at: DateTime<Utc>,
}

const ORDER_COLUMNS: &str = "id, user_id, provider, amount_usd, amount_local, currency, status, \
     transaction_id, metadata, paid_at, created_at, updated_at";

impl From<OrderRow> for OrderRecord {
    fn from(row: OrderRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            provider: row.provider,
            amount_usd: row.amount_usd,
            amount_local: row.amount_local,
            currency: row.currency,
            status: row.status,
            transaction_id: row.transaction_id,
            metadata: row.metadata,
            paid_at: row.paid_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AmountRequest {
    amount: f64,
}

/// 对外的订单号：`<provider>-<6 位补零 id>`。Ports `publicOrderID`。
///
/// 这个格式是**双向**契约：`/payment/*/status?order_id=` 收到的就是它，
/// [`parse_public_order_id`] 必须能拆回来。
#[must_use]
pub fn public_order_id(provider: &str, id: i64) -> String {
    format!("{provider}-{id:06}")
}

/// 把对外订单号拆回 `(provider, id)`。Ports `paymentOrderByPublicID` 的解析部分。
///
/// 旧实现用 `strings.Split(orderID, "-")` 并要求**恰好两段**，所以 provider 名里带
/// 连字符的订单号会被判无效。照抄这个严格性：宽松解析会让 `a-b-123` 这类输入
/// 落到一个意料之外的订单上。
#[must_use]
pub fn parse_public_order_id(raw: &str) -> Option<(&str, i64)> {
    let mut parts = raw.split('-');
    let provider = parts.next()?;
    let id = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let id = id.parse::<u64>().ok()?;
    i64::try_from(id).ok().map(|id| (provider, id))
}

/// 把一张订单标记为已付，并通过账本给用户入账。**幂等**。
///
/// Ports `PanelRouter.settlePaymentOrder`。整个结算在**一个事务**里完成：锁住
/// 订单行，pending 与 paid 两种状态都调用 [`Ledger::credit_tx`]（入账与订单状态
/// 原子；已 paid 但缺流水的历史行由同一事务补入恰好一次），只有 pending 在入账后
/// 翻 `paid`。返回 `Ok(true)` 表示"这次调用真的动了钱"，`Ok(false)` 表示
/// "没有新入账"（订单不存在、状态不可处理，或入账早已发生过）——**不是错误**。
/// 任何失败都回滚整个事务，订单保持原状，调用方重试即可。
///
/// 参数是 `(&Db, &Ledger)` 而不是 `&PanelState`：这条路径是整个 crate 里最该被
/// 连库测试盯住的一段，而 `PanelState` 还拖着 Redis 连接、定价缓存、凭据仓库 ——
/// 那些和"订单怎么变成余额"毫无关系。收窄参数之后，一个只有 Postgres 的测试就
/// 能把幂等性验完（`Ledger::new(pg, None)` 本来就允许没有 Redis）。
///
/// # Errors
/// 数据库或账本失败。事务已整体回滚，调用方重试即可。
pub async fn settle_payment_order(
    pg: &Db,
    ledger: &Ledger,
    order_id: i64,
) -> Result<bool, SettleError> {
    let mut tx = pg.begin().await.map_err(SettleError::Db)?;

    // 锁序：订单行 -> 用户行（credit_tx 内）-> balance_logs。订单行锁同时把
    // 同一张单的并发确认 / 回调全部串行化，后面的 credit_tx 预查因此不会与
    // 另一个结算竞态。
    let order: Option<(i64, String, f64)> = sqlx::query_as(
        "SELECT user_id, COALESCE(status,''), COALESCE(amount_usd,0)::float8 \
         FROM payment_orders WHERE id = $1 FOR UPDATE",
    )
    .bind(order_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(SettleError::Db)?;

    let Some((user_id, status, amount_usd)) = order else {
        // 订单不存在：按"没有新入账"处理。webhook 侧继续 200，管理员侧的
        // 404 由 admin_confirm 自己的预读负责。
        return Ok(false);
    };

    // 只有 pending / paid 是可处理状态；其它（手工写坏的 status）一律按
    // "已结过"返回，不碰钱。
    let is_pending = status == STATUS_PENDING;
    if !is_pending && status != STATUS_PAID {
        return Ok(false);
    }

    // pending 与 paid 都走同一个入账：pending 是首次入账，paid 是对历史
    // "已付但缺流水"行的恰好一次补入（credit_tx 的预查 / 0010 唯一索引挡住
    // 一切重复）。
    let outcome = ledger
        .credit_tx(
            &mut tx,
            user_id,
            amount_usd,
            &format!("payment_order:{order_id}"),
        )
        .await
        .map_err(|error| SettleError::Ledger(error.to_string()))?;

    if is_pending {
        // 只有 pending 翻状态；paid 修复路径保留既有时间戳。
        sqlx::query(
            "UPDATE payment_orders SET status = $2, paid_at = $3, updated_at = $3 \
             WHERE id = $1",
        )
        .bind(order_id)
        .bind(STATUS_PAID)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await
        .map_err(SettleError::Db)?;
    }

    tx.commit().await.map_err(SettleError::Db)?;

    if let BalanceChange::Applied {
        balance_after,
        balance_version,
    } = outcome
    {
        let _ = ledger
            .publish_balance(user_id, balance_after, balance_version)
            .await;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// [`settle_payment_order`] 的失败原因。
#[derive(Debug, thiserror::Error)]
pub enum SettleError {
    /// 查询/更新失败。
    #[error(transparent)]
    Db(sqlx::Error),
    /// 账本入账失败；整个事务已回滚。
    #[error("ledger credit failed: {0}")]
    Ledger(String),
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// 管理员查询里 `user_id = 0` 表示不过滤。用户路径绝不能落到这个哨兵，
/// 否则 `?user_id=` 或漏绑会把全站订单交出去。
const UNSCOPED_USER_ID: i64 = 0;

async fn list_orders_page(
    pg: &Db,
    provider: &str,
    status: &str,
    user_id: i64,
    page: i64,
    page_size: i64,
) -> Result<(Vec<OrderRow>, i64), sqlx::Error> {
    let filter = "($1 = '' OR provider = $1) AND ($2 = '' OR status = $2) \
         AND ($3 = 0 OR user_id = $3)";

    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*)::bigint FROM payment_orders WHERE {filter}"
    ))
    .bind(provider)
    .bind(status)
    .bind(user_id)
    .fetch_one(pg)
    .await?;

    let rows: Vec<OrderRow> = sqlx::query_as(&format!(
        "SELECT {ORDER_COLUMNS} FROM payment_orders WHERE {filter} \
         ORDER BY id DESC LIMIT $4 OFFSET $5"
    ))
    .bind(provider)
    .bind(status)
    .bind(user_id)
    .bind(page_size)
    .bind(offset(page, page_size))
    .fetch_all(pg)
    .await?;

    Ok((rows, total))
}

fn orders_response(
    result: Result<(Vec<OrderRow>, i64), sqlx::Error>,
    page: i64,
    page_size: i64,
) -> Response {
    match result {
        Ok((rows, total)) => ok(ListPage::new(
            rows.into_iter().map(OrderRecord::from).collect(),
            total,
            page,
            page_size,
        )),
        Err(error) => db_failure("list_orders", &error, "获取订单失败，请稍后重试"),
    }
}

/// `GET /user/orders` —— 当前用户自己的订单分页。
///
/// 前端订单页调的就是这条。`?user_id=` 即使带了也丢掉，只看登录身份。
pub async fn list_own(
    State(state): State<PanelState>,
    user: AuthUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (page, page_size) = page_params(
        params.get("page").map(String::as_str),
        params.get("page_size").map(String::as_str),
        ADMIN_ORDERS_DEFAULT_PAGE_SIZE,
    );
    let status = params.get("status").map(|s| s.trim()).unwrap_or_default();
    // `?user_id=` 即使带了也丢掉，只看登录身份 —— 0 是管理员的「不过滤」哨兵，
    // 用户路径绝不能落到它上面。
    let user_id = user.user_id;
    orders_response(
        list_orders_page(&state.pg, "", status, user_id, page, page_size).await,
        page,
        page_size,
    )
}

/// `GET /admin/orders` —— 分页 + provider/status/user_id 过滤。
///
/// Ports `AdminListOrdersHandler`。注意 `user_id` 参数**解析失败时被忽略**
/// （旧实现解析失败时忽略该参数），不是 400。
pub async fn admin_list_orders(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (page, page_size) = page_params(
        params.get("page").map(String::as_str),
        params.get("page_size").map(String::as_str),
        ADMIN_ORDERS_DEFAULT_PAGE_SIZE,
    );
    let provider = params.get("provider").map(|s| s.trim()).unwrap_or_default();
    let status = params.get("status").map(|s| s.trim()).unwrap_or_default();
    let user_id = params
        .get("user_id")
        .map(|s| s.trim())
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|id| *id != 0)
        .unwrap_or(UNSCOPED_USER_ID);

    orders_response(
        list_orders_page(&state.pg, provider, status, user_id, page, page_size).await,
        page,
        page_size,
    )
}

/// `PUT /admin/orders/{id}/confirm` —— 线下对账后的人工确认。
///
/// Ports `AdminConfirmOrderHandler`。已结过的单回 **409**「订单已处理」，让管理员
/// 知道这次没有产生新的入账 —— 静默回 200 会诱使人反复点确认。
pub async fn admin_confirm(
    State(state): State<PanelState>,
    admin: AdminUser,
    ReqMeta(meta): ReqMeta,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = parse_id(&id) else {
        return bad_request("无效的 ID");
    };

    let existing: Result<Option<(i64, f64)>, _> = sqlx::query_as(
        "SELECT user_id, COALESCE(amount_usd,0)::float8 FROM payment_orders WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pg)
    .await;
    let (user_id, amount_usd) = match existing {
        Ok(Some(pair)) => pair,
        Ok(None) => return not_found("未找到该订单"),
        Err(error) => return db_failure("load_order", &error, "查询订单失败，请稍后重试"),
    };

    match settle_payment_order(&state.pg, &state.ledger, id).await {
        Ok(true) => {}
        Ok(false) => return conflict("订单已处理"),
        Err(error) => {
            tracing::warn!(event = "order_confirm_failed", order_id = id, error = %error);
            return crate::identity::internal("确认订单失败，请稍后重试");
        }
    }

    crate::identity::oplog::record(
        &state,
        &meta,
        Some(&Actor {
            user_id: admin.0.user_id,
            email: admin.0.email.clone(),
            role: admin.0.role.clone(),
        }),
        "admin.order.confirm",
        &format!("order:{id}"),
        200,
        Some(serde_json::json!({ "amount_usd": amount_usd, "user_id": user_id })),
    )
    .await;
    ok(serde_json::json!({ "confirmed": true }))
}

/// 系统设置里的三个渠道卡片。支付下单目前是本地模拟，这里不落库：
/// GET 给出禁用桩，PUT 明确拒绝，避免管理员以为密钥已经存上了。
const PAYMENT_CONFIG_PROVIDERS: [&str; 3] = [PROVIDER_STRIPE, PROVIDER_ALIPAY, PROVIDER_WECHAT];

fn payment_config_stubs() -> Vec<serde_json::Value> {
    PAYMENT_CONFIG_PROVIDERS
        .iter()
        .map(|provider| {
            serde_json::json!({
                "provider": provider,
                "app_id": null,
                "app_secret": "",
                "webhook_secret": "",
                "mode": "sandbox",
                "enabled": false,
            })
        })
        .collect()
}

/// `GET /admin/payment-config` —— 三个渠道的只读桩，全部未启用。
pub async fn admin_list_payment_config(_admin: AdminUser) -> Response {
    ok(payment_config_stubs())
}

/// `PUT /admin/payment-config` —— 渠道还是本地模拟，拒绝落库。
pub async fn admin_put_payment_config(_admin: AdminUser) -> Response {
    bad_request("支付渠道目前为本地模拟，配置不会落库")
}

/// `GET /payment/stripe/config` —— 前端用来决定是否显示 Stripe 入口。
///
/// Ports `PaymentStripeConfigHandler`：三个字段全是写死的占位值，没有真实账户。
pub async fn stripe_config() -> Response {
    ok(serde_json::json!({
        "publishable_key": "",
        "mode": "sandbox",
        "enabled": false,
    }))
}

/// `POST /payment/stripe/create`。Ports `PaymentStripeCreateHandler`。
pub async fn stripe_create(
    State(state): State<PanelState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> Response {
    let Some(amount) = bind_amount(&body) else {
        return bad_request("金额无效");
    };
    let order =
        match create_order(&state, user.user_id, PROVIDER_STRIPE, amount, amount, "USD").await {
            Ok(order) => order,
            Err(error) => return db_failure("create_order", &error, "创建订单失败，请稍后重试"),
        };
    let payment_intent_id = format!("pi_{:06}", order.id);
    ok(serde_json::json!({
        "client_secret": format!("{payment_intent_id}_secret_local_mock"),
        "order_id": public_order_id(PROVIDER_STRIPE, order.id),
        "payment_intent_id": payment_intent_id,
        "amount_usd": amount,
    }))
}

/// `POST /payment/alipay/create`。Ports `PaymentAlipayCreateHandler`。
pub async fn alipay_create(
    State(state): State<PanelState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> Response {
    let Some(amount) = bind_amount(&body) else {
        return bad_request("金额无效");
    };
    let local = amount * CNY_PER_USD;
    let order =
        match create_order(&state, user.user_id, PROVIDER_ALIPAY, amount, local, "CNY").await {
            Ok(order) => order,
            Err(error) => return db_failure("create_order", &error, "创建订单失败，请稍后重试"),
        };
    let public_id = public_order_id(PROVIDER_ALIPAY, order.id);
    let pay_url = format!("ai-gateway://payment/alipay/{public_id}");
    ok(serde_json::json!({
        "order_id": public_id,
        "pay_url": pay_url,
        "qr_code": pay_url,
        "amount_usd": amount,
        "amount_local": local,
        "currency": "CNY",
    }))
}

/// `GET /payment/alipay/status`。
pub async fn alipay_status(
    State(state): State<PanelState>,
    user: AuthUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    payment_status(&state, &user, &params, PROVIDER_ALIPAY).await
}

/// `POST /payment/wechat/create`。Ports `PaymentWechatCreateHandler`。
pub async fn wechat_create(
    State(state): State<PanelState>,
    user: AuthUser,
    body: axum::body::Bytes,
) -> Response {
    let Some(amount) = bind_amount(&body) else {
        return bad_request("金额无效");
    };
    let local = amount * CNY_PER_USD;
    let order =
        match create_order(&state, user.user_id, PROVIDER_WECHAT, amount, local, "CNY").await {
            Ok(order) => order,
            Err(error) => return db_failure("create_order", &error, "创建订单失败，请稍后重试"),
        };
    let public_id = public_order_id(PROVIDER_WECHAT, order.id);
    ok(serde_json::json!({
        "order_id": public_id,
        // 旧实现这里把 code_url 也填成订单号（没有真实网关可给二维码链接）。
        "code_url": public_id,
        "amount_usd": amount,
        "amount_local": local,
        "currency": "CNY",
    }))
}

/// `GET /payment/wechat/status`。
pub async fn wechat_status(
    State(state): State<PanelState>,
    user: AuthUser,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    payment_status(&state, &user, &params, PROVIDER_WECHAT).await
}

/// 查单，并强制「调用者是单主或管理员」。
///
/// Ports `paymentStatus`。**「不存在」和「不是你的」返回同一个 404** —— 否则订单号
/// 就成了一个可枚举的存在性预言机（`TestPaymentStatusRequiresOrderOwnerOrAdmin`
/// 盯的正是这一点）。
async fn payment_status(
    state: &PanelState,
    user: &AuthUser,
    params: &HashMap<String, String>,
    provider: &str,
) -> Response {
    let order_id = params.get("order_id").map(|s| s.trim()).unwrap_or_default();
    if order_id.is_empty() {
        return bad_request("缺少 order_id 参数");
    }

    let Some((parsed_provider, id)) = parse_public_order_id(order_id) else {
        return not_found("未找到该订单");
    };

    let row: Option<(i64, String, f64, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT user_id, COALESCE(status,''), COALESCE(amount_usd,0)::float8, paid_at \
         FROM payment_orders WHERE id = $1 AND provider = $2 LIMIT 1",
    )
    .bind(id)
    .bind(parsed_provider)
    .fetch_optional(&state.pg)
    .await
    .unwrap_or(None);

    let Some((owner_id, status, amount, paid_at)) = row else {
        return not_found("未找到该订单");
    };
    if parsed_provider != provider || (owner_id != user.user_id && !user.is_admin()) {
        return not_found("未找到该订单");
    }

    ok(serde_json::json!({
        "status": status,
        "order_id": order_id,
        "amount": amount,
        "paid_at": paid_at,
    }))
}

/// Ports `bindPaymentAmount`：解析失败或 `amount <= 0` 都是同一个 400。
fn bind_amount(body: &[u8]) -> Option<f64> {
    let req: AmountRequest = parse_json_body(body, "金额无效").ok()?;
    if !is_positive_amount(req.amount) {
        return None;
    }
    Some(req.amount)
}

/// 落一行 `pending` 订单。Ports `createPaymentOrder`。
async fn create_order(
    state: &PanelState,
    user_id: i64,
    provider: &str,
    amount_usd: f64,
    amount_local: f64,
    currency: &str,
) -> Result<OrderRow, sqlx::Error> {
    let now = Utc::now();
    sqlx::query_as(&format!(
        "INSERT INTO payment_orders \
             (user_id, provider, amount_usd, amount_local, currency, status, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $7) RETURNING {ORDER_COLUMNS}"
    ))
    .bind(user_id)
    .bind(provider)
    .bind(amount_usd)
    .bind(amount_local)
    .bind(currency)
    .bind(STATUS_PENDING)
    .bind(now)
    .fetch_one(&state.pg)
    .await
}

//! Narrow trait surfaces the `/v1/*` kernel depends on.
//!
//! Minimal interfaces (`BillingLedger`, `PricingCalculator`, ...) are declared
//! here instead of importing the concrete `ledger` / `pricing` types so the
//! middleware can be unit-tested with in-memory fakes; the same reasoning
//! applies, with the extra benefit that `gw-proxy` compiles against these
//! traits while `gw-ledger` / `gw-pricing` / `gw-infra` are written in
//! parallel.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gw_authcore::Claims;
use gw_ledger::{BillingOperationId, NewOperation, OperationConflict};
use gw_pricing::PricingQuote;

/// Primary-key type used across every entity.
///
/// Declared locally rather than re-exported from `gw-model` so this crate's
/// trait surface does not move when the entity layer does; the two must stay
/// the same width (`gw_model::Id`).
pub type Id = i64;

/// Why a ledger operation refused.
///
/// A typed enum distinguishes the structured 402 from the generic one without
/// the string matching; it mirrors the failure modes of `Ledger::hold` /
/// `Ledger::settle`.
#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    #[error("insufficient balance")]
    InsufficientBalance,
    #[error("outstanding debt")]
    OutstandingDebt,
    #[error("hold not found")]
    HoldNotFound,
    /// The server-minted operation id is taken by a *different* hold — a
    /// different tenant, amount or request, or one that already terminated.
    /// Never an overwrite: see [`gw_ledger::operation::admit`].
    #[error("billing operation conflict: {0}")]
    OperationConflict(#[from] OperationConflict),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// ---------------------------------------------------------------- billing

/// Minimal ledger surface consumed by [`crate::hold`] and [`crate::usage`].
/// A 1:1 narrowing of the operation-level `gw_ledger::Ledger` API.
///
/// Every method keys on a [`BillingOperationId`] — the **server-minted** money
/// key. There is deliberately no overload taking a `&str`: an inbound
/// `X-Trace-ID` is a [`gw_ledger::ClientTraceId`], and the type system keeps it
/// out of these arguments.
#[async_trait]
pub trait BillingLedger: Send + Sync {
    /// Admits a billing operation: writes the durable `billing_operations`
    /// row, then takes the reservation.
    ///
    /// `redis_ttl` is `Some` on the normal path and `None` when the
    /// reservation already came from the process-local budget token — the
    /// durable row is written either way, because it is the operation's
    /// identity rather than its cache entry.
    ///
    /// A floor refusal is [`HoldAdmit::Insufficient`] and must not leave a
    /// reservation or a `held` row behind; that is what keeps a 402
    /// `insufficient_balance` from stranding money.
    async fn admit_operation(
        &self,
        operation: &NewOperation,
        redis_ttl: Option<Duration>,
    ) -> Result<HoldAdmit, BillingError>;

    /// Debits `min(balance, actual_amount)` for the operation and clears the
    /// reservation — **exactly once**, however many callers race.
    ///
    /// Returns the un-debited shortfall on the call that performed the debit,
    /// and [`SettleTerminal::AlreadyTerminal`] for every other one. A
    /// non-positive `actual_amount` still terminates the operation; it just
    /// debits nothing.
    async fn settle_once(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
        actual_amount: f64,
    ) -> Result<SettleTerminal, BillingError>;

    /// Terminates the operation without touching the persistent balance —
    /// also exactly once. Used when the upstream request failed.
    async fn release_once(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
    ) -> Result<(), BillingError>;

    /// Amount currently reserved for the operation, without mutating any
    /// state. `Ok(None)` means "no reservation"; `Err` means "unknown" and the
    /// caller must NOT treat it as zero (see [`crate::usage`] fallback path).
    async fn active_hold_amount(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
    ) -> Result<Option<f64>, BillingError>;

    /// 把预留的租约到期时刻往后推一片，返回**没有变化**的预留金额。
    ///
    /// 刻意不收金额参数：续租不是第二次准入，「换个金额续租」这种操作
    /// 不存在。调用方是流式回写包装（[`crate::routes`]），中继引擎不碰它。
    ///
    /// 一个没有活预留的操作是 [`BillingError::HoldNotFound`] ——
    /// 续租绝不凭空造出一笔预留。
    async fn renew_lease(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
    ) -> Result<f64, BillingError>;

    /// Reports whether the user owns a settle `balance_logs` row with a
    /// positive `metadata.shortfall_usd` that has not been paired with a
    /// `shortfall_resolve:<reference>:<id>` credit.
    async fn has_unresolved_shortfall(&self, user_id: Id) -> Result<bool, BillingError>;

    /// Available balance (persisted balance minus active holds), used for the
    /// structured 402 body.
    async fn available_balance(&self, user_id: Id) -> Result<f64, BillingError>;
}

/// What a [`BillingLedger::settle_once`] call actually did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettleTerminal {
    /// This call performed the one debit. Carries the un-debited shortfall.
    Debited { shortfall: f64 },
    /// The operation had already settled or released. **Nothing was debited.**
    AlreadyTerminal,
}

/// Outcome of [`BillingLedger::admit_operation`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HoldAdmit {
    /// The reservation is live (or this request id already had one).
    Reserved,
    /// Available balance is below the floor. No reservation was created.
    Insufficient {
        /// Balance the 402 body should quote.
        available: f64,
    },
}

/// 计价面：**只有一个方法**，因为计价在一次请求里只发生一次。
///
/// Hold 处报一次价，把四列单价与倍率冻进 [`PricingQuote`]，此后估算（预扣）
/// 与精算（结算）都在那个报价上做。所以这里没有 `estimate` / `compute` ——
/// 它们在 [`PricingQuote`] 上，而且拿不到价目表缓存，也就无从二次查价。
///
/// 这正是「在途请求不会因为管理员改价、或上游回一个别的模型名就换一个价钱
/// 结算」的类型层面保证。
pub trait PricingCalculator: Send + Sync {
    /// 冻结 `model`（**请求**里那个模型名）在此刻的四列单价与倍率。
    fn quote(&self, model: &str, rate_mult: f64) -> PricingQuote;
}

// ---------------------------------------------------------------- identity

/// Cryptographic primitives owned by `gw-authcore`.
///
/// Behind a port so `gw-proxy` neither pulls in a hashing dependency of its own
/// nor hard-codes the JWT secret.
pub trait AuthCrypto: Send + Sync {
    /// Lowercase hex SHA-256 of the plaintext API key.
    fn hash_api_key(&self, plaintext: &str) -> String;

    /// Lowercase hex SHA-256 of arbitrary input, used to scope idempotency
    /// keys.
    fn sha256_hex(&self, input: &str) -> String;

    /// Validates an HS256 JWT against the configured secret.
    fn verify_jwt(&self, token: &str) -> Option<Claims>;
}

/// One `api_keys` row as far as `/v1/*` authentication is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyRow {
    pub id: Id,
    pub user_id: Id,
    pub group_id: Option<Id>,
    pub status: String,
}

/// Quota + entitlement state of the newest active subscription.
///
/// Shared by the access provider (publication) and the hold pre-flight
/// (locking/rotation), so the two call sites share one shape. A `None` reset
/// timestamp means rotation is disabled for that period.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubscriptionQuota {
    pub id: Id,
    pub group_id: Id,
    pub daily_usage_usd: f64,
    pub weekly_usage_usd: f64,
    pub monthly_usage_usd: f64,
    pub daily_limit_usd: Option<f64>,
    pub weekly_limit_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
    pub daily_reset_at: Option<DateTime<Utc>>,
    pub weekly_reset_at: Option<DateTime<Utc>>,
    pub monthly_reset_at: Option<DateTime<Utc>>,
}

/// Read-mostly tenant lookups behind the `/v1/*` access provider.
///
/// Implementations are expected to sit in front of `gw_infra::ApiKeyCache` /
/// `UserStatusCache`; the caching policy is theirs, the authorization policy is
/// [`crate::access`]'s.
#[async_trait]
pub trait TenantDirectory: Send + Sync {
    /// `SELECT * FROM api_keys WHERE key_hash = ?`. Status filtering is the
    /// caller's job so a cached-but-deactivated key is rejected identically to
    /// a DB-side deactivation.
    async fn api_key_by_hash(&self, key_hash: &str) -> anyhow::Result<Option<ApiKeyRow>>;

    /// `SELECT rate_multiplier FROM groups WHERE id = ?`.
    async fn group_rate_multiplier(&self, group_id: Id) -> anyhow::Result<Option<f64>>;

    /// `SELECT status FROM users WHERE id = ?`. `None` means the row is absent.
    async fn user_status(&self, user_id: Id) -> anyhow::Result<Option<String>>;

    /// Newest `status = 'active' AND expires_at > now()` subscription.
    async fn active_subscription(&self, user_id: Id) -> anyhow::Result<Option<SubscriptionQuota>>;

    /// Whether the user holds a live subscription bound to `group_id`.
    async fn holds_group_entitlement(&self, user_id: Id, group_id: Id) -> anyhow::Result<bool>;

    /// Fire-and-forget `UPDATE api_keys SET last_used_at = now()`.
    async fn touch_api_key(&self, api_key_id: Id);
}

/// 订阅配额的**在途预留**，键是 [`BillingOperationId`]。
///
/// 与余额预扣同一个精神：**比较必须发生在锁里**。实现方在一个事务里
/// 锁订阅行 → 应用 [`crate::hold::rotate_counters`] → 用
/// [`crate::hold::evaluate_quota`] 把「已用 + 在途预留 + 这一笔」和限额比
/// → 落预留行；超限就整个回滚，一行都不留下。
///
/// *决定*（轮转什么、超没超）留在 [`crate::hold`] 的纯函数里，实现方只提供
/// 那把锁和那份持久化 —— 拿 SQL 再写一遍边界算术是第二份实现。
#[async_trait]
pub trait SubscriptionQuotaStore: Send + Sync {
    /// 在**一个事务**里锁行、轮转、比限额、落预留。
    ///
    /// `amount` 是准入时拿去和余额比的那个上限（预付模式下 = 预留住的数），
    /// 于是配额看见的在途负债和账本看见的是同一个数。
    ///
    /// 同一个 `operation` 重复预留是**恢复**，不是第二笔：金额已经在
    /// 「在途合计」里了，再比一次会把自己算两遍。
    async fn reserve(
        &self,
        subscription_id: Id,
        operation: &BillingOperationId,
        amount: f64,
        now: DateTime<Utc>,
    ) -> anyhow::Result<QuotaAdmission>;

    /// 丢掉预留，**不**累加任何计数器。请求被拒、上游失败、准入失败都走它。
    ///
    /// 删一行不存在的预留是成功：调用方已经在错误路径上，没有东西要还。
    async fn release_reservation(&self, operation: &BillingOperationId) -> anyhow::Result<()>;
}

/// [`SubscriptionQuotaStore::reserve`] 的三种结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaAdmission {
    /// 预留已落（或这个操作本来就有一笔同样的预留）。
    Reserved,
    /// 订阅行不存在。配额是**可选**的，这样的用户纯按余额计费 ——
    /// 这不是「拒绝」。
    NoSubscription,
    /// 某个周期会被这一笔顶穿。没有留下任何预留行。
    Exceeded { reason: &'static str },
}

// ---------------------------------------------------------------- settlement

/// A `usage_logs` row, field-for-field with `model.UsageLog`.
///
/// Built by [`crate::usage`]; written verbatim by [`UsageStore`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageLogEntry {
    pub user_id: Id,
    pub api_key_id: Id,
    pub group_id: Option<Id>,
    /// The client-facing trace id. **Observability**: it is what the tenant
    /// sees in `X-Trace-ID` and what a support ticket quotes.
    pub request_id: String,
    /// The server-minted [`BillingOperationId`] as text. **This is the money
    /// key**, and it is never empty on a billed or settled row.
    pub event_key: String,
    pub idempotency_key: String,
    pub model: String,
    pub provider: String,
    pub auth_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_cost: f64,
    pub actual_cost: f64,
    pub cost: f64,
    pub rate_multiplier: f64,
    pub stream: bool,
    pub duration_ms: i64,
    pub ip_address: String,
    /// `jsonb` column; `None` keeps the column NULL.
    pub raw_metadata: Option<serde_json::Value>,
    pub failed: bool,
}

/// A `balance_logs` row written for threshold events.
#[derive(Debug, Clone, PartialEq)]
pub struct BalanceEvent {
    pub user_id: Id,
    pub amount: f64,
    pub event_type: String,
    pub reference: String,
    pub metadata: serde_json::Value,
}

/// Everything that must commit atomically for one settled request.
///
/// Idempotency is **not** a caller option here. The implementation moves the
/// operation to a terminal state inside the same transaction as the debit, so
/// a second commit for the same [`BillingOperationId`] — a retry, a concurrent
/// reconciler, a duplicated finalizer — reports
/// [`SettleReceipt::AlreadyTerminal`] and moves no money. There is no flag to
/// forget to set.
#[derive(Debug, Clone)]
pub struct SettlementCommit {
    pub user_id: Id,
    /// The money key. Also written to `usage_logs.event_key`.
    pub operation: BillingOperationId,
    pub actual_cost: f64,
    /// The row to insert. Its `raw_metadata` already carries any
    /// `billing_fallback` tag; the implementation MUST fold the partial-debit
    /// shortfall in with [`crate::usage::merge_shortfall`] before inserting,
    /// because the shortfall is only known inside the transaction.
    pub entry: UsageLogEntry,
    /// Accumulate the three usage counters on this subscription when the cost
    /// is positive and the row is still active.
    pub subscription_id: Option<Id>,
}

/// Outcome of [`UsageStore::commit_settlement`].
#[derive(Debug, Clone, PartialEq)]
pub enum SettleReceipt {
    Committed {
        /// `actual_cost` minus the portion that could actually be debited.
        shortfall: f64,
        balance_before: f64,
        balance_after: f64,
    },
    /// The operation was already settled or released. No debit, no usage row.
    AlreadyTerminal,
}

/// Transactional writes owned by the settlement pipeline.
///
/// One implementation runs `ledger.SettleTx` + the `usage_logs` insert + the
/// subscription accumulation in a single Postgres transaction, then the caller
/// clears the Redis reservation only after commit — the settle transaction
/// contract.
#[async_trait]
pub trait UsageStore: Send + Sync {
    /// ONE transaction: debit, read balance, insert `usage_logs`, accumulate
    /// subscription counters.
    async fn commit_settlement(&self, commit: &SettlementCommit) -> anyhow::Result<SettleReceipt>;

    /// Standalone insert used on failure paths, outside any transaction.
    async fn insert_usage_log(&self, entry: &UsageLogEntry) -> anyhow::Result<()>;

    /// Insert a threshold balance event.
    async fn insert_balance_event(&self, event: &BalanceEvent) -> anyhow::Result<()>;

    /// Releases the Redis reservation after the settle transaction commits.
    async fn clear_hold(&self, user_id: Id, operation: &BillingOperationId) -> anyhow::Result<()>;

    /// 当地今日零点以来、按模型折叠的 token 消耗。供 `GET /v1/usage`。
    async fn model_usage_since(
        &self,
        user_id: Id,
        since: DateTime<Utc>,
    ) -> anyhow::Result<Vec<ModelTokenUsage>>;
}

/// 一个模型在某个时间窗内的 token 小计。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTokenUsage {
    pub model: String,
    pub requests: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
}

impl ModelTokenUsage {
    /// `tokens_in + tokens_out`，给 JSON 的 `tokens` 字段。
    #[must_use]
    pub fn tokens(&self) -> i64 {
        self.tokens_in.saturating_add(self.tokens_out)
    }
}

/// 把逐条 usage 折成按模型的 token 小计。空模型名变成 `unknown`。
///
/// 排序与面板 `buildUsageModels` 同口径：请求数降序，同数按模型名升序。
#[must_use]
pub fn fold_model_usage<'a>(
    logs: impl IntoIterator<Item = &'a UsageLogEntry>,
) -> Vec<ModelTokenUsage> {
    let mut table: HashMap<String, ModelTokenUsage> = HashMap::new();
    for entry in logs {
        let name = entry.model.trim();
        let name = if name.is_empty() { "unknown" } else { name };
        let point = table
            .entry(name.to_owned())
            .or_insert_with(|| ModelTokenUsage {
                model: name.to_owned(),
                requests: 0,
                tokens_in: 0,
                tokens_out: 0,
            });
        point.requests += 1;
        point.tokens_in += entry.input_tokens;
        point.tokens_out += entry.output_tokens;
    }
    let mut items: Vec<ModelTokenUsage> = table.into_values().collect();
    items.sort_by(|left, right| {
        right
            .requests
            .cmp(&left.requests)
            .then_with(|| left.model.cmp(&right.model))
    });
    items
}

// ---------------------------------------------------------------- infra

/// Distributed rate limiter.
#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Returns `(allowed, release_id)`. A non-empty `release_id` must later be
    /// handed to [`Self::release_concurrency`].
    async fn allow(
        &self,
        identity: &str,
        tokens: i64,
        model: &str,
        group_id: Option<Id>,
    ) -> anyhow::Result<(bool, Option<String>)>;

    /// Frees a reserved concurrency slot.
    async fn release_concurrency(&self, identity: &str, release_id: &str) -> anyhow::Result<()>;
}

/// Per-upstream circuit breaker.
#[async_trait]
pub trait CircuitBreaker: Send + Sync {
    /// Whether the breaker currently allows a request.
    async fn allow(&self, provider: &str) -> anyhow::Result<bool>;
    /// Record a success or failure against the breaker.
    async fn record(&self, provider: &str, success: bool);
}

/// Redis key/value operations backing idempotent replay.
///
/// The serialization and the claim/replay policy live in
/// [`crate::idempotency`].
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// `GET key`.
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>>;
    /// `SET key value EX ttl`.
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Duration) -> anyhow::Result<()>;
    /// `SET key value NX EX ttl`; `Ok(true)` when the caller now owns the key.
    async fn set_nx(&self, key: &str, value: Vec<u8>, ttl: Duration) -> anyhow::Result<bool>;
    /// `DEL key`.
    async fn delete(&self, key: &str) -> anyhow::Result<()>;
}

/// Per-upstream-account routing policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPolicy {
    pub auth_id: String,
    pub weight: i64,
    pub priority: i64,
    pub enabled: bool,
}

/// Source of [`ChannelPolicy`] rows.
#[async_trait]
pub trait ChannelPolicyStore: Send + Sync {
    async fn list_channel_policies(&self) -> anyhow::Result<Vec<ChannelPolicy>>;
}

/// Sink for the two gauges this crate observes but does not export.
///
/// `/metrics/prometheus` belongs to the composition root, fed from the
/// channel-health benched count and the orphaned-holds counter. `gw-server`
/// owns the route and the exposition format; this trait is how the values reach
/// it, and it lines up 1:1 with `gw_server::Metrics::set_channel_benched` /
/// `set_orphaned_holds`.
pub trait MetricsSink: Send + Sync {
    /// Upstream accounts currently benched by [`crate::channel::ChannelHealth`].
    /// Feeds `agw_channel_benched_total`.
    fn set_channel_benched(&self, count: i64);

    /// Holds orphaned by a prior crash, as of the last reconcile scan.
    /// Feeds `agw_orphaned_holds`.
    fn set_orphaned_holds(&self, count: i64);
}

/// The sink used when no exporter is wired, so tests and embedders need not
/// supply one. Dropping the values is correct here: they are observability
/// only, and nothing in the billing path reads them back.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscardMetrics;

impl MetricsSink for DiscardMetrics {
    fn set_channel_benched(&self, _count: i64) {}
    fn set_orphaned_holds(&self, _count: i64) {}
}

/// One entry of the `GET /v1/models` catalogue.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelEntry {
    pub id: String,
    /// Unix seconds, as the OpenAI models payload expects.
    pub created: i64,
    pub owned_by: String,
    /// Combined input+output context when the catalog recorded one.
    pub context_length: Option<i64>,
    /// Per-request output cap when the catalog recorded one.
    pub max_output_tokens: Option<i64>,
    /// Only `text` and `image` are accepted; unknown values are dropped.
    pub input_modalities: Vec<String>,
    /// Selectable reasoning efforts; absent when the model has no thinking.
    pub reasoning: Option<ModelReasoning>,
}

/// Adapter-facing reasoning list copied from `model_catalog_entries.capabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelReasoning {
    pub efforts: Vec<ModelReasoningEffort>,
    pub default_effort: Option<String>,
}

/// One opaque effort id the adapter may send back as `reasoning_effort`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelReasoningEffort {
    pub id: String,
    pub name: String,
}

/// Source of the model catalogue served by `GET /v1/models`.
#[async_trait]
pub trait ModelCatalog: Send + Sync {
    async fn list_models(&self) -> anyhow::Result<Vec<ModelEntry>>;

    /// One visible catalogue entry, or `None` when the id is hidden or absent.
    ///
    /// The default walks [`Self::list_models`]. Adapters that can look up a
    /// single row — or a snapshot keyed by id — override this so a detail
    /// request does not rescan the listing.
    async fn get_model(&self, id: &str) -> anyhow::Result<Option<ModelEntry>> {
        Ok(self
            .list_models()
            .await?
            .into_iter()
            .find(|model| model.id == id))
    }

    /// **路由用**的 `model_id → channel_key[]`，顺序即优先级。
    ///
    /// 这是 `gw_relay::endpoint::upstream` 四级链 L2 的数据源，喂给
    /// [`crate::adapters::catalog::CatalogChannelResolver`]。
    ///
    /// # 为什么不能复用 [`Self::list_models`]
    ///
    /// 那条查询带 `WHERE visible = TRUE`，而 `visible` 是**「对租户展示」**开关，
    /// 不是**「允许调用」**开关 —— 今天一个 `visible = false` 的模型照样能被调用
    /// （前缀猜测根本不看这张表）。路由查询若继承了它，会**静默地**把所有隐藏模型
    /// 变成不可调用，表现为「某些模型突然 503」，极难归因。
    /// 所以这是**独立的一条 SQL**，不是给 `list_models` 加参数。
    ///
    /// 默认实现返回空 —— 对不提供路由数据的实现，四级链直接落 L4 兜底，
    /// 行为与收敛前逐字节相同。
    async fn resolve_channels(&self, _model_id: &str) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// 全量的 `model_id → channel_key[]`，供快照式缓存一次拉完。
    ///
    /// 默认实现返回空，理由同 [`Self::resolve_channels`]。
    async fn model_routes(&self) -> anyhow::Result<Vec<(String, Vec<String>)>> {
        Ok(Vec::new())
    }
}

/// Access metadata handed from [`crate::access`] to [`crate::hold`].
///
/// The fields keep their types (no stringification) because the boundary is
/// in-process. [`AccessMetadata::to_map`] reproduces the wire shape for logging
/// and parity tests.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccessMetadata {
    pub user_id: Id,
    pub api_key_id: Id,
    pub group_id: Option<Id>,
    pub rate_mult: f64,
    pub subscription: Option<SubscriptionQuota>,
}

impl AccessMetadata {
    /// Reproduces the `map[string]string` metadata payload.
    pub fn to_map(&self) -> HashMap<String, String> {
        let mut meta = HashMap::new();
        meta.insert("user_id".to_owned(), self.user_id.to_string());
        meta.insert("rate_mult".to_owned(), fmt_float(self.rate_mult));
        if self.api_key_id != 0 {
            meta.insert("api_key_id".to_owned(), self.api_key_id.to_string());
        }
        if let Some(gid) = self.group_id {
            meta.insert("group_id".to_owned(), gid.to_string());
        }
        if let Some(sub) = &self.subscription {
            meta.insert("subscription_id".to_owned(), sub.id.to_string());
            meta.insert("subscription_group_id".to_owned(), sub.group_id.to_string());
            if let Some(v) = sub.daily_limit_usd {
                meta.insert("daily_limit".to_owned(), fmt_float(v));
            }
            if let Some(v) = sub.weekly_limit_usd {
                meta.insert("weekly_limit".to_owned(), fmt_float(v));
            }
            if let Some(v) = sub.monthly_limit_usd {
                meta.insert("monthly_limit".to_owned(), fmt_float(v));
            }
            meta.insert("daily_used".to_owned(), fmt_float(sub.daily_usage_usd));
            meta.insert("weekly_used".to_owned(), fmt_float(sub.weekly_usage_usd));
            meta.insert("monthly_used".to_owned(), fmt_float(sub.monthly_usage_usd));
        }
        meta
    }
}

/// Shortest decimal round-trip, never exponent notation for the magnitudes
/// this codebase deals in.
fn fmt_float(f: f64) -> String {
    let s = format!("{f}");
    if s.contains(['e', 'E']) {
        format!("{f:.10}")
    } else {
        s
    }
}

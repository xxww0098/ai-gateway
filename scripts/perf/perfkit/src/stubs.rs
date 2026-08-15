//! 把真 `gw-proxy` 装起来所需的最小端口实现。
//!
//! # 这些桩测的是什么、不测什么
//!
//! 它们是 **in-memory 常量应答**，不碰 Postgres / Redis。这是刻意的：
//! 本基线要量的是「转发路径本身的固定开销」，也就是 gw-relay 将来要优化的
//! 那部分。真 Postgres / Redis 的 RTT 是另一条曲线，它不随 body 大小变化，
//! 也不会被零拷贝改掉，混进来只会把网关自身的 µs 级开销淹没在 ms 级 IO 里。
//!
//! **代价必须写清楚**：因此本基线量到的 hold/settle 开销是**下界**。生产上
//! hold 还要多一次 Redis Lua、settle 还要一次 PG 事务。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gw_authcore::{AuthRecord, AuthStore, Claims};
use gw_proxy::ports::{
    AccessMetadata, ApiKeyRow, AuthCrypto, BalanceEvent, BillingError, BillingLedger,
    ChannelPolicy, ChannelPolicyStore, CircuitBreaker, Id, IdempotencyStore, ModelCatalog,
    ModelEntry, PricingCalculator, RateLimiter, SettleReceipt, SettlementCommit,
    SubscriptionQuota, SubscriptionQuotaStore, TenantDirectory, TokenUsage, UsageLogEntry,
    UsageStore,
};

/// 压测租户。
pub const PERF_USER_ID: Id = 7;

pub use crate::PERF_API_KEY;

// ---------------------------------------------------------------- 计费

/// 永远有钱、永远成功的账本。
#[derive(Default)]
pub struct NullLedger {
    pub holds: AtomicU64,
    pub settles: AtomicU64,
}

#[async_trait]
impl BillingLedger for NullLedger {
    async fn hold(
        &self,
        _user_id: Id,
        _amount: f64,
        _request_id: &str,
        _ttl: Duration,
    ) -> Result<(), BillingError> {
        self.holds.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn settle(
        &self,
        _user_id: Id,
        _request_id: &str,
        _actual_amount: f64,
    ) -> Result<f64, BillingError> {
        self.settles.fetch_add(1, Ordering::Relaxed);
        Ok(0.0)
    }

    async fn release(&self, _user_id: Id, _request_id: &str) -> Result<(), BillingError> {
        Ok(())
    }

    async fn active_hold_amount(
        &self,
        _user_id: Id,
        _request_id: &str,
    ) -> Result<Option<f64>, BillingError> {
        Ok(Some(0.004))
    }

    async fn has_unresolved_shortfall(&self, _user_id: Id) -> Result<bool, BillingError> {
        Ok(false)
    }

    async fn available_balance(&self, _user_id: Id) -> Result<f64, BillingError> {
        Ok(1_000_000.0)
    }
}

/// 定价：常量算术，无查表、无锁。
///
/// 真 `gw-pricing` 会查 `ModelPriceCache`（一次 `ArcSwap` 读 + HashMap 查）。
/// 那部分开销**不在**本基线里，理由同模块头。
pub struct FlatCalculator;

impl PricingCalculator for FlatCalculator {
    fn estimate(&self, _model: &str, stream: bool, rate_mult: f64) -> f64 {
        (if stream { 0.004 } else { 0.002 }) * rate_mult
    }

    fn estimate_with_max_tokens(
        &self,
        _model: &str,
        max_output_tokens: i64,
        stream: bool,
        rate_mult: f64,
    ) -> f64 {
        let cap = if max_output_tokens > 0 {
            max_output_tokens as f64 * 1e-6
        } else if stream {
            0.004
        } else {
            0.002
        };
        cap * rate_mult
    }

    fn estimate_with_tokens(
        &self,
        _model: &str,
        input_tokens: i64,
        max_output_tokens: i64,
        _stream: bool,
        rate_mult: f64,
    ) -> f64 {
        (input_tokens as f64 * 1e-6 + max_output_tokens as f64 * 1e-6) * rate_mult
    }

    fn compute(&self, _model: &str, tokens: TokenUsage, rate_mult: f64) -> f64 {
        (tokens.input as f64 * 1e-6 + tokens.output as f64 * 2e-6) * rate_mult
    }
}

/// 结算落库：全部丢弃，只记次数。
#[derive(Default)]
pub struct NullUsageStore {
    pub commits: AtomicU64,
    pub logs: AtomicU64,
}

#[async_trait]
impl UsageStore for NullUsageStore {
    async fn commit_settlement(
        &self,
        _commit: &SettlementCommit,
    ) -> anyhow::Result<SettleReceipt> {
        self.commits.fetch_add(1, Ordering::Relaxed);
        Ok(SettleReceipt::Committed {
            shortfall: 0.0,
            balance_before: 1_000_000.0,
            balance_after: 999_999.0,
        })
    }

    async fn insert_usage_log(&self, _entry: &UsageLogEntry) -> anyhow::Result<()> {
        self.logs.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn insert_balance_event(&self, _event: &BalanceEvent) -> anyhow::Result<()> {
        Ok(())
    }

    async fn clear_hold(&self, _user_id: Id, _request_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- 身份

/// 真 SHA-256（`gw_authcore::hash_api_key`），因为它是 access 层每请求都要
/// 付的真实成本，桩掉它会让 access 的数字失真。JWT 不测。
pub struct RealCrypto;

impl AuthCrypto for RealCrypto {
    fn hash_api_key(&self, plaintext: &str) -> String {
        gw_authcore::hash_api_key(plaintext)
    }

    fn sha256_hex(&self, input: &str) -> String {
        gw_authcore::hash_api_key(input)
    }

    fn verify_jwt(&self, _token: &str) -> Option<Claims> {
        None
    }
}

/// 单租户目录：命中即返回，无缓存层、无 DB。
pub struct StaticDirectory {
    key_hash: String,
    row: ApiKeyRow,
}

impl StaticDirectory {
    /// 用 [`PERF_API_KEY`] 的 SHA-256 建一个活跃租户。
    #[must_use]
    pub fn new() -> Self {
        Self {
            key_hash: gw_authcore::hash_api_key(PERF_API_KEY),
            row: ApiKeyRow {
                id: 3,
                user_id: PERF_USER_ID,
                group_id: None,
                status: "active".to_owned(),
            },
        }
    }
}

impl Default for StaticDirectory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TenantDirectory for StaticDirectory {
    async fn api_key_by_hash(&self, key_hash: &str) -> anyhow::Result<Option<ApiKeyRow>> {
        Ok((key_hash == self.key_hash).then(|| self.row.clone()))
    }

    async fn group_rate_multiplier(&self, _group_id: Id) -> anyhow::Result<Option<f64>> {
        Ok(None)
    }

    async fn user_status(&self, _user_id: Id) -> anyhow::Result<Option<String>> {
        Ok(Some("active".to_owned()))
    }

    async fn active_subscription(&self, _user_id: Id) -> anyhow::Result<Option<SubscriptionQuota>> {
        Ok(None)
    }

    async fn holds_group_entitlement(
        &self,
        _user_id: Id,
        _group_id: Id,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }

    async fn touch_api_key(&self, _api_key_id: Id) {}
}

/// 订阅配额：不存在（等价于生产上没买订阅的租户）。
pub struct NoQuotaStore;

#[async_trait]
impl SubscriptionQuotaStore for NoQuotaStore {
    async fn lock_and_rotate(
        &self,
        _subscription_id: Id,
        _now: DateTime<Utc>,
    ) -> anyhow::Result<Option<SubscriptionQuota>> {
        Ok(None)
    }
}

// ---------------------------------------------------------------- infra

/// 永远放行的限流器。生产上这里是一次 Redis Lua。
pub struct AllowAllRateLimiter;

#[async_trait]
impl RateLimiter for AllowAllRateLimiter {
    async fn allow(
        &self,
        _identity: &str,
        _tokens: i64,
        _model: &str,
        _group_id: Option<Id>,
    ) -> anyhow::Result<(bool, Option<String>)> {
        Ok((true, None))
    }

    async fn release_concurrency(&self, _identity: &str, _release_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

/// 永远闭合的熔断器。
pub struct ClosedBreaker;

#[async_trait]
impl CircuitBreaker for ClosedBreaker {
    async fn allow(&self, _provider: &str) -> anyhow::Result<bool> {
        Ok(true)
    }

    async fn record(&self, _provider: &str, _success: bool) {}
}

/// 进程内幂等存储。用来量「带 Idempotency-Key 时 hold 层缓冲整个响应体」
/// 这条开销 —— 那是 `hold::capture_body`，只在有 key 时才走。
#[derive(Default)]
pub struct MemIdempotencyStore {
    map: std::sync::Mutex<HashMap<String, Vec<u8>>>,
}

#[async_trait]
impl IdempotencyStore for MemIdempotencyStore {
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self.map.lock().expect("idempotency mutex").get(key).cloned())
    }

    async fn set(&self, key: &str, value: Vec<u8>, _ttl: Duration) -> anyhow::Result<()> {
        self.map
            .lock()
            .expect("idempotency mutex")
            .insert(key.to_owned(), value);
        Ok(())
    }

    async fn set_nx(&self, key: &str, value: Vec<u8>, _ttl: Duration) -> anyhow::Result<bool> {
        let mut map = self.map.lock().expect("idempotency mutex");
        if map.contains_key(key) {
            return Ok(false);
        }
        map.insert(key.to_owned(), value);
        Ok(true)
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.map.lock().expect("idempotency mutex").remove(key);
        Ok(())
    }
}

/// 无 channel 策略行 —— `ChannelPool` 退化为等权轮询。
pub struct EmptyPolicyStore;

#[async_trait]
impl ChannelPolicyStore for EmptyPolicyStore {
    async fn list_channel_policies(&self) -> anyhow::Result<Vec<ChannelPolicy>> {
        Ok(Vec::new())
    }
}

/// 单条目录。
pub struct OneModelCatalog;

#[async_trait]
impl ModelCatalog for OneModelCatalog {
    async fn list_models(&self) -> anyhow::Result<Vec<ModelEntry>> {
        Ok(vec![ModelEntry {
            id: "gpt-4o".to_owned(),
            created: 0,
            owned_by: "openai".to_owned(),
        }])
    }
}

// ---------------------------------------------------------------- 凭证

/// 固定一条 openai 凭证的 `AuthStore`。
///
/// 注意 `Dispatcher::auths_for` 每请求都会 `list()` 再 `filter` 再 `collect`，
/// 也就是每请求克隆一整份 `Vec<AuthRecord>` —— 这条开销**在**基线里，是有意
/// 保留的，它正是热路径清单里要量的东西。
pub struct OneAuthStore {
    records: Vec<AuthRecord>,
}

impl OneAuthStore {
    /// 一条 `openai` 活跃凭证，`api_key` 写进 metadata 供 executor 取用。
    #[must_use]
    pub fn new(api_key: &str) -> Arc<Self> {
        let mut record = AuthRecord::new("perf-acct-1", "openai", Utc::now());
        record.metadata = serde_json::json!({ "api_key": api_key });
        Arc::new(Self {
            records: vec![record],
        })
    }
}

#[async_trait]
impl AuthStore for OneAuthStore {
    async fn list(&self) -> anyhow::Result<Vec<AuthRecord>> {
        Ok(self.records.clone())
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<AuthRecord>> {
        Ok(self.records.iter().find(|r| r.id == id).cloned())
    }

    async fn save(&self, _record: &AuthRecord) -> anyhow::Result<()> {
        Ok(())
    }

    async fn delete(&self, _id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

/// `AccessMetadata` 的形状检查用不到，这里只是让 `use` 不白引。
#[must_use]
pub fn perf_access_metadata() -> AccessMetadata {
    AccessMetadata {
        user_id: PERF_USER_ID,
        api_key_id: 3,
        group_id: None,
        rate_mult: 1.0,
        subscription: None,
    }
}

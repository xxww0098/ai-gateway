//! Doubles for the money path: ledger, calculator, quota and usage stores.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::ports::{
    BalanceEvent, BillingError, BillingLedger, HoldAdmit, Id, ModelTokenUsage, PricingCalculator,
    SettleReceipt, SettlementCommit, SubscriptionQuota, SubscriptionQuotaStore, TokenUsage,
    UsageLogEntry, UsageStore,
};
use crate::reconcile::{StaleHold, StaleHoldScanner};

/// 把逐条 usage 折成按模型的 token 小计。空模型名变成 `unknown`。
///
/// 排序与面板 `buildUsageModels` 同口径：请求数降序，同数按模型名升序。
/// 只有测试侧用它 —— 生产的 `SqlUsageStore::model_usage_since` 在 SQL 里
/// GROUP BY，这份内存实现给 `FakeUsageStore` 与对拍断言共用。
pub(crate) fn fold_model_usage<'a>(
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

/// Records every ledger call so a test can assert on ordering and amounts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LedgerCall {
    Hold { user_id: Id, amount: f64 },
    Release { user_id: Id },
}

#[derive(Default)]
pub(crate) struct FakeLedger {
    pub(crate) calls: Mutex<Vec<LedgerCall>>,
    pub(crate) balance: Mutex<f64>,
    pub(crate) holds: Mutex<HashMap<String, f64>>,
    pub(crate) shortfall: Mutex<bool>,
    pub(crate) shortfall_errors: Mutex<bool>,
    pub(crate) hold_lookup_errors: Mutex<bool>,
    pub(crate) hold_fails_with: Mutex<Option<BillingError>>,
}

impl FakeLedger {
    pub(crate) fn with_balance(balance: f64) -> Arc<Self> {
        let ledger = Self::default();
        *ledger.balance.lock() = balance;
        Arc::new(ledger)
    }

    pub(crate) fn calls(&self) -> Vec<LedgerCall> {
        self.calls.lock().clone()
    }

    pub(crate) fn held_amount(&self, request_id: &str) -> Option<f64> {
        self.holds.lock().get(request_id).copied()
    }

    fn available_after_holds(balance: f64, holds: &HashMap<String, f64>) -> f64 {
        balance - holds.values().sum::<f64>()
    }

    /// 直接落一个 hold，测试造「已有预扣」的初始状态用（不是 trait 方法：
    /// 生产路径只走 `hold_gated`）。
    pub(crate) async fn hold(
        &self,
        user_id: Id,
        amount: f64,
        request_id: &str,
        _ttl: Duration,
    ) -> Result<(), BillingError> {
        if let Some(err) = self.hold_fails_with.lock().take() {
            return Err(err);
        }
        self.calls.lock().push(LedgerCall::Hold { user_id, amount });
        self.holds.lock().insert(request_id.to_owned(), amount);
        Ok(())
    }
}

#[async_trait]
impl BillingLedger for FakeLedger {
    async fn release(&self, user_id: Id, request_id: &str) -> Result<(), BillingError> {
        self.calls.lock().push(LedgerCall::Release { user_id });
        self.holds.lock().remove(request_id);
        Ok(())
    }

    async fn active_hold_amount(
        &self,
        _user_id: Id,
        request_id: &str,
    ) -> Result<Option<f64>, BillingError> {
        if *self.hold_lookup_errors.lock() {
            return Err(BillingError::Other(anyhow::anyhow!("redis down")));
        }
        Ok(self.holds.lock().get(request_id).copied())
    }

    async fn has_unresolved_shortfall(&self, _user_id: Id) -> Result<bool, BillingError> {
        if *self.shortfall_errors.lock() {
            return Err(BillingError::Other(anyhow::anyhow!("db down")));
        }
        Ok(*self.shortfall.lock())
    }

    async fn available_balance(&self, _user_id: Id) -> Result<f64, BillingError> {
        let balance = self.balance.lock();
        let holds = self.holds.lock();
        Ok(Self::available_after_holds(*balance, &holds))
    }

    /// 单 Mutex 内完成 floor 检查与预扣，模拟生产 Lua 的原子语义。
    async fn hold_gated(
        &self,
        user_id: Id,
        amount: f64,
        min_available: f64,
        request_id: &str,
        _ttl: Duration,
    ) -> Result<HoldAdmit, BillingError> {
        if let Some(err) = self.hold_fails_with.lock().take() {
            return Err(err);
        }
        let balance = self.balance.lock();
        let mut holds = self.holds.lock();
        // Same contract as the Lua hold script: a repeated request id is a
        // no-op, not a second reservation.
        if holds.contains_key(request_id) {
            return Ok(HoldAdmit::Reserved);
        }
        let available = Self::available_after_holds(*balance, &holds);
        if available < min_available {
            return Ok(HoldAdmit::Insufficient { available });
        }
        self.calls.lock().push(LedgerCall::Hold { user_id, amount });
        holds.insert(request_id.to_owned(), amount);
        Ok(HoldAdmit::Reserved)
    }
}

/// Linear calculator: cost is a fixed rate per token, and the estimates are
/// monotone in the inputs. Deliberately NOT the production price table — the
/// tests assert relationships (ordering, monotonicity), not magic numbers.
pub(crate) struct FakeCalculator {
    pub(crate) per_token: f64,
    pub(crate) nominal_output: i64,
}

impl Default for FakeCalculator {
    fn default() -> Self {
        Self {
            per_token: 0.001,
            nominal_output: 1000,
        }
    }
}

impl FakeCalculator {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl PricingCalculator for FakeCalculator {
    fn estimate(&self, _model: &str, stream: bool, rate_mult: f64) -> f64 {
        let output = if stream {
            self.nominal_output * 2
        } else {
            self.nominal_output
        };
        output as f64 * self.per_token * rate_mult
    }

    fn estimate_with_max_tokens(
        &self,
        model: &str,
        max_output_tokens: i64,
        stream: bool,
        rate_mult: f64,
    ) -> f64 {
        if max_output_tokens <= 0 {
            return self.estimate(model, stream, rate_mult);
        }
        max_output_tokens as f64 * self.per_token * rate_mult
    }

    fn estimate_with_tokens(
        &self,
        _model: &str,
        input_tokens: i64,
        max_output_tokens: i64,
        stream: bool,
        rate_mult: f64,
    ) -> f64 {
        let output = if max_output_tokens > 0 {
            max_output_tokens
        } else if stream {
            self.nominal_output * 2
        } else {
            self.nominal_output
        };
        (input_tokens + output) as f64 * self.per_token * rate_mult
    }

    fn compute(&self, _model: &str, tokens: TokenUsage, rate_mult: f64) -> f64 {
        (tokens.input + tokens.output + tokens.cached + tokens.reasoning) as f64
            * self.per_token
            * rate_mult
    }
}

/// Applies [`crate::hold::rotate_counters`] to an in-memory snapshot, the way a
/// real store applies it inside `SELECT ... FOR UPDATE`.
#[derive(Default)]
pub(crate) struct FakeQuotaStore {
    pub(crate) quotas: Mutex<HashMap<Id, SubscriptionQuota>>,
    pub(crate) errors: Mutex<bool>,
}

impl FakeQuotaStore {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl SubscriptionQuotaStore for FakeQuotaStore {
    async fn lock_and_rotate(
        &self,
        subscription_id: Id,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<SubscriptionQuota>> {
        if *self.errors.lock() {
            anyhow::bail!("lock failed");
        }
        let mut quotas = self.quotas.lock();
        let Some(quota) = quotas.get_mut(&subscription_id) else {
            return Ok(None);
        };
        crate::hold::rotate_counters(quota, now);
        Ok(Some(quota.clone()))
    }
}

#[derive(Default)]
pub(crate) struct FakeUsageStore {
    pub(crate) commits: Mutex<Vec<SettlementCommit>>,
    pub(crate) logs: Mutex<Vec<UsageLogEntry>>,
    pub(crate) balance_events: Mutex<Vec<BalanceEvent>>,
    pub(crate) cleared_holds: Mutex<Vec<String>>,
    pub(crate) logged_requests: Mutex<Vec<String>>,
    pub(crate) commit_fails: Mutex<bool>,
    pub(crate) shortfall: Mutex<f64>,
    pub(crate) balance_after: Mutex<f64>,
    pub(crate) balance_before: Mutex<f64>,
    /// When set, the next `commit_settlement` waits until the sender fires.
    /// Tests use this to prove a unary HTTP response is not blocked on ledger I/O.
    pub(crate) commit_gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl FakeUsageStore {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn settled_costs(&self) -> Vec<f64> {
        self.commits.lock().iter().map(|c| c.actual_cost).collect()
    }

    /// Park the next settlement commit until the returned sender is fired.
    pub(crate) fn hold_commits(&self) -> tokio::sync::oneshot::Sender<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        *self.commit_gate.lock() = Some(rx);
        tx
    }
}

#[async_trait]
impl UsageStore for FakeUsageStore {
    async fn commit_settlement(&self, commit: &SettlementCommit) -> anyhow::Result<SettleReceipt> {
        let gate = self.commit_gate.lock().take();
        if let Some(rx) = gate {
            let _ = rx.await;
        }
        if *self.commit_fails.lock() {
            anyhow::bail!("settle transaction failed");
        }
        if self.logged_requests.lock().contains(&commit.request_id) {
            return Ok(SettleReceipt::AlreadySettled);
        }
        self.commits.lock().push(commit.clone());
        let mut entry = commit.entry.clone();
        let shortfall = *self.shortfall.lock();
        entry.raw_metadata = crate::usage::merge_shortfall(entry.raw_metadata, shortfall);
        self.logs.lock().push(entry);
        self.logged_requests.lock().push(commit.request_id.clone());
        Ok(SettleReceipt::Committed {
            shortfall,
            balance_before: *self.balance_before.lock(),
            balance_after: *self.balance_after.lock(),
            balance_version: Some(1),
        })
    }

    async fn insert_usage_log(&self, entry: &UsageLogEntry) -> anyhow::Result<()> {
        self.logs.lock().push(entry.clone());
        self.logged_requests.lock().push(entry.request_id.clone());
        Ok(())
    }

    async fn insert_balance_event(&self, event: &BalanceEvent) -> anyhow::Result<()> {
        self.balance_events.lock().push(event.clone());
        Ok(())
    }

    async fn clear_hold(
        &self,
        _user_id: Id,
        request_id: &str,
        _published: Option<(f64, i64)>,
    ) -> anyhow::Result<()> {
        self.cleared_holds.lock().push(request_id.to_owned());
        Ok(())
    }

    async fn model_usage_since(
        &self,
        user_id: Id,
        _since: DateTime<Utc>,
    ) -> anyhow::Result<Vec<ModelTokenUsage>> {
        // 内存双没有 created_at：测试只种「今日」行，窗口过滤交给 SQL 实现。
        let logs = self.logs.lock();
        Ok(fold_model_usage(
            logs.iter().filter(|entry| entry.user_id == user_id),
        ))
    }
}

#[derive(Default)]
pub(crate) struct FakeScanner {
    pub(crate) stale: Mutex<Vec<StaleHold>>,
    pub(crate) errors: Mutex<bool>,
}

#[async_trait]
impl StaleHoldScanner for FakeScanner {
    async fn scan_stale_holds(&self, _older_than: Duration) -> anyhow::Result<Vec<StaleHold>> {
        if *self.errors.lock() {
            anyhow::bail!("scan failed");
        }
        Ok(self.stale.lock().clone())
    }
}

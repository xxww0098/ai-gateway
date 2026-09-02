//! Doubles for the money path: ledger, calculator, quota and usage stores.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use gw_ledger::operation::{OperationState, Transition, terminate};
use gw_ledger::{BillingOperationId, NewOperation};
use gw_pricing::PricingQuote;

use crate::ports::{
    BalanceEvent, BillingError, BillingLedger, HoldAdmit, Id, ModelTokenUsage, PricingCalculator,
    QuotaAdmission, SettleReceipt, SettleTerminal, SettlementCommit, SubscriptionQuota,
    SubscriptionQuotaStore, UsageLogEntry, UsageStore, fold_model_usage,
};
use crate::reconcile::{NonTerminalOperationScanner, OrphanedOperation};

/// Records every ledger call so a test can assert on ordering and amounts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LedgerCall {
    Hold { user_id: Id, amount: f64 },
    Settle { user_id: Id, amount: f64 },
    Release { user_id: Id },
}

#[derive(Default)]
pub(crate) struct FakeLedger {
    pub(crate) calls: Mutex<Vec<LedgerCall>>,
    pub(crate) balance: Mutex<f64>,
    pub(crate) holds: Mutex<HashMap<String, f64>>,
    /// The durable half of the operation state machine, keyed by operation id.
    /// The double runs the *same* [`terminate`] the Postgres path encodes in
    /// SQL, so "settle once" means the same thing in both.
    pub(crate) operations: Mutex<HashMap<String, OperationState>>,
    pub(crate) shortfall: Mutex<bool>,
    pub(crate) shortfall_errors: Mutex<bool>,
    pub(crate) hold_lookup_errors: Mutex<bool>,
    pub(crate) hold_fails_with: Mutex<Option<BillingError>>,
    /// 每一次成功的续租留下的操作 id，供测试断言「续了几次」。
    pub(crate) renewals: Mutex<Vec<String>>,
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

    pub(crate) fn held_amount(&self, operation: &BillingOperationId) -> Option<f64> {
        self.holds.lock().get(operation.as_str()).copied()
    }

    /// The persisted state of one operation, or `None` if it was never
    /// admitted.
    pub(crate) fn operation_state(&self, operation: &BillingOperationId) -> Option<OperationState> {
        self.operations.lock().get(operation.as_str()).copied()
    }

    /// Every operation id this ledger has admitted.
    pub(crate) fn admitted_operations(&self) -> Vec<String> {
        self.operations.lock().keys().cloned().collect()
    }

    /// Runs the shared terminal decision. `Some(state)` when this caller owns
    /// the transition; `None` when someone already terminated it.
    fn claim_terminal(&self, operation: &BillingOperationId, to: OperationState) -> Option<()> {
        let mut operations = self.operations.lock();
        let current = *operations.get(operation.as_str())?;
        match terminate(current, to) {
            Transition::AlreadyTerminal(_) => None,
            Transition::Apply(next) => {
                operations.insert(operation.as_str().to_owned(), next);
                Some(())
            }
        }
    }

    /// Admits `operation` and reserves `amount`, the way the hold middleware
    /// would. A test that wants a live reservation to settle against calls
    /// this instead of reaching for the ledger's request path.
    pub(crate) async fn plant_hold(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
        amount: f64,
    ) {
        self.admit_operation(
            &NewOperation {
                operation_id: operation.clone(),
                user_id,
                reserved_amount: amount,
                admitted_liability: amount,
                request_fingerprint: "fingerprint".to_owned(),
                client_trace_id: "trace-the-client-saw".to_owned(),
            },
            Some(Duration::from_secs(60)),
        )
        .await
        .expect("plant hold");
    }

    fn available_after_holds(balance: f64, holds: &HashMap<String, f64>) -> f64 {
        balance - holds.values().sum::<f64>()
    }
}

#[async_trait]
impl BillingLedger for FakeLedger {
    /// 单 Mutex 内完成 floor 检查与预扣，模拟生产 Lua 的原子语义 ——
    /// 外加持久那一行，两条预留路径都写。
    async fn admit_operation(
        &self,
        operation: &NewOperation,
        redis_ttl: Option<Duration>,
    ) -> Result<HoldAdmit, BillingError> {
        if let Some(err) = self.hold_fails_with.lock().take() {
            return Err(err);
        }
        let balance = self.balance.lock();
        let mut holds = self.holds.lock();
        let available = Self::available_after_holds(*balance, &holds);
        if available < operation.admitted_liability {
            return Ok(HoldAdmit::Insufficient { available });
        }
        self.operations.lock().insert(
            operation.operation_id.as_str().to_owned(),
            OperationState::Held,
        );
        self.calls.lock().push(LedgerCall::Hold {
            user_id: operation.user_id,
            amount: operation.reserved_amount,
        });
        // `None` means the reservation came from the budget token, so only the
        // durable row exists — exactly as in production.
        if redis_ttl.is_some() {
            holds.insert(
                operation.operation_id.as_str().to_owned(),
                operation.reserved_amount,
            );
        }
        Ok(HoldAdmit::Reserved)
    }

    async fn settle_once(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
        actual_amount: f64,
    ) -> Result<SettleTerminal, BillingError> {
        if self
            .claim_terminal(operation, OperationState::Settled)
            .is_none()
        {
            return Ok(SettleTerminal::AlreadyTerminal);
        }
        self.calls.lock().push(LedgerCall::Settle {
            user_id,
            amount: actual_amount,
        });
        self.holds.lock().remove(operation.as_str());
        let mut balance = self.balance.lock();
        let debited = actual_amount.min(*balance);
        *balance -= debited;
        Ok(SettleTerminal::Debited {
            shortfall: actual_amount - debited,
        })
    }

    async fn release_once(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
    ) -> Result<(), BillingError> {
        if self
            .claim_terminal(operation, OperationState::Released)
            .is_none()
        {
            return Ok(());
        }
        self.calls.lock().push(LedgerCall::Release { user_id });
        self.holds.lock().remove(operation.as_str());
        Ok(())
    }

    async fn active_hold_amount(
        &self,
        _user_id: Id,
        operation: &BillingOperationId,
    ) -> Result<Option<f64>, BillingError> {
        if *self.hold_lookup_errors.lock() {
            return Err(BillingError::Other(anyhow::anyhow!("redis down")));
        }
        Ok(self.holds.lock().get(operation.as_str()).copied())
    }

    /// 续租只推时间：这里唯一能观察到的「时间」就是它**没动**分数。
    /// 缺失的预留不许被续租凭空造出来。
    async fn renew_lease(
        &self,
        _user_id: Id,
        operation: &BillingOperationId,
    ) -> Result<f64, BillingError> {
        let amount = self
            .holds
            .lock()
            .get(operation.as_str())
            .copied()
            .ok_or(BillingError::HoldNotFound)?;
        self.renewals.lock().push(operation.to_string());
        Ok(amount)
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
}

/// 一个「四列同价」的计价器：每 1M token 一个固定价，倍率线性。
///
/// 刻意**不是**生产价目表 —— 测试断言的是关系（次序、单调性），不是魔法数字。
/// 它返回真的 [`PricingQuote`]，所以估算与精算走的是生产那一份算术。
pub(crate) struct FakeCalculator {
    pub(crate) per_1m: Mutex<f64>,
}

impl Default for FakeCalculator {
    fn default() -> Self {
        Self {
            per_1m: Mutex::new(1_000.0),
        }
    }
}

impl FakeCalculator {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 改价。模拟「管理员在请求在途时改了价目表」：**已经铸造出去的报价
    /// 不该受影响**，只有下一次 `quote` 才看得见新价。
    pub(crate) fn set_price(&self, per_1m: f64) {
        *self.per_1m.lock() = per_1m;
    }
}

impl PricingCalculator for FakeCalculator {
    fn quote(&self, model: &str, rate_mult: f64) -> PricingQuote {
        PricingQuote::flat(model, *self.per_1m.lock(), rate_mult, 0)
    }
}

/// 在内存快照上跑与生产**同一条**准入链：锁 → 轮转 → 比「已用 + 在途 + 这一笔」
/// → 落预留。
///
/// 锁是 [`tokio::sync::Mutex`] 而不是 `parking_lot`，而且临界区里有一个真的
/// `yield_now().await` —— 那正是生产实现里的 SQL 往返。没有这一下，
/// 「两个并发请求抢最后一格」的测试会因为临界区不可能被打断而**永远通过**，
/// 也就测不出「比较搬进锁里」这件事。
#[derive(Default)]
pub(crate) struct FakeQuotaStore {
    state: tokio::sync::Mutex<QuotaState>,
    pub(crate) errors: Mutex<bool>,
}

#[derive(Default)]
pub(crate) struct QuotaState {
    quotas: HashMap<Id, SubscriptionQuota>,
    /// operation id → (subscription id, 预留金额)。
    reservations: HashMap<String, (Id, f64)>,
}

impl FakeQuotaStore {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 种一个订阅。
    pub(crate) async fn seed(&self, quota: SubscriptionQuota) {
        self.state.lock().await.quotas.insert(quota.id, quota);
    }

    /// 一个订阅当前的快照（已轮转的那份）。
    pub(crate) async fn quota(&self, subscription_id: Id) -> Option<SubscriptionQuota> {
        self.state
            .lock()
            .await
            .quotas
            .get(&subscription_id)
            .cloned()
    }

    /// 一个订阅上全部在途预留的合计。
    pub(crate) async fn reserved_total(&self, subscription_id: Id) -> f64 {
        self.state
            .lock()
            .await
            .reservations
            .values()
            .filter(|(id, _)| *id == subscription_id)
            .map(|(_, amount)| amount)
            .sum()
    }

    /// 预留 → 实际：删掉那一行，把 `actual` 加进三个计数器。
    ///
    /// 生产实现把这两步放在**扣款那个事务里**（`SqlUsageStore`），
    /// 这里由 [`FakeUsageStore`] 在它的「事务」里调，语义对齐。
    pub(crate) async fn settle_reservation(&self, operation: &BillingOperationId, actual: f64) {
        let mut state = self.state.lock().await;
        let Some((subscription_id, _)) = state.reservations.remove(operation.as_str()) else {
            return;
        };
        if actual <= 0.0 {
            return;
        }
        if let Some(quota) = state.quotas.get_mut(&subscription_id) {
            quota.daily_usage_usd += actual;
            quota.weekly_usage_usd += actual;
            quota.monthly_usage_usd += actual;
        }
    }
}

#[async_trait]
impl SubscriptionQuotaStore for FakeQuotaStore {
    async fn reserve(
        &self,
        subscription_id: Id,
        operation: &BillingOperationId,
        amount: f64,
        now: DateTime<Utc>,
    ) -> anyhow::Result<QuotaAdmission> {
        if *self.errors.lock() {
            anyhow::bail!("quota reserve failed");
        }
        let mut state = self.state.lock().await;
        if !state.quotas.contains_key(&subscription_id) {
            return Ok(QuotaAdmission::NoSubscription);
        }
        // 生产实现在这里要打好几趟 SQL；让出执行权把那段窗口如实建模出来。
        tokio::task::yield_now().await;

        // 同一个操作重复预留是恢复，不是第二笔。
        if state.reservations.contains_key(operation.as_str()) {
            return Ok(QuotaAdmission::Reserved);
        }

        let reserved: f64 = state
            .reservations
            .values()
            .filter(|(id, _)| *id == subscription_id)
            .map(|(_, amount)| amount)
            .sum();
        let quota = state
            .quotas
            .get_mut(&subscription_id)
            .expect("presence checked above");
        crate::hold::rotate_counters(quota, now);
        if let Some(reason) = crate::hold::evaluate_quota(quota, reserved, amount) {
            return Ok(QuotaAdmission::Exceeded { reason });
        }
        state
            .reservations
            .insert(operation.as_str().to_owned(), (subscription_id, amount));
        Ok(QuotaAdmission::Reserved)
    }

    async fn release_reservation(&self, operation: &BillingOperationId) -> anyhow::Result<()> {
        self.state
            .lock()
            .await
            .reservations
            .remove(operation.as_str());
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct FakeUsageStore {
    pub(crate) commits: Mutex<Vec<SettlementCommit>>,
    pub(crate) logs: Mutex<Vec<UsageLogEntry>>,
    pub(crate) balance_events: Mutex<Vec<BalanceEvent>>,
    pub(crate) cleared_holds: Mutex<Vec<String>>,
    /// Terminal state per operation. Mirrors the production guard: the state
    /// move and the debit happen together, so a second commit for the same
    /// operation moves nothing.
    pub(crate) terminated: Mutex<HashMap<String, OperationState>>,
    pub(crate) commit_fails: Mutex<bool>,
    pub(crate) shortfall: Mutex<f64>,
    pub(crate) balance_after: Mutex<f64>,
    pub(crate) balance_before: Mutex<f64>,
    /// When set, the next `commit_settlement` waits until the sender fires.
    /// Tests use this to prove a unary HTTP response is not blocked on ledger I/O.
    pub(crate) commit_gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    /// 挂上之后，结算会在它的「事务」里把配额预留转成实际用量 ——
    /// 与 `SqlUsageStore` 同一个事务边界。
    quota: Mutex<Option<Arc<FakeQuotaStore>>>,
}

impl FakeUsageStore {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn settled_costs(&self) -> Vec<f64> {
        self.commits.lock().iter().map(|c| c.actual_cost).collect()
    }

    /// 把配额存储接进结算事务。
    pub(crate) fn with_quota(&self, quota: Arc<FakeQuotaStore>) {
        *self.quota.lock() = Some(quota);
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
        // Unconditional once-guard, inside the "transaction" — no caller flag.
        {
            let mut terminated = self.terminated.lock();
            let key = commit.operation.as_str().to_owned();
            let current = terminated
                .get(&key)
                .copied()
                .unwrap_or(OperationState::Held);
            match terminate(current, OperationState::Settled) {
                Transition::AlreadyTerminal(_) => return Ok(SettleReceipt::AlreadyTerminal),
                Transition::Apply(next) => terminated.insert(key, next),
            };
        }
        // 与生产同一个事务边界：删预留 + 加实际，要么都发生，要么都不发生。
        let quota = self.quota.lock().clone();
        if let Some(quota) = quota {
            quota
                .settle_reservation(&commit.operation, commit.actual_cost)
                .await;
        }
        self.commits.lock().push(commit.clone());
        let mut entry = commit.entry.clone();
        let shortfall = *self.shortfall.lock();
        entry.raw_metadata = crate::usage::merge_shortfall(entry.raw_metadata, shortfall);
        self.logs.lock().push(entry);
        Ok(SettleReceipt::Committed {
            shortfall,
            balance_before: *self.balance_before.lock(),
            balance_after: *self.balance_after.lock(),
        })
    }

    async fn insert_usage_log(&self, entry: &UsageLogEntry) -> anyhow::Result<()> {
        self.logs.lock().push(entry.clone());
        Ok(())
    }

    async fn insert_balance_event(&self, event: &BalanceEvent) -> anyhow::Result<()> {
        self.balance_events.lock().push(event.clone());
        Ok(())
    }

    async fn clear_hold(&self, _user_id: Id, operation: &BillingOperationId) -> anyhow::Result<()> {
        self.cleared_holds.lock().push(operation.to_string());
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
    pub(crate) stale: Mutex<Vec<OrphanedOperation>>,
    pub(crate) errors: Mutex<bool>,
}

#[async_trait]
impl NonTerminalOperationScanner for FakeScanner {
    async fn scan_non_terminal(
        &self,
        _older_than: Duration,
        limit: i64,
    ) -> anyhow::Result<Vec<OrphanedOperation>> {
        if *self.errors.lock() {
            anyhow::bail!("scan failed");
        }
        let mut stale = self.stale.lock().clone();
        stale.truncate(limit.max(0) as usize);
        Ok(stale)
    }
}

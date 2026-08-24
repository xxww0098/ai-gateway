//! Doubles for the money path: ledger, calculator, quota and usage stores.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use gw_ledger::operation::{OperationState, Transition, terminate};
use gw_ledger::{BillingOperationId, NewOperation};

use crate::ports::{
    BalanceEvent, BillingError, BillingLedger, HoldAdmit, Id, ModelTokenUsage, PricingCalculator,
    SettleReceipt, SettleTerminal, SettlementCommit, SubscriptionQuota, SubscriptionQuotaStore,
    TokenUsage, UsageLogEntry, UsageStore, fold_model_usage,
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
    pub(crate) async fn plant_hold(&self, user_id: Id, operation: &BillingOperationId, amount: f64) {
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
        // Unconditional once-guard, inside the "transaction" — no caller flag.
        {
            let mut terminated = self.terminated.lock();
            let key = commit.operation.as_str().to_owned();
            let current = terminated.get(&key).copied().unwrap_or(OperationState::Held);
            match terminate(current, OperationState::Settled) {
                Transition::AlreadyTerminal(_) => return Ok(SettleReceipt::AlreadyTerminal),
                Transition::Apply(next) => terminated.insert(key, next),
            };
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

    async fn clear_hold(
        &self,
        _user_id: Id,
        operation: &BillingOperationId,
    ) -> anyhow::Result<()> {
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

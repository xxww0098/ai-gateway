//! Doubles for the money path: ledger, calculator, quota and usage stores.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::ports::{
    BalanceEvent, BillingError, BillingLedger, Id, PricingCalculator, SettleReceipt,
    SettlementCommit, SubscriptionQuota, SubscriptionQuotaStore, TokenUsage, UsageLogEntry,
    UsageStore,
};
use crate::reconcile::{StaleHold, StaleHoldScanner};

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
}

#[async_trait]
impl BillingLedger for FakeLedger {
    async fn hold(
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

    async fn settle(
        &self,
        user_id: Id,
        request_id: &str,
        actual_amount: f64,
    ) -> Result<f64, BillingError> {
        self.calls.lock().push(LedgerCall::Settle {
            user_id,
            amount: actual_amount,
        });
        self.holds.lock().remove(request_id);
        let mut balance = self.balance.lock();
        let debited = actual_amount.min(*balance);
        *balance -= debited;
        Ok(actual_amount - debited)
    }

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
        Ok(*self.balance.lock())
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
}

impl FakeUsageStore {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn settled_costs(&self) -> Vec<f64> {
        self.commits.lock().iter().map(|c| c.actual_cost).collect()
    }
}

#[async_trait]
impl UsageStore for FakeUsageStore {
    async fn commit_settlement(&self, commit: &SettlementCommit) -> anyhow::Result<SettleReceipt> {
        if *self.commit_fails.lock() {
            anyhow::bail!("settle transaction failed");
        }
        if commit.skip_if_already_logged && self.logged_requests.lock().contains(&commit.request_id)
        {
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

    async fn clear_hold(&self, _user_id: Id, request_id: &str) -> anyhow::Result<()> {
        self.cleared_holds.lock().push(request_id.to_owned());
        Ok(())
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

//! [`BillingLedger`] and [`StaleHoldScanner`] over `gw_ledger::Ledger`.
//!
//! The concrete ledger already satisfies the narrow billing interface
//! structurally — the port's signatures are "a 1:1 match of the concrete ledger
//! methods". They still are; this file is the conformance Rust makes explicit.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use gw_ledger::{BillingOperationId, HoldError, Ledger, LedgerError, NewOperation, SettleOnce};

use crate::ports::{BillingError, BillingLedger, HoldAdmit, Id, SettleTerminal};
use crate::reconcile::{NonTerminalOperationScanner, OrphanedOperation};

/// The production ledger, shared with the panel's billing handlers.
#[derive(Debug, Clone)]
pub struct SharedLedger(Arc<Ledger>);

impl SharedLedger {
    pub fn new(ledger: Arc<Ledger>) -> Self {
        Self(ledger)
    }

    /// The underlying ledger, for callers that need more than the port.
    pub fn inner(&self) -> &Arc<Ledger> {
        &self.0
    }
}

impl From<Arc<Ledger>> for SharedLedger {
    fn from(ledger: Arc<Ledger>) -> Self {
        Self::new(ledger)
    }
}

/// Translates a ledger failure into the narrow error the middleware branches on.
///
/// Only the two conditions the hold pre-flight distinguishes survive as
/// variants; everything else is an infrastructure failure, and the middleware
/// treats every one of them the same way — 402 `payment_required`, no hold —
/// rather than distinguishing them by error-string matching.
pub fn map_error(err: LedgerError) -> BillingError {
    match err {
        LedgerError::InsufficientBalance => BillingError::InsufficientBalance,
        LedgerError::OutstandingDebt => BillingError::OutstandingDebt,
        LedgerError::HoldNotFound => BillingError::HoldNotFound,
        other => BillingError::Other(anyhow::Error::new(other)),
    }
}

#[async_trait]
impl BillingLedger for SharedLedger {
    async fn admit_operation(
        &self,
        operation: &NewOperation,
        redis_ttl: Option<Duration>,
    ) -> Result<HoldAdmit, BillingError> {
        // gw_ledger::HoldError → ports::{HoldAdmit, BillingError}：枚举映射只在
        // 这一层做，middleware 只认 HoldAdmit，不碰具体 ledger crate 的类型。
        match self.0.admit_operation(operation, redis_ttl).await {
            Ok(()) => Ok(HoldAdmit::Reserved),
            Err(HoldError::InsufficientBalance { available }) => {
                Ok(HoldAdmit::Insufficient { available })
            }
            Err(HoldError::OperationConflict(conflict)) => {
                Err(BillingError::OperationConflict(conflict))
            }
            // Redis-less + broke: quote Postgres and return a structured 402
            // rather than a 500. Same shape the pre-Redis path produced.
            Err(HoldError::Ledger(LedgerError::RedisNotConfigured)) => {
                let available = self.0.get_balance(operation.user_id).await.unwrap_or(0.0);
                if available < operation.admitted_liability {
                    Ok(HoldAdmit::Insufficient { available })
                } else {
                    Err(map_error(LedgerError::RedisNotConfigured))
                }
            }
            // Fail closed the way `available.unwrap_or(0.0)` used to: a missing
            // user or a Redis blip on the peek must not become a way to spend.
            Err(HoldError::Ledger(LedgerError::UserNotFound | LedgerError::Redis(_))) => {
                Ok(HoldAdmit::Insufficient { available: 0.0 })
            }
            Err(HoldError::Ledger(err)) => Err(map_error(err)),
        }
    }

    async fn settle_once(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
        actual_amount: f64,
    ) -> Result<SettleTerminal, BillingError> {
        // The standalone settle reports no shortfall of its own — it debits
        // what it can and records the rest in `balance_logs`. The number a
        // caller wants comes from `settle_once_tx`, which is what
        // `SqlUsageStore` uses on the hot path; this method only runs for a
        // ledger used without the usage store.
        match self
            .0
            .settle_once(operation, user_id, actual_amount)
            .await
            .map_err(map_error)?
        {
            SettleOnce::Debited(outcome) => Ok(SettleTerminal::Debited {
                shortfall: outcome.shortfall,
            }),
            SettleOnce::AlreadyTerminal(_) => Ok(SettleTerminal::AlreadyTerminal),
        }
    }

    async fn release_once(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
    ) -> Result<(), BillingError> {
        match self.0.release_once(operation, user_id).await {
            Ok(_) => Ok(()),
            // Releasing an operation that was never persisted is a no-op, not
            // a failure: the caller is already on an error path and there is
            // nothing to give back.
            Err(LedgerError::HoldNotFound) => Ok(()),
            Err(err) => Err(map_error(err)),
        }
    }

    async fn active_hold_amount(
        &self,
        user_id: Id,
        operation: &BillingOperationId,
    ) -> Result<Option<f64>, BillingError> {
        // An `Err` here MUST NOT be flattened into `Ok(None)`: the settlement
        // fallback reads "no hold" as a definite zero and "unknown" as a reason
        // to leave the reservation alone. Collapsing the two bills the request
        // at zero — free upstream output.
        self.0
            .active_hold_amount(user_id, operation.as_str())
            .await
            .map_err(map_error)
    }

    async fn has_unresolved_shortfall(&self, user_id: Id) -> Result<bool, BillingError> {
        self.0
            .has_unresolved_shortfall(user_id)
            .await
            .map_err(map_error)
    }

    async fn available_balance(&self, user_id: Id) -> Result<f64, BillingError> {
        self.0.get_balance(user_id).await.map_err(map_error)
    }
}

#[async_trait]
impl NonTerminalOperationScanner for SharedLedger {
    /// Reads `billing_operations`, **not** the Redis reservation keys: an
    /// expired or evicted reservation says nothing about whether the money was
    /// accounted for, and a `held` row says exactly that.
    async fn scan_non_terminal(
        &self,
        older_than: Duration,
        limit: i64,
    ) -> anyhow::Result<Vec<OrphanedOperation>> {
        let stale = self
            .0
            .scan_non_terminal_operations(older_than, limit)
            .await?;
        Ok(stale
            .into_iter()
            .map(|op| OrphanedOperation {
                user_id: op.user_id,
                operation: op.operation_id,
                client_trace_id: op.client_trace_id,
                reserved_amount: op.reserved_amount,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;

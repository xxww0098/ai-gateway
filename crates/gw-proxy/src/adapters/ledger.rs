//! [`BillingLedger`] and [`StaleHoldScanner`] over `gw_ledger::Ledger`.
//!
//! The concrete ledger already satisfies the narrow billing interface
//! structurally — the port's signatures are "a 1:1 match of the concrete ledger
//! methods". They still are; this file is the conformance Rust makes explicit.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use gw_ledger::{Ledger, LedgerError};

use crate::ports::{BillingError, BillingLedger, HoldAdmit, Id};
use crate::reconcile::{StaleHold, StaleHoldScanner};

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
    async fn hold(
        &self,
        user_id: Id,
        amount: f64,
        request_id: &str,
        ttl: Duration,
    ) -> Result<(), BillingError> {
        self.0
            .hold(user_id, amount, request_id, ttl)
            .await
            .map_err(map_error)
    }

    async fn settle(
        &self,
        user_id: Id,
        request_id: &str,
        actual_amount: f64,
    ) -> Result<f64, BillingError> {
        // The standalone settle reports no shortfall — it debits what it can and
        // records the rest in `balance_logs`. The number the caller wants comes
        // from `settle_tx`, which is what `SqlUsageStore` uses on the hot path;
        // this method only runs for a ledger used without the usage store.
        self.0
            .settle(user_id, request_id, actual_amount)
            .await
            .map(|()| 0.0)
            .map_err(map_error)
    }

    async fn release(&self, user_id: Id, request_id: &str) -> Result<(), BillingError> {
        self.0.release(user_id, request_id).await.map_err(map_error)
    }

    async fn active_hold_amount(
        &self,
        user_id: Id,
        request_id: &str,
    ) -> Result<Option<f64>, BillingError> {
        // An `Err` here MUST NOT be flattened into `Ok(None)`: the settlement
        // fallback reads "no hold" as a definite zero and "unknown" as a reason
        // to leave the reservation alone. Collapsing the two bills the request
        // at zero — free upstream output.
        self.0
            .active_hold_amount(user_id, request_id)
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

    async fn hold_gated(
        &self,
        user_id: Id,
        amount: f64,
        min_available: f64,
        request_id: &str,
        ttl: Duration,
    ) -> Result<HoldAdmit, BillingError> {
        // gw_ledger::HoldOutcome → ports::HoldAdmit：仅此适配层做枚举映射，
        // middleware 只认 HoldAdmit，不碰具体 ledger crate 的类型。
        match self
            .0
            .hold_with_floor(user_id, amount, min_available, request_id, ttl)
            .await
        {
            Ok(gw_ledger::HoldOutcome::Reserved) => Ok(HoldAdmit::Reserved),
            Ok(gw_ledger::HoldOutcome::Insufficient { available }) => {
                Ok(HoldAdmit::Insufficient { available })
            }
            // Redis-less + broke: the old get_balance path quoted PG as 0 and
            // returned a structured 402 without calling hold. Keep that shape.
            Err(LedgerError::RedisNotConfigured) => {
                let available = self.0.get_balance(user_id).await.unwrap_or(0.0);
                if available < min_available {
                    Ok(HoldAdmit::Insufficient { available })
                } else {
                    Err(map_error(LedgerError::RedisNotConfigured))
                }
            }
            // Fail closed the way `available.unwrap_or(0.0)` used to: a
            // missing user or a Redis blip on the peek must not become a
            // way to spend.
            Err(LedgerError::UserNotFound) | Err(LedgerError::Redis(_)) => {
                Ok(HoldAdmit::Insufficient { available: 0.0 })
            }
            Err(err) => Err(map_error(err)),
        }
    }
}

#[async_trait]
impl StaleHoldScanner for SharedLedger {
    async fn scan_stale_holds(&self, older_than: Duration) -> anyhow::Result<Vec<StaleHold>> {
        let stale = self.0.scan_stale_holds(older_than).await?;
        Ok(stale
            .into_iter()
            .map(|hold| StaleHold {
                user_id: hold.user_id,
                request_id: hold.request_id,
                amount: hold.amount,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;

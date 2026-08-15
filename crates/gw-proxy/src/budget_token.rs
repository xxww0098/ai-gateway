//! Process-local batch pre-deduction, to spare the hot path a Redis round-trip.
//!
//! A token covers many requests for one user: [`BudgetTokenStore::try_deduct`] spends against it, and the settlement
//! pipeline reconciles the real cost with [`BudgetTokenStore::deduct_settle`].
//!
//! The remaining balance is allowed to go negative — that is the signal that
//! the batch is spent and the next request must fall back to a Redis hold.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::ports::Id;

/// A batch reservation covering multiple requests for one user.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetToken {
    /// The Redis hold request id this batch was reserved under.
    pub batch_request_id: String,
    pub user_id: Id,
    /// Remaining available budget; may go negative after settlement.
    pub remaining: f64,
    pub expires_at: Instant,
}

/// Concurrency-safe `user_id -> BudgetToken` map.
#[derive(Debug, Default)]
pub struct BudgetTokenStore {
    tokens: Mutex<HashMap<Id, BudgetToken>>,
}

impl BudgetTokenStore {
    /// Builds an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spends `amount` from the user's token.
    ///
    /// Returns `false` — so the caller falls back to a Redis hold — when there
    /// is no token, it has expired, or it cannot cover the amount.
    /// Deduct from the active token or signal a fallback to a Redis hold.
    pub fn try_deduct(&self, user_id: Id, amount: f64) -> bool {
        let mut tokens = self.tokens.lock();
        let Some(token) = tokens.get_mut(&user_id) else {
            return false;
        };
        if Instant::now() > token.expires_at || token.remaining < amount {
            return false;
        }
        token.remaining -= amount;
        true
    }

    /// Creates and stores a token for the user, replacing any existing one.
    /// Reserve a fresh batch for the user.
    pub fn acquire(&self, user_id: Id, batch_id: &str, budget: f64, ttl: Duration) -> BudgetToken {
        let token = BudgetToken {
            batch_request_id: batch_id.to_owned(),
            user_id,
            remaining: budget,
            expires_at: Instant::now() + ttl,
        };
        self.tokens.lock().insert(user_id, token.clone());
        token
    }

    /// Removes the user's token and returns `(batch_request_id, remaining)` so
    /// the caller can release the unused amount back to Redis.
    /// Return the unused batch amount to the caller.
    pub fn release(&self, user_id: Id) -> Option<(String, f64)> {
        self.tokens
            .lock()
            .remove(&user_id)
            .map(|t| (t.batch_request_id, t.remaining))
    }

    /// Applies the settled cost after a request completes. The remaining value
    /// may go negative, which signals that a new batch is needed next time.
    /// Apply the settled cost against the batch.
    pub fn deduct_settle(&self, user_id: Id, actual_cost: f64) {
        if let Some(token) = self.tokens.lock().get_mut(&user_id) {
            token.remaining -= actual_cost;
        }
    }

    /// Remaining budget for the user, if a token exists. Observability helper.
    pub fn remaining(&self, user_id: Id) -> Option<f64> {
        self.tokens.lock().get(&user_id).map(|t| t.remaining)
    }
}

#[cfg(test)]
mod tests;

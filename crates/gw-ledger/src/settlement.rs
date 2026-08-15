//! The partial-debit split at the heart of settle.
//!
//! The money arithmetic is kept here so it can be stated — and
//! property-tested — without a database. [`crate::Ledger::settle_tx`] calls
//! straight into it; there is no second copy of this rule.

/// How one settlement divides between what the balance could actually pay and
/// what it could not.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Settlement {
    /// What comes off the persistent balance: `min(balance, actual)`, floored
    /// at zero.
    pub debited: f64,
    /// What the balance could not cover: `actual - debited`. Recorded as
    /// `metadata.shortfall_usd` on a zero-amount marker row so operators — and
    /// [`crate::Ledger::has_unresolved_shortfall`] — can find the debt later.
    pub shortfall: f64,
}

impl Settlement {
    /// Splits `actual` against the balance on hand.
    ///
    /// Two rules, both load-bearing:
    ///
    /// * The ledger never debits more than the user has, so a request that
    ///   overruns its hold cannot drive the balance negative — the excess
    ///   becomes tracked debt instead.
    /// * `debited` is floored at zero, so a balance that is already negative
    ///   (from a pathological earlier state) cannot silently *credit* the user
    ///   through a settle.
    ///
    /// A non-positive `actual` settles to nothing at all: `Settlement::default()`.
    #[must_use]
    pub fn split(balance: f64, actual: f64) -> Self {
        if actual <= 0.0 {
            return Self::default();
        }
        let debited = balance.min(actual).max(0.0);
        Self {
            debited,
            shortfall: actual - debited,
        }
    }

    /// Whether this settlement leaves the user in debt, i.e. whether the
    /// zero-amount `shortfall_usd` marker row has to be written.
    #[must_use]
    pub fn has_shortfall(&self) -> bool {
        self.shortfall > 0.0
    }
}

#[cfg(test)]
mod tests;

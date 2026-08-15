//! [`PricingCalculator`] over `gw_pricing::Calculator`.
//!
//! The calculator and token estimator were once two interfaces because old test
//! stubs only implemented the first; the production `*pricing.Calculator` always
//! implemented both, and the middleware type-asserted its way to the better
//! estimate. Here they are one trait, so the assertion — and the silent fallback
//! to a flat reservation when it failed — is gone.

use std::sync::Arc;

use gw_pricing::{Calculator, TokenUsage as PricingTokens};

use crate::ports::{PricingCalculator, TokenUsage};

/// The production calculator, sharing one `ModelPriceCache` with the panel's
/// price editor so an admin upsert invalidates the cache this reads.
#[derive(Debug, Clone)]
pub struct SharedCalculator(Arc<Calculator>);

impl SharedCalculator {
    pub fn new(calculator: Arc<Calculator>) -> Self {
        Self(calculator)
    }

    /// The underlying calculator, for callers that need more than the port.
    pub fn inner(&self) -> &Arc<Calculator> {
        &self.0
    }
}

impl From<Arc<Calculator>> for SharedCalculator {
    fn from(calculator: Arc<Calculator>) -> Self {
        Self::new(calculator)
    }
}

impl PricingCalculator for SharedCalculator {
    fn estimate(&self, model: &str, stream: bool, rate_mult: f64) -> f64 {
        self.0.estimate(model, stream, rate_mult)
    }

    fn estimate_with_max_tokens(
        &self,
        model: &str,
        max_output_tokens: i64,
        stream: bool,
        rate_mult: f64,
    ) -> f64 {
        self.0
            .estimate_with_max_tokens(model, max_output_tokens, stream, rate_mult)
    }

    fn estimate_with_tokens(
        &self,
        model: &str,
        input_tokens: i64,
        max_output_tokens: i64,
        stream: bool,
        rate_mult: f64,
    ) -> f64 {
        self.0
            .estimate_with_tokens(model, input_tokens, max_output_tokens, stream, rate_mult)
    }

    fn compute(&self, model: &str, tokens: TokenUsage, rate_mult: f64) -> f64 {
        // The port wants the single number that gets debited; the itemised
        // breakdown is for the `usage_logs` cost columns, which stay zero on
        // this path (input_cost / output_cost are never set).
        self.0
            .compute(model, into_pricing(tokens), rate_mult)
            .total_cost
    }
}

/// The two token structs are the same four columns under different names.
fn into_pricing(tokens: TokenUsage) -> PricingTokens {
    PricingTokens {
        input: tokens.input,
        output: tokens.output,
        cached: tokens.cached,
        reasoning: tokens.reasoning,
    }
}

#[cfg(test)]
mod tests;

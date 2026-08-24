//! Model id + token counts → USD.

use std::sync::Arc;

use gw_model::ModelPrice;

use crate::cache::ModelPriceCache;
use crate::money::{RateMultiplier, TokenCount, UnitPrice};

/// Nominal token count [`Calculator::estimate`] reserves for one request
/// before the real response is seen. Deliberately a small, generous constant
/// rather than a per-model heuristic: a hold only has to be close enough,
/// because settle reconciles against the true usage afterwards.
pub const ESTIMATED_TOKENS: i64 = 1000;

/// Streaming estimates are scaled by this to give the hold headroom —
/// streaming completions routinely emit more output tokens than single-shot
/// calls.
pub const STREAM_MULTIPLIER: f64 = 2.0;

/// Prices in `model_prices` are quoted per this many tokens.
pub const TOKENS_PER_UNIT: f64 = 1_000_000.0;

/// The four token counts priced independently, one per `model_prices` column.
/// Units are raw tokens; the math normalizes by [`TOKENS_PER_UNIT`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: i64,
    pub output: i64,
    pub cached: i64,
    pub reasoning: i64,
}

/// Itemised result of [`Calculator::compute`].
///
/// The per-column figures here are additive, for the `usage_logs.input_cost` / `output_cost` columns
/// and for operator-facing breakdowns.
///
/// [`total_cost`](Self::total_cost) is **not** the sum of the four component
/// fields: it sums the raw price × token products first, then divides and
/// scales once — the same operation order the ledger's conservation
/// invariants are stated against. Summing the rounded components instead
/// would drift.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CostBreakdown {
    pub input_cost: f64,
    pub output_cost: f64,
    pub cached_cost: f64,
    pub reasoning_cost: f64,
    pub total_cost: f64,
}

/// Turns a model id and token counts into a USD cost.
///
/// The cache is read-only from here; refresh
/// is owned by admin flows through [`ModelPriceCache::invalidate`].
///
/// Cheap to clone (an `Arc` and an `f64`) — clone it per request rather than
/// wrapping it in another `Arc`.
#[derive(Debug, Clone, Default)]
pub struct Calculator {
    cache: Option<Arc<ModelPriceCache>>,
    /// Checked at construction, so no lookup miss can inject a `NaN` or a
    /// negative price into the arithmetic below. See [`crate::money`].
    default_price_per_1m: UnitPrice,
}

impl Calculator {
    /// Builds a calculator. `default_price_per_1m` is the per-1M fallback used
    /// for any model missing from the cache, so its unit matches the
    /// `model_prices` columns.
    ///
    /// A `None` cache is tolerated — every lookup then misses and the default
    /// price applies. That keeps bootstrap paths simple when the pricing table
    /// has not been loaded yet.
    ///
    /// [`Calculator::default()`] is a no-cache, zero-default-price value, so
    /// every method returns 0.
    /// A `NaN`, infinite or negative default price is refused at this
    /// boundary and becomes [`UnitPrice::ZERO`]: a misconfigured fallback
    /// price must not be able to reach the ledger, and it is the only value
    /// that provably cannot over-charge.
    #[must_use]
    pub fn new(cache: Option<Arc<ModelPriceCache>>, default_price_per_1m: f64) -> Self {
        Self {
            cache,
            default_price_per_1m: UnitPrice::new(default_price_per_1m).unwrap_or(UnitPrice::ZERO),
        }
    }

    /// The USD cost we expect a request to incur, used by the hold middleware
    /// to reserve funds up front. Deliberately an over-approximation; settle
    /// refunds the unused portion.
    ///
    /// Rules:
    ///
    /// * Known model: price [`ESTIMATED_TOKENS`] of input *and* of output at
    ///   the model's per-1M rates.
    /// * Unknown model: both columns fall back to `default_price_per_1m`.
    /// * `stream`: scale by [`STREAM_MULTIPLIER`].
    /// * `rate_mult`: final linear scaling.
    ///
    /// A zero default price on the unknown-model path collapses to 0, and so
    /// does a zero `rate_mult` — both deliberate. Enforcing a floor is the
    /// caller's decision.
    #[must_use]
    pub fn estimate(&self, model_id: &str, stream: bool, rate_mult: f64) -> f64 {
        let (input_per_1m, output_per_1m) = self.io_prices(model_id);
        let tokens = ESTIMATED_TOKENS as f64;
        let mut base =
            (input_per_1m.get() * tokens + output_per_1m.get() * tokens) / TOKENS_PER_UNIT;
        if stream {
            base *= STREAM_MULTIPLIER;
        }
        base * rate(rate_mult).get()
    }

    /// A tighter USD upper bound for a request whose client supplied an output
    /// cap (`max_tokens` / `max_completion_tokens`).
    ///
    /// The hold middleware uses this only for its "reject if the upper bound
    /// exceeds the available balance" preflight gate — the amount actually
    /// reserved still comes from [`estimate`](Self::estimate) /
    /// [`estimate_with_tokens`](Self::estimate_with_tokens).
    ///
    /// Rules:
    ///
    /// * `max_output_tokens <= 0`: fall back to
    ///   `estimate(model_id, streaming, rate_mult)` — a client that omits or
    ///   malforms the cap does not get a tighter bound.
    /// * Known model: input priced at [`ESTIMATED_TOKENS`], output at
    ///   `max_output_tokens`.
    /// * Unknown model: both columns fall back to `default_price_per_1m`.
    /// * `streaming` scales by [`STREAM_MULTIPLIER`]; `rate_mult` scales last.
    #[must_use]
    pub fn estimate_with_max_tokens(
        &self,
        model_id: &str,
        max_output_tokens: i64,
        streaming: bool,
        rate_mult: f64,
    ) -> f64 {
        if max_output_tokens <= 0 {
            return self.estimate(model_id, streaming, rate_mult);
        }
        let (input_per_1m, output_per_1m) = self.io_prices(model_id);
        let mut base = (input_per_1m.get() * ESTIMATED_TOKENS as f64
            + output_per_1m.get() * max_output_tokens as f64)
            / TOKENS_PER_UNIT;
        if streaming {
            base *= STREAM_MULTIPLIER;
        }
        base * rate(rate_mult).get()
    }

    /// The hold reservation computed from a *real* input-token count
    /// (approximated from the request body) instead of the nominal constant.
    ///
    /// This is the fix for the systematic under-hold on large prompts:
    /// [`estimate`](Self::estimate) prices input at a flat
    /// [`ESTIMATED_TOKENS`] regardless of prompt size, so a 100k-token request
    /// reserves far too little, slips past the balance gate, and can drive the
    /// balance negative at settle time (producing a shortfall).
    ///
    /// Rules:
    ///
    /// * input priced at `max(input_tokens, ESTIMATED_TOKENS)` — never below
    ///   the historical nominal floor for tiny or empty bodies.
    /// * output priced at `max_output_tokens` when the client supplied a cap,
    ///   otherwise at [`ESTIMATED_TOKENS`].
    /// * `stream` scales by [`STREAM_MULTIPLIER`]; `rate_mult` scales last.
    #[must_use]
    pub fn estimate_with_tokens(
        &self,
        model_id: &str,
        input_tokens: i64,
        max_output_tokens: i64,
        stream: bool,
        rate_mult: f64,
    ) -> f64 {
        let (input_per_1m, output_per_1m) = self.io_prices(model_id);
        let in_tok = input_tokens.max(ESTIMATED_TOKENS);
        let out_tok = if max_output_tokens <= 0 {
            ESTIMATED_TOKENS
        } else {
            max_output_tokens
        };
        let mut base = (input_per_1m.get() * in_tok as f64 + output_per_1m.get() * out_tok as f64)
            / TOKENS_PER_UNIT;
        if stream {
            base *= STREAM_MULTIPLIER;
        }
        base * rate(rate_mult).get()
    }

    /// The exact USD cost of a completed request: each of the four token
    /// columns against its per-1M price, scaled by `rate_mult`.
    ///
    /// [`CostBreakdown::total_cost`] uses the same operation order as the
    /// individual columns, so the itemised figures reconcile. Unknown models
    /// price every column at `default_price_per_1m`.
    /// All-zero [`TokenUsage`] yields exactly 0 whatever the price or
    /// `rate_mult`.
    #[must_use]
    pub fn compute(&self, model_id: &str, tokens: TokenUsage, rate_mult: f64) -> CostBreakdown {
        let (input_per_1m, output_per_1m, cached_per_1m, reasoning_per_1m) =
            match self.lookup(model_id) {
                Some(p) => (
                    self.price(p.input_price_per_1m),
                    self.price(p.output_price_per_1m),
                    self.price(p.cached_input_price_per_1m),
                    self.price(p.reasoning_price_per_1m),
                ),
                None => {
                    let d = self.default_price_per_1m;
                    (d, d, d, d)
                }
            };

        // An upstream that reports a negative column would otherwise *credit*
        // the tenant for that column; clamping bills nothing for it instead.
        let input = TokenCount::clamped(tokens.input).as_f64();
        let output = TokenCount::clamped(tokens.output).as_f64();
        let cached = TokenCount::clamped(tokens.cached).as_f64();
        let reasoning = TokenCount::clamped(tokens.reasoning).as_f64();
        let rate = rate(rate_mult).get();

        // Sum the raw products, divide once, scale once.
        let raw = input_per_1m.get() * input
            + output_per_1m.get() * output
            + cached_per_1m.get() * cached
            + reasoning_per_1m.get() * reasoning;

        CostBreakdown {
            input_cost: input_per_1m.get() * input / TOKENS_PER_UNIT * rate,
            output_cost: output_per_1m.get() * output / TOKENS_PER_UNIT * rate,
            cached_cost: cached_per_1m.get() * cached / TOKENS_PER_UNIT * rate,
            reasoning_cost: reasoning_per_1m.get() * reasoning / TOKENS_PER_UNIT * rate,
            total_cost: raw / TOKENS_PER_UNIT * rate,
        }
    }

    /// The `(input, output)` per-1M pair the three estimate methods share:
    /// the model's own columns on a cache hit, the default price on a miss.
    fn io_prices(&self, model_id: &str) -> (UnitPrice, UnitPrice) {
        match self.lookup(model_id) {
            Some(p) => (
                self.price(p.input_price_per_1m),
                self.price(p.output_price_per_1m),
            ),
            None => (self.default_price_per_1m, self.default_price_per_1m),
        }
    }

    /// One `model_prices` column, checked. A row carrying a `NaN` or a
    /// negative price falls back to the configured default rather than
    /// poisoning the arithmetic — the column is `numeric` and nullable, so a
    /// hostile value there is a data problem, not a request problem.
    fn price(&self, column: f64) -> UnitPrice {
        UnitPrice::new(column).unwrap_or(self.default_price_per_1m)
    }

    /// Resolves a model id through the cache, tolerating an absent cache.
    fn lookup(&self, model_id: &str) -> Option<Arc<ModelPrice>> {
        self.cache.as_ref()?.get(model_id)
    }
}

/// The group multiplier, checked.
///
/// A `NaN` or negative multiplier falls back to the identity: billing at the
/// un-discounted rate is the safe direction to fail, and a `NaN` would slip
/// past every later comparison. Zero stays zero — a zero-rate group is a real
/// configuration, and the estimate methods document it.
fn rate(rate_mult: f64) -> RateMultiplier {
    RateMultiplier::new(rate_mult).unwrap_or(RateMultiplier::ONE)
}

#[cfg(test)]
mod tests;

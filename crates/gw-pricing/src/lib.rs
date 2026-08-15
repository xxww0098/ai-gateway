//! Four-column token pricing (input / output / cache / reasoning).
//!
//! Two pieces:
//!
//! * [`ModelPriceCache`] — an in-memory snapshot of the `model_prices` table
//!   (rows are [`gw_model::ModelPrice`], the one entity for that table),
//!   refreshed explicitly (admin edited a price) or periodically. It is a
//!   *shared handle*: the panel's price editor and the proxy's [`Calculator`]
//!   hold `Arc`s to the same instance, so an [`invalidate`](ModelPriceCache::invalidate)
//!   is visible to the calculator on its very next lookup.
//! * [`Calculator`] — turns a model id + token counts into USD, using the four
//!   per-1M price columns and a `default_price_per_1m` fallback.
//!
//! Money never rounds here. Every method returns the raw `f64` computed in
//! the reference operation order, because the ledger's conservation
//! invariants are asserted against these values.
//!
//! OWNER: worker `ledger-pricing`.

// Rule 5.3's ratchet: this crate's public paths carry no `todo!()` /
// `unimplemented!()`, so pin those two shut here. The root manifest keeps them
// at `warn` for the crates that still have skeleton stubs; the coordinator
// flips the workspace-level deny once the last one is clear.
#![deny(clippy::todo, clippy::unimplemented)]

mod cache;
mod calculator;

#[cfg(test)]
mod testsupport;

pub use cache::ModelPriceCache;
pub use calculator::{
    Calculator, CostBreakdown, ESTIMATED_TOKENS, STREAM_MULTIPLIER, TOKENS_PER_UNIT, TokenUsage,
};

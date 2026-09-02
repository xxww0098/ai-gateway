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
//! * [`Calculator`] — 在 Hold 处把四列单价、倍率与缓存代次冻成一个
//!   [`PricingQuote`]。它**只**做这件事。
//! * [`PricingQuote`] — 那份冻结的价格。估算（预扣）与精算（结算）都在它上面做，
//!   **不再回头查缓存**。这是「在途请求不会因为管理员改价、或上游换个模型名
//!   就换一个价钱结算」的实现方式。
//! * [`ObservedUsage`] / [`BillableUsage`] / [`normalize`] — 上游原话的四个数
//!   → 四个**互斥**的可计价列。计价只看后者；`usage_logs` 写前者。
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
pub mod money;
mod quote;
mod usage;

#[cfg(test)]
mod testsupport;

pub use cache::{ModelPriceCache, normalize_model_key};
pub use calculator::Calculator;
pub use money::{Money, RateMultiplier, TokenCount, UnitPrice, ValueError};
pub use quote::{
    CostBreakdown, ESTIMATED_TOKENS, PricingQuote, STREAM_MULTIPLIER, TOKENS_PER_UNIT,
};
pub use usage::{BillableUsage, ObservedUsage, UsageDialect, normalize};

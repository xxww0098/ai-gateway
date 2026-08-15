//! Concrete implementations of every [`crate::ports`] trait.
//!
//! # Why these live here
//!
//! The narrow interfaces were originally satisfied by concrete types that
//! already had a matching method set, so "the adapter" was the assignment
//! itself. Rust has no structural conformance, so somebody has to write the
//! `impl`s, and that somebody is this crate: the port docs are the contract,
//! and two of the ports (see below) require calling back into `gw-proxy`'s own
//! logic. Putting the `impl`s next to the traits keeps that contract in one
//! place instead of splitting it across a crate boundary.
//!
//! With these in hand the composition root only assembles values — it never has
//! to know what a shortfall is or when a quota counter rotates.
//!
//! # The two callbacks that are not optional
//!
//! * [`usage::SqlUsageStore`] folds the partial-debit shortfall into the
//!   `usage_logs` row with [`crate::usage::merge_shortfall`], **inside** the
//!   settle transaction — the shortfall is not knowable before `settle_tx`
//!   runs, and re-deriving the annotation shape elsewhere is how a "free
//!   request" and a "partially paid request" stop being distinguishable.
//! * [`quota::SqlSubscriptionQuotaStore`] applies
//!   [`crate::hold::rotate_counters`] under its `SELECT ... FOR UPDATE` rather
//!   than expressing the reset rule in SQL, so the boundary arithmetic has one
//!   implementation and one set of tests.
//!
//! # Testing
//!
//! The pure mappings — error translation, row projection, cache policy — are
//! unit-tested here. Everything that needs Postgres or Redis is behind
//! `#[ignore]`, following the convention the rest of the workspace uses
//! (rule 2.9: fail loud with instructions, never silently skip).

pub mod authcore;
pub mod catalog;
pub mod directory;
pub mod infra;
pub mod ledger;
pub mod pricing;
pub mod quota;
pub mod usage;

pub use authcore::AuthcoreCrypto;
pub use catalog::{CatalogChannelResolver, SqlChannelPolicyStore, SqlModelCatalog};
pub use directory::SqlTenantDirectory;
pub use infra::{RedisIdempotencyStore, SharedCircuitBreaker, SharedRateLimiter};
pub use ledger::SharedLedger;
pub use pricing::SharedCalculator;
pub use quota::SqlSubscriptionQuotaStore;
pub use usage::SqlUsageStore;

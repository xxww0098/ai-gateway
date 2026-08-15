//! In-memory doubles for the [`crate::ports`] traits.
//!
//! Exercise the 402,
//! quota, fallback and strict branches without Redis or Postgres. Nothing here
//! ships — the module is `#[cfg(test)]`-gated from `lib.rs`.
//!
//! Split by the collaborator being faked rather than kept as one file, so no
//! single module outgrows rule 1.10's 1,000-line ceiling as more doubles land.
//! The re-exports below keep every call site writing
//! `use crate::testsupport::Thing`, with no knowledge of which submodule holds it.

mod billing;
mod harness;
mod identity;
mod infra;
mod pg;
mod upstream;

pub(crate) use billing::{
    FakeCalculator, FakeLedger, FakeQuotaStore, FakeScanner, FakeUsageStore, LedgerCall,
};
pub(crate) use harness::{
    Harness, TEST_API_KEY, TEST_USER_ID, anonymous_request, chat_body, send, signed_get,
    signed_request,
};
pub(crate) use identity::{FakeCrypto, FakeDirectory};
pub(crate) use infra::{
    FakeCircuitBreaker, FakeIdempotencyStore, FakePolicyStore, FakeRateLimiter, RecordingMetrics,
};
pub(crate) use pg::{fresh_db, seed_user};
pub(crate) use upstream::{
    FakeAuthStore, FakeCatalog, FakeProvider, auth_record, ok_response, ok_response_without_usage,
};

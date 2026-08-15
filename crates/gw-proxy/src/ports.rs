//! Narrow trait surfaces the `/v1/*` kernel depends on.
//!
//! Minimal interfaces (`BillingLedger`, `PricingCalculator`, ...) are declared
//! here instead of importing the concrete `ledger` / `pricing` types so the
//! middleware can be unit-tested with in-memory fakes; the same reasoning
//! applies, with the extra benefit that `gw-proxy` compiles against these
//! traits while `gw-ledger` / `gw-pricing` / `gw-infra` are written in
//! parallel.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gw_authcore::Claims;

/// Primary-key type used across every entity.
///
/// Declared locally rather than re-exported from `gw-model` so this crate's
/// trait surface does not move when the entity layer does; the two must stay
/// the same width (`gw_model::Id`).
pub type Id = i64;

/// Per-column token counts fed into [`PricingCalculator::compute`].
/// Matches `gw_pricing::TokenUsage`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: i64,
    pub output: i64,
    pub cached: i64,
    pub reasoning: i64,
}

/// Why a ledger operation refused.
///
/// A typed enum distinguishes the structured 402 from the generic one without
/// the string matching; it mirrors the failure modes of `Ledger::hold` /
/// `Ledger::settle`.
#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    #[error("insufficient balance")]
    InsufficientBalance,
    #[error("outstanding debt")]
    OutstandingDebt,
    #[error("hold not found")]
    HoldNotFound,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// ---------------------------------------------------------------- billing

/// Minimal ledger surface consumed by [`crate::hold`] and [`crate::usage`].
/// A 1:1 narrowing of `gw_ledger::Ledger`.
#[async_trait]
pub trait BillingLedger: Send + Sync {
    /// Reserves `amount` against `user_id` for `request_id`, valid for `ttl`.
    async fn hold(
        &self,
        user_id: Id,
        amount: f64,
        request_id: &str,
        ttl: Duration,
    ) -> Result<(), BillingError>;

    /// Clears the outstanding hold for `request_id` and debits
    /// `min(balance, actual_amount)`, returning the un-debited shortfall.
    /// A non-positive `actual_amount` only clears the hold.
    async fn settle(
        &self,
        user_id: Id,
        request_id: &str,
        actual_amount: f64,
    ) -> Result<f64, BillingError>;

    /// Clears the hold without touching the persistent balance. Used when the
    /// upstream request failed.
    async fn release(&self, user_id: Id, request_id: &str) -> Result<(), BillingError>;

    /// Amount currently held for `(user_id, request_id)` without mutating any
    /// state. `Ok(None)` means "no hold exists"; `Err` means "unknown" and the
    /// caller must NOT treat it as zero (see [`crate::usage`] fallback path).
    async fn active_hold_amount(
        &self,
        user_id: Id,
        request_id: &str,
    ) -> Result<Option<f64>, BillingError>;

    /// Reports whether the user owns a settle `balance_logs` row with a
    /// positive `metadata.shortfall_usd` that has not been paired with a
    /// `shortfall_resolve:<reference>:<id>` credit.
    async fn has_unresolved_shortfall(&self, user_id: Id) -> Result<bool, BillingError>;

    /// Available balance (persisted balance minus active holds), used for the
    /// structured 402 body.
    async fn available_balance(&self, user_id: Id) -> Result<f64, BillingError>;
}

/// Pricing surface consumed by the hold pre-flight and the settlement pipeline.
///
/// The calculator and the token estimator are merged into one trait because
/// every production calculator implements both.
pub trait PricingCalculator: Send + Sync {
    /// Over-approximate USD cost used by Hold.
    fn estimate(&self, model: &str, stream: bool, rate_mult: f64) -> f64;

    /// Tighter USD upper bound when the client supplied an output cap. A
    /// non-positive `max_output_tokens` MUST fall back to [`Self::estimate`].
    fn estimate_with_max_tokens(
        &self,
        model: &str,
        max_output_tokens: i64,
        stream: bool,
        rate_mult: f64,
    ) -> f64;

    /// Reservation priced from the real (approximated) input-token count so a
    /// large prompt reserves proportional funds.
    fn estimate_with_tokens(
        &self,
        model: &str,
        input_tokens: i64,
        max_output_tokens: i64,
        stream: bool,
        rate_mult: f64,
    ) -> f64;

    /// Exact USD cost from per-column token counts, used by Settle.
    fn compute(&self, model: &str, tokens: TokenUsage, rate_mult: f64) -> f64;
}

// ---------------------------------------------------------------- identity

/// Cryptographic primitives owned by `gw-authcore`.
///
/// Behind a port so `gw-proxy` neither pulls in a hashing dependency of its own
/// nor hard-codes the JWT secret.
pub trait AuthCrypto: Send + Sync {
    /// Lowercase hex SHA-256 of the plaintext API key.
    fn hash_api_key(&self, plaintext: &str) -> String;

    /// Lowercase hex SHA-256 of arbitrary input, used to scope idempotency
    /// keys.
    fn sha256_hex(&self, input: &str) -> String;

    /// Validates an HS256 JWT against the configured secret.
    fn verify_jwt(&self, token: &str) -> Option<Claims>;
}

/// One `api_keys` row as far as `/v1/*` authentication is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyRow {
    pub id: Id,
    pub user_id: Id,
    pub group_id: Option<Id>,
    pub status: String,
}

/// Quota + entitlement state of the newest active subscription.
///
/// Shared by the access provider (publication) and the hold pre-flight
/// (locking/rotation), so the two call sites share one shape. A `None` reset
/// timestamp means rotation is disabled for that period.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubscriptionQuota {
    pub id: Id,
    pub group_id: Id,
    pub daily_usage_usd: f64,
    pub weekly_usage_usd: f64,
    pub monthly_usage_usd: f64,
    pub daily_limit_usd: Option<f64>,
    pub weekly_limit_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
    pub daily_reset_at: Option<DateTime<Utc>>,
    pub weekly_reset_at: Option<DateTime<Utc>>,
    pub monthly_reset_at: Option<DateTime<Utc>>,
}

/// Read-mostly tenant lookups behind the `/v1/*` access provider.
///
/// Implementations are expected to sit in front of `gw_infra::ApiKeyCache` /
/// `UserStatusCache`; the caching policy is theirs, the authorization policy is
/// [`crate::access`]'s.
#[async_trait]
pub trait TenantDirectory: Send + Sync {
    /// `SELECT * FROM api_keys WHERE key_hash = ?`. Status filtering is the
    /// caller's job so a cached-but-deactivated key is rejected identically to
    /// a DB-side deactivation.
    async fn api_key_by_hash(&self, key_hash: &str) -> anyhow::Result<Option<ApiKeyRow>>;

    /// `SELECT rate_multiplier FROM groups WHERE id = ?`.
    async fn group_rate_multiplier(&self, group_id: Id) -> anyhow::Result<Option<f64>>;

    /// `SELECT status FROM users WHERE id = ?`. `None` means the row is absent.
    async fn user_status(&self, user_id: Id) -> anyhow::Result<Option<String>>;

    /// Newest `status = 'active' AND expires_at > now()` subscription.
    async fn active_subscription(&self, user_id: Id) -> anyhow::Result<Option<SubscriptionQuota>>;

    /// Whether the user holds a live subscription bound to `group_id`.
    async fn holds_group_entitlement(&self, user_id: Id, group_id: Id) -> anyhow::Result<bool>;

    /// Fire-and-forget `UPDATE api_keys SET last_used_at = now()`.
    async fn touch_api_key(&self, api_key_id: Id);
}

/// Row-locking quota store used by the hold pre-flight.
///
/// The implementation performs a `SELECT ... FOR UPDATE` lock and persists
/// rotated counters; the *decision* of what to rotate and
/// whether the quota is exceeded stays in [`crate::hold`] via
/// [`crate::hold::rotate_counters`] and [`crate::hold::evaluate_quota`].
#[async_trait]
pub trait SubscriptionQuotaStore: Send + Sync {
    /// Locks the subscription row, applies [`crate::hold::rotate_counters`],
    /// persists it when dirty, and returns the post-rotation snapshot.
    /// `Ok(None)` for a missing row, treated as permissive.
    async fn lock_and_rotate(
        &self,
        subscription_id: Id,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<SubscriptionQuota>>;
}

// ---------------------------------------------------------------- settlement

/// A `usage_logs` row, field-for-field with `model.UsageLog`.
///
/// Built by [`crate::usage`]; written verbatim by [`UsageStore`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageLogEntry {
    pub user_id: Id,
    pub api_key_id: Id,
    pub group_id: Option<Id>,
    pub request_id: String,
    pub idempotency_key: String,
    pub model: String,
    pub provider: String,
    pub auth_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_cost: f64,
    pub actual_cost: f64,
    pub cost: f64,
    pub rate_multiplier: f64,
    pub stream: bool,
    pub duration_ms: i64,
    pub ip_address: String,
    /// `jsonb` column; `None` keeps the column NULL.
    pub raw_metadata: Option<serde_json::Value>,
    pub failed: bool,
}

/// A `balance_logs` row written for threshold events.
#[derive(Debug, Clone, PartialEq)]
pub struct BalanceEvent {
    pub user_id: Id,
    pub amount: f64,
    pub event_type: String,
    pub reference: String,
    pub metadata: serde_json::Value,
}

/// Everything that must commit atomically for one settled request.
#[derive(Debug, Clone)]
pub struct SettlementCommit {
    pub user_id: Id,
    pub request_id: String,
    pub actual_cost: f64,
    /// The row to insert. Its `raw_metadata` already carries any
    /// `billing_fallback` tag; the implementation MUST fold the partial-debit
    /// shortfall in with [`crate::usage::merge_shortfall`] before inserting,
    /// because the shortfall is only known inside the transaction.
    pub entry: UsageLogEntry,
    /// Accumulate the three usage counters on this subscription when the cost
    /// is positive and the row is still active.
    pub subscription_id: Option<Id>,
    /// Re-check `usage_logs.request_id` INSIDE the transaction and abort with
    /// [`SettleReceipt::AlreadySettled`] if a row exists. Used by
    /// [`crate::reconcile`] so an orphaned hold can never be double-charged.
    pub skip_if_already_logged: bool,
}

/// Outcome of [`UsageStore::commit_settlement`].
#[derive(Debug, Clone, PartialEq)]
pub enum SettleReceipt {
    Committed {
        /// `actual_cost` minus the portion that could actually be debited.
        shortfall: f64,
        balance_before: f64,
        balance_after: f64,
    },
    /// `skip_if_already_logged` was set and a `usage_logs` row already existed.
    AlreadySettled,
}

/// Transactional writes owned by the settlement pipeline.
///
/// One implementation runs `ledger.SettleTx` + the `usage_logs` insert + the
/// subscription accumulation in a single Postgres transaction, then the caller
/// clears the Redis reservation only after commit — the settle transaction
/// contract.
#[async_trait]
pub trait UsageStore: Send + Sync {
    /// ONE transaction: debit, read balance, insert `usage_logs`, accumulate
    /// subscription counters.
    async fn commit_settlement(&self, commit: &SettlementCommit) -> anyhow::Result<SettleReceipt>;

    /// Standalone insert used on failure paths, outside any transaction.
    async fn insert_usage_log(&self, entry: &UsageLogEntry) -> anyhow::Result<()>;

    /// Insert a threshold balance event.
    async fn insert_balance_event(&self, event: &BalanceEvent) -> anyhow::Result<()>;

    /// Releases the Redis reservation after the settle transaction commits.
    async fn clear_hold(&self, user_id: Id, request_id: &str) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------- infra

/// Distributed rate limiter.
#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Returns `(allowed, release_id)`. A non-empty `release_id` must later be
    /// handed to [`Self::release_concurrency`].
    async fn allow(
        &self,
        identity: &str,
        tokens: i64,
        model: &str,
        group_id: Option<Id>,
    ) -> anyhow::Result<(bool, Option<String>)>;

    /// Frees a reserved concurrency slot.
    async fn release_concurrency(&self, identity: &str, release_id: &str) -> anyhow::Result<()>;
}

/// Per-upstream circuit breaker.
#[async_trait]
pub trait CircuitBreaker: Send + Sync {
    /// Whether the breaker currently allows a request.
    async fn allow(&self, provider: &str) -> anyhow::Result<bool>;
    /// Record a success or failure against the breaker.
    async fn record(&self, provider: &str, success: bool);
}

/// Redis key/value operations backing idempotent replay.
///
/// The serialization and the claim/replay policy live in
/// [`crate::idempotency`].
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// `GET key`.
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>>;
    /// `SET key value EX ttl`.
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Duration) -> anyhow::Result<()>;
    /// `SET key value NX EX ttl`; `Ok(true)` when the caller now owns the key.
    async fn set_nx(&self, key: &str, value: Vec<u8>, ttl: Duration) -> anyhow::Result<bool>;
    /// `DEL key`.
    async fn delete(&self, key: &str) -> anyhow::Result<()>;
}

/// Per-upstream-account routing policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPolicy {
    pub auth_id: String,
    pub weight: i64,
    pub priority: i64,
    pub enabled: bool,
}

/// Source of [`ChannelPolicy`] rows.
#[async_trait]
pub trait ChannelPolicyStore: Send + Sync {
    async fn list_channel_policies(&self) -> anyhow::Result<Vec<ChannelPolicy>>;
}

/// Sink for the two gauges this crate observes but does not export.
///
/// `/metrics/prometheus` belongs to the composition root, fed from the
/// channel-health benched count and the orphaned-holds counter. `gw-server`
/// owns the route and the exposition format; this trait is how the values reach
/// it, and it lines up 1:1 with `gw_server::Metrics::set_channel_benched` /
/// `set_orphaned_holds`.
pub trait MetricsSink: Send + Sync {
    /// Upstream accounts currently benched by [`crate::channel::ChannelHealth`].
    /// Feeds `cpa_channel_benched_total`.
    fn set_channel_benched(&self, count: i64);

    /// Holds orphaned by a prior crash, as of the last reconcile scan.
    /// Feeds `cpa_orphaned_holds`.
    fn set_orphaned_holds(&self, count: i64);
}

/// The sink used when no exporter is wired, so tests and embedders need not
/// supply one. Dropping the values is correct here: they are observability
/// only, and nothing in the billing path reads them back.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscardMetrics;

impl MetricsSink for DiscardMetrics {
    fn set_channel_benched(&self, _count: i64) {}
    fn set_orphaned_holds(&self, _count: i64) {}
}

/// One entry of the `GET /v1/models` catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    pub id: String,
    /// Unix seconds, as the OpenAI models payload expects.
    pub created: i64,
    pub owned_by: String,
}

/// Source of the model catalogue served by `GET /v1/models`.
#[async_trait]
pub trait ModelCatalog: Send + Sync {
    async fn list_models(&self) -> anyhow::Result<Vec<ModelEntry>>;
}

/// Access metadata handed from [`crate::access`] to [`crate::hold`].
///
/// The fields keep their types (no stringification) because the boundary is
/// in-process. [`AccessMetadata::to_map`] reproduces the wire shape for logging
/// and parity tests.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccessMetadata {
    pub user_id: Id,
    pub api_key_id: Id,
    pub group_id: Option<Id>,
    pub rate_mult: f64,
    pub subscription: Option<SubscriptionQuota>,
}

impl AccessMetadata {
    /// Reproduces the `map[string]string` metadata payload.
    pub fn to_map(&self) -> HashMap<String, String> {
        let mut meta = HashMap::new();
        meta.insert("user_id".to_owned(), self.user_id.to_string());
        meta.insert("rate_mult".to_owned(), fmt_float(self.rate_mult));
        if self.api_key_id != 0 {
            meta.insert("api_key_id".to_owned(), self.api_key_id.to_string());
        }
        if let Some(gid) = self.group_id {
            meta.insert("group_id".to_owned(), gid.to_string());
        }
        if let Some(sub) = &self.subscription {
            meta.insert("subscription_id".to_owned(), sub.id.to_string());
            meta.insert("subscription_group_id".to_owned(), sub.group_id.to_string());
            if let Some(v) = sub.daily_limit_usd {
                meta.insert("daily_limit".to_owned(), fmt_float(v));
            }
            if let Some(v) = sub.weekly_limit_usd {
                meta.insert("weekly_limit".to_owned(), fmt_float(v));
            }
            if let Some(v) = sub.monthly_limit_usd {
                meta.insert("monthly_limit".to_owned(), fmt_float(v));
            }
            meta.insert("daily_used".to_owned(), fmt_float(sub.daily_usage_usd));
            meta.insert("weekly_used".to_owned(), fmt_float(sub.weekly_usage_usd));
            meta.insert("monthly_used".to_owned(), fmt_float(sub.monthly_usage_usd));
        }
        meta
    }
}

/// Shortest decimal round-trip, never exponent notation for the magnitudes
/// this codebase deals in.
fn fmt_float(f: f64) -> String {
    let s = format!("{f}");
    if s.contains(['e', 'E']) {
        format!("{f:.10}")
    } else {
        s
    }
}

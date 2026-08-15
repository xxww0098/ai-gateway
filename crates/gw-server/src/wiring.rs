//! The composition root's build step: turn a [`Config`] into live
//! infrastructure, the billing pipeline, and the merged `/v1` + `/api/panel`
//! router.
//!
//! Everything between database init and building the router. It lives in its
//! own module
//! rather than in [`crate::run`] because `run` owns the *process* (flags,
//! tracing, the runtime) while this owns the *object graph*, and only the
//! second half is worth testing.
//!
//! # Instances that must be shared, not re-created
//!
//! Three of them, each one a bug a comment in the original flagged:
//!
//! * [`gw_pricing::ModelPriceCache`] — the panel's admin price upsert
//!   invalidates the cache the [`Calculator`] reads from. A second cache here
//!   would leave the calculator serving stale prices forever.
//! * [`gw_infra::UserStatusCache`] — a suspension observed on `/v1/*` must be
//!   visible to `/api/panel/**` on the next request, so the access provider and
//!   the panel middleware read one map.
//! * [`gw_infra::ApiKeyCache`] — same argument for key revocation.
//!
//! (`ApiKeyCache` / `UserStatusCache` are `Arc`-backed handles: cloning is what
//! *shares* them. The `Arc<…>` the panel takes wraps the same handle.)
//!
//! # The drain tracker
//!
//! [`ProxyState::new`] takes the tracker `gw_server` drains after graceful
//! shutdown. `StreamSettler::drop` spawns its settlement into it; a fresh
//! `TaskTracker::new()` here would compile and then silently lose every
//! settlement whose client hung up near shutdown.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use axum::Router;
use gw_config::Config;
use gw_infra::{
    ApiKeyCache, CircuitBreaker as InfraCircuitBreaker, CircuitBreakerSettings, Db, DbSettings,
    RateLimitSettings, RateLimiter as InfraRateLimiter, Redis, SqlLogLevel, SweepHandle,
    UserStatusCache,
};
use gw_ledger::Ledger;
use gw_model::seed::{BootstrapAdmin, SdkSeedConfig, SdkSeedProvider};
use gw_pricing::{Calculator, ModelPriceCache};
use gw_provider::claude::ClaudeProvider;
use gw_provider::codex::CodexProvider;
use gw_provider::common::ProviderConfig;
use gw_provider::gemini::GeminiProvider;
use gw_provider::openai::OpenAiCompatibleProvider;
use gw_provider::types::Provider;
use gw_provider::vertex::VertexProvider;
use gw_proxy::adapters::{
    AuthcoreCrypto, RedisIdempotencyStore, SharedCalculator, SharedCircuitBreaker, SharedLedger,
    SharedRateLimiter, SqlChannelPolicyStore, SqlModelCatalog, SqlSubscriptionQuotaStore,
    SqlTenantDirectory, SqlUsageStore,
};
use gw_proxy::budget_token::BudgetTokenStore;
use gw_proxy::channel::{ChannelHealth, ChannelPolicyCache, ChannelPool};
use gw_proxy::idempotency::IdempotencyManager;
use gw_proxy::{AccessProvider, Dispatcher, HoldMiddleware, ProxyState, Settlement, reconcile};
use tokio_util::task::TaskTracker;
use tracing::{info, warn};

use crate::health::{HealthState, ProbeFuture, StoreProbe};
use crate::metrics::Metrics;

/// How often the L1 caches evict expired entries.
pub const CACHE_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// How often `channel_policies` is reloaded.
pub const CHANNEL_POLICY_REFRESH: Duration = Duration::from_secs(60);

/// The wired gateway: what [`crate::serve`] needs, plus the handles that must
/// outlive this function.
pub struct Wiring {
    /// Readiness probes over the live Postgres pool and Redis client.
    pub health: HealthState,
    /// `gw_proxy::router(..)` merged with `gw_panel::router(..)` — the
    /// `domains` argument of [`crate::app_router`].
    pub domains: Router,
    /// Background tasks. Dropping this aborts them, so `run` holds it until
    /// the server returns.
    pub guards: Guards,
}

/// Background work started during wiring.
///
/// Every field is a live task handle. They are grouped into one value so the
/// caller cannot keep some and drop others by accident — in particular the
/// price-cache refresher, whose task exits as soon as the last `Arc` to the
/// cache goes away.
pub struct Guards {
    /// `api_keys` L1 sweeper.
    pub api_key_sweeper: SweepHandle,
    /// `users.status` L1 sweeper.
    pub user_status_sweeper: SweepHandle,
    /// Periodic `model_prices` reload.
    pub price_refresh: Option<tokio::task::JoinHandle<()>>,
    /// Periodic `channel_policies` reload.
    pub channel_policy_refresh: tokio::task::JoinHandle<()>,
    /// Startup + 5-minute orphaned-hold scan.
    pub orphan_scanner: tokio::task::JoinHandle<()>,
}

impl Drop for Guards {
    fn drop(&mut self) {
        if let Some(handle) = self.price_refresh.take() {
            handle.abort();
        }
        self.channel_policy_refresh.abort();
        self.orphan_scanner.abort();
    }
}

/// Builds every component the gateway serves with.
///
/// `metrics` and `drain` come from the [`crate::AppState`] the caller already
/// owns: the same counters back `/metrics/prometheus`, and the same tracker is
/// drained after shutdown.
///
/// `shutdown` resolves when the process is asked to stop; it ends the
/// orphaned-hold loop, which otherwise runs forever (see
/// [`gw_proxy::reconcile::spawn_scanner`] for why that loop must NOT be
/// registered on the drain tracker).
///
/// # Errors
///
/// Postgres, the migrations, the auth store's encryption key and any
/// unparseable upstream base URL are all hard failures — a gateway that cannot
/// reach its database or cannot decrypt its credentials must not accept
/// traffic. Redis is a hard failure too: `PanelState` holds a live client
/// rather than a nullable one, and without Redis the ledger cannot place
/// a hold, so "degraded" would mean "serving `/v1` for free".
pub async fn wire(
    config: &Config,
    metrics: Arc<Metrics>,
    drain: TaskTracker,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<Wiring> {
    // ---------------------------------------------------------------- infra
    let pg = gw_infra::init_db(
        &DbSettings::from(&config.database),
        SqlLogLevel::parse(&config.server.log_level),
    )
    .await
    .context("connecting to Postgres")?;

    gw_model::run_migrations(&pg)
        .await
        .context("running database migrations")?;

    let redis = gw_infra::init_redis(&config.redis.addr, &config.redis.password, config.redis.db)
        .await
        .context(
            "Redis is unavailable — the ledger cannot place holds without it, so the gateway would \
         serve /v1 without billing; fix redis.addr / the server and restart",
        )?;

    run_seeds(&pg, config).await;

    let health = HealthState::new()
        .with_database(database_probe(pg.clone()))
        .with_redis(redis_probe(redis.clone()));

    // ------------------------------------------------------ billing + pricing
    // B7: one uniform hold TTL shared by the ledger and the hold middleware.
    // A per-request TTL desyncs the cleanup cutoffs inside `ledger.Hold`.
    let ledger = Arc::new(Ledger::with_config(
        pg.clone(),
        Some(redis.clone()),
        config.billing.balance_cache_ttl(),
        config.billing.hold_ttl(),
    ));

    let price_cache = Arc::new(
        ModelPriceCache::load(&pg)
            .await
            .context("loading model prices")?,
    );
    let price_refresh = price_cache.start_refresh(pg.clone(), config.billing.price_cache_refresh());
    let calc = Arc::new(Calculator::new(
        Some(Arc::clone(&price_cache)),
        config.billing.default_price_per_1m_tokens(),
    ));

    // --------------------------------------------------------------- identity
    let api_key_cache = ApiKeyCache::new();
    let user_status_cache = UserStatusCache::new();
    let api_key_sweeper = api_key_cache.spawn_sweeper(CACHE_SWEEP_INTERVAL);
    let user_status_sweeper = user_status_cache.spawn_sweeper(CACHE_SWEEP_INTERVAL);

    let auth_store: Arc<dyn gw_authcore::AuthStore> = Arc::new(
        gw_authcore::PostgresAuthStore::new(pg.clone(), &config.auth.credential_encryption_key)
            .context("building the upstream credential store")?,
    );
    // B3: the config-seeded upstreams are runtime-only — they are never
    // persisted, so a store that only reads `auth_records` would leave /v1 with
    // no upstream at all after the first reload.
    let runtime_auths = gw_authcore::build_runtime_upstreams(&config.sdk)
        .context("building config-seeded upstream credentials")?;
    info!(
        runtime_upstreams = runtime_auths.len(),
        "registered runtime-only upstream credentials"
    );
    let dispatch_auth_store =
        gw_authcore::RuntimeAuthStore::wrap(Arc::clone(&auth_store), runtime_auths);

    // ------------------------------------------------------------ proxy state
    let crypto = Arc::new(AuthcoreCrypto::new(config.auth.jwt.secret.clone()));
    let access = Arc::new(AccessProvider::new(
        Arc::new(SqlTenantDirectory::new(
            pg.clone(),
            api_key_cache.clone(),
            user_status_cache.clone(),
        )),
        Arc::clone(&crypto) as Arc<_>,
    ));

    let shared_ledger = Arc::new(SharedLedger::new(Arc::clone(&ledger)));
    let shared_calc = Arc::new(SharedCalculator::new(Arc::clone(&calc)));
    let budget_tokens = Arc::new(BudgetTokenStore::new());

    let settlement = Arc::new(
        Settlement::new(
            Arc::clone(&shared_ledger) as Arc<_>,
            Arc::clone(&shared_calc) as Arc<_>,
            Arc::new(SqlUsageStore::new(pg.clone(), Arc::clone(&ledger))),
        )
        .with_budget_tokens(Arc::clone(&budget_tokens))
        .with_low_balance_threshold(config.billing.low_balance_threshold_usd),
    );
    settlement.set_strict_usage_metadata(config.billing.strict_usage_metadata_mode);

    let rate_limiter = Arc::new(InfraRateLimiter::new(
        Some(redis.clone()),
        RateLimitSettings::from(&config.rate_limit),
    ));
    let circuit_breaker = Arc::new(InfraCircuitBreaker::new(
        Some(redis.clone()),
        CircuitBreakerSettings::from(&config.circuit_breaker),
    ));
    let shared_breaker = Arc::new(SharedCircuitBreaker::new(Arc::clone(&circuit_breaker)));
    // 0 → the manager's default 24h TTL.
    let idempotency = Arc::new(IdempotencyManager::new(
        Arc::new(RedisIdempotencyStore::new(redis.clone())),
        Arc::clone(&crypto) as Arc<_>,
        Duration::ZERO,
    ));

    let hold = Arc::new(
        HoldMiddleware::new(
            Arc::clone(&shared_ledger) as Arc<_>,
            Arc::clone(&shared_calc) as Arc<_>,
            Arc::clone(&settlement),
            config.billing.hold_ttl(),
        )
        .with_quota_store(Arc::new(SqlSubscriptionQuotaStore::new(pg.clone())))
        .with_rate_limiter(Arc::new(SharedRateLimiter::new(Arc::clone(&rate_limiter))))
        .with_circuit_breaker(Arc::clone(&shared_breaker) as Arc<_>)
        .with_idempotency(Arc::clone(&idempotency))
        .with_budget_tokens(Arc::clone(&budget_tokens)),
    );

    // Defaults (3 failures / 30s cooldown).
    let channel_health = Arc::new(ChannelHealth::new(0, Duration::ZERO));
    let channel_policies = Arc::new(ChannelPolicyCache::new(Arc::new(
        SqlChannelPolicyStore::new(pg.clone()),
    )));
    if let Err(err) = channel_policies.refresh().await {
        warn!(%err, "failed to load channel policies; falling back to defaults");
    }
    let channel_policy_refresh =
        Arc::clone(&channel_policies).spawn_refresh(CHANNEL_POLICY_REFRESH);
    let channels = Arc::new(
        ChannelPool::new(Arc::clone(&channel_health)).with_policies(Arc::clone(&channel_policies)),
    );

    let dispatch = Arc::new(
        Dispatcher::new(
            build_providers(&config.sdk)?,
            dispatch_auth_store,
            channels,
            Arc::clone(&settlement),
        )
        .with_circuit_breaker(Arc::clone(&shared_breaker) as Arc<_>)
        .with_catalog(Arc::new(SqlModelCatalog::new(pg.clone()))),
    );

    let proxy_state = ProxyState::new(access, hold, dispatch, drain.clone())
        .with_metrics(Arc::clone(&metrics) as Arc<_>);
    proxy_state.publish_gauges();

    // B8: the scan always runs (it feeds `cpa_orphaned_holds`); charging the
    // reserved amount is opt-in through BILLING_AUTO_RECONCILE_HOLDS.
    let auto_reconcile = reconcile::auto_reconcile_enabled();
    let orphan_scanner = reconcile::spawn_scanner(
        Arc::clone(&shared_ledger) as Arc<_>,
        Arc::clone(&settlement),
        Arc::clone(&metrics) as Arc<_>,
        reconcile::DEFAULT_SCAN_INTERVAL,
        drain,
        auto_reconcile,
        shutdown,
    );

    // ------------------------------------------------------------ panel state
    let panel_state = gw_panel::PanelState {
        pg: pg.clone(),
        redis,
        cfg: Arc::new(config.clone()),
        // Same instance as the Calculator's, so an admin price upsert
        // invalidates the cache the calculator actually reads.
        price_cache,
        calc,
        ledger,
        auth_store,
        // Same instances as the access provider's, so a status flip or a key
        // revocation on one surface is honored on the other.
        user_status_cache: Arc::new(user_status_cache),
        api_key_cache: Arc::new(api_key_cache),
        audit_hmac_key: gw_panel::audit::derive_audit_key(&config.auth.credential_encryption_key)
            .map(Arc::new),
        stripe_webhook_secret: std::env::var("STRIPE_WEBHOOK_SECRET")
            .ok()
            .filter(|secret| !secret.trim().is_empty())
            .map(Arc::new),
    };

    let domains = gw_proxy::router(proxy_state).merge(gw_panel::router(panel_state));

    Ok(Wiring {
        health,
        domains,
        guards: Guards {
            api_key_sweeper,
            user_status_sweeper,
            price_refresh,
            channel_policy_refresh,
            orphan_scanner,
        },
    })
}

/// The four startup seeds, in run order.
///
/// The posture is copied exactly, including which failures are fatal: the price
/// and bootstrap-admin seeds only warn (an operator-maintained price
/// table and a missing bootstrap user are both normal), while the subscription
/// and SDK-management seeds returned an error there. Neither is worth refusing
/// traffic over on a running deployment, so both are logged at `error` and the
/// gateway continues — the panel surfaces the missing rows immediately.
async fn run_seeds(pg: &Db, config: &Config) {
    match gw_model::seed::seed_model_prices(pg).await {
        Ok(rows) => info!(inserted = rows, "model price seeds applied"),
        Err(err) => warn!(%err, "failed to seed model prices; continuing startup"),
    }
    match gw_model::seed::ensure_subscription_seeds(pg).await {
        Ok(created) => info!(created, "subscription package seeds applied"),
        Err(err) => tracing::error!(%err, "failed to seed subscription packages"),
    }
    match gw_model::seed::ensure_sdk_management_seeds(pg, &sdk_seed_config(config)).await {
        Ok(outcome) => info!(
            sdk_config_created = outcome.sdk_config_created,
            ampcode_config_created = outcome.ampcode_config_created,
            "SDK management seeds applied"
        ),
        Err(err) => tracing::error!(%err, "failed to seed SDK management records"),
    }
    match gw_model::seed::ensure_bootstrap_admin(pg, &config.auth.bootstrap_admin_email).await {
        Ok(BootstrapAdmin::Promoted { user_id }) => {
            info!(user_id, "bootstrap admin promoted")
        }
        Ok(outcome) => info!(?outcome, "bootstrap admin check complete"),
        Err(err) => warn!(%err, "failed to ensure bootstrap admin; continuing startup"),
    }
}

/// The `provider_configs.sdk_config` document.
/// six providers by name, each contributing only `base_url` + `enabled` —
/// deliberately no `api_key`, so the seeded row carries no secret.
fn sdk_seed_config(config: &Config) -> SdkSeedConfig {
    let sdk = &config.sdk;
    let provider = |p: &gw_config::SdkProviderConfig| SdkSeedProvider {
        base_url: p.base_url.clone(),
        enabled: p.enabled,
    };
    SdkSeedConfig {
        base_url: sdk.base_url.clone(),
        timeout_seconds: i64::from(sdk.timeout_seconds),
        providers: [
            ("openai".to_owned(), provider(&sdk.openai)),
            (
                "openai_compatible".to_owned(),
                provider(&sdk.openai_compatible),
            ),
            ("claude".to_owned(), provider(&sdk.claude)),
            ("gemini".to_owned(), provider(&sdk.gemini)),
            ("codex".to_owned(), provider(&sdk.codex)),
            ("vertex".to_owned(), provider(&sdk.vertex)),
        ]
        .into_iter()
        .collect(),
    }
}

/// The upstream executors, gated by the same configuration rules as the
/// runtime upstream builder.
///
/// Claude / Gemini / Codex / Vertex are **always** built, even with no
/// configured credential: a persisted `auth_records` row can supply one at
/// request time, and without the executor that row would have nothing to
/// dispatch through. The OpenAI-compatible executor is the exception — it has
/// no default host, so an incomplete config leaves nothing to build.
///
/// # Errors
/// An unparseable `base_url` is rejected at wiring time rather than on the
/// first request.
fn build_providers(sdk: &gw_config::SdkConfig) -> anyhow::Result<Vec<Arc<dyn Provider>>> {
    let timeout = i64::from(sdk.timeout_seconds);
    let cfg = |p: &gw_config::SdkProviderConfig| ProviderConfig {
        base_url: p.base_url.clone(),
        api_key: p.api_key.clone(),
        enabled: p.enabled,
    };

    let mut providers: Vec<Arc<dyn Provider>> = Vec::with_capacity(5);

    let openai = sdk.openai_provider_config();
    if openai.complete() {
        providers.push(Arc::new(
            OpenAiCompatibleProvider::new(&cfg(&openai), timeout)
                .context("building the OpenAI-compatible upstream")?,
        ));
    } else {
        warn!(
            "OpenAI-compatible upstream disabled: sdk.openai/openai_compatible or legacy sdk.base_url/api_key is missing"
        );
    }

    providers.push(Arc::new(
        ClaudeProvider::new(&cfg(&sdk.claude), timeout).context("building the Claude upstream")?,
    ));
    providers.push(Arc::new(
        GeminiProvider::new(&cfg(&sdk.gemini), timeout).context("building the Gemini upstream")?,
    ));
    providers.push(Arc::new(
        CodexProvider::new(&cfg(&sdk.codex), timeout).context("building the Codex upstream")?,
    ));
    providers.push(Arc::new(
        VertexProvider::new(&cfg(&sdk.vertex), timeout).context("building the Vertex upstream")?,
    ));

    Ok(providers)
}

/// `SELECT 1` against the pool. sqlx has no pool-level ping, and a trivial
/// query is what a driver-level ping degrades to anyway.
fn database_probe(pg: Db) -> Arc<dyn StoreProbe> {
    Arc::new(move || -> ProbeFuture<'static> {
        let pg = pg.clone();
        Box::pin(async move {
            sqlx::query("SELECT 1")
                .execute(&pg)
                .await
                .map(|_| ())
                .map_err(|err| err.to_string())
        })
    })
}

/// A Redis `PING` probe.
fn redis_probe(redis: Redis) -> Arc<dyn StoreProbe> {
    Arc::new(move || -> ProbeFuture<'static> {
        let mut conn = redis.clone();
        Box::pin(async move {
            redis::cmd("PING")
                .query_async::<String>(&mut conn)
                .await
                .map(|_| ())
                .map_err(|err| err.to_string())
        })
    })
}

#[cfg(test)]
mod tests;

//! Composition root. Everything lives here rather than in `main.rs` because
//! integration tests under `tests/` can only reach the LIBRARY (rule 1.5) —
//! logic in `main.rs` is permanently untestable.
//!
//! What this crate owns:
//!
//! * the three flags (`--config`, `--version`, `--health-check`) and the
//!   container health probe behind the last one;
//! * boot-time validation (JWT secret strength, `sslmode`) and logger setup;
//! * `/api/health`, `/healthz`, `/api/health/ready`, `/metrics`,
//!   `/metrics/prometheus`;
//! * the axum server, its SIGINT/SIGTERM graceful shutdown, and the settlement
//!   drain that follows it (see [`drain`]).
//!
//! Everything else — the `/v1/*` proxy surface and `/api/panel/**` — is merged
//! in from [`gw_proxy`] and [`gw_panel`]; see [`app_router`].
//!
//! OWNER: worker `platform`.

#![deny(clippy::todo, clippy::unimplemented)]

pub mod cli;
pub mod drain;
pub mod health;
pub mod metrics;
pub mod probe;
pub mod response;
pub mod startup;
pub mod wiring;

#[cfg(test)]
mod tests;

use std::future::Future;
use std::sync::Arc;

use anyhow::Context as _;
use axum::Router;
use axum::extract::FromRef;
use axum::routing::get;
use gw_config::Config;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};

pub use cli::{APP_VERSION, Cli};
pub use drain::{DrainOutcome, drain_settlements, drain_timeout};
pub use health::{HealthState, StoreProbe};
pub use metrics::Metrics;
pub use wiring::{Guards, Wiring, wire};

/// Everything the endpoints this crate owns need, cloned into each handler.
#[derive(Clone, Debug, Default)]
pub struct AppState {
    /// Backing stores probed by `/api/health/ready`.
    pub health: HealthState,
    /// Shared request metrics. `gw-proxy` and `gw-panel` publish gauges into
    /// the same instance (see [`Metrics::set_channel_benched`] /
    /// [`Metrics::set_orphaned_holds`]).
    pub metrics: Arc<Metrics>,
    /// Tracker for detached settlement tasks, drained after graceful shutdown.
    ///
    /// The composition root creates ONE tracker and hands a clone to both this
    /// state and `gw_proxy::ProxyState`, whose `StreamSettler::drop` spawns
    /// into it instead of calling `tokio::spawn`. `TaskTracker` clones share
    /// one set of tasks, so closing it here closes the proxy's view too.
    /// Leaving it at its default (an empty tracker nobody spawns into) simply
    /// makes the drain a no-op. See [`drain`] for the ordering rules.
    pub drain: TaskTracker,
}

impl FromRef<AppState> for HealthState {
    fn from_ref(state: &AppState) -> Self {
        state.health.clone()
    }
}

impl FromRef<AppState> for Arc<Metrics> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.metrics)
    }
}

/// The endpoints this crate implements, with state already applied.
pub fn base_router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health::liveness))
        // The CPA SDK's internal server owned `/healthz`, and dropping the
        // SDK dropped the route while `frontend/vite.config.ts` kept proxying
        // it. See [`health::healthz`] for why its body differs from
        // `/api/health`'s.
        .route("/healthz", get(health::healthz))
        .route("/api/health/ready", get(health::readiness))
        .route("/metrics", get(metrics::json))
        .route("/metrics/prometheus", get(metrics::prometheus))
        .with_state(state)
}

/// The whole HTTP surface: this crate's endpoints, plus `domains` — the merged
/// `gw_proxy::router(..)` / `gw_panel::router(..)` — wrapped in the metrics
/// layer.
///
/// Reproduces the SDK Builder wiring: `WithRouterConfigurator` mounted the
/// panel routes and `WithMiddleware` installed the outermost metrics
/// middleware. Two things are load-bearing:
///
/// * the layer goes on LAST, after every merge, so it spans the merged routes
///   too — an `axum` layer only wraps routes registered before it;
/// * `domains` arrives as an already-built [`Router`] rather than being
///   constructed here, so this crate does not track the shape of
///   `gw_proxy::ProxyState` / `gw_panel::PanelState` while those crates are
///   still moving.
pub fn app_router(state: AppState, domains: Option<Router>) -> Router {
    let metrics = Arc::clone(&state.metrics);
    let mut app = base_router(state);
    if let Some(domains) = domains {
        app = app.merge(domains);
    }
    app.layer(axum::middleware::from_fn_with_state(
        metrics,
        metrics::track,
    ))
}

/// Bind, serve, and drain.
///
/// Returns once SIGINT/SIGTERM has been handled, every in-flight request has
/// finished, and the settlements they left behind have been drained.
pub async fn serve(
    config: &Config,
    state: AppState,
    domains: Option<Router>,
) -> anyhow::Result<()> {
    serve_with_shutdown(config, state, domains, shutdown_signal()).await
}

/// [`serve`] with an explicit shutdown trigger, so the drain-before-return
/// guarantee is testable without sending the test process a signal.
pub async fn serve_with_shutdown(
    config: &Config,
    state: AppState,
    domains: Option<Router>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let tracker = state.drain.clone();
    let metrics = Arc::clone(&state.metrics);
    let app = app_router(state, domains);

    let listener = tokio::net::TcpListener::bind((config.server.host.as_str(), config.server.port))
        .await
        .with_context(|| format!("binding {}:{}", config.server.host, config.server.port))?;

    info!(
        host = %config.server.host,
        port = config.server.port,
        "starting CPA-Gateway",
    );

    let served = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .context("serving HTTP");

    // The drain runs even when `serve` failed: settlements spawned before the
    // failure are still owed to the ledger. ORDERING — this must come after the
    // await above, never before it (see the `drain` module docs).
    let outcome = drain_settlements(&tracker, drain_timeout(&config.billing)).await;
    metrics.set_abandoned_settlements(outcome.abandoned as i64);
    served?;

    info!("CPA-Gateway stopped");
    Ok(())
}

/// Resolve when the process is asked to stop.
///
/// On deploy/restart the server drains in-flight requests instead of being
/// hard-killed mid-settlement.
async fn shutdown_signal() {
    let interrupt = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            warn!(error = %err, "cannot listen for SIGINT; graceful shutdown disabled");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                warn!(error = %err, "cannot listen for SIGTERM; graceful shutdown disabled");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }

    info!("shutdown signal received; draining in-flight requests");
}

/// Parse args, load config, wire every component, serve, shut down gracefully.
///
/// This function owns the *process* — flags, the version/health-check exits,
/// tracing, boot-time validation and the tokio runtime. The object graph is
/// [`wiring::wire`]'s, and [`boot`] joins the two. Splitting them is what makes
/// the wiring reachable from `tests/` at all: `run` cannot be called twice in a
/// process, and half of it is `std::process::exit`.
pub fn run() -> anyhow::Result<()> {
    let cli = match cli::parse(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(err) => {
            // Print the error + usage to stderr and exit 2.
            eprintln!("{err}\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };

    if cli.show_help {
        eprintln!("{}", cli::USAGE);
        return Ok(());
    }

    if cli.show_version {
        startup::init_tracing(gw_config::DEFAULT_LOG_LEVEL);
        info!(version = APP_VERSION, "cpa-gateway");
        return Ok(());
    }

    if cli.health_check {
        std::process::exit(probe::health_check_exit_code());
    }

    let config = Config::load(&cli.config_path)
        .with_context(|| format!("loading config from {}", cli.config_path))?;

    startup::init_tracing(&config.server.log_level);

    // Fail fast on a misconfigured JWT secret, then nudge operators off
    // cleartext Postgres and cleartext upstream credentials.
    startup::validate_jwt_secret(&config.auth.jwt.secret)?;
    startup::warn_insecure_sslmode(&config.database.sslmode);
    if config.auth.credential_encryption_key.is_empty() {
        warn!(
            "CREDENTIAL_ENCRYPTION_KEY not set — upstream provider credentials will be stored in cleartext; set a 32-byte key (hex/base64) to encrypt auth_records.metadata at rest"
        );
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the tokio runtime")?;

    runtime.block_on(boot(config))
}

/// Wire the object graph, serve it, and keep the background tasks alive for
/// exactly as long as the server runs.
///
/// Everything after config validation lives here. The `state.drain.clone()`
/// handed to [`wiring::wire`] is the load-bearing detail:
/// `StreamSettler::drop` must spawn into THAT tracker, or its settlements die
/// with the runtime and the holds behind them leak (see the [`drain`] module).
pub async fn boot(config: Config) -> anyhow::Result<()> {
    let mut state = AppState::default();

    // One shutdown event with two consumers. `serve_with_shutdown` stops
    // accepting; the orphaned-hold loop ends instead of running forever —
    // it is deliberately NOT on the drain tracker, because a perpetual task
    // there would make every shutdown wait out the drain timeout and then
    // report itself as an abandoned settlement (see
    // `gw_proxy::reconcile::spawn_scanner`).
    let (stopping, mut stopped) = tokio::sync::watch::channel(false);
    let scan_until_shutdown = async move {
        let _ = stopped.changed().await;
    };

    let Wiring {
        health,
        domains,
        // Sweepers and refresh loops. Held until `serve` returns: dropping
        // this value aborts them.
        guards,
    } = wiring::wire(
        &config,
        Arc::clone(&state.metrics),
        state.drain.clone(),
        scan_until_shutdown,
    )
    .await?;
    state.health = health;

    let shutdown = async move {
        shutdown_signal().await;
        let _ = stopping.send(true);
    };

    let result = serve_with_shutdown(&config, state, Some(domains), shutdown).await;
    drop(guards);
    result
}

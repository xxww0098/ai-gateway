//! PostgreSQL pool bootstrap.

use std::str::FromStr;
use std::time::Duration;

use sqlx::ConnectOptions;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};

use crate::{Db, InfraError};

#[cfg(test)]
mod tests;

/// Idle-connection default (`pool_value_or_default(_, 25)`).
pub const DEFAULT_MAX_IDLE_CONNS: u32 = 25;
/// Open-connection default (`pool_value_or_default(_, 200)`).
pub const DEFAULT_MAX_OPEN_CONNS: u32 = 200;
/// Connection-lifetime default (`pool_value_or_default(_, 30)`).
pub const DEFAULT_CONN_MAX_LIFETIME_MINUTES: u32 = 30;

/// Connection + pool settings. Mirrors `gw_config::DatabaseConfig`
/// field-for-field, including the "0 means use the gateway default"
/// convention of the three pool knobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbSettings {
    /// `database.host`.
    pub host: String,
    /// `database.port`.
    pub port: u16,
    /// `database.user`.
    pub user: String,
    /// `database.password`.
    pub password: String,
    /// `database.dbname`.
    pub dbname: String,
    /// `database.sslmode`; empty falls back to libpq's `prefer`.
    pub sslmode: String,
    /// `database.max_idle_conns`; `<= 0` → [`DEFAULT_MAX_IDLE_CONNS`].
    pub max_idle_conns: i32,
    /// `database.max_open_conns`; `<= 0` → [`DEFAULT_MAX_OPEN_CONNS`].
    pub max_open_conns: i32,
    /// `database.conn_max_lifetime_minutes`; `<= 0` →
    /// [`DEFAULT_CONN_MAX_LIFETIME_MINUTES`].
    pub conn_max_lifetime_minutes: i32,
}

impl Default for DbSettings {
    /// The values `config.example.yaml` ships with, so a `DbSettings::default()`
    /// points at a local development Postgres rather than at nothing.
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 5432,
            user: "ai_gateway".to_owned(),
            password: String::new(),
            dbname: "ai_gateway".to_owned(),
            sslmode: "disable".to_owned(),
            max_idle_conns: 0,
            max_open_conns: 0,
            conn_max_lifetime_minutes: 0,
        }
    }
}

impl From<&gw_config::DatabaseConfig> for DbSettings {
    /// Lifts the `database:` block of `config.yaml` into this crate's settings.
    /// `gw_config` deliberately leaves the pool knobs un-normalized, so the
    /// "0 means default" resolution stays here, where the defaults live.
    fn from(cfg: &gw_config::DatabaseConfig) -> Self {
        Self {
            host: cfg.host.clone(),
            port: cfg.port,
            user: cfg.user.clone(),
            password: cfg.password.clone(),
            dbname: cfg.dbname.clone(),
            sslmode: cfg.sslmode.clone(),
            max_idle_conns: cfg.max_idle_conns,
            max_open_conns: cfg.max_open_conns,
            conn_max_lifetime_minutes: cfg.conn_max_lifetime_minutes,
        }
    }
}

/// Resolved pool sizing, after the "0 means default" fallbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolSizing {
    /// Retained-idle floor (sqlx `min_connections`); see
    /// [`DbSettings::pool_sizing`].
    pub idle: u32,
    /// Hard ceiling on open connections (sqlx `max_connections`).
    pub open: u32,
    /// Connection lifetime (sqlx `max_lifetime`).
    pub lifetime: Duration,
}

impl DbSettings {
    /// The libpq keyword/value connection string.
    ///
    /// [`init_db`] does *not* go through this — it builds [`PgConnectOptions`]
    /// programmatically so a password containing spaces or quotes cannot break
    /// the string. The DSN is kept for tooling that wants a connection string
    /// (psql, migration runners).
    ///
    /// **Contains the password in clear text — never log the result.**
    pub fn dsn(&self) -> String {
        format!(
            "host={} port={} user={} password={} dbname={} sslmode={}",
            self.host, self.port, self.user, self.password, self.dbname, self.sslmode
        )
    }

    /// Resolves the three pool knobs, applying the gateway defaults for
    /// non-positive values.
    ///
    /// **Mapping note.** An idle-connection cap is a *cap* on retained idle
    /// connections; sqlx has no such cap (it keeps connections until
    /// `idle_timeout`/`max_lifetime`). The closest sqlx knob is
    /// `min_connections`, a keep-warm *floor*, which is what
    /// `max_idle_conns` maps to here: both express "this many connections
    /// should stay ready between bursts". The floor is clamped to the open
    /// ceiling so a misconfiguration cannot ask for more idle than max.
    pub fn pool_sizing(&self) -> PoolSizing {
        let open = pool_value_or_default(self.max_open_conns, DEFAULT_MAX_OPEN_CONNS);
        let idle = pool_value_or_default(self.max_idle_conns, DEFAULT_MAX_IDLE_CONNS).min(open);
        let minutes = u64::from(pool_value_or_default(
            self.conn_max_lifetime_minutes,
            DEFAULT_CONN_MAX_LIFETIME_MINUTES,
        ));
        PoolSizing {
            idle,
            open,
            lifetime: Duration::from_secs(minutes * 60),
        }
    }

    /// Builds the sqlx connect options for these settings.
    ///
    /// Errors when `sslmode` is not one of libpq's six values.
    pub fn connect_options(&self, log_level: SqlLogLevel) -> Result<PgConnectOptions, InfraError> {
        let mut options = PgConnectOptions::new()
            .host(&self.host)
            .port(self.port)
            .username(&self.user)
            .database(&self.dbname)
            .ssl_mode(parse_ssl_mode(&self.sslmode)?);

        if !self.password.is_empty() {
            options = options.password(&self.password);
        }
        if log_level == SqlLogLevel::Silent {
            options = options.disable_statement_logging();
        }
        Ok(options)
    }
}

/// Any non-positive value means "use the gateway default".
fn pool_value_or_default(value: i32, default: u32) -> u32 {
    u32::try_from(value)
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

/// The accepted SQL statement log-level vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SqlLogLevel {
    /// `silent` — no statement logging at all.
    Silent,
    /// `error`.
    Error,
    /// `warn` — the default, quiet enough for the hot billing path.
    #[default]
    Warn,
    /// `info` — logs every statement.
    Info,
}

impl SqlLogLevel {
    /// Case-insensitive, whitespace-trimmed, and *anything* unrecognized
    /// (including the empty string) falls back to [`SqlLogLevel::Warn`].
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "silent" => Self::Silent,
            "error" => Self::Error,
            "info" => Self::Info,
            _ => Self::Warn,
        }
    }

    /// The canonical spelling, i.e. the input [`SqlLogLevel::parse`] round-trips.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Silent => "silent",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
        }
    }
}

/// Maps `database.sslmode` onto [`PgSslMode`]. Empty means "unset", which
/// libpq resolves to `prefer`.
fn parse_ssl_mode(raw: &str) -> Result<PgSslMode, InfraError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(PgSslMode::Prefer);
    }
    PgSslMode::from_str(trimmed).map_err(|err| InfraError::Invalid {
        field: "database.sslmode",
        reason: err.to_string(),
    })
}

/// Opens the Postgres pool.
///
/// Notes on the sqlx mapping:
///
/// * Statement-preparing has no counterpart to set — sqlx caches prepared
///   statements per connection by default.
/// * `log_level` only distinguishes silent from non-silent: sqlx's per-level
///   statement logging is configured with a `log::LevelFilter`, and this crate
///   does not depend on `log`. Non-silent levels keep sqlx's default (statements
///   at DEBUG, slow statements at WARN).
pub async fn init_db(settings: &DbSettings, log_level: SqlLogLevel) -> Result<Db, InfraError> {
    let sizing = settings.pool_sizing();
    let pool = PgPoolOptions::new()
        .max_connections(sizing.open)
        .min_connections(sizing.idle)
        .max_lifetime(sizing.lifetime)
        .connect_with(settings.connect_options(log_level)?)
        .await?;

    tracing::info!(
        log_level = log_level.as_str(),
        max_idle = sizing.idle,
        max_open = sizing.open,
        conn_max_lifetime_min = sizing.lifetime.as_secs() / 60,
        "database connection established"
    );
    Ok(pool)
}

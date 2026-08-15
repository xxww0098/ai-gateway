//! Boot-time validation and logging setup: JWT secret strength, the sslmode
//! warning, and logger initialization.

use tracing::warn;

/// 32 bytes (256 bits) matches the HMAC-SHA256 key size; anything shorter is
/// brute-forceable.
pub const MIN_JWT_SECRET_BYTES: usize = 32;

/// A misconfiguration that must stop the process before it serves traffic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartupError {
    #[error("auth.jwt.secret is too weak: {bytes} bytes, need at least {minimum}")]
    WeakJwtSecret { bytes: usize, minimum: usize },
}

/// Reject an unacceptably weak JWT secret.
///
/// An empty secret is tolerated — `gw-authcore` fails closed at request time,
/// and the shipped example config carries no secret so dev/CI can still boot —
/// but a non-empty secret shorter than [`MIN_JWT_SECRET_BYTES`] is a hard
/// error: it boots and then silently accepts brute-forceable HS256 tokens.
pub fn validate_jwt_secret(secret: &str) -> Result<(), StartupError> {
    if secret.is_empty() {
        warn!(
            "JWT secret is empty — panel auth will fail closed; set auth.jwt.secret or JWT_SECRET before serving traffic"
        );
        return Ok(());
    }
    if secret.len() < MIN_JWT_SECRET_BYTES {
        return Err(StartupError::WeakJwtSecret {
            bytes: secret.len(),
            minimum: MIN_JWT_SECRET_BYTES,
        });
    }
    Ok(())
}

/// Whether a `database.sslmode` value leaves the Postgres connection in
/// cleartext (`""` or `"disable"`).
pub fn sslmode_is_insecure(sslmode: &str) -> bool {
    let mode = sslmode.trim().to_ascii_lowercase();
    mode.is_empty() || mode == "disable"
}

/// Nudge operators off plaintext DB connections: `sslmode=disable` sends
/// credentials and the gateway's secret-bearing rows in cleartext.
pub fn warn_insecure_sslmode(sslmode: &str) {
    if sslmode_is_insecure(sslmode) {
        warn!(
            sslmode,
            "database sslmode is disabled — connection to Postgres is unencrypted; set database.sslmode=require (or verify-full) in production"
        );
    }
}

/// Translate `server.log_level` into a `tracing-subscriber` filter directive.
///
/// `server.log_level` drives **only** the SQL logger while application logging
/// always runs at Info. That split is preserved here: application spans stay at
/// `info` so startup and billing lines survive, and the level is applied to the
/// SQL target (`sqlx`). Unknown/empty values fall back to `warn`.
pub fn log_filter(log_level: &str) -> String {
    let sql = match log_level.trim().to_ascii_lowercase().as_str() {
        "silent" => "off",
        "error" => "error",
        "info" => "info",
        _ => "warn",
    };
    format!("info,sqlx={sql}")
}

/// Install the global tracing subscriber. `RUST_LOG`, when set, wins — it is
/// the escape hatch for debugging a running container without a config edit.
///
/// Repeat calls are no-ops (the first subscriber stays installed), so tests may
/// call this freely.
pub fn init_tracing(log_level: &str) {
    let directives = std::env::var("RUST_LOG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| log_filter(log_level));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(directives))
        .try_init();
}

#[cfg(test)]
mod tests;

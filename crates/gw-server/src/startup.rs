//! Boot-time validation and logging setup: JWT secret strength, credential
//! encryption key, the sslmode warning, and logger initialization.

use tracing::warn;

/// 32 bytes (256 bits) matches the HMAC-SHA256 key size; anything shorter is
/// brute-forceable.
pub const MIN_JWT_SECRET_BYTES: usize = 32;

/// Env flag: when truthy (`1`/`true`/`yes`/`on`), empty JWT secret or empty
/// credential encryption key refuses to boot.
pub const STRICT_SECRETS_ENV: &str = "AI_GATEWAY_STRICT_SECRETS";

/// A misconfiguration that must stop the process before it serves traffic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartupError {
    #[error("auth.jwt.secret is too weak: {bytes} bytes, need at least {minimum}")]
    WeakJwtSecret { bytes: usize, minimum: usize },
    #[error(
        "auth.jwt.secret is empty — set auth.jwt.secret / JWT_SECRET (or unset AI_GATEWAY_STRICT_SECRETS)"
    )]
    MissingJwtSecret,
    #[error(
        "CREDENTIAL_ENCRYPTION_KEY is empty — upstream credentials would be stored in cleartext; set a 32-byte key (hex/base64) or unset AI_GATEWAY_STRICT_SECRETS"
    )]
    MissingCredentialEncryptionKey,
}

/// Whether [`STRICT_SECRETS_ENV`] demands fail-closed secret checks.
#[must_use]
pub fn strict_secrets_enabled() -> bool {
    matches!(
        std::env::var(STRICT_SECRETS_ENV)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Reject an unacceptably weak JWT secret.
///
/// An empty secret is tolerated unless [`strict_secrets_enabled`] — then it is
/// a hard error. A non-empty secret shorter than [`MIN_JWT_SECRET_BYTES`] is
/// always a hard error.
pub fn validate_jwt_secret(secret: &str) -> Result<(), StartupError> {
    validate_jwt_secret_with(secret, strict_secrets_enabled())
}

/// Testable form of [`validate_jwt_secret`].
pub fn validate_jwt_secret_with(secret: &str, strict: bool) -> Result<(), StartupError> {
    if secret.is_empty() {
        if strict {
            return Err(StartupError::MissingJwtSecret);
        }
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

/// Reject a missing credential encryption key when production discipline applies.
///
/// Rules:
/// * `strict` (from [`STRICT_SECRETS_ENV`]) → empty is a hard error;
/// * otherwise, if JWT is already configured (non-empty) → empty CEK is a hard
///   error (partial secret setup must not store upstream tokens in cleartext);
/// * both empty → warn only so example/dev configs still boot.
pub fn validate_credential_encryption_key(key: &str, jwt_secret: &str) -> Result<(), StartupError> {
    validate_credential_encryption_key_with(key, jwt_secret, strict_secrets_enabled())
}

/// Testable form of [`validate_credential_encryption_key`].
pub fn validate_credential_encryption_key_with(
    key: &str,
    jwt_secret: &str,
    strict: bool,
) -> Result<(), StartupError> {
    if !key.is_empty() {
        return Ok(());
    }
    if strict || !jwt_secret.is_empty() {
        return Err(StartupError::MissingCredentialEncryptionKey);
    }
    warn!(
        "CREDENTIAL_ENCRYPTION_KEY not set — upstream provider credentials will be stored in cleartext; set a 32-byte key (hex/base64) to encrypt auth_records.metadata at rest"
    );
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

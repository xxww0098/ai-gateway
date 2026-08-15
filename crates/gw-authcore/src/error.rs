//! The crate-wide error type.
//!
//! Every variant carries the exact error message it has always produced, so log
//! lines and panel error bodies stay recognisable.

/// Failure modes of every fallible operation in `gw-authcore`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    /// Raised when minting or validating a token without a configured secret.
    #[error("JWT secret not configured")]
    MissingJwtSecret,

    /// A token failed cryptographic parsing or validation.
    #[error("invalid JWT: {0}")]
    InvalidJwt(#[from] jsonwebtoken::errors::Error),

    /// The requested expiry cannot be represented as a timestamp. We refuse
    /// instead of silently overflowing.
    #[error("invalid JWT expiry: {0} hours")]
    InvalidExpiry(i64),

    /// Any bcrypt failure other than an over-long password.
    #[error("bcrypt: {0}")]
    Bcrypt(#[from] bcrypt::BcryptError),

    /// A password exceeds bcrypt's 72-byte limit.
    #[error("bcrypt: password length exceeds 72 bytes")]
    PasswordTooLong,

    /// The credential-encryption key is neither hex nor base64.
    #[error("credential encryption key must be 32-byte hex or base64")]
    CredentialKeyEncoding,

    /// The credential-encryption key decodes to the wrong length.
    #[error("credential encryption key must be 32 bytes (AES-256); got {0}")]
    CredentialKeyLength(usize),

    /// Encrypted metadata was found but no key is configured.
    #[error("auth metadata is encrypted but CREDENTIAL_ENCRYPTION_KEY is not configured")]
    CredentialKeyMissing,

    /// The encrypted envelope is not valid base64/JSON.
    #[error("decoding encrypted metadata: {0}")]
    CredentialEnvelopeDecode(String),

    /// The encrypted envelope is too short to hold a nonce.
    #[error("encrypted metadata too short")]
    CredentialEnvelopeTooShort,

    /// Decryption failed, usually a wrong key. The AEAD failure itself is
    /// deliberately not surfaced — it only ever says "aead::Error" and would leak
    /// nothing useful.
    #[error("decrypting auth metadata (wrong key?)")]
    CredentialDecrypt,

    /// AES-GCM sealing failed (only possible on a >64GiB plaintext).
    #[error("encrypting auth metadata")]
    CredentialEncrypt,

    /// The OS entropy source failed while generating random bytes.
    #[error("failed to generate random bytes: {0}")]
    Random(String),

    /// A configured provider `base_url` is not an absolute URL.
    #[error("invalid {provider} base_url: {value}")]
    InvalidBaseUrl {
        /// Provider whose configuration is bad.
        provider: &'static str,
        /// The offending `base_url`.
        value: String,
    },

    /// A credential with an empty id was written.
    #[error("auth id is required")]
    MissingAuthId,

    /// A JSON blob in an `auth_records` column could not be encoded/decoded.
    /// `field` is the column name.
    #[error("{field}: {source}")]
    Json {
        /// Column the blob came from (`attributes`, `metadata`, ...).
        field: &'static str,
        /// Underlying serde failure.
        #[source]
        source: serde_json::Error,
    },

    /// Any SQL failure, with the call site's message as context (e.g.
    /// `"listing auth records"`).
    #[error("{context}: {source}")]
    Database {
        /// What the query was doing.
        context: &'static str,
        /// Underlying sqlx failure.
        #[source]
        source: sqlx::Error,
    },
}

impl AuthError {
    /// Wraps a [`sqlx::Error`] with the call site's context string.
    pub(crate) fn db(context: &'static str, source: sqlx::Error) -> Self {
        Self::Database { context, source }
    }

    /// Wraps a [`serde_json::Error`] with the offending column name.
    pub(crate) fn json(field: &'static str, source: serde_json::Error) -> Self {
        Self::Json { field, source }
    }
}

//! Named errors for vendor OAuth. Callers map these onto panel HTTP codes.

use thiserror::Error;

/// Recoverable OAuth failure. Permanent variants mean the stored session
/// must be dropped; the operator has to sign in again.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The family does not implement this login method.
    #[error("unsupported oauth flow")]
    UnsupportedFlow,
    /// PKCE verifier missing when the family requires S256.
    #[error("missing pkce verifier")]
    MissingVerifier,
    /// Operator-supplied import payload cannot be parsed.
    #[error("invalid import: {0}")]
    InvalidImport(String),
    /// Token / device / poll endpoint returned a non-success status.
    #[error("token endpoint status {status}: {body}")]
    TokenEndpoint { status: u16, body: String },
    /// Refresh failed in a way that cannot be retried with this session.
    #[error("oauth login expired; sign in again")]
    Permanent,
    /// Transport or protocol failure talking to the vendor.
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    /// JSON body was not the shape this family documents.
    #[error("malformed oauth payload: {0}")]
    Payload(String),
    /// OS entropy failed while minting PKCE / state.
    #[error("entropy source failed")]
    Entropy,
}

impl Error {
    /// True when the stored session must be deleted.
    #[must_use]
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent)
    }
}

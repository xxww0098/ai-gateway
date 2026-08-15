//! Encryption-at-rest for `auth_records.metadata`.
//!
//! The metadata blob holds upstream secrets (`api_key`, `access_token`,
//! `refresh_token`, Vertex service-account JSON); `attributes` stays cleartext
//! because it only holds `base_url` / proxy settings that operators need to
//! query.
//!
//! The envelope format is frozen — rows written by existing binaries must
//! decrypt here:
//!
//! ```text
//! {"__cpa_enc_v1": base64_std( nonce[12] || AES-256-GCM(metadata_json) )}
//! ```

use crate::error::AuthError;
use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rand::RngCore;
use serde_json::{Map, Value};

#[cfg(test)]
mod tests;

/// The single JSON key wrapping the ciphertext, so an encrypted blob is still
/// valid JSON for the `jsonb` column.
pub const CRED_ENC_ENVELOPE_KEY: &str = "__cpa_enc_v1";

/// Environment variable holding the credential-encryption key.
pub const CREDENTIAL_ENCRYPTION_KEY_ENV: &str = "CREDENTIAL_ENCRYPTION_KEY";

/// AES-256 key length.
const KEY_LEN: usize = 32;

/// AES-GCM standard nonce length.
const NONCE_LEN: usize = 12;

/// Encrypts/decrypts the secret-bearing auth metadata blob at rest.
///
/// A cipher built from an empty key is a **passthrough** — plaintext in,
/// plaintext out — so deployments that never opted in keep working and legacy
/// plaintext rows still load.
#[derive(Clone)]
pub struct CredentialCipher {
    /// `None` → encryption disabled (passthrough).
    aead: Option<Aes256Gcm>,
}

impl std::fmt::Debug for CredentialCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialCipher")
            .field("enabled", &self.enabled())
            .finish()
    }
}

impl CredentialCipher {
    /// Builds a cipher from a 32-byte key given as hex (64 chars) or standard
    /// base64 (44 chars).
    ///
    /// An empty key yields a disabled passthrough cipher; a non-empty but
    /// malformed key is a hard error, so a misconfigured deployment fails fast
    /// instead of silently storing plaintext.
    ///
    /// # Errors
    /// [`AuthError::CredentialKeyEncoding`] when the key is neither hex nor
    /// base64, [`AuthError::CredentialKeyLength`] when it decodes to something
    /// other than 32 bytes.
    pub fn new(key: &str) -> Result<Self, AuthError> {
        let key = key.trim();
        if key.is_empty() {
            return Ok(Self { aead: None });
        }
        let raw = decode_credential_key(key)?;
        if raw.len() != KEY_LEN {
            return Err(AuthError::CredentialKeyLength(raw.len()));
        }
        Ok(Self {
            aead: Some(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&raw))),
        })
    }

    /// Builds a cipher from [`CREDENTIAL_ENCRYPTION_KEY_ENV`], for callers that
    /// have no [`gw_config::Config`] at hand. An unset variable disables
    /// encryption, matching an empty config value.
    ///
    /// # Errors
    /// Same as [`CredentialCipher::new`].
    pub fn from_env() -> Result<Self, AuthError> {
        Self::new(&std::env::var(CREDENTIAL_ENCRYPTION_KEY_ENV).unwrap_or_default())
    }

    /// Whether encryption is active.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.aead.is_some()
    }

    /// Wraps a metadata blob in an AES-GCM envelope.
    ///
    /// Disabled cipher, `null` and `{}` blobs pass through untouched — there is
    /// nothing secret to protect and it keeps the row debuggable.
    ///
    /// # Errors
    /// [`AuthError::Random`] when the OS entropy source fails,
    /// [`AuthError::CredentialEncrypt`] when sealing fails.
    pub fn encrypt(&self, plain: &Value) -> Result<Value, AuthError> {
        let Some(aead) = self.aead.as_ref() else {
            return Ok(plain.clone());
        };
        if is_empty_metadata(plain) {
            return Ok(plain.clone());
        }

        let payload = serde_json::to_vec(plain).map_err(|err| AuthError::json("metadata", err))?;

        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|err| AuthError::Random(err.to_string()))?;

        let sealed = aead
            .encrypt(Nonce::from_slice(&nonce), payload.as_ref())
            .map_err(|_| AuthError::CredentialEncrypt)?;

        // The nonce is prepended so decrypt can recover it; the wire layout is
        // nonce || ciphertext || tag.
        let mut envelope_bytes = Vec::with_capacity(NONCE_LEN + sealed.len());
        envelope_bytes.extend_from_slice(&nonce);
        envelope_bytes.extend_from_slice(&sealed);

        let mut envelope = Map::with_capacity(1);
        envelope.insert(
            CRED_ENC_ENVELOPE_KEY.to_owned(),
            Value::String(BASE64.encode(envelope_bytes)),
        );
        Ok(Value::Object(envelope))
    }

    /// Reverses [`CredentialCipher::encrypt`].
    ///
    /// A blob that is not one of our envelopes (legacy plaintext, or written
    /// while encryption was disabled) is returned unchanged so reads work across
    /// the migration — except that an encrypted blob with no key configured is a
    /// hard error, because handing a still encrypted credential to a provider
    /// would look like a corrupt token.
    ///
    /// # Errors
    /// [`AuthError::CredentialKeyMissing`], [`AuthError::CredentialEnvelopeDecode`],
    /// [`AuthError::CredentialEnvelopeTooShort`], [`AuthError::CredentialDecrypt`].
    pub fn decrypt(&self, stored: &Value) -> Result<Value, AuthError> {
        let Some(encoded) = envelope_payload(stored) else {
            return Ok(stored.clone()); // not our envelope → legacy/plaintext passthrough
        };
        let Some(aead) = self.aead.as_ref() else {
            return Err(AuthError::CredentialKeyMissing);
        };

        let raw = BASE64
            .decode(encoded)
            .map_err(|err| AuthError::CredentialEnvelopeDecode(err.to_string()))?;
        if raw.len() < NONCE_LEN {
            return Err(AuthError::CredentialEnvelopeTooShort);
        }
        let (nonce, body) = raw.split_at(NONCE_LEN);

        let plain = aead
            .decrypt(Nonce::from_slice(nonce), body)
            .map_err(|_| AuthError::CredentialDecrypt)?;

        serde_json::from_slice(&plain).map_err(|err| AuthError::json("metadata", err))
    }
}

/// Decodes a credential key: hex first, then standard base64.
fn decode_credential_key(key: &str) -> Result<Vec<u8>, AuthError> {
    if let Ok(raw) = hex::decode(key) {
        return Ok(raw);
    }
    if let Ok(raw) = BASE64.decode(key) {
        return Ok(raw);
    }
    Err(AuthError::CredentialKeyEncoding)
}

/// Whether a metadata blob holds nothing worth encrypting.
fn is_empty_metadata(plain: &Value) -> bool {
    match plain {
        Value::Null => true,
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

/// The base64 payload when `stored` is exactly an encrypted envelope — a JSON
/// object whose *only* key is [`CRED_ENC_ENVELOPE_KEY`] and whose value is a
/// string.
fn envelope_payload(stored: &Value) -> Option<&str> {
    let map = stored.as_object()?;
    if map.len() != 1 {
        return None;
    }
    map.get(CRED_ENC_ENVELOPE_KEY)?.as_str()
}

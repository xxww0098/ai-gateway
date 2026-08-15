//! Authentication primitives + upstream-credential storage.
//!
//! The crate is split by module, each owning one concern:
//!
//! | module | responsibility |
//! | --- | --- |
//! | [`jwt`] | HS256 panel session tokens (`generate_jwt*` / `validate_jwt`) |
//! | [`apikey`] | API-key hashing, prefixing and minting |
//! | [`password`] | panel password hashing and verification |
//! | [`credcrypto`] | AES-GCM encryption-at-rest for credential metadata |
//! | [`record`] | the upstream-credential record and its persistence contract |
//! | [`store`] | the PostgreSQL-backed `AuthStore` |
//! | [`runtime`] | config-seeded credentials injected at runtime |
//! | [`token_version`] | per-user session epochs for revocation |
//!
//! OWNER: worker `authcore`.

// Rule 5.3 ratchet: this crate has zero `todo!()` / `unimplemented!()`, so it
// keeps them out by name. Naming two specific lints (never `warnings` wholesale)
// means a future rustc/clippy release cannot turn this into a spurious failure.
#![deny(clippy::todo, clippy::unimplemented)]

pub mod apikey;
pub mod credcrypto;
pub mod error;
pub mod jwt;
pub mod password;
pub mod record;
pub mod runtime;
pub mod store;
pub mod token_version;

pub use apikey::{API_KEY_PREFIX, KEY_PREFIX_LEN, api_key_prefix, hash_api_key, new_api_key};
pub use credcrypto::{CRED_ENC_ENVELOPE_KEY, CredentialCipher};
pub use error::AuthError;
pub use jwt::{Claims, generate_jwt, generate_jwt_with_version, validate_jwt};
pub use password::{BCRYPT_COST, hash_password, verify_password};
pub use record::{AuthRecord, AuthStatus, AuthStore};
pub use runtime::{RuntimeAuthStore, build_runtime_upstreams};
pub use store::PostgresAuthStore;
pub use token_version::{TokenVersionStore, token_revoked};

/// User primary key. Aliases [`gw_model::Id`] so a JWT subject, an
/// `api_keys.user_id` and a `user_token_versions.user_id` cannot drift apart
/// (the `bigint` those columns store).
pub type UserId = gw_model::Id;

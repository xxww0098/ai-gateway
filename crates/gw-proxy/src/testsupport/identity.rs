//! Doubles for the credential path: hashing, JWTs and tenant lookups.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use gw_authcore::Claims;
use parking_lot::Mutex;

use crate::ports::{ApiKeyRow, AuthCrypto, Id, SubscriptionQuota, TenantDirectory};

/// Deterministic, reversible stand-ins for the real SHA-256 / HS256 work.
#[derive(Default)]
pub(crate) struct FakeCrypto {
    /// token -> claims, for the JWT branch.
    pub(crate) jwts: Mutex<HashMap<String, Claims>>,
}

impl FakeCrypto {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn with_jwt(self: &Arc<Self>, token: &str, user_id: Id) {
        self.jwts.lock().insert(
            token.to_owned(),
            Claims {
                user_id,
                email: format!("user{user_id}@example.test"),
                token_version: 1,
                exp: 0,
                iat: 0,
                nbf: None,
                iss: None,
                sub: None,
            },
        );
    }
}

impl AuthCrypto for FakeCrypto {
    fn hash_api_key(&self, plaintext: &str) -> String {
        format!("hash::{plaintext}")
    }

    fn sha256_hex(&self, input: &str) -> String {
        format!("digest::{}", input.replace('\0', "|"))
    }

    fn verify_jwt(&self, token: &str) -> Option<Claims> {
        self.jwts.lock().get(token).cloned()
    }
}

#[derive(Default)]
pub(crate) struct FakeDirectory {
    pub(crate) api_keys: Mutex<HashMap<String, ApiKeyRow>>,
    pub(crate) users: Mutex<HashMap<Id, String>>,
    pub(crate) groups: Mutex<HashMap<Id, f64>>,
    pub(crate) entitlements: Mutex<Vec<(Id, Id)>>,
    pub(crate) subscriptions: Mutex<HashMap<Id, SubscriptionQuota>>,
    pub(crate) touched: Mutex<Vec<Id>>,
    pub(crate) user_status_errors: Mutex<bool>,
}

impl FakeDirectory {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Registers an active key for an active user, the common happy path.
    pub(crate) fn with_active_key(self: &Arc<Self>, plaintext_hash: &str, row: ApiKeyRow) {
        self.users.lock().insert(row.user_id, "active".to_owned());
        self.api_keys.lock().insert(plaintext_hash.to_owned(), row);
    }
}

#[async_trait]
impl TenantDirectory for FakeDirectory {
    async fn api_key_by_hash(&self, key_hash: &str) -> anyhow::Result<Option<ApiKeyRow>> {
        Ok(self.api_keys.lock().get(key_hash).cloned())
    }

    async fn group_rate_multiplier(&self, group_id: Id) -> anyhow::Result<Option<f64>> {
        Ok(self.groups.lock().get(&group_id).copied())
    }

    async fn user_status(&self, user_id: Id) -> anyhow::Result<Option<String>> {
        if *self.user_status_errors.lock() {
            anyhow::bail!("db down");
        }
        Ok(self.users.lock().get(&user_id).cloned())
    }

    async fn active_subscription(&self, user_id: Id) -> anyhow::Result<Option<SubscriptionQuota>> {
        Ok(self.subscriptions.lock().get(&user_id).cloned())
    }

    async fn holds_group_entitlement(&self, user_id: Id, group_id: Id) -> anyhow::Result<bool> {
        Ok(self.entitlements.lock().contains(&(user_id, group_id)))
    }

    async fn touch_api_key(&self, api_key_id: Id) {
        self.touched.lock().push(api_key_id);
    }
}

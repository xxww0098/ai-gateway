//! PostgreSQL-backed [`AuthStore`].
//!
//! Column names are whatever the historical build scripts produced for the
//! credential record, so an existing database is read and written unchanged.

use crate::{
    credcrypto::CredentialCipher,
    error::AuthError,
    record::{AuthRecord, AuthStatus, AuthStore},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;

#[cfg(test)]
mod tests;

// The column list is spelled out in each statement rather than concatenated at
// runtime, so the SQL a reader sees is the SQL that runs. `tests::…` checks the
// three statements stay in agreement.

/// Lists rows oldest-first, ordered by creation time then id.
const SELECT_ALL: &str = "SELECT id, provider, prefix, label, status, status_message, disabled, \
     unavailable, proxy_url, attributes, metadata, quota, model_states, last_error, created_at, \
     updated_at, last_refreshed_at, next_refresh_after, next_retry_after \
     FROM auth_records ORDER BY created_at ASC, id ASC";

/// `SELECT` for a single id — the `Get` the SDK store never had.
const SELECT_ONE: &str = "SELECT id, provider, prefix, label, status, status_message, disabled, \
     unavailable, proxy_url, attributes, metadata, quota, model_states, last_error, created_at, \
     updated_at, last_refreshed_at, next_refresh_after, next_retry_after \
     FROM auth_records WHERE id = $1";

/// Upsert on `id`, refreshing every other column on conflict.
const UPSERT: &str = "INSERT INTO auth_records (id, provider, prefix, label, status, \
     status_message, disabled, unavailable, proxy_url, attributes, metadata, quota, model_states, \
     last_error, created_at, updated_at, last_refreshed_at, next_refresh_after, next_retry_after) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19) \
     ON CONFLICT (id) DO UPDATE SET \
     provider = EXCLUDED.provider, prefix = EXCLUDED.prefix, label = EXCLUDED.label, \
     status = EXCLUDED.status, status_message = EXCLUDED.status_message, \
     disabled = EXCLUDED.disabled, unavailable = EXCLUDED.unavailable, \
     proxy_url = EXCLUDED.proxy_url, attributes = EXCLUDED.attributes, \
     metadata = EXCLUDED.metadata, quota = EXCLUDED.quota, \
     model_states = EXCLUDED.model_states, last_error = EXCLUDED.last_error, \
     created_at = EXCLUDED.created_at, updated_at = EXCLUDED.updated_at, \
     last_refreshed_at = EXCLUDED.last_refreshed_at, \
     next_refresh_after = EXCLUDED.next_refresh_after, \
     next_retry_after = EXCLUDED.next_retry_after";

/// Deletes a row by id.
const DELETE_ONE: &str = "DELETE FROM auth_records WHERE id = $1";

/// Credentials persisted in `auth_records`.
#[derive(Debug, Clone)]
pub struct PostgresAuthStore {
    pool: PgPool,
    cipher: CredentialCipher,
}

impl PostgresAuthStore {
    /// Builds a store over `pool`, encrypting `metadata` at rest when
    /// `encryption_key` is non-empty.
    ///
    /// A malformed key is a hard error so a misconfigured deployment cannot
    /// silently fall back to plaintext.
    ///
    /// # Errors
    /// Whatever [`CredentialCipher::new`] rejects.
    pub fn new(pool: PgPool, encryption_key: &str) -> Result<Self, AuthError> {
        Ok(Self {
            pool,
            cipher: CredentialCipher::new(encryption_key)?,
        })
    }

    /// Builds a store with an already-constructed cipher.
    #[must_use]
    pub fn with_cipher(pool: PgPool, cipher: CredentialCipher) -> Self {
        Self { pool, cipher }
    }

    /// Whether credentials are encrypted at rest.
    #[must_use]
    pub fn encryption_enabled(&self) -> bool {
        self.cipher.enabled()
    }

    /// Loads every credential.
    async fn list_records(&self) -> Result<Vec<AuthRecord>, AuthError> {
        let rows: Vec<AuthRow> = sqlx::query_as(SELECT_ALL)
            .fetch_all(&self.pool)
            .await
            .map_err(|err| AuthError::db("listing auth records", err))?;

        rows.into_iter()
            .map(|row| record_from_row(row, &self.cipher))
            .collect()
    }

    /// Loads one credential by id.
    async fn get_record(&self, id: &str) -> Result<Option<AuthRecord>, AuthError> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(None);
        }
        let row: Option<AuthRow> = sqlx::query_as(SELECT_ONE)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| AuthError::db("loading auth record", err))?;

        row.map(|row| record_from_row(row, &self.cipher))
            .transpose()
    }

    /// Inserts or updates a credential.
    async fn save_record(&self, record: &AuthRecord) -> Result<(), AuthError> {
        if record.id.trim().is_empty() {
            return Err(AuthError::MissingAuthId);
        }
        if record.is_runtime_only() {
            return Ok(()); // config-seeded credentials never touch the database
        }

        let row = row_from_record(record, &self.cipher, Utc::now())?;
        sqlx::query(UPSERT)
            .bind(&row.id)
            .bind(&row.provider)
            .bind(&row.prefix)
            .bind(&row.label)
            .bind(&row.status)
            .bind(&row.status_message)
            .bind(row.disabled)
            .bind(row.unavailable)
            .bind(&row.proxy_url)
            .bind(&row.attributes)
            .bind(&row.metadata)
            .bind(&row.quota)
            .bind(&row.model_states)
            .bind(&row.last_error)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.last_refreshed_at)
            .bind(row.next_refresh_after)
            .bind(row.next_retry_after)
            .execute(&self.pool)
            .await
            .map_err(|err| AuthError::db("saving auth record", err))?;
        Ok(())
    }

    /// Removes a credential by id.
    async fn delete_record(&self, id: &str) -> Result<(), AuthError> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(()); // an empty id is a no-op
        }
        sqlx::query(DELETE_ONE)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|err| AuthError::db("deleting auth record", err))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl AuthStore for PostgresAuthStore {
    async fn list(&self) -> anyhow::Result<Vec<AuthRecord>> {
        Ok(self.list_records().await?)
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<AuthRecord>> {
        Ok(self.get_record(id).await?)
    }

    async fn save(&self, record: &AuthRecord) -> anyhow::Result<()> {
        Ok(self.save_record(record).await?)
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        Ok(self.delete_record(id).await?)
    }
}

/// One `auth_records` row as it sits on the wire.
///
/// Everything the existing schema left nullable is read as an [`Option`]: rows
/// written by older binaries really do carry NULLs, and a decode error here would
/// take the whole credential list down.
#[derive(Debug, Clone, sqlx::FromRow)]
struct AuthRow {
    id: String,
    provider: Option<String>,
    prefix: Option<String>,
    label: Option<String>,
    status: Option<String>,
    status_message: Option<String>,
    disabled: Option<bool>,
    unavailable: Option<bool>,
    proxy_url: Option<String>,
    attributes: Option<Value>,
    metadata: Option<Value>,
    quota: Option<Value>,
    model_states: Option<Value>,
    last_error: Option<Value>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    last_refreshed_at: Option<DateTime<Utc>>,
    next_refresh_after: Option<DateTime<Utc>>,
    next_retry_after: Option<DateTime<Utc>>,
}

/// Wraps a wire row into an [`AuthRecord`]: decrypt `metadata`, then decode.
fn record_from_row(row: AuthRow, cipher: &CredentialCipher) -> Result<AuthRecord, AuthError> {
    let metadata = cipher.decrypt(row.metadata.as_ref().unwrap_or(&Value::Null))?;

    Ok(AuthRecord {
        provider: row.provider.unwrap_or_default(),
        prefix: row.prefix.unwrap_or_default(),
        label: row.label.unwrap_or_default(),
        status: AuthStatus::from(row.status.unwrap_or_default().as_str()),
        status_message: row.status_message.unwrap_or_default(),
        disabled: row.disabled.unwrap_or_default(),
        unavailable: row.unavailable.unwrap_or_default(),
        proxy_url: row.proxy_url.unwrap_or_default(),
        attributes: decode_attributes(row.attributes)?,
        metadata: object_or_empty(Some(metadata)),
        quota: object_or_empty(row.quota),
        model_states: object_or_empty(row.model_states),
        last_error: row.last_error.filter(|value| !value.is_null()),
        created_at: row.created_at.unwrap_or(DateTime::UNIX_EPOCH),
        updated_at: row.updated_at.unwrap_or(DateTime::UNIX_EPOCH),
        last_refreshed_at: row.last_refreshed_at,
        next_refresh_after: row.next_refresh_after,
        next_retry_after: row.next_retry_after,
        id: row.id,
    })
}

/// Converts an [`AuthRecord`] to a wire row: encode, then encrypt `metadata`.
///
/// `now` supplies the auto-create/update timestamps: a record that never carried
/// a timestamp is stamped instead of being written as 1970, and `updated_at`
/// always advances on write.
fn row_from_record(
    record: &AuthRecord,
    cipher: &CredentialCipher,
    now: DateTime<Utc>,
) -> Result<AuthRow, AuthError> {
    let attributes = serde_json::to_value(&record.attributes)
        .map_err(|err| AuthError::json("attributes", err))?;
    let metadata = cipher.encrypt(&record.metadata)?;

    Ok(AuthRow {
        id: record.id.clone(),
        provider: Some(record.provider.clone()),
        prefix: Some(record.prefix.clone()),
        label: Some(record.label.clone()),
        status: Some(record.status.as_str().to_owned()),
        status_message: Some(record.status_message.clone()),
        disabled: Some(record.disabled),
        unavailable: Some(record.unavailable),
        proxy_url: Some(record.proxy_url.clone()),
        attributes: Some(object_or_empty(Some(attributes))),
        metadata: Some(object_or_empty(Some(metadata))),
        quota: Some(object_or_empty(Some(record.quota.clone()))),
        model_states: Some(object_or_empty(Some(record.model_states.clone()))),
        last_error: record.last_error.clone().filter(|value| !value.is_null()),
        created_at: Some(stamp(record.created_at, now)),
        updated_at: Some(now),
        last_refreshed_at: record.last_refreshed_at,
        next_refresh_after: record.next_refresh_after,
        next_retry_after: record.next_retry_after,
    })
}

/// Decodes the `attributes` column; a NULL or `null` value leaves the map empty
/// rather than failing the whole list.
fn decode_attributes(raw: Option<Value>) -> Result<HashMap<String, String>, AuthError> {
    match raw {
        None | Some(Value::Null) => Ok(HashMap::new()),
        Some(value) => {
            serde_json::from_value(value).map_err(|err| AuthError::json("attributes", err))
        }
    }
}

/// Normalises a JSON column: a nil/`null` blob is stored as `{}` so it always
/// holds a JSON object.
fn object_or_empty(value: Option<Value>) -> Value {
    match value {
        None | Some(Value::Null) => Value::Object(serde_json::Map::new()),
        Some(value) => value,
    }
}

/// Stamps an unset (UNIX epoch) timestamp with "now".
fn stamp(value: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
    if value == DateTime::UNIX_EPOCH {
        now
    } else {
        value
    }
}

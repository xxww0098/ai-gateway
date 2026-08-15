use super::{
    AuthRow, DELETE_ONE, PostgresAuthStore, SELECT_ALL, SELECT_ONE, UPSERT, record_from_row,
    row_from_record, stamp,
};
use crate::{
    credcrypto::{CRED_ENC_ENVELOPE_KEY, CredentialCipher},
    record::{AuthRecord, AuthStatus, AuthStore, RUNTIME_ONLY_ATTRIBUTE},
};
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{Value, json};

const KEY_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

/// The environment variable an operator sets to run the ignored Postgres tests.
const DB_URL_ENV: &str = "GW_TEST_DATABASE_URL";

fn sample_record() -> AuthRecord {
    let created = DateTime::UNIX_EPOCH + TimeDelta::try_days(19_000).expect("valid delta");
    let mut record = AuthRecord::new("auth-1", "claude", created);
    record.label = "Claude upstream".to_owned();
    record.prefix = "team-a".to_owned();
    record.status = AuthStatus::Error;
    record.status_message = "401 from upstream".to_owned();
    record.proxy_url = "http://127.0.0.1:8080".to_owned();
    record.set_attribute("base_url", "https://api.anthropic.com");
    record.metadata = json!({"api_key": "sk-secret", "token_data": {"access_token": "at"}});
    record.quota = json!({"remaining": 42});
    record.model_states = json!({"claude-3-opus": "ok"});
    record.last_error = Some(json!({"message": "boom"}));
    record.last_refreshed_at = Some(created);
    record
}

/// Column list between the first `(` and `)` of an INSERT, or after `SELECT`.
fn columns_of(sql: &str) -> Vec<&str> {
    let list = if let Some(rest) = sql.strip_prefix("SELECT ") {
        rest.split(" FROM ").next().expect("a FROM clause")
    } else {
        let start = sql.find('(').expect("an INSERT column list") + 1;
        let end = sql.find(')').expect("a closed column list");
        &sql[start..end]
    };
    list.split(',').map(str::trim).collect()
}

#[test]
fn every_statement_agrees_on_the_column_set() {
    let select_all = columns_of(SELECT_ALL);

    assert_eq!(select_all, columns_of(SELECT_ONE));
    assert_eq!(select_all, columns_of(UPSERT));
    assert_eq!(
        UPSERT.matches('$').count(),
        select_all.len(),
        "a bind placeholder per column, or the bind order in save_record is wrong"
    );
    for column in &select_all {
        assert!(
            UPSERT.contains(&format!("{column} = EXCLUDED.{column}")) || *column == "id",
            "{column} is never refreshed on conflict, so an update would silently drop it"
        );
    }
    assert!(DELETE_ONE.contains("WHERE id = $1"));
}

#[test]
fn the_secret_blob_is_the_only_encrypted_column() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");
    let record = sample_record();

    let row = row_from_record(&record, &cipher, Utc::now()).expect("encoding succeeds");

    assert!(
        row.metadata
            .as_ref()
            .and_then(|m| m.get(CRED_ENC_ENVELOPE_KEY))
            .is_some(),
        "metadata must reach the database sealed"
    );
    assert!(
        !row.metadata
            .as_ref()
            .expect("metadata")
            .to_string()
            .contains("sk-secret")
    );
    assert_eq!(
        row.attributes,
        Some(json!({"base_url": "https://api.anthropic.com"})),
        "attributes stay queryable: operators filter on base_url"
    );
    assert_eq!(row.quota, Some(json!({"remaining": 42})));
}

#[test]
fn a_record_survives_the_encrypt_store_decrypt_round_trip() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");
    let record = sample_record();

    let row = row_from_record(&record, &cipher, Utc::now()).expect("encoding succeeds");
    let decoded = record_from_row(row, &cipher).expect("decoding succeeds");

    assert_eq!(decoded.id, record.id);
    assert_eq!(decoded.provider, record.provider);
    assert_eq!(decoded.prefix, record.prefix);
    assert_eq!(decoded.label, record.label);
    assert_eq!(decoded.status, record.status);
    assert_eq!(decoded.status_message, record.status_message);
    assert_eq!(decoded.proxy_url, record.proxy_url);
    assert_eq!(decoded.attributes, record.attributes);
    assert_eq!(decoded.metadata, record.metadata);
    assert_eq!(decoded.quota, record.quota);
    assert_eq!(decoded.model_states, record.model_states);
    assert_eq!(decoded.last_error, record.last_error);
    assert_eq!(decoded.created_at, record.created_at);
    assert_eq!(decoded.last_refreshed_at, record.last_refreshed_at);
}

#[test]
fn a_plaintext_deployment_round_trips_too() {
    let cipher = CredentialCipher::new("").expect("an empty key disables encryption");
    let record = sample_record();

    let row = row_from_record(&record, &cipher, Utc::now()).expect("encoding succeeds");
    assert_eq!(
        row.metadata,
        Some(record.metadata.clone()),
        "without a key the column stays readable, as it was before encryption existed"
    );
    assert_eq!(
        record_from_row(row, &cipher)
            .expect("decoding succeeds")
            .metadata,
        record.metadata
    );
}

#[test]
fn a_legacy_row_full_of_nulls_still_decodes() {
    let cipher = CredentialCipher::new(KEY_HEX).expect("hex key is accepted");
    let row = AuthRow {
        id: "legacy".to_owned(),
        provider: None,
        prefix: None,
        label: None,
        status: None,
        status_message: None,
        disabled: None,
        unavailable: None,
        proxy_url: None,
        attributes: None,
        metadata: None,
        quota: None,
        model_states: None,
        last_error: Some(Value::Null),
        created_at: None,
        updated_at: None,
        last_refreshed_at: None,
        next_refresh_after: None,
        next_retry_after: None,
    };

    let decoded = record_from_row(row, &cipher).expect("a NULL-heavy row must not fail the list");

    assert_eq!(decoded.id, "legacy");
    assert_eq!(
        decoded.status,
        AuthStatus::Active,
        "empty status means active"
    );
    assert!(decoded.attributes.is_empty());
    assert!(decoded.metadata.is_object());
    assert!(decoded.last_error.is_none(), "a JSON null is not an error");
    assert!(!decoded.disabled);
}

#[test]
fn timestamps_follow_auto_stamping() {
    let cipher = CredentialCipher::new("").expect("an empty key disables encryption");
    let now = Utc::now();

    let unstamped = AuthRecord::new("id", "codex", DateTime::UNIX_EPOCH);
    let row = row_from_record(&unstamped, &cipher, now).expect("encoding succeeds");
    assert_eq!(
        row.created_at,
        Some(now),
        "an unset created_at is stamped, not written as 1970"
    );
    assert_eq!(row.updated_at, Some(now));

    let existing = sample_record();
    let row = row_from_record(&existing, &cipher, now).expect("encoding succeeds");
    assert_eq!(
        row.created_at,
        Some(existing.created_at),
        "creation time is never rewritten"
    );
    assert_eq!(row.updated_at, Some(now), "every write advances updated_at");

    assert_eq!(stamp(DateTime::UNIX_EPOCH, now), now);
    assert_eq!(stamp(existing.created_at, now), existing.created_at);
}

// ---------------------------------------------------------------------------
// Postgres-backed tests. They need a real database because the whole point of
// the store is the existing schema; run them with
//   GW_TEST_DATABASE_URL=postgres://user:pw@localhost/cpa cargo test -p gw-authcore -- --ignored
// against a database that has had the gateway migrations applied.
// ---------------------------------------------------------------------------

async fn test_pool() -> sqlx::PgPool {
    let url = std::env::var(DB_URL_ENV).unwrap_or_else(|_| {
        panic!(
            "{DB_URL_ENV} is not set. These tests are --ignored by default; to run them, point \
             {DB_URL_ENV} at a Postgres database with the gateway migrations applied."
        )
    });
    let pool = sqlx::PgPool::connect(&url)
        .await
        .unwrap_or_else(|err| panic!("connecting to {DB_URL_ENV}: {err}"));

    let exists: bool = sqlx::query_scalar("SELECT to_regclass('public.auth_records') IS NOT NULL")
        .fetch_one(&pool)
        .await
        .expect("probing for auth_records");
    assert!(
        exists,
        "auth_records is missing — apply the gateway migrations to {DB_URL_ENV} first"
    );
    pool
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn credentials_round_trip_through_postgres() {
    let store = PostgresAuthStore::new(test_pool().await, KEY_HEX).expect("hex key is accepted");
    let mut record = sample_record();
    record.id = format!(
        "gw-authcore-test-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );

    store.save(&record).await.expect("saving succeeds");

    let loaded = store
        .get(&record.id)
        .await
        .expect("loading succeeds")
        .expect("the record we just saved exists");
    assert_eq!(
        loaded.metadata, record.metadata,
        "metadata decrypts on the way out"
    );
    assert_eq!(loaded.status, record.status);
    assert_eq!(loaded.attributes, record.attributes);

    let stored_column: Value =
        sqlx::query_scalar("SELECT metadata FROM auth_records WHERE id = $1")
            .bind(&record.id)
            .fetch_one(&store.pool)
            .await
            .expect("reading the raw column");
    assert!(
        stored_column.get(CRED_ENC_ENVELOPE_KEY).is_some(),
        "the secret must be sealed at rest, not merely on the way through"
    );

    // Upsert: a second save updates in place instead of failing on the PK.
    let mut updated = loaded;
    updated.label = "renamed".to_owned();
    store.save(&updated).await.expect("upsert succeeds");
    let reloaded = store
        .get(&updated.id)
        .await
        .expect("loading")
        .expect("still there");
    assert_eq!(reloaded.label, "renamed");

    assert!(
        store
            .list()
            .await
            .expect("listing succeeds")
            .iter()
            .any(|r| r.id == updated.id)
    );

    store.delete(&updated.id).await.expect("deleting succeeds");
    assert!(store.get(&updated.id).await.expect("loading").is_none());
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn config_seeded_credentials_are_never_written() {
    let store = PostgresAuthStore::new(test_pool().await, KEY_HEX).expect("hex key is accepted");
    let mut record = sample_record();
    record.id = "cpa-gateway-claude-test".to_owned();
    record.set_attribute(RUNTIME_ONLY_ATTRIBUTE, "true");

    store
        .save(&record)
        .await
        .expect("saving a runtime-only auth is a no-op");

    assert!(
        store
            .get(&record.id)
            .await
            .expect("loading succeeds")
            .is_none(),
        "a runtime-only credential in the database would outlive its config entry"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_empty_id_is_rejected_on_save_and_ignored_on_delete() {
    let store = PostgresAuthStore::new(test_pool().await, KEY_HEX).expect("hex key is accepted");
    let mut record = sample_record();
    record.id = "  ".to_owned();

    assert!(store.save(&record).await.is_err());
    store.delete("").await.expect("deleting nothing is a no-op");
}

//! Shared fixture for the ledger's integration binaries.
//!
//! Each fixture gets a throwaway Postgres schema with `rust/migrations` run
//! into it — the same SQL production runs, which `model-migrations` verified
//! against the historical schema. So these tests assert against the real
//! column types (`numeric` money, nullable everywhere) rather than against a
//! convenient approximation of them.
//!
//! Every test that uses this is `#[ignore]`d, because it needs a live
//! Postgres (and, for `redis_ledger`, a live Redis). Run them with:
//!
//! ```text
//! GW_TEST_DATABASE_URL=postgres://user:pass@127.0.0.1:5432/db \
//! GW_TEST_REDIS_URL=redis://127.0.0.1:6379 \
//!   cargo test -p gw-ledger -- --ignored
//! ```
//!
//! Once `--ignored` is passed, a missing variable is a hard failure with the
//! command above in the message — never a silent skip that would make the
//! coverage a lie.

#![allow(dead_code)] // each integration binary uses its own subset

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use gw_ledger::Ledger;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};

pub(crate) const PG_ENV: &str = "GW_TEST_DATABASE_URL";
pub(crate) const REDIS_ENV: &str = "GW_TEST_REDIS_URL";

const HOW_TO_RUN: &str = "these tests are #[ignore]d; run them with \
     GW_TEST_DATABASE_URL=postgres://user:pass@host:5432/db \
     GW_TEST_REDIS_URL=redis://127.0.0.1:6379 \
     cargo test -p gw-ledger -- --ignored";

pub(crate) fn database_url() -> String {
    std::env::var(PG_ENV).unwrap_or_else(|_| panic!("{PG_ENV} is not set — {HOW_TO_RUN}"))
}

pub(crate) fn redis_url() -> String {
    std::env::var(REDIS_ENV).unwrap_or_else(|_| panic!("{REDIS_ENV} is not set — {HOW_TO_RUN}"))
}

/// Hands out user ids that no other test in this process is using, so the
/// globally-keyed Redis reservations of two tests can never overlap.
pub(crate) fn next_user_id() -> i64 {
    static NEXT: AtomicI64 = AtomicI64::new(900_000);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A throwaway Postgres schema plus (optionally) a Redis handle, wrapping one
/// [`Ledger`] under test.
pub(crate) struct Fixture {
    pub(crate) ledger: Ledger,
    pub(crate) pool: PgPool,
    pub(crate) redis: Option<ConnectionManager>,
    schema: String,
    admin: PgPool,
    users: Vec<i64>,
}

impl Fixture {
    /// A Postgres-only ledger. Holds are unavailable; everything else works.
    pub(crate) async fn postgres_only() -> Self {
        Self::build(None, Duration::from_secs(300)).await
    }

    /// A ledger wired to both Postgres and Redis, with an explicit hold TTL —
    /// the value both Lua scripts use as their expiry cutoff.
    pub(crate) async fn with_redis(hold_ttl: Duration) -> Self {
        let client = redis::Client::open(redis_url()).expect("redis url parses");
        let conn = ConnectionManager::new(client)
            .await
            .expect("connect to the test Redis");
        Self::build(Some(conn), hold_ttl).await
    }

    async fn build(redis: Option<ConnectionManager>, hold_ttl: Duration) -> Self {
        let url = database_url();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect to the test Postgres");

        // One schema per fixture keeps concurrently-running tests from seeing
        // each other's rows without serializing them.
        let schema = format!(
            "gw_ledger_test_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..16]
        );
        admin
            .execute(format!("CREATE SCHEMA {schema}").as_str())
            .await
            .expect("create test schema");

        let search_path = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .after_connect(move |conn, _| {
                let search_path = search_path.clone();
                Box::pin(async move {
                    conn.execute(format!("SET search_path TO {search_path}").as_str())
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .expect("connect the fixture pool");

        // The schema under test is the *real* one: `rust/migrations`, the same
        // SQL production runs. Hand-writing the handful of tables the ledger
        // touches would be a second declaration of the same schema — one that
        // agrees with the migrations today and silently stops agreeing later,
        // at which point these tests would be passing against a shape no
        // deployment has.
        gw_model::run_migrations(&pool)
            .await
            .expect("run rust/migrations into the test schema");
        pool.execute(FAULT_INJECTION_DDL)
            .await
            .expect("install the fault-injection trigger");

        let ledger = Ledger::with_config(
            pool.clone(),
            redis.clone(),
            Duration::from_secs(30),
            hold_ttl,
        );
        Self {
            ledger,
            pool,
            redis,
            schema,
            admin,
            users: Vec::new(),
        }
    }

    /// Creates a user with the given balance and clears any Redis state a
    /// previous run may have left under that id.
    pub(crate) async fn seed_user(&mut self, balance: f64) -> i64 {
        let user_id = next_user_id();
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, role, username, balance, status, \
             concurrency, created_at, updated_at) \
             VALUES ($1, $2, 'hash', 'user', 'test', $3, 'active', 0, NOW(), NOW())",
        )
        .bind(user_id)
        .bind(format!("ledger-{user_id}@test.local"))
        .bind(balance)
        .execute(&self.pool)
        .await
        .expect("seed user");

        self.forget_redis_state(user_id).await;
        self.users.push(user_id);
        user_id
    }

    /// The persistent balance, read the same way the ledger reads it.
    pub(crate) async fn balance(&self, user_id: i64) -> f64 {
        let balance: Option<f64> =
            sqlx::query_scalar("SELECT balance::float8 FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&self.pool)
                .await
                .expect("read balance");
        balance.unwrap_or_default()
    }

    /// Overwrites the balance without journaling it — the tamper
    /// `verify_balance_integrity` is supposed to catch.
    pub(crate) async fn tamper_balance(&self, user_id: i64, balance: f64) {
        sqlx::query("UPDATE users SET balance = $1 WHERE id = $2")
            .bind(balance)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .expect("tamper balance");
    }

    /// Every journal row for one request, as `(type, amount, metadata)`.
    pub(crate) async fn logs_for(
        &self,
        user_id: i64,
        reference: &str,
    ) -> Vec<(String, f64, Value)> {
        sqlx::query_as::<_, (String, Option<f64>, Option<Value>)>(
            "SELECT type, amount::float8, metadata FROM balance_logs \
             WHERE user_id = $1 AND reference = $2 ORDER BY id",
        )
        .bind(user_id)
        .bind(reference)
        .fetch_all(&self.pool)
        .await
        .expect("read balance logs")
        .into_iter()
        .map(|(kind, amount, meta)| {
            (
                kind,
                amount.unwrap_or_default(),
                meta.unwrap_or(Value::Null),
            )
        })
        .collect()
    }

    /// Inserts a settle row carrying a positive `shortfall_usd`, the shape
    /// `settle_tx`'s partial-debit branch produces, and returns its id.
    pub(crate) async fn insert_shortfall_row(
        &self,
        user_id: i64,
        reference: &str,
        shortfall: f64,
    ) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO balance_logs (user_id, amount, type, reference, metadata, created_at) \
             VALUES ($1, 0, 'settle', $2, $3, NOW()) RETURNING id",
        )
        .bind(user_id)
        .bind(reference)
        .bind(serde_json::json!({
            "user_id": user_id,
            "shortfall_usd": shortfall,
            "actual_cost": shortfall,
        }))
        .fetch_one(&self.pool)
        .await
        .expect("insert shortfall row")
    }

    /// Inserts a credit row with an arbitrary reference — used to build the
    /// orphan resolves that must *not* clear a real debt.
    pub(crate) async fn insert_credit_row(&self, user_id: i64, reference: &str) {
        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, metadata, created_at) \
             VALUES ($1, 1, 'credit', $2, NULL, NOW())",
        )
        .bind(user_id)
        .bind(reference)
        .execute(&self.pool)
        .await
        .expect("insert credit row");
    }

    // ------------------------------------------------------------- redis

    pub(crate) fn conn(&self) -> ConnectionManager {
        self.redis.clone().expect("fixture was built with Redis")
    }

    /// Writes a reservation straight into Redis with a chosen creation time,
    /// so a test can stage a hold that is already past its TTL.
    pub(crate) async fn plant_hold(
        &self,
        user_id: i64,
        request_id: &str,
        amount: f64,
        created_at: i64,
    ) {
        let mut conn = self.conn();
        let _: () = conn
            .zadd(gw_ledger::holds_key(user_id), request_id, amount)
            .await
            .expect("plant hold");
        let _: () = conn
            .hset(
                gw_ledger::holds_ts_key(user_id),
                request_id,
                created_at.to_string(),
            )
            .await
            .expect("plant hold timestamp");
    }

    pub(crate) async fn hold_members(&self, user_id: i64) -> Vec<String> {
        let mut conn = self.conn();
        conn.zrange(gw_ledger::holds_key(user_id), 0, -1)
            .await
            .expect("read hold members")
    }

    pub(crate) async fn hold_timestamp(&self, user_id: i64, request_id: &str) -> Option<String> {
        let mut conn = self.conn();
        conn.hget(gw_ledger::holds_ts_key(user_id), request_id)
            .await
            .expect("read hold timestamp")
    }

    pub(crate) async fn balance_cache_exists(&self, user_id: i64) -> bool {
        let mut conn = self.conn();
        conn.exists(gw_ledger::balance_key(user_id))
            .await
            .expect("probe balance cache")
    }

    pub(crate) async fn hold_key_ttl(&self, user_id: i64) -> i64 {
        let mut conn = self.conn();
        conn.ttl(gw_ledger::holds_key(user_id))
            .await
            .expect("read hold key ttl")
    }

    async fn forget_redis_state(&self, user_id: i64) {
        let Some(mut conn) = self.redis.clone() else {
            return;
        };
        let _: () = conn
            .del((
                gw_ledger::balance_key(user_id),
                gw_ledger::holds_key(user_id),
                gw_ledger::holds_ts_key(user_id),
            ))
            .await
            .expect("clear redis state");
    }

    /// Drops the schema and every Redis key this fixture touched. Call it at
    /// the end of a test; a panicking test leaves the schema behind on
    /// purpose, for post-mortem inspection.
    pub(crate) async fn cleanup(self) {
        for user_id in &self.users {
            self.forget_redis_state(*user_id).await;
        }
        self.pool.close().await;
        let _ = self
            .admin
            .execute(format!("DROP SCHEMA {} CASCADE", self.schema).as_str())
            .await;
        self.admin.close().await;
    }
}

/// Fault injection, layered on top of the migrated schema: any journal row
/// whose reference starts with [`FAULT_PREFIX`] fails to insert.
///
/// This is the Postgres-side fault-injection harness for the settle path, and
/// it is the only DDL this fixture writes — it is test apparatus, not schema,
/// so it has no business being in a migration.
const FAULT_INJECTION_DDL: &str = "
    CREATE FUNCTION reject_sentinel_balance_log() RETURNS trigger LANGUAGE plpgsql AS $$
    BEGIN
      IF NEW.reference LIKE 'FAULT-%' THEN
        RAISE EXCEPTION 'injected balance log insert failure';
      END IF;
      RETURN NEW;
    END;
    $$;
    CREATE TRIGGER reject_sentinel BEFORE INSERT ON balance_logs
    FOR EACH ROW EXECUTE FUNCTION reject_sentinel_balance_log();
";

/// The reference prefix [`FAULT_INJECTION_DDL`]'s trigger rejects.
pub(crate) const FAULT_PREFIX: &str = "FAULT-";

/// SplitMix64, so the randomized integration properties are reproducible.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(crate) fn f64_range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.unit() * (hi - lo)
    }

    pub(crate) fn i64_range(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (hi - lo) as u64 + 1;
        lo + (self.next_u64() % span) as i64
    }

    pub(crate) fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

//! The pricing cache against a real `model_prices` table.
//!
//! Pins the two things a DB-free test cannot check: that the column names
//! match the historical schema (`per1_m`, not `per_1m`), and that `numeric` /
//! nullable price columns decode the way the live schema reads them.
//!
//! The table comes from `rust/migrations` — the same SQL production runs.
//! Restating the `model_prices` DDL here would only prove the cache agrees
//! with this file.
//!
//! `#[ignore]`d — run with:
//!
//! ```text
//! GW_TEST_DATABASE_URL=postgres://user:pass@127.0.0.1:5432/db \
//!   cargo test -p gw-pricing -- --ignored
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use gw_pricing::{Calculator, ModelPriceCache, TokenUsage};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};

const PG_ENV: &str = "GW_TEST_DATABASE_URL";
const NEEDS_PG: &str = "requires a local Postgres (set GW_TEST_DATABASE_URL)";

struct Fixture {
    pool: PgPool,
    schema: String,
    admin: PgPool,
}

impl Fixture {
    async fn new() -> Self {
        let url = std::env::var(PG_ENV).unwrap_or_else(|_| {
            panic!("{PG_ENV} is not set — these tests are #[ignore]d; {NEEDS_PG}")
        });
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect to the test Postgres");

        static SEQ: AtomicU32 = AtomicU32::new(0);
        let schema = format!(
            "gw_pricing_test_{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        admin
            .execute(format!("CREATE SCHEMA {schema}").as_str())
            .await
            .expect("create test schema");

        let search_path = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(4)
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
        gw_model::run_migrations(&pool)
            .await
            .expect("run rust/migrations into the test schema");

        Self {
            pool,
            schema,
            admin,
        }
    }

    async fn upsert(&self, model_id: &str, input: Option<f64>, output: Option<f64>) {
        sqlx::query(
            "INSERT INTO model_prices \
             (model_id, input_price_per1_m, output_price_per1_m, \
              cached_input_price_per1_m, reasoning_price_per1_m, created_at, updated_at) \
             VALUES ($1, $2, $3, 0, 0, NOW(), NOW()) \
             ON CONFLICT (model_id) DO UPDATE SET \
               input_price_per1_m = EXCLUDED.input_price_per1_m, \
               output_price_per1_m = EXCLUDED.output_price_per1_m, \
               updated_at = NOW()",
        )
        .bind(model_id)
        .bind(input)
        .bind(output)
        .execute(&self.pool)
        .await
        .expect("upsert price");
    }

    async fn cleanup(self) {
        self.pool.close().await;
        let _ = self
            .admin
            .execute(format!("DROP SCHEMA {} CASCADE", self.schema).as_str())
            .await;
        self.admin.close().await;
    }
}

/// The load path reads the real column names. If `input_price_per1_m` were
/// ever "corrected" to `input_price_per_1m`, this is where it fails — instead
/// of in production, where every model would silently reprice at the default.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn the_cache_reads_the_existing_price_columns() {
    let fx = Fixture::new().await;
    fx.upsert("gpt-4o", Some(2.5), Some(10.0)).await;

    let cache = ModelPriceCache::load(&fx.pool).await.expect("load");
    let row = cache
        .get("gpt-4o")
        .expect("the seeded model must be priced");

    assert_eq!(row.input_price_per_1m, 2.5);
    assert_eq!(row.output_price_per_1m, 10.0);
    assert_eq!(cache.len(), 1);

    fx.cleanup().await;
}

/// The existing schema reads a NULL price column back as `0`. A row with
/// unset prices must therefore be *present and free*, not missing — a
/// missing row would silently fall back to the default price instead.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn null_price_columns_read_as_zero_rather_than_dropping_the_row() {
    let fx = Fixture::new().await;
    fx.upsert("half-priced", None, Some(7.0)).await;

    let cache = ModelPriceCache::load(&fx.pool).await.expect("load");
    let row = cache
        .get("half-priced")
        .expect("the row must survive a NULL price");

    assert_eq!(row.input_price_per_1m, 0.0);
    assert_eq!(row.output_price_per_1m, 7.0);

    fx.cleanup().await;
}

/// The whole point of sharing one cache handle: an admin price edit followed
/// by `invalidate` is visible to a `Calculator` that was built before the
/// edit, with no restart and no second cache.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn an_admin_price_edit_reaches_an_already_built_calculator() {
    let fx = Fixture::new().await;
    fx.upsert("gpt-test", Some(1.0), Some(0.0)).await;

    let cache = Arc::new(ModelPriceCache::load(&fx.pool).await.expect("load"));
    let calc = Calculator::new(Some(Arc::clone(&cache)), 0.0);
    let one_million_input = TokenUsage {
        input: 1_000_000,
        ..TokenUsage::default()
    };

    let before = calc.compute("gpt-test", one_million_input, 1.0).total_cost;
    assert_eq!(before, 1.0);

    fx.upsert("gpt-test", Some(2.0), Some(0.0)).await;
    cache.invalidate(&fx.pool).await.expect("invalidate");

    let after = calc.compute("gpt-test", one_million_input, 1.0).total_cost;
    assert_eq!(after, 2.0, "the calculator kept a stale price");

    fx.cleanup().await;
}

/// A row deleted from the table stops being priced, so a withdrawn model falls
/// back to the default rate instead of billing forever at its old price.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn invalidating_forgets_rows_that_were_deleted() {
    let fx = Fixture::new().await;
    fx.upsert("doomed", Some(1.0), Some(1.0)).await;

    let cache = ModelPriceCache::load(&fx.pool).await.expect("load");
    assert!(cache.get("doomed").is_some());

    sqlx::query("DELETE FROM model_prices WHERE model_id = 'doomed'")
        .execute(&fx.pool)
        .await
        .expect("delete");
    cache.invalidate(&fx.pool).await.expect("invalidate");

    assert!(cache.get("doomed").is_none());
    assert!(cache.is_empty());

    fx.cleanup().await;
}

/// A price changed out of band — by another replica, or straight in the
/// database — converges everywhere within one refresh interval, without a
/// restart.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn a_periodic_refresh_converges_to_an_out_of_band_edit() {
    let fx = Fixture::new().await;
    fx.upsert("gpt-test", Some(1.0), Some(0.0)).await;

    let cache = Arc::new(ModelPriceCache::load(&fx.pool).await.expect("load"));
    assert_eq!(
        cache.get("gpt-test").expect("seeded").input_price_per_1m,
        1.0
    );

    let handle = cache
        .start_refresh(fx.pool.clone(), Duration::from_millis(25))
        .expect("a positive interval spawns a refresher");

    // Simulate another replica editing the price.
    fx.upsert("gpt-test", Some(2.0), Some(0.0)).await;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if cache
            .get("gpt-test")
            .expect("still priced")
            .input_price_per_1m
            == 2.0
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the price cache never converged to the out-of-band edit"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    handle.abort();
    fx.cleanup().await;
}

//! 不变量：改完价，**计费正在读的那个 cache** 立刻是新价。
//!
//! 对应原实现的 admin_pricing_cache 测试（`TestAdminUpsertRefreshesCache`）与
//! admin_pricing_validation 测试的 `TestZeroPriceAccepted`。这是 `billing` 域唯一
//! 一条跨进程的不变量：既有实现专门写了注释解释「各建一个 cache，管理员改价就永远到不了
//! 计费」。校验它必须有真库 —— 单测能覆盖的只有拒绝负价那一半，那部分在
//! `src/billing/prices/tests.rs`。
//!
//! 走的是 [`gw_panel::billing::prices::upsert_price`] 而不是 HTTP handler：
//! handler 还要 Redis 与一整个 `PanelState`，而这里要证的性质在那之下 ——
//! 「写进去的四列能被 `invalidate` 读回来」。

use std::sync::Arc;

use gw_panel::billing::prices::{UpsertModelPriceRequest, upsert_price};
use gw_pricing::{Calculator, ModelPriceCache, TokenUsage};
use sqlx::PgPool;

use crate::common::fresh_db;

/// 判等用的容差。价格在 Postgres 里是 `numeric`，经 `f64` 往返会有末位差异，
/// 与原实现 admin_pricing_cache 测试的 `pricesEqual` 同因。
const EPSILON: f64 = 1e-9;

fn request(model_id: &str, prices: [f64; 4]) -> UpsertModelPriceRequest {
    UpsertModelPriceRequest {
        model_id: model_id.to_owned(),
        input_price_per_1m: prices[0],
        output_price_per_1m: prices[1],
        cached_input_price_per_1m: prices[2],
        reasoning_price_per_1m: prices[3],
    }
}

/// 从共享 cache 里读出四列。缺失返回 `None`。
fn cached(cache: &ModelPriceCache, model_id: &str) -> Option<[f64; 4]> {
    cache.get(model_id).map(|price| {
        [
            price.input_price_per_1m,
            price.output_price_per_1m,
            price.cached_input_price_per_1m,
            price.reasoning_price_per_1m,
        ]
    })
}

fn assert_prices(got: [f64; 4], want: [f64; 4]) {
    for (index, (got, want)) in got.iter().zip(want.iter()).enumerate() {
        assert!(
            (got - want).abs() <= EPSILON,
            "第 {index} 列: {got} != {want}"
        );
    }
}

async fn seeded_cache(pool: &PgPool, model_id: &str, prices: [f64; 4]) -> Arc<ModelPriceCache> {
    upsert_price(pool, model_id, &request(model_id, prices))
        .await
        .expect("seed price");
    let cache = Arc::new(ModelPriceCache::load(pool).await.expect("load cache"));
    assert_prices(
        cached(&cache, model_id).expect("预热后应命中；命中不了说明这个测试本身坏了"),
        prices,
    );
    cache
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_upsert_reaches_the_cache_the_calculator_reads() {
    let pool = fresh_db("pricing_cache_refresh").await;
    const MODEL: &str = "gpt-4o";
    const OLD: [f64; 4] = [1.0, 2.0, 3.0, 4.0];
    const NEW: [f64; 4] = [10.0, 20.0, 30.0, 40.0];

    let cache = seeded_cache(&pool, MODEL, OLD).await;
    // Calculator 拿的是同一个 Arc —— 这正是既有实现那条注释要求的接法。
    let calculator = Calculator::new(Some(Arc::clone(&cache)), 0.0);
    let one_million_input = TokenUsage {
        input: 1_000_000,
        ..TokenUsage::default()
    };
    let before = calculator.compute(MODEL, one_million_input, 1.0).total_cost;

    upsert_price(&pool, MODEL, &request(MODEL, NEW))
        .await
        .expect("upsert");
    cache.invalidate(&pool).await.expect("invalidate");

    assert_prices(cached(&cache, MODEL).expect("失效后应仍命中"), NEW);

    // 真正要证的是「钱变了」，不是「map 里的数变了」。
    let after = calculator.compute(MODEL, one_million_input, 1.0).total_cost;
    assert!(
        after > before,
        "涨价后算出来的钱没变：{before} -> {after}；Calculator 读的不是这个 cache"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_price_can_be_dropped_all_the_way_to_zero() {
    // 对应 `TestZeroPriceAccepted`。「降价到 0」是真实的运营动作，
    // 必须可表达 —— 「零值即未设置」的启发式当年就是在这里咬人的。
    let pool = fresh_db("pricing_cache_zero").await;
    const MODEL: &str = "free-model";

    let cache = seeded_cache(&pool, MODEL, [1.0, 2.0, 3.0, 4.0]).await;
    upsert_price(&pool, MODEL, &request(MODEL, [0.0; 4]))
        .await
        .expect("upsert");
    cache.invalidate(&pool).await.expect("invalidate");

    assert_prices(
        cached(&cache, MODEL).expect("降到 0 的行不该消失"),
        [0.0; 4],
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_new_model_is_inserted_rather_than_silently_dropped() {
    let pool = fresh_db("pricing_cache_insert").await;
    const MODEL: &str = "brand-new-model";
    const PRICES: [f64; 4] = [5.0, 6.0, 7.0, 8.0];

    let cache = Arc::new(ModelPriceCache::load(&pool).await.expect("load cache"));
    assert!(cached(&cache, MODEL).is_none(), "空库不该有这个模型");

    upsert_price(&pool, MODEL, &request(MODEL, PRICES))
        .await
        .expect("upsert");
    cache.invalidate(&pool).await.expect("invalidate");

    assert_prices(cached(&cache, MODEL).expect("插入后应命中"), PRICES);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_upsert_never_creates_a_second_row_for_one_model() {
    // 唯一索引在 `model_id` 上；退化成 INSERT 会让同一个模型有两条价，
    // 之后读到哪一条取决于扫描顺序。
    let pool = fresh_db("pricing_cache_single_row").await;
    const MODEL: &str = "repeated-model";

    for round in 0..3 {
        let base = f64::from(round);
        upsert_price(&pool, MODEL, &request(MODEL, [base, base, base, base]))
            .await
            .expect("upsert");
    }

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_prices WHERE model_id = $1")
        .bind(MODEL)
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(rows, 1);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn one_models_price_change_leaves_the_others_alone() {
    let pool = fresh_db("pricing_cache_isolation").await;
    const TARGET: &str = "changed-model";
    const NEIGHBOUR: &str = "untouched-model";
    const NEIGHBOUR_PRICES: [f64; 4] = [9.0, 9.0, 9.0, 9.0];

    upsert_price(&pool, NEIGHBOUR, &request(NEIGHBOUR, NEIGHBOUR_PRICES))
        .await
        .expect("seed neighbour");
    let cache = seeded_cache(&pool, TARGET, [1.0, 1.0, 1.0, 1.0]).await;

    upsert_price(&pool, TARGET, &request(TARGET, [2.0, 2.0, 2.0, 2.0]))
        .await
        .expect("upsert");
    cache.invalidate(&pool).await.expect("invalidate");

    assert_prices(
        cached(&cache, NEIGHBOUR).expect("邻居不该被顺手删掉"),
        NEIGHBOUR_PRICES,
    );
}

use crate::migrate::MIGRATOR;
use crate::testsupport::fresh_db;

use super::*;

/// `ON CONFLICT (model_id) DO NOTHING` 只处理「表里已有」的冲突；同一批里出现两次
/// 相同 model_id 会在同一条语句内撞车（PG 报 "cannot affect row a second time"）。
#[test]
fn model_price_seeds_have_unique_ids() {
    let mut ids: Vec<&str> = MODEL_PRICE_SEEDS.iter().map(|s| s.0).collect();
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), total, "种子价目表里有重复的 model_id");
}

/// 价格是 USD / 1M tokens，负数会让计费出现负成本 —— 直接把结算方向搞反。
#[test]
fn model_price_seeds_are_non_negative() {
    for (id, input, output, cached, reasoning) in MODEL_PRICE_SEEDS {
        for (label, v) in [
            ("input", input),
            ("output", output),
            ("cached", cached),
            ("reasoning", reasoning),
        ] {
            assert!(*v >= 0.0, "{id} 的 {label} 价格为负");
            assert!(v.is_finite(), "{id} 的 {label} 价格不是有限值");
        }
    }
}

/// 缓存命中价必须不高于同一模型的普通输入价，否则「命中缓存反而更贵」，
/// 计费方向就是错的。
#[test]
fn cached_input_never_costs_more_than_input() {
    for (id, input, _output, cached, _reasoning) in MODEL_PRICE_SEEDS {
        assert!(cached <= input, "{id} 的缓存输入价高于输入价");
    }
}

/// 三个套餐按 `group_id` 定位（UPDATE/COUNT/INSERT 都 `WHERE group_id = ?`），
/// 重复的 group_id 会让后一条把前一条覆盖掉。
#[test]
fn subscription_seeds_target_distinct_groups() {
    let mut groups: Vec<i64> = SUBSCRIPTION_SEEDS.iter().map(|s| s.0).collect();
    let total = groups.len();
    groups.sort_unstable();
    groups.dedup();
    assert_eq!(groups.len(), total, "订阅种子里有重复的 group_id");
}

/// 越贵的套餐倍率越低、限额越高 —— 这是这三档产品的定价意图，写反了没人会发现，
/// 直到用户按错的倍率被扣钱。
#[test]
fn subscription_seeds_are_monotonic_by_tier() {
    let tiers: Vec<_> = SUBSCRIPTION_SEEDS.iter().collect();
    for pair in tiers.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        assert!(hi.6 > lo.6, "{} 的价格没有高于 {}", hi.1, lo.1);
        assert!(hi.5 > lo.5, "{} 的月限额没有高于 {}", hi.1, lo.1);
        assert!(hi.3 <= lo.3, "{} 的倍率没有低于/等于 {}", hi.1, lo.1);
        assert!(hi.4 > 0 && lo.4 > 0, "有效期必须是正数天");
    }
}

/// `provider_configs.config_data` 的 JSON 形状必须稳定：
/// 键名是 snake_case，provider 块只有 `base_url` / `enabled` 两个键。
/// 前端和 SDK 管理接口都按这个形状读，键名改了就读不到。
#[test]
fn sdk_seed_config_serializes_with_snake_case_keys() {
    let cfg = SdkSeedConfig {
        base_url: "https://example.test".into(),
        timeout_seconds: 30,
        providers: [(
            "openai".to_owned(),
            SdkSeedProvider {
                base_url: "https://api.openai.test".into(),
                enabled: true,
            },
        )]
        .into_iter()
        .collect(),
    };

    let v = serde_json::to_value(&cfg).expect("可序列化");
    assert!(v.get("base_url").is_some());
    assert!(v.get("timeout_seconds").is_some());
    let openai = v
        .pointer("/providers/openai")
        .expect("providers 是按 provider 名索引的对象");
    assert_eq!(
        openai.as_object().map(|o| o.len()),
        Some(2),
        "provider 块只应有 base_url 和 enabled"
    );
    assert!(openai.get("base_url").is_some());
    assert!(openai.get("enabled").is_some());
}

/// serde 往返：写进 `config_data` 的东西必须能原样读回来。
#[test]
fn sdk_seed_config_round_trips() {
    let cfg = SdkSeedConfig {
        base_url: "https://gw.test".into(),
        timeout_seconds: 7,
        providers: [
            "openai",
            "openai_compatible",
            "claude",
            "gemini",
            "codex",
            "vertex",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            (
                name.to_owned(),
                SdkSeedProvider {
                    base_url: format!("https://{name}.test"),
                    enabled: i % 2 == 0,
                },
            )
        })
        .collect(),
    };

    let json = serde_json::to_string(&cfg).expect("可序列化");
    let back: SdkSeedConfig = serde_json::from_str(&json).expect("可反序列化");
    assert_eq!(back.base_url, cfg.base_url);
    assert_eq!(back.timeout_seconds, cfg.timeout_seconds);
    assert_eq!(back.providers.len(), cfg.providers.len());
    for (name, p) in &cfg.providers {
        let got = back.providers.get(name).expect("provider 丢了");
        assert_eq!((&got.base_url, got.enabled), (&p.base_url, p.enabled));
    }
}

/// 四个种子函数都会在**每次进程启动**时跑一遍，所以幂等是硬要求：跑两遍以后
/// 行数必须完全一样，第二遍必须一行都不新建。
#[sqlx::test]
#[ignore = "需要本地 Postgres（见 testsupport::fresh_db 的用法说明）"]
async fn seeds_are_idempotent() {
    let pool = fresh_db("seeds").await;
    MIGRATOR.run(&pool).await.expect("迁移");

    let cfg = SdkSeedConfig {
        base_url: "https://gw.test".into(),
        timeout_seconds: 30,
        providers: BTreeMap::new(),
    };

    let first_prices = seed_model_prices(&pool).await.expect("首次预置价目");
    let first_packages = ensure_subscription_seeds(&pool)
        .await
        .expect("首次预置套餐");
    let first_sdk = ensure_sdk_management_seeds(&pool, &cfg)
        .await
        .expect("首次预置 SDK");
    assert_eq!(first_prices as usize, MODEL_PRICE_SEEDS.len());
    assert_eq!(first_packages as usize, SUBSCRIPTION_SEEDS.len());
    assert_eq!(
        (
            first_sdk.sdk_config_created,
            first_sdk.ampcode_config_created
        ),
        (true, true)
    );

    let second_prices = seed_model_prices(&pool).await.expect("再次预置价目");
    let second_packages = ensure_subscription_seeds(&pool)
        .await
        .expect("再次预置套餐");
    let second_sdk = ensure_sdk_management_seeds(&pool, &cfg)
        .await
        .expect("再次预置 SDK");
    assert_eq!(second_prices, 0, "已存在的价目不该被重复插入");
    assert_eq!(second_packages, 0, "已存在的套餐不该被重复插入");
    assert_eq!(
        (
            second_sdk.sdk_config_created,
            second_sdk.ampcode_config_created
        ),
        (false, false)
    );

    for (table, expected) in [
        ("model_prices", MODEL_PRICE_SEEDS.len() as i64),
        ("subscription_packages", SUBSCRIPTION_SEEDS.len() as i64),
        ("provider_configs", 1),
        ("ampcode_configs", 1),
    ] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .expect("计数");
        assert_eq!(n, expected, "{table} 的行数不对");
    }
}

/// 已有价目不被覆盖 —— 运维手工维护的生产价目不能被启动种子抹掉
/// （`ON CONFLICT DO NOTHING` 的全部意义）。
#[sqlx::test]
#[ignore = "需要本地 Postgres（见 testsupport::fresh_db 的用法说明）"]
async fn existing_model_prices_are_never_overwritten() {
    let pool = fresh_db("price_overwrite").await;
    MIGRATOR.run(&pool).await.expect("迁移");

    let (seeded_id, _, _, _, _) = MODEL_PRICE_SEEDS[0];
    let operator_price = 123.456_f64;
    sqlx::query(
        "INSERT INTO model_prices (model_id, input_price_per1_m, created_at, updated_at)
         VALUES ($1, $2, now(), now())",
    )
    .bind(seeded_id)
    .bind(operator_price)
    .execute(&pool)
    .await
    .expect("先写一条运维价目");

    seed_model_prices(&pool).await.expect("预置价目");

    let kept: f64 = sqlx::query_scalar(
        "SELECT input_price_per1_m::float8 FROM model_prices WHERE model_id = $1",
    )
    .bind(seeded_id)
    .fetch_one(&pool)
    .await
    .expect("读回价目");
    assert_eq!(kept, operator_price, "种子覆盖了运维已经维护过的价目");
}

/// 引导管理员的四个分支：没配 → 用户不存在 → 提权 → 已有管理员后永久失效。
#[sqlx::test]
#[ignore = "需要本地 Postgres（见 testsupport::fresh_db 的用法说明）"]
async fn bootstrap_admin_only_fires_when_no_admin_exists() {
    let pool = fresh_db("bootstrap_admin").await;
    MIGRATOR.run(&pool).await.expect("迁移");

    assert_eq!(
        ensure_bootstrap_admin(&pool, "   ").await.expect("空邮箱"),
        BootstrapAdmin::NotConfigured
    );
    assert_eq!(
        ensure_bootstrap_admin(&pool, "boss@example.test")
            .await
            .expect("用户不存在"),
        BootstrapAdmin::UserNotFound
    );

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash, role, username, balance, status, concurrency,
                            created_at, updated_at)
         VALUES ($1, 'x', 'user', '', 0, 'active', 1, now(), now()) RETURNING id",
    )
    .bind("boss@example.test")
    .fetch_one(&pool)
    .await
    .expect("建用户");

    assert_eq!(
        ensure_bootstrap_admin(&pool, " BOSS@example.test ")
            .await
            .expect("提权"),
        BootstrapAdmin::Promoted { user_id },
        "邮箱应当先 trim + 小写再匹配"
    );
    assert_eq!(
        ensure_bootstrap_admin(&pool, "boss@example.test")
            .await
            .expect("再来一次"),
        BootstrapAdmin::AlreadyAdministered,
        "已经有活跃管理员之后这条路径必须永久失效"
    );
}

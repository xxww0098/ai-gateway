//! 启动种子 —— 四个入口在启动时按固定顺序调用：
//!
//! * [`seed_model_prices`]：占位价目表。
//! * [`ensure_subscription_seeds`]：默认订阅套餐。
//! * [`ensure_sdk_management_seeds`]：SDK 管理相关的初始记录。
//! * [`ensure_bootstrap_admin`]：服务端管理员引导。
//!
//! 四个函数都必须是**幂等**的：每次进程启动都会跑一遍。
//!
//! 这里不打日志（`gw-model` 不依赖 `tracing`）—— 每个函数把「实际发生了什么」
//! 作为返回值交出去，由 `gw-server` 决定怎么记。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// 占位价目表，12 条。
///
/// 单位是 USD / 1M tokens，字段顺序：input / output / cached input / reasoning。
///
/// ⚠️ 这些是**占位价**，生产环境要按各家官方定价对齐后再启用；写入是
/// `ON CONFLICT DO NOTHING`，已存在的 `model_id` 不会被覆盖，运维手工维护的价目安全。
const MODEL_PRICE_SEEDS: &[(&str, f64, f64, f64, f64)] = &[
    // OpenAI
    ("gpt-4o", 2.50, 10.00, 1.25, 0.0),
    ("gpt-4o-mini", 0.15, 0.60, 0.075, 0.0),
    ("o3", 10.00, 40.00, 2.50, 60.00),
    ("o3-mini", 1.10, 4.40, 0.55, 4.40),
    ("o4-mini", 1.10, 4.40, 0.55, 4.40),
    // Anthropic Claude
    ("claude-sonnet-4-20250514", 3.00, 15.00, 0.30, 0.0),
    ("claude-opus-4-20250514", 15.00, 75.00, 1.50, 0.0),
    ("claude-haiku-3-5-20241022", 0.80, 4.00, 0.08, 0.0),
    // Google Gemini
    ("gemini-2.5-pro", 1.25, 10.00, 0.3125, 0.0),
    ("gemini-2.5-flash", 0.15, 0.60, 0.0375, 0.35),
    // Codex
    ("codex-mini", 1.50, 6.00, 0.375, 0.0),
    // Vertex AI
    ("vertex-gemini-2.5-pro", 1.25, 10.00, 0.3125, 0.0),
];

/// 默认订阅套餐，三条。
/// 字段顺序：group_id / name / description / rate_multiplier / validity_days /
/// monthly_limit_usd / price_usd。
const SUBSCRIPTION_SEEDS: &[(i64, &str, &str, f64, i64, f64, f64)] = &[
    (1, "Basic", "适合个人和轻量开发", 1.0, 30, 20.0, 9.9),
    (2, "Pro", "适合中等负载与团队协作", 0.95, 30, 100.0, 29.9),
    (
        3,
        "Enterprise",
        "适合高负载与企业场景",
        0.9,
        30,
        500.0,
        99.9,
    ),
];

/// 幂等预置常用模型的占位价格，返回**实际插入**的行数（已存在的不覆盖）。
///
/// 用一条批量 INSERT + `ON CONFLICT (model_id) DO NOTHING` 实现。
pub async fn seed_model_prices(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let mut ids: Vec<String> = Vec::with_capacity(MODEL_PRICE_SEEDS.len());
    let mut input: Vec<f64> = Vec::with_capacity(MODEL_PRICE_SEEDS.len());
    let mut output: Vec<f64> = Vec::with_capacity(MODEL_PRICE_SEEDS.len());
    let mut cached: Vec<f64> = Vec::with_capacity(MODEL_PRICE_SEEDS.len());
    let mut reasoning: Vec<f64> = Vec::with_capacity(MODEL_PRICE_SEEDS.len());
    for (id, i, o, c, r) in MODEL_PRICE_SEEDS {
        ids.push((*id).to_owned());
        input.push(*i);
        output.push(*o);
        cached.push(*c);
        reasoning.push(*r);
    }

    let done = sqlx::query(
        r#"
        INSERT INTO model_prices (
            model_id, input_price_per1_m, output_price_per1_m,
            cached_input_price_per1_m, reasoning_price_per1_m, created_at, updated_at
        )
        SELECT m, i, o, c, r, now(), now()
        FROM UNNEST($1::text[], $2::float8[], $3::float8[], $4::float8[], $5::float8[])
             AS t(m, i, o, c, r)
        ON CONFLICT (model_id) DO NOTHING
        "#,
    )
    .bind(&ids)
    .bind(&input)
    .bind(&output)
    .bind(&cached)
    .bind(&reasoning)
    .execute(pool)
    .await?;

    Ok(done.rows_affected())
}

/// 预置默认订阅套餐，返回**新建**的套餐数。
///
/// 先对同 `group_id` 的行做 UPDATE（把改过的默认值刷回去），再数一次；
/// 只有一条都没有时才 INSERT。注意 UPDATE 不覆盖 `daily_limit_usd` /
/// `weekly_limit_usd` —— updates 集合里就没有它们，运维手工设的日/周限额
/// 不会被启动种子抹掉。
pub async fn ensure_subscription_seeds(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let mut created = 0u64;
    for (group_id, name, description, rate_multiplier, validity_days, monthly_limit, price) in
        SUBSCRIPTION_SEEDS
    {
        sqlx::query(
            r#"
            UPDATE subscription_packages
               SET name = $1,
                   description = $2,
                   rate_multiplier = $3,
                   default_validity_days = $4,
                   monthly_limit_usd = $5,
                   subscription_price_usd = $6,
                   enabled = true,
                   updated_at = now()
             WHERE group_id = $7
            "#,
        )
        .bind(name)
        .bind(description)
        .bind(rate_multiplier)
        .bind(validity_days)
        .bind(monthly_limit)
        .bind(price)
        .bind(group_id)
        .execute(pool)
        .await?;

        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM subscription_packages WHERE group_id = $1")
                .bind(group_id)
                .fetch_one(pool)
                .await?;
        if count > 0 {
            continue;
        }

        sqlx::query(
            r#"
            INSERT INTO subscription_packages (
                name, description, group_id, rate_multiplier, default_validity_days,
                daily_limit_usd, weekly_limit_usd, monthly_limit_usd,
                subscription_price_usd, enabled, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, NULL, NULL, $6, $7, true, now(), now())
            "#,
        )
        .bind(name)
        .bind(description)
        .bind(group_id)
        .bind(rate_multiplier)
        .bind(validity_days)
        .bind(monthly_limit)
        .bind(price)
        .execute(pool)
        .await?;
        created += 1;
    }
    Ok(created)
}

/// `provider_configs` 里那条 `sdk_config` 的 JSON 形状（key 为 snake_case）。
///
/// `gw-model` 不依赖 `gw-config`（包依赖方向：`gw-config` 在下游），所以这里定义
/// 一个纯输入结构，由 `gw-server` 从 `gw_config::SdkConfig` 填。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SdkSeedConfig {
    pub base_url: String,
    pub timeout_seconds: i64,
    /// key 是 provider 名，历史写死的 6 个：`openai`、`openai_compatible`、
    /// `claude`、`gemini`、`codex`、`vertex`。用 `BTreeMap` 是为了 key 有序
    /// （这样序列化 map 时 key 保持不变）。
    pub providers: BTreeMap<String, SdkSeedProvider>,
}

/// [`SdkSeedConfig::providers`] 的值。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SdkSeedProvider {
    pub base_url: String,
    pub enabled: bool,
}

/// [`ensure_sdk_management_seeds`] 实际做了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SdkSeedOutcome {
    /// 新建了 `provider_configs` 里的 `sdk_config` 行。
    pub sdk_config_created: bool,
    /// 新建了默认的 `ampcode_configs` 行（id = 1，内容 `{}`）。
    pub ampcode_config_created: bool,
}

/// 建立 SDK 管理相关的初始记录。
///
/// **不种 `auth_records`**：配置里带的上游凭证（OpenAI 兼容 / Claude / Gemini /
/// Codex / Vertex）是 runtime-only 注册，落库反而会被下次启动的注册覆盖。
/// 通过管理接口添加的凭证在创建时就已经落库，也不需要启动种子。
pub async fn ensure_sdk_management_seeds(
    pool: &PgPool,
    cfg: &SdkSeedConfig,
) -> Result<SdkSeedOutcome, sqlx::Error> {
    let mut outcome = SdkSeedOutcome::default();

    let provider_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM provider_configs WHERE provider = $1")
            .bind("sdk_config")
            .fetch_one(pool)
            .await?;
    if provider_count == 0 {
        let data = serde_json::to_value(cfg).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        sqlx::query(
            r#"
            INSERT INTO provider_configs (provider, config_data, created_at, updated_at)
            VALUES ($1, $2, now(), now())
            "#,
        )
        .bind("sdk_config")
        .bind(&data)
        .execute(pool)
        .await?;
        outcome.sdk_config_created = true;
    }

    let ampcode_count: i64 = sqlx::query_scalar("SELECT count(*) FROM ampcode_configs")
        .fetch_one(pool)
        .await?;
    if ampcode_count == 0 {
        sqlx::query(
            r#"
            INSERT INTO ampcode_configs (id, config_data, created_at, updated_at)
            VALUES (1, '{}'::jsonb, now(), now())
            "#,
        )
        .execute(pool)
        .await?;
        outcome.ampcode_config_created = true;
    }

    Ok(outcome)
}

/// [`ensure_bootstrap_admin`] 的结果。每一个分支都对应一次 early return。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapAdmin {
    /// 没配 `auth.bootstrap_admin_email`，整个机制关闭。
    NotConfigured,
    /// 已经有活跃管理员 —— 这条路径从此永久失效，不可能再提权。
    AlreadyAdministered,
    /// 配了邮箱但该用户还没注册；注册后由注册流程完成提权。
    UserNotFound,
    /// 该用户本来就是管理员。
    AlreadyAdmin { user_id: i64 },
    /// 刚刚把该用户提成了管理员。
    Promoted { user_id: i64 },
}

/// 一次性的服务端管理员引导。
///
/// 安全性完全依赖两个同时成立的前提：(a) 邮箱来自服务端配置，**永远不取请求输入**；
/// (b) 当前一个活跃管理员都没有。只要系统里已经有管理员，这条路径就永久失效，
/// 不可能用来提权一个已经在运营的系统。
pub async fn ensure_bootstrap_admin(
    pool: &PgPool,
    email: &str,
) -> Result<BootstrapAdmin, sqlx::Error> {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return Ok(BootstrapAdmin::NotConfigured);
    }

    let admin_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE role = $1 AND status = $2")
            .bind("admin")
            .bind("active")
            .fetch_one(pool)
            .await?;
    if admin_count > 0 {
        return Ok(BootstrapAdmin::AlreadyAdministered);
    }

    // 这里是等值匹配（`WHERE email = ?`），不是大小写不敏感匹配：
    // 用大写邮箱注册的账号不会被这条路径命中。
    let found: Option<(i64, Option<String>)> =
        sqlx::query_as("SELECT id, role FROM users WHERE email = $1 ORDER BY id LIMIT 1")
            .bind(&email)
            .fetch_optional(pool)
            .await?;
    let Some((user_id, role)) = found else {
        return Ok(BootstrapAdmin::UserNotFound);
    };
    if role
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("admin")
    {
        return Ok(BootstrapAdmin::AlreadyAdmin { user_id });
    }

    sqlx::query("UPDATE users SET role = $1, updated_at = now() WHERE id = $2")
        .bind("admin")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(BootstrapAdmin::Promoted { user_id })
}

#[cfg(test)]
mod tests;

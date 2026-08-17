//! 实体的 serde 往返。
//!
//! 每个实体一条：JSON → 结构体 → JSON 必须是恒等映射。这同时钉住了两件事 ——
//! 字段名（打错一个 serde 就少一个键，往返立刻不等）和可空性（指针字段必须
//! 能接住 `null`）。
//!
//! 注意这里测的是**内部**表示：历史 handler 从不直接把实体序列化给前端，
//! 而是逐个字段抄进带 json tag 的 DTO。所以这些键名不是对前端的契约，
//! 面板路由该按自己的 DTO 另建响应结构体。

use super::*;

macro_rules! round_trip {
    ($name:ident, $ty:ty, {$($json:tt)*}) => {
        #[test]
        fn $name() {
            let raw = serde_json::json!({$($json)*});
            let parsed: $ty =
                serde_json::from_value(raw.clone()).expect("这份 JSON 应当能反序列化");
            let again = serde_json::to_value(&parsed).expect("应当能序列化回去");
            assert_eq!(
                raw,
                again,
                concat!(stringify!($ty), " 的 serde 往返不是恒等映射")
            );
        }
    };
}

round_trip!(user_round_trip, User, {
    "id": 7,
    "email": "a@b.test",
    "password_hash": "$2a$10$hash",
    "role": "user",
    "username": "",
    "balance": 12.5,
    "status": "active",
    "concurrency": 1,
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(api_key_round_trip, ApiKey, {
    "id": 1,
    "user_id": 7,
    "key_hash": "deadbeef",
    "key_prefix": "agw",
    "name": "default",
    "status": "active",
    "group_id": null,
    "last_used_at": null,
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(group_round_trip, Group, {
    "id": 2,
    "name": "pro",
    "rate_multiplier": 0.95,
    "quota_limit": 0.0,
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(user_token_version_round_trip, UserTokenVersion, {
    "user_id": 7,
    "version": 3,
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(balance_log_round_trip, BalanceLog, {
    "id": 11,
    "user_id": 7,
    "amount": -0.0042,
    "type": "settle",
    "reference": "shortfall_resolve:req-1:42",
    "metadata": {"shortfall_usd": 0.5},
    "created_at": "2026-01-02T03:04:05Z"
});

round_trip!(operation_log_round_trip, OperationLog, {
    "id": 12,
    "source": "panel",
    "actor_id": 0,
    "actor_email": "",
    "actor_role": "",
    "action": "auth.login",
    "target": "user:42",
    "method": "POST",
    "path": "/api/panel/auth/login",
    "status_code": 401,
    "ip_address": "127.0.0.1",
    "request_id": "req-1",
    "metadata": null,
    "created_at": "2026-01-02T03:04:05Z",
    "entry_hash": ""
});

round_trip!(usage_log_round_trip, UsageLog, {
    "id": 13,
    "user_id": 7,
    "api_key_id": 1,
    "group_id": 2,
    "request_id": "req-1",
    "idempotency_key": "",
    "event_key": "",
    "model": "gpt-4o",
    "provider": "openai",
    "auth_id": "auth-1",
    "tokens_in": 10,
    "tokens_out": 20,
    "input_tokens": 10,
    "output_tokens": 20,
    "reasoning_tokens": 0,
    "cached_tokens": 5,
    "input_cost": 0.000025,
    "output_cost": 0.0002,
    "total_cost": 0.000225,
    "actual_cost": 0.000225,
    "cost": 0.000225,
    "rate_multiplier": 1.0,
    "stream": true,
    "duration_ms": 1234,
    "ip_address": "127.0.0.1",
    "raw_metadata": {"billing_fallback": {"reason": "missing_upstream_usage"}},
    "failed": false,
    "created_at": "2026-01-02T03:04:05Z"
});

round_trip!(model_price_round_trip, ModelPrice, {
    "id": 3,
    "model_id": "gpt-4o",
    "input_price_per_1m": 2.5,
    "output_price_per_1m": 10.0,
    "cached_input_price_per_1m": 1.25,
    "reasoning_price_per_1m": 0.0,
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(model_catalog_entry_round_trip, ModelCatalogEntry, {
    "id": 4,
    "channel_key": "openai",
    "model_id": "gpt-4o",
    "visible": true,
    "models_url": "",
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(channel_policy_round_trip, ChannelPolicy, {
    "id": 5,
    "auth_id": "auth-1",
    "weight": 1,
    "priority": 0,
    "enabled": true,
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(subscription_package_round_trip, SubscriptionPackage, {
    "id": 1,
    "name": "Basic",
    "description": "适合个人和轻量开发",
    "group_id": 1,
    "rate_multiplier": 1.0,
    "default_validity_days": 30,
    "daily_limit_usd": null,
    "weekly_limit_usd": null,
    "monthly_limit_usd": 20.0,
    "subscription_price_usd": 9.9,
    "enabled": true,
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(subscription_round_trip, Subscription, {
    "id": 6,
    "user_id": 7,
    "package_id": 1,
    "group_id": 1,
    "group_name": "basic",
    "status": "active",
    "starts_at": "2026-01-02T03:04:05Z",
    "expires_at": "2026-02-01T03:04:05Z",
    "daily_usage_usd": 0.0,
    "daily_reset_at": "2026-01-03T00:00:00Z",
    "weekly_usage_usd": 1.5,
    "weekly_reset_at": "2026-01-05T00:00:00Z",
    "monthly_usage_usd": 3.25,
    "monthly_reset_at": "2026-02-01T00:00:00Z",
    "daily_limit_usd": null,
    "weekly_limit_usd": null,
    "monthly_limit_usd": 20.0,
    "funding_source": "balance",
    "funding_reference": "subscription_purchase:1",
    "price_paid_usd": 9.9,
    "notes": "",
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(ticket_round_trip, Ticket, {
    "id": 8,
    "user_id": 7,
    "title": "无法登录",
    "category": "other",
    "priority": "medium",
    "status": "open",
    "assignee_id": null,
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(ticket_reply_round_trip, TicketReply, {
    "id": 9,
    "ticket_id": 8,
    "user_id": 7,
    "is_admin": false,
    "content": "已收到",
    "created_at": "2026-01-02T03:04:05Z"
});

round_trip!(announcement_round_trip, Announcement, {
    "id": 10,
    "title": "维护通知",
    "content": "今晚 02:00 维护",
    "type": "info",
    "is_active": true,
    "created_at": "2026-01-02T03:04:05Z"
});

round_trip!(payment_order_round_trip, PaymentOrder, {
    "id": 14,
    "user_id": 7,
    "provider": "stripe",
    "amount_usd": 20.0,
    "amount_local": 0.0,
    "currency": "USD",
    "status": "pending",
    "transaction_id": null,
    "metadata": null,
    "paid_at": null,
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(redeem_code_round_trip, RedeemCode, {
    "id": 15,
    "code": "AGW-XXXX",
    "amount": 10.0,
    "status": "unused",
    "used_by_id": null,
    "used_by": null,
    "used_at": null,
    "created_at": "2026-01-02T03:04:05Z"
});

round_trip!(refund_round_trip, Refund, {
    "id": 16,
    "user_id": 7,
    "subscription_id": 6,
    "amount": 5.0,
    "reason": "不再需要",
    "status": "pending",
    "days_used": 3,
    "total_days": 30,
    "daily_rate": 0.33,
    "processed_at": null,
    "processed_by": null,
    "created_at": "2026-01-02T03:04:05Z"
});

round_trip!(auth_record_round_trip, AuthRecord, {
    "id": "auth-1",
    "provider": "claude",
    "prefix": "sk-ant",
    "label": "主账号",
    "status": "active",
    "status_message": "",
    "disabled": false,
    "unavailable": false,
    "proxy_url": "",
    "attributes": {"email": "a@b.test"},
    "metadata": null,
    "quota": null,
    "model_states": null,
    "last_error": null,
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z",
    "last_refreshed_at": "0001-01-01T00:00:00Z",
    "next_refresh_after": "0001-01-01T00:00:00Z",
    "next_retry_after": "0001-01-01T00:00:00Z"
});

round_trip!(provider_config_round_trip, ProviderConfig, {
    "id": 17,
    "provider": "sdk_config",
    "config_data": {"base_url": "", "timeout_seconds": 30, "providers": {}},
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

round_trip!(oauth_session_round_trip, OAuthSession, {
    "id": 18,
    "provider": "gemini",
    "state": "state-1",
    "auth_url": "https://accounts.example.test/o/oauth2",
    "status": "pending",
    "auth_id": null,
    "config_data": null,
    "created_at": "2026-01-02T03:04:05Z",
    "expires_at": "2026-01-02T03:14:05Z"
});

round_trip!(ampcode_config_round_trip, AmpcodeConfig, {
    "id": 1,
    "config_data": {},
    "created_at": "2026-01-02T03:04:05Z",
    "updated_at": "2026-01-02T03:04:05Z"
});

/// `BalanceLog` / `Announcement` 的 kind 字段落在名叫 `type` 的列上，
/// Rust 字段只能改名。这条钉住「改名了但对外仍然是 type」这件事 ——
/// 少了 rename，往返测试会过（键变成 kind），但 SQL 和既有数据就对不上了。
#[test]
fn type_column_keeps_its_name_in_json() {
    let log: BalanceLog = serde_json::from_value(serde_json::json!({
        "id": 1, "user_id": 1, "amount": 1.0, "type": "precharge",
        "reference": "", "metadata": null, "created_at": "2026-01-02T03:04:05Z"
    }))
    .expect("反序列化");
    assert_eq!(log.kind, "precharge");
    let v = serde_json::to_value(&log).expect("序列化");
    assert!(v.get("type").is_some(), "序列化后必须仍然是 type 键");
    assert!(v.get("kind").is_none());
}

/// 缺行即默认值：没有 `channel_policies` 行的上游账号必须拿到「权重 1 / 优先级 0 /
/// 启用」，而不是被当成禁用。
#[test]
fn channel_policy_default_is_enabled_with_weight_one() {
    let p = ChannelPolicy::default_for("auth-x");
    assert_eq!(p.auth_id, "auth-x");
    assert_eq!((p.weight, p.priority, p.enabled), (1, 0, true));
}

// ── 连库：实体 ↔ 列名映射 ────────────────────────────────────────────────────

/// 上面的 serde 往返钉的是 JSON 键名，钉不住**列名** —— 列名藏在
/// `#[sqlx(rename = …)]` 和字段名里，写错了只有在真库上 `SELECT *` 才会露出来。
///
/// 这个测试对每张表塞两行：一行**每列都有值**，一行**只填 NOT NULL 的列**
/// （其余留 NULL，模拟老库里 `ALTER TABLE ADD COLUMN` 之后没回填的行），
/// 然后把整张表解进对应的实体。任何一个列名对不上、或者哪个 NULL 没被
/// `compat::*` 适配器接住，这里都会红。
///
/// 行的内容是从 `information_schema` 现推的，不是手写的 INSERT —— 表结构长出新列
/// 时这个测试会自动覆盖到，不需要有人记得来补。
#[sqlx::test]
#[ignore = "需要本地 Postgres（见 testsupport::fresh_db 的用法说明）"]
async fn every_entity_decodes_from_its_table() {
    use crate::testsupport::fresh_db;

    let pool = fresh_db("entity_decode").await;
    MIGRATOR.run(&pool).await.expect("迁移");

    macro_rules! check {
        ($($table:literal => $ty:ty),* $(,)?) => {$(
            insert_probe_rows(&pool, $table).await;
            let rows = sqlx::query_as::<_, $ty>(concat!("SELECT * FROM \"", $table, "\""))
                .fetch_all(&pool)
                .await
                .unwrap_or_else(|e| panic!(
                    concat!("把 ", $table, " 解进 ", stringify!($ty), " 失败: {}"), e));
            assert!(rows.len() >= 2, concat!($table, " 应当至少有两行探针"));
        )*};
    }

    check! {
        "users" => User,
        "api_keys" => ApiKey,
        "groups" => Group,
        "user_token_versions" => UserTokenVersion,
        "balance_logs" => BalanceLog,
        "operation_logs" => OperationLog,
        "usage_logs" => UsageLog,
        "model_prices" => ModelPrice,
        "model_catalog_entries" => ModelCatalogEntry,
        "channel_policies" => ChannelPolicy,
        "subscription_packages" => SubscriptionPackage,
        "subscriptions" => Subscription,
        "tickets" => Ticket,
        "ticket_replies" => TicketReply,
        "announcements" => Announcement,
        "payment_orders" => PaymentOrder,
        "redeem_codes" => RedeemCode,
        "refunds" => Refund,
        "auth_records" => AuthRecord,
        "provider_configs" => ProviderConfig,
        "o_auth_sessions" => OAuthSession,
        "ampcode_configs" => AmpcodeConfig,
    }
}

/// 给一张表塞两行探针：全列填满的一行 + 只填 NOT NULL 列的一行。
#[cfg(test)]
async fn insert_probe_rows(pool: &sqlx::PgPool, table: &str) {
    let columns: Vec<(String, String, String, bool)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text, is_nullable::text,
                coalesce(column_default, '') LIKE 'nextval%'
           FROM information_schema.columns
          WHERE table_schema = 'public' AND table_name = $1
          ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|e| panic!("读 {table} 的列定义失败: {e}"));
    assert!(!columns.is_empty(), "{table} 在库里不存在");

    for (tag, only_required) in [("x", false), ("y", true)] {
        let mut names = Vec::new();
        let mut values = Vec::new();
        for (name, data_type, nullable, is_serial) in &columns {
            if *is_serial || (only_required && nullable == "YES") {
                continue;
            }
            names.push(format!("\"{name}\""));
            values.push(probe_literal(data_type, tag, table, name));
        }
        let stmt = if names.is_empty() {
            format!("INSERT INTO \"{table}\" DEFAULT VALUES")
        } else {
            format!(
                "INSERT INTO \"{table}\" ({}) VALUES ({})",
                names.join(", "),
                values.join(", ")
            )
        };
        sqlx::raw_sql(&stmt)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("{stmt} 失败: {e}"));
    }
}

/// 按列类型给一个 SQL 字面量。遇到没见过的类型直接 panic —— schema 长出新类型时
/// 要有人来决定实体侧怎么映射，静默跳过等于假覆盖。
#[cfg(test)]
fn probe_literal(data_type: &str, tag: &str, table: &str, column: &str) -> String {
    match data_type {
        "text" | "character varying" => format!("'{tag}'"),
        "bigint" | "integer" | "smallint" => "1".to_owned(),
        "numeric" | "double precision" | "real" => "1.5".to_owned(),
        "boolean" => "true".to_owned(),
        "timestamp with time zone" | "timestamp without time zone" => "now()".to_owned(),
        "jsonb" | "json" => "'{}'::jsonb".to_owned(),
        other => panic!("{table}.{column} 是没见过的列类型 {other}，实体侧要先决定怎么映射"),
    }
}

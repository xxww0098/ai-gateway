//! Per-upstream-account consumption, aggregated from `usage_logs`.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::PgPool;

/// Rolling windows the console paints for one credential.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct AccountUsage {
    pub requests_today: i64,
    pub failed_today: i64,
    pub tokens_in_today: i64,
    pub tokens_out_today: i64,
    pub cost_today: f64,
    pub requests_7d: i64,
    pub failed_7d: i64,
    pub tokens_in_7d: i64,
    pub tokens_out_7d: i64,
    pub cost_7d: f64,
}

/// Loads 7-day + today totals keyed by `auth_id`. Empty input is a no-op so a
/// credential list of zero never hits Postgres.
pub async fn load(
    pool: &PgPool,
    auth_ids: &[String],
) -> anyhow::Result<HashMap<String, AccountUsage>> {
    if auth_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<UsageRow> = sqlx::query_as(
        "SELECT auth_id, \
                COUNT(*) FILTER (WHERE created_at >= date_trunc('day', NOW()) AND NOT COALESCE(failed, false)), \
                COUNT(*) FILTER (WHERE created_at >= date_trunc('day', NOW()) AND COALESCE(failed, false)), \
                COALESCE(SUM(tokens_in) FILTER (WHERE created_at >= date_trunc('day', NOW())), 0), \
                COALESCE(SUM(tokens_out) FILTER (WHERE created_at >= date_trunc('day', NOW())), 0), \
                COALESCE(SUM(total_cost) FILTER (WHERE created_at >= date_trunc('day', NOW())), 0)::float8, \
                COUNT(*) FILTER (WHERE NOT COALESCE(failed, false)), \
                COUNT(*) FILTER (WHERE COALESCE(failed, false)), \
                COALESCE(SUM(tokens_in), 0), \
                COALESCE(SUM(tokens_out), 0), \
                COALESCE(SUM(total_cost), 0)::float8 \
         FROM usage_logs \
         WHERE auth_id = ANY($1) AND created_at >= NOW() - INTERVAL '7 days' \
         GROUP BY auth_id",
    )
    .bind(auth_ids)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.auth_id,
                AccountUsage {
                    requests_today: row.requests_today,
                    failed_today: row.failed_today,
                    tokens_in_today: row.tokens_in_today,
                    tokens_out_today: row.tokens_out_today,
                    cost_today: row.cost_today,
                    requests_7d: row.requests_7d,
                    failed_7d: row.failed_7d,
                    tokens_in_7d: row.tokens_in_7d,
                    tokens_out_7d: row.tokens_out_7d,
                    cost_7d: row.cost_7d,
                },
            )
        })
        .collect())
}

/// `channel_policies.max_concurrent` for the given auth ids. Missing rows are
/// absent from the map (callers treat that as 0 = unlimited).
pub async fn load_max_concurrent(
    pool: &PgPool,
    auth_ids: &[String],
) -> anyhow::Result<HashMap<String, i64>> {
    if auth_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT auth_id, COALESCE(max_concurrent, 0) FROM channel_policies WHERE auth_id = ANY($1)",
    )
    .bind(auth_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Upserts only the concurrency cap, leaving weight/priority/enabled alone on
/// conflict. A missing row is created with the default routing policy.
pub async fn upsert_max_concurrent(
    pool: &PgPool,
    auth_id: &str,
    max_concurrent: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO channel_policies \
             (auth_id, weight, priority, enabled, max_concurrent, created_at, updated_at) \
         VALUES ($1, 1, 0, TRUE, $2, NOW(), NOW()) \
         ON CONFLICT (auth_id) DO UPDATE SET \
             max_concurrent = EXCLUDED.max_concurrent, \
             updated_at = NOW()",
    )
    .bind(auth_id)
    .bind(max_concurrent.max(0))
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct UsageRow {
    #[sqlx(try_from = "gw_model::compat::Text")]
    auth_id: String,
    #[sqlx(try_from = "gw_model::compat::Int")]
    requests_today: i64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    failed_today: i64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    tokens_in_today: i64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    tokens_out_today: i64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    cost_today: f64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    requests_7d: i64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    failed_7d: i64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    tokens_in_7d: i64,
    #[sqlx(try_from = "gw_model::compat::Int")]
    tokens_out_7d: i64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    cost_7d: f64,
}

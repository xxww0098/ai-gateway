//! `/logs`, `/request-error-logs`, `/model-definitions/{channel}` — the
//! upstream console's read-only panes.
//!
//! 对应 SDK 管理面的日志与模型目录 handler：
//! `SDKMgmtLogsHandler`、`SDKMgmtLogsDeleteHandler`、
//! `SDKMgmtRequestErrorLogsHandler`、`SDKMgmtRequestErrorLogsDeleteHandler`、
//! `SDKMgmtModelDefinitionsHandler` 与 `sdkMgmtStaticModels`。
//!
//! # These are `usage_logs`, not a log file
//!
//! The SDK used to tail its own rotating log files here. The gateway serves the
//! `usage_logs` table instead, which is why the two `DELETE` routes exist but
//! do nothing: the console has a "clear logs" button, and a 404 would look like
//! a broken build while silently succeeding would look like data loss. Both
//! answer 200 with a message saying so.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

const ERR_QUERY_FAILED: i32 = 5006;
const ERR_UNKNOWN_CHANNEL: i32 = 4040;

/// Default and ceiling for `?limit=`（默认 50，上限 200）。
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

/// Query string of the two log routes.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct LogsQuery {
    pub limit: Option<String>,
    pub level: Option<String>,
}

/// Resolves `?limit=`：解析为整数，须 `> 0`，再夹紧到上限 200。
///
/// Note this is **not** the panel's usual `queryInt` clamp: a non-positive
/// value falls back to the default rather than being clamped up to 1.
#[must_use]
pub fn limit_of(raw: Option<&str>) -> i64 {
    raw.and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .map_or(DEFAULT_LIMIT, |value| value.min(MAX_LIMIT))
}

/// Translates `?level=` into the `failed` predicate（error → true，其余 → 不过滤）。
#[must_use]
pub fn level_filter(level: Option<&str>) -> Option<bool> {
    match level {
        Some("error") => Some(true),
        Some("info") => Some(false),
        _ => None,
    }
}

#[derive(Debug, sqlx::FromRow)]
struct LogRow {
    id: i64,
    request_id: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    total_cost: Option<f64>,
    duration_ms: Option<i64>,
    ip_address: Option<String>,
    failed: Option<bool>,
    created_at: DateTime<Utc>,
}

impl LogRow {
    fn to_json(&self) -> Value {
        let failed = self.failed.unwrap_or_default();
        json!({
            "id": self.id,
            "request_id": self.request_id.clone().unwrap_or_default(),
            "model": self.model.clone().unwrap_or_default(),
            "provider": self.provider.clone().unwrap_or_default(),
            "tokens_in": self.tokens_in.unwrap_or_default(),
            "tokens_out": self.tokens_out.unwrap_or_default(),
            "total_cost": self.total_cost.unwrap_or_default(),
            "duration_ms": self.duration_ms.unwrap_or_default(),
            "failed": failed,
            // The console colours rows on `level`, which is derived rather than
            // stored — there is no severity column on a usage row.
            "level": if failed { "error" } else { "info" },
            "ip_address": self.ip_address.clone().unwrap_or_default(),
            "created_at": self.created_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        })
    }
}

const LOG_COLUMNS: &str = "id, request_id, model, provider, tokens_in, tokens_out, \
     total_cost::float8 AS total_cost, duration_ms, ip_address, failed, created_at";

/// `GET /logs`。对应 `SDKMgmtLogsHandler`。
pub async fn list(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<LogsQuery>,
) -> Response {
    let rows: Result<Vec<LogRow>, _> = sqlx::query_as(&format!(
        "SELECT {LOG_COLUMNS} FROM usage_logs \
         WHERE ($1::bool IS NULL OR failed = $1) ORDER BY created_at DESC LIMIT $2"
    ))
    .bind(level_filter(query.level.as_deref()))
    .bind(limit_of(query.limit.as_deref()))
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(json!({"logs": rows.iter().map(LogRow::to_json).collect::<Vec<_>>()})),
        Err(error) => {
            tracing::error!(%error, "failed to query usage logs");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_QUERY_FAILED,
                "failed to query usage logs",
            )
        }
    }
}

/// `DELETE /logs`. 对应 `SDKMgmtLogsDeleteHandler` —— 有意地 no-op。
pub async fn clear(_admin: AdminUser) -> Response {
    ok(json!({"message": "logs clear not supported on UsageLog-backed endpoint"}))
}

/// `GET /request-error-logs`. 对应 `SDKMgmtRequestErrorLogsHandler`。
///
/// Same rows as [`list`] with `?level=error`, but the `failed` filter is not
/// optional here — this pane is defined as the failures.
pub async fn list_errors(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<LogsQuery>,
) -> Response {
    let rows: Result<Vec<LogRow>, _> = sqlx::query_as(&format!(
        "SELECT {LOG_COLUMNS} FROM usage_logs WHERE failed = true \
         ORDER BY created_at DESC LIMIT $1"
    ))
    .bind(limit_of(query.limit.as_deref()))
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(json!({"logs": rows.iter().map(LogRow::to_json).collect::<Vec<_>>()})),
        Err(error) => {
            tracing::error!(%error, "failed to query error logs");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_QUERY_FAILED,
                "failed to query error logs",
            )
        }
    }
}

/// `DELETE /request-error-logs`. 对应 `SDKMgmtRequestErrorLogsDeleteHandler`。
pub async fn clear_errors(_admin: AdminUser) -> Response {
    ok(json!({"message": "error logs clear not supported on UsageLog-backed endpoint"}))
}

// ---------------------------------------------------------------- catalog

/// `GET /model-definitions/{channel}`. 对应 `SDKMgmtModelDefinitionsHandler`。
///
/// The catalog table wins when it has rows for the channel; otherwise a
/// built-in list stands in so a freshly installed gateway still shows something
/// pickable. An unknown channel with no rows is a 404, which is how the console
/// knows the channel key was wrong rather than merely empty.
pub async fn model_definitions(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(channel): Path<String>,
) -> Response {
    let rows: Vec<(String, Option<bool>, Option<String>)> = sqlx::query_as(
        "SELECT model_id, visible, models_url FROM model_catalog_entries \
         WHERE channel_key = $1 ORDER BY model_id ASC",
    )
    .bind(&channel)
    .fetch_all(&state.pg)
    .await
    .unwrap_or_default();

    if !rows.is_empty() {
        let models: Vec<Value> = rows
            .into_iter()
            .map(|(model_id, visible, models_url)| {
                json!({
                    "id": model_id,
                    "model": model_id,
                    "provider": channel,
                    "name": model_id,
                    "visible": visible.unwrap_or_default(),
                    "models_url": models_url.unwrap_or_default(),
                })
            })
            .collect();
        return ok(json!({"models": models}));
    }

    let Some(fallback) = static_models(&channel) else {
        return err(
            StatusCode::NOT_FOUND,
            ERR_UNKNOWN_CHANNEL,
            format!("unknown channel: {channel}"),
        );
    };
    let models: Vec<Value> = fallback
        .iter()
        .map(|model_id| {
            json!({
                "id": model_id,
                "model": model_id,
                "provider": channel,
                "name": model_id,
            })
        })
        .collect();
    ok(json!({"models": models}))
}

/// The built-in model list for a channel, or `None` when the channel is not one
/// of the five（对应 `SDKMgmtModelDefinitionsHandler` 里的 channel 分发）。
///
/// These are a *seed list for the picker*, not a capability claim: whether a
/// given credential can actually serve one of them is what
/// `/auth-files/models` answers.
#[must_use]
pub fn static_models(channel: &str) -> Option<&'static [&'static str]> {
    Some(match channel {
        "openai" => &[
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4-turbo",
            "gpt-4",
            "gpt-3.5-turbo",
            "o1",
            "o1-mini",
            "o3-mini",
        ],
        "claude" => &[
            "claude-sonnet-4-20250514",
            "claude-sonnet-4",
            "claude-3-opus-latest",
            "claude-3-sonnet-latest",
            "claude-3-haiku-latest",
            "claude-3-5-sonnet-latest",
            "claude-3-5-haiku-latest",
        ],
        "gemini" => &[
            "gemini-2.5-pro-exp-03-25",
            "gemini-2.0-flash",
            "gemini-2.0-flash-lite",
            "gemini-1.5-pro",
            "gemini-1.5-flash",
        ],
        "codex" => &["o1", "o1-mini", "o3-mini", "gpt-4o", "gpt-4o-mini"],
        "vertex" => &[
            "claude-sonnet-4-20250514",
            "claude-3-opus-latest",
            "claude-3-sonnet-latest",
            "claude-3-haiku-latest",
            "gemini-2.0-flash",
            "gemini-1.5-pro",
        ],
        _ => return None,
    })
}

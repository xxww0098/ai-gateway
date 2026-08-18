//! `/admin/audit-logs` — the unified operations feed, plus the offline
//! verification of the tamper-evident chain.
//!
//! Corresponds to `AdminListAuditLogsHandler` and the three `*LogToEntry` mappers
//! plus `VerifyAuditLog`。
//! *Writing* an operation log is cross-cutting and lives in [`crate::audit`];
//! only reading and verifying are `ops` concerns.
//!
//! # Three tables, one feed
//!
//! | `?source=` | table | what it records |
//! | --- | --- | --- |
//! | `panel` | `operation_logs` | admin/panel actions |
//! | `sdk` | `usage_logs` | `/v1/*` proxy calls |
//! | `balance` | `balance_logs` | Hold / Settle / Release / Credit / Debit |
//! | `all` (default) | all three | merged, newest first |
//!
//! 旧实现从每张表 over-fetch `page * page_size` 行（有上限）并在内存中合并
//! in memory rather than writing a `UNION ALL` across heterogeneous shapes.
//! That is kept: the three row types have almost nothing in common, and the
//! console only ever asks for the most recent N.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgPool;

use crate::audit::{OperationEntry, SOURCE_PANEL, entry_hash};
use crate::paging::query_int;
use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

/// 对应 `opSourceSDK`。
pub const SOURCE_SDK: &str = "sdk";
/// 对应 `opSourceBalance`。
pub const SOURCE_BALANCE: &str = "balance";

/// 对应 `apiErrorInternal`。
const ERR_INTERNAL: i32 = 5000;

/// Ceiling on the per-table over-fetch. 对应既有实现的 `if fetchLimit > 1000 { fetchLimit = 1000 }`。
const MAX_FETCH: i64 = 1000;

// ---------------------------------------------------------------- verification

/// Row shape used only for verification.
///
/// `metadata` is read as `::text` rather than as a parsed value on purpose: the
/// hash covers the *bytes* Postgres stores, and re-serialising a parsed value
/// would reorder keys and invalidate every row written by the original binary.
#[derive(Debug, sqlx::FromRow)]
struct AuditRow {
    id: i64,
    source: Option<String>,
    actor_id: Option<i64>,
    actor_email: Option<String>,
    actor_role: Option<String>,
    action: Option<String>,
    target: Option<String>,
    method: Option<String>,
    path: Option<String>,
    status_code: Option<i64>,
    ip_address: Option<String>,
    request_id: Option<String>,
    metadata: Option<String>,
    created_at: DateTime<Utc>,
    entry_hash: Option<String>,
}

impl AuditRow {
    fn to_entry(&self) -> OperationEntry {
        OperationEntry {
            source: self.source.clone().unwrap_or_default(),
            actor_id: self.actor_id.unwrap_or_default(),
            actor_email: self.actor_email.clone().unwrap_or_default(),
            actor_role: self.actor_role.clone().unwrap_or_default(),
            action: self.action.clone().unwrap_or_default(),
            target: self.target.clone().unwrap_or_default(),
            method: self.method.clone().unwrap_or_default(),
            path: self.path.clone().unwrap_or_default(),
            status_code: self.status_code.unwrap_or_default(),
            ip_address: self.ip_address.clone().unwrap_or_default(),
            request_id: self.request_id.clone().unwrap_or_default(),
            // A NULL column is 旧实现里 `[]byte(nil)`，其 `string(...)` 为 ""。
            metadata: self.metadata.clone().unwrap_or_default().into_bytes(),
            created_at: self.created_at,
        }
    }
}

const SELECT_FOR_VERIFY: &str = "SELECT id, source, actor_id, actor_email, actor_role, action, \
     target, method, path, status_code, ip_address, request_id, metadata::text AS metadata, \
     created_at, entry_hash FROM operation_logs ORDER BY id ASC";

/// Recomputes every stored HMAC and returns the ids that no longer match.
///
/// 对应 `VerifyAuditLog`。Rows with an empty `entry_hash` — written before
/// the feature landed, or with hashing disabled — are skipped rather than
/// reported. This is an operator tool, not a request path: it reads the whole
/// table.
///
/// # Errors
/// When `key` is empty (verification without a key would report "nothing
/// tampered", which is worse than refusing) or when the table cannot be read.
pub async fn verify_audit_log(pool: &PgPool, key: &[u8]) -> anyhow::Result<Vec<i64>> {
    anyhow::ensure!(!key.is_empty(), "audit key is required to verify");

    let rows: Vec<AuditRow> = sqlx::query_as(SELECT_FOR_VERIFY).fetch_all(pool).await?;
    Ok(rows
        .iter()
        .filter(|row| {
            row.entry_hash
                .as_deref()
                .is_some_and(|hash| !hash.is_empty())
        })
        .filter(|row| {
            let want = entry_hash(Some(key), &row.to_entry());
            // Constant-time, mirroring the original `hmac.Equal`.
            !constant_time_eq(
                want.as_bytes(),
                row.entry_hash.as_deref().unwrap_or("").as_bytes(),
            )
        })
        .map(|row| row.id)
        .collect())
}

/// Length-independent constant-time comparison. 对应 `hmac.Equal`。
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

// ---------------------------------------------------------------- the feed

/// One row of the merged feed. 对应 `auditLogEntry` **including its
/// `omitempty` tags** — an absent optional key and an empty-string key are
/// different things to the console.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditLogEntry {
    /// `"<source>-<row id>"`, unique across the three tables.
    pub id: String,
    pub source: String,
    pub actor_id: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub actor_email: String,
    pub action: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(skip_serializing_if = "is_zero_status")]
    pub status_code: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ip_address: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub request_id: String,
    /// 旧实现在这张 `map` 上打 `omitempty` 标记，会同时丢弃 NULL **与** 空对象
    /// object — `{}` and "absent" are the same thing to the console.
    #[serde(skip_serializing_if = "is_empty_map")]
    pub metadata: Option<Map<String, Value>>,
    pub created_at: DateTime<Utc>,
}

/// 旧实现的 `omitempty` 作用于 `map` 时，会丢弃 nil *与* 长度为 0 的值。
fn is_empty_map(metadata: &Option<Map<String, Value>>) -> bool {
    metadata.as_ref().is_none_or(Map::is_empty)
}

/// 旧实现的 `omitempty` 作用于 `int` 时会丢弃零值。The balance feed leaves
/// `status_code` unset, and the console must not read that as "HTTP 0".
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde predicate signature"
)]
fn is_zero_status(status: &i64) -> bool {
    *status == 0
}

/// Query string of `GET /admin/audit-logs`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct AuditLogsQuery {
    pub page: Option<String>,
    pub page_size: Option<String>,
    pub source: Option<String>,
    pub action: Option<String>,
    pub user_id: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub q: Option<String>,
}

/// Whether a source is selected. 对应旧实现的 `want` 闭包 —— 空或 `all`
/// selector means every source.
#[must_use]
pub fn source_selected(selector: &str, source: &str) -> bool {
    selector.is_empty() || selector == "all" || selector == source
}

fn trimmed(raw: Option<&String>) -> String {
    raw.map(|value| value.trim().to_owned()).unwrap_or_default()
}

/// `GET /admin/audit-logs`. 对应 `AdminListAuditLogsHandler`。
pub async fn list_audit_logs(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<AuditLogsQuery>,
) -> Response {
    let (page, page_size) = audit_page_params(query.page.as_deref(), query.page_size.as_deref());
    let selector = trimmed(query.source.as_ref()).to_lowercase();
    let action = trimmed(query.action.as_ref());
    let start_date = trimmed(query.start_date.as_ref());
    let end_date = trimmed(query.end_date.as_ref());
    let search = trimmed(query.q.as_ref());

    // 旧实现用 ParseUint 解析，失败时留下 0，于是 0 意味着「无过滤」
    // filter" — a malformed user_id widens the query rather than rejecting.
    let user_id: i64 = query
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|value| i64::try_from(value).ok())
        .unwrap_or_default();

    let fetch_limit = (page * page_size).min(MAX_FETCH);
    let action = Option::from(action).filter(|value: &String| !value.is_empty());
    let start_date = Option::from(start_date).filter(|value: &String| !value.is_empty());
    // 旧实现追加 " 23:59:59"，使 end_date 为 2026-08-15 时包含那一整天
    // day; a bare date would compare against midnight and exclude it.
    let end_date = Option::from(end_date)
        .filter(|value: &String| !value.is_empty())
        .map(|value| format!("{value} 23:59:59"));
    let like = Option::from(search)
        .filter(|value: &String| !value.is_empty())
        .map(|value| format!("%{value}%"));
    let user_id = (user_id != 0).then_some(user_id);

    let mut entries: Vec<AuditLogEntry> = Vec::new();

    if source_selected(&selector, SOURCE_PANEL) {
        match fetch_panel(
            &state,
            action.as_deref(),
            user_id,
            start_date.as_deref(),
            end_date.as_deref(),
            like.as_deref(),
            fetch_limit,
        )
        .await
        {
            Ok(rows) => entries.extend(rows),
            Err(error) => {
                tracing::error!(%error, "failed to list panel logs");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ERR_INTERNAL,
                    "failed to list panel logs",
                );
            }
        }
    }

    if source_selected(&selector, SOURCE_SDK) {
        match fetch_sdk(
            &state,
            action.as_deref(),
            user_id,
            start_date.as_deref(),
            end_date.as_deref(),
            like.as_deref(),
            fetch_limit,
        )
        .await
        {
            Ok(rows) => entries.extend(rows),
            Err(error) => {
                tracing::error!(%error, "failed to list sdk logs");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ERR_INTERNAL,
                    "failed to list sdk logs",
                );
            }
        }
    }

    if source_selected(&selector, SOURCE_BALANCE) {
        match fetch_balance(
            &state,
            action.as_deref(),
            user_id,
            start_date.as_deref(),
            end_date.as_deref(),
            like.as_deref(),
            fetch_limit,
        )
        .await
        {
            Ok(rows) => entries.extend(rows),
            Err(error) => {
                tracing::error!(%error, "failed to list balance logs");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ERR_INTERNAL,
                    "failed to list balance logs",
                );
            }
        }
    }

    let (items, total) = paginate(entries, page, page_size);

    ok(json!({
        "items": items,
        "total": total,
        "page": page,
        "page_size": page_size,
        // Echoed verbatim, including the empty string when unset — the console
        // uses it to keep its filter chip in sync.
        "source": selector,
    }))
}

/// `page ∈ [1, 1_000_000]`, `page_size ∈ [1, 200]`.
///
/// Not [`page_params`]: this feed's page-size ceiling is 200, not the panel's
/// usual 100, because the console pulls wider pages.
#[must_use]
pub fn audit_page_params(page: Option<&str>, page_size: Option<&str>) -> (i64, i64) {
    (
        query_int(page, 1, 1, 1_000_000),
        query_int(page_size, 30, 1, 200),
    )
}

/// Sorts newest-first and cuts the requested window.
///
/// 旧实现先在内存中合并再分页，所以 `total` 是 *合并后* 候选集的大小
/// candidate set — which is bounded by the per-table over-fetch, not by the
/// true row count. Reporting a different `total` here would change what the
/// console's pager shows.
#[must_use]
pub fn paginate(
    mut entries: Vec<AuditLogEntry>,
    page: i64,
    page_size: i64,
) -> (Vec<AuditLogEntry>, usize) {
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    let total = entries.len();
    let start = usize::try_from((page - 1) * page_size)
        .unwrap_or(usize::MAX)
        .min(total);
    let end = start
        .saturating_add(usize::try_from(page_size).unwrap_or(usize::MAX))
        .min(total);
    (entries[start..end].to_vec(), total)
}

// ---------------------------------------------------------------- per-source

/// `operation_logs`. Free-text search covers the three operator-meaningful
/// columns 旧实现会检索：action、target、actor email。
#[derive(Debug, sqlx::FromRow)]
struct PanelLogRow {
    id: i64,
    actor_id: Option<i64>,
    actor_email: Option<String>,
    action: Option<String>,
    target: Option<String>,
    method: Option<String>,
    path: Option<String>,
    status_code: Option<i64>,
    ip_address: Option<String>,
    request_id: Option<String>,
    metadata: Option<Value>,
    created_at: DateTime<Utc>,
}

async fn fetch_panel(
    state: &PanelState,
    action: Option<&str>,
    user_id: Option<i64>,
    start_date: Option<&str>,
    end_date: Option<&str>,
    like: Option<&str>,
    limit: i64,
) -> Result<Vec<AuditLogEntry>, sqlx::Error> {
    let rows: Vec<PanelLogRow> = sqlx::query_as(
        "SELECT id, actor_id, actor_email, action, target, method, path, status_code, \
          ip_address, request_id, metadata, created_at \
         FROM operation_logs \
         WHERE ($1::text IS NULL OR action = $1) \
           AND ($2::bigint IS NULL OR actor_id = $2) \
           AND ($3::timestamptz IS NULL OR created_at >= $3::timestamptz) \
           AND ($4::timestamptz IS NULL OR created_at <= $4::timestamptz) \
           AND ($5::text IS NULL OR action ILIKE $5 OR target ILIKE $5 OR actor_email ILIKE $5) \
         ORDER BY created_at DESC LIMIT $6",
    )
    .bind(action)
    .bind(user_id)
    .bind(start_date)
    .bind(end_date)
    .bind(like)
    .bind(limit)
    .fetch_all(&state.pg)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| AuditLogEntry {
            id: format!("{SOURCE_PANEL}-{}", row.id),
            source: SOURCE_PANEL.to_owned(),
            actor_id: row.actor_id.unwrap_or_default(),
            actor_email: row.actor_email.unwrap_or_default(),
            action: row.action.unwrap_or_default(),
            target: row.target.unwrap_or_default(),
            method: row.method.unwrap_or_default(),
            path: row.path.unwrap_or_default(),
            status_code: row.status_code.unwrap_or_default(),
            ip_address: row.ip_address.unwrap_or_default(),
            request_id: row.request_id.unwrap_or_default(),
            metadata: as_object(row.metadata),
            created_at: row.created_at,
        })
        .collect())
}

/// `usage_logs`. The `?action=` filter only bites when it is `sdk:<provider>`,
/// which is the same string this mapper puts in `action` — so filtering by a
/// value the console displays works, and any other action excludes this source
/// by matching nothing.
async fn fetch_sdk(
    state: &PanelState,
    action: Option<&str>,
    user_id: Option<i64>,
    start_date: Option<&str>,
    end_date: Option<&str>,
    like: Option<&str>,
    limit: i64,
) -> Result<Vec<AuditLogEntry>, sqlx::Error> {
    let provider = action.and_then(|value| value.strip_prefix("sdk:"));

    let rows: Vec<UsageEntryRow> = sqlx::query_as(
        "SELECT id, user_id, api_key_id, request_id, idempotency_key, model, provider, \
          tokens_in, tokens_out, input_cost::float8 AS input_cost, \
          output_cost::float8 AS output_cost, total_cost::float8 AS total_cost, \
          actual_cost::float8 AS actual_cost, stream, duration_ms, ip_address, failed, created_at \
         FROM usage_logs \
         WHERE ($1::text IS NULL OR provider = $1) \
           AND ($2::bigint IS NULL OR user_id = $2) \
           AND ($3::timestamptz IS NULL OR created_at >= $3::timestamptz) \
           AND ($4::timestamptz IS NULL OR created_at <= $4::timestamptz) \
           AND ($5::text IS NULL OR model ILIKE $5 OR provider ILIKE $5 OR request_id ILIKE $5) \
         ORDER BY created_at DESC LIMIT $6",
    )
    .bind(provider)
    .bind(user_id)
    .bind(start_date)
    .bind(end_date)
    .bind(like)
    .bind(limit)
    .fetch_all(&state.pg)
    .await?;

    Ok(rows.iter().map(UsageEntryRow::to_entry).collect())
}

#[derive(Debug, sqlx::FromRow)]
struct UsageEntryRow {
    id: i64,
    user_id: i64,
    api_key_id: i64,
    request_id: Option<String>,
    idempotency_key: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    input_cost: Option<f64>,
    output_cost: Option<f64>,
    total_cost: Option<f64>,
    actual_cost: Option<f64>,
    stream: Option<bool>,
    duration_ms: Option<i64>,
    ip_address: Option<String>,
    failed: Option<bool>,
    created_at: DateTime<Utc>,
}

impl UsageEntryRow {
    /// 对应 `usageLogToEntry`。
    fn to_entry(&self) -> AuditLogEntry {
        let provider = self.provider.clone().unwrap_or_default();
        let model = self.model.clone().unwrap_or_default();
        let request_id = self.request_id.clone().unwrap_or_default();
        let failed = self.failed.unwrap_or_default();

        // A usage row has no provider only when the request died before the
        // upstream was chosen; it still belongs in the feed.
        let action = if provider.is_empty() {
            "sdk:request".to_owned()
        } else {
            format!("{SOURCE_SDK}:{provider}")
        };
        // Same for the model: fall back to something that identifies the call.
        let target = if model.is_empty() {
            format!("request:{request_id}")
        } else {
            model.clone()
        };

        let metadata = json!({
            "model": model,
            "provider": provider,
            "tokens_in": self.tokens_in.unwrap_or_default(),
            "tokens_out": self.tokens_out.unwrap_or_default(),
            "input_cost": self.input_cost.unwrap_or_default(),
            "output_cost": self.output_cost.unwrap_or_default(),
            "total_cost": self.total_cost.unwrap_or_default(),
            "actual_cost": self.actual_cost.unwrap_or_default(),
            "stream": self.stream.unwrap_or_default(),
            "duration_ms": self.duration_ms.unwrap_or_default(),
            "failed": failed,
            "api_key_id": self.api_key_id,
            "idempotency": self.idempotency_key.clone().unwrap_or_default(),
        });

        AuditLogEntry {
            id: format!("{SOURCE_SDK}-{}", self.id),
            source: SOURCE_SDK.to_owned(),
            actor_id: self.user_id,
            actor_email: String::new(),
            action,
            target,
            method: String::new(),
            path: String::new(),
            // A failed proxy call is reported as a bad gateway, since the
            // failure was upstream's, not the caller's.
            status_code: if failed { 502 } else { 200 },
            ip_address: self.ip_address.clone().unwrap_or_default(),
            request_id,
            metadata: as_object(Some(metadata)),
            created_at: self.created_at,
        }
    }
}

/// `balance_logs`. Same `<prefix>:<value>` convention as the SDK source, with
/// `balance:` in front of the ledger operation.
async fn fetch_balance(
    state: &PanelState,
    action: Option<&str>,
    user_id: Option<i64>,
    start_date: Option<&str>,
    end_date: Option<&str>,
    like: Option<&str>,
    limit: i64,
) -> Result<Vec<AuditLogEntry>, sqlx::Error> {
    let kind = action.and_then(|value| value.strip_prefix("balance:"));

    let rows: Vec<BalanceLogRow> = sqlx::query_as(
        "SELECT id, user_id, amount::float8 AS amount, type AS kind, reference, metadata, \
          created_at \
         FROM balance_logs \
         WHERE ($1::text IS NULL OR type = $1) \
           AND ($2::bigint IS NULL OR user_id = $2) \
           AND ($3::timestamptz IS NULL OR created_at >= $3::timestamptz) \
           AND ($4::timestamptz IS NULL OR created_at <= $4::timestamptz) \
           AND ($5::text IS NULL OR type ILIKE $5 OR reference ILIKE $5) \
         ORDER BY created_at DESC LIMIT $6",
    )
    .bind(kind)
    .bind(user_id)
    .bind(start_date)
    .bind(end_date)
    .bind(like)
    .bind(limit)
    .fetch_all(&state.pg)
    .await?;

    Ok(rows.into_iter().map(BalanceLogRow::into_entry).collect())
}

/// `type` is a reserved word in Rust, so the column is aliased to `kind` in the
/// query rather than renamed with an attribute — the SQL a reader sees is then
/// the SQL that runs.
#[derive(Debug, sqlx::FromRow)]
struct BalanceLogRow {
    id: i64,
    user_id: i64,
    amount: f64,
    kind: String,
    reference: Option<String>,
    metadata: Option<Value>,
    created_at: DateTime<Utc>,
}

impl BalanceLogRow {
    /// 对应 `balanceLogToEntry`。
    fn into_entry(self) -> AuditLogEntry {
        let reference = self.reference.unwrap_or_default();
        // The ledger's own metadata is preserved and the two summary fields are
        // layered on top, so `shortfall_usd` and friends stay visible.
        let mut meta = as_object(self.metadata).unwrap_or_default();
        meta.insert("amount".to_owned(), json!(self.amount));
        meta.insert("reference".to_owned(), json!(reference));

        AuditLogEntry {
            id: format!("{SOURCE_BALANCE}-{}", self.id),
            source: SOURCE_BALANCE.to_owned(),
            actor_id: self.user_id,
            actor_email: String::new(),
            action: format!("{SOURCE_BALANCE}:{}", self.kind),
            target: reference,
            method: String::new(),
            path: String::new(),
            status_code: 0,
            ip_address: String::new(),
            request_id: String::new(),
            metadata: Some(meta),
            created_at: self.created_at,
        }
    }
}

/// A JSON column as an object, or `None` when it is NULL or not an object.
///
/// 对应 `decodeJSONMap` —— anything that does not unmarshal into a
/// `map[string]any` (a JSON array, a scalar, malformed bytes) becomes nil, and
/// the `omitempty` tag then drops the key entirely.
fn as_object(value: Option<Value>) -> Option<Map<String, Value>> {
    match value {
        Some(Value::Object(map)) => Some(map),
        _ => None,
    }
}

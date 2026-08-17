//! `/{provider}` — the per-provider API-key pools, and `/api-key-usage`.
//!
//! 对应 `SDKMgmtProvider{Get,Post,Put,Delete}Handler`、
//! `SDKMgmtAPIKeyUsageHandler` 及其周围的 payload 机制
//! （`sdkMgmtPayloadItems`、`sdkMgmtExpandProviderRecords`、
//! `sdkMgmtAuthFromPayload`、`sdkMgmtFindProviderAuth`、
//! `sdkMgmtDeleteProviderAuths`）。
//!
//! # One route, three meanings
//!
//! `/{provider}` is matched by three different things, in this order:
//!
//! 1. a `*-auth-url` suffix, which is an OAuth start and belongs to
//!    [`super::oauth`] — dispatched here because a sibling `/{provider}-auth-url`
//!    route would be an axum path conflict, exactly as it was a gin one;
//! 2. one of the five endpoint keys in [`PROVIDER_ENDPOINTS`];
//! 3. nothing, which is a 404.
//!
//! # `PUT`/`DELETE` semantics are desired-state, not patch
//!
//! `DELETE` with an *array* body means "these are the credentials that should
//! remain" — everything else in the pool is removed. With an *object* body it
//! means "remove this one". 这里用请求体的 JSON 类型来区分两种语义，
//! 搞反了会清空整个池。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::Utc;
use gw_authcore::{AuthRecord, AuthStatus};
use serde_json::{Map, Value, json};

use super::record::{api_key, attr, is_deleted, looks_masked, mask_secret, serialize_pool_entry};
use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

/// Endpoint key → provider id. 对应 `sdkMgmtProviderEndpoints`。
///
/// The endpoint key is also the response's top-level key, which is why the pool
/// payload is `{"claude-api-key": [...]}` rather than `{"items": [...]}`:
/// `frontend/src/features/admin-proxy/providerConfig.ts` keys off it.
pub const PROVIDER_ENDPOINTS: [(&str, &str); 5] = [
    ("openai-compatibility", "openai"),
    ("claude-api-key", "claude"),
    ("gemini-api-key", "gemini"),
    ("codex-api-key", "codex"),
    ("vertex-api-key", "vertex"),
];

/// Suffix that turns `/{provider}` into an OAuth start —— 即
/// `strings.HasSuffix(provider, "-auth-url")` 那类判断。
pub const AUTH_URL_SUFFIX: &str = "-auth-url";

const ERR_INVALID_JSON: i32 = 4000;
const ERR_API_KEY_REQUIRED: i32 = 4001;
const ERR_UNKNOWN_PROVIDER: i32 = 4040;
const ERR_UPDATE_NOT_FOUND: i32 = 4041;
const ERR_DELETE_NOT_FOUND: i32 = 4042;
const ERR_REGISTER_FAILED: i32 = 5001;
const ERR_UPDATE_FAILED: i32 = 5002;

/// Provider id behind an endpoint key —— 即 `sdkMgmtProviderFromRequest`
/// 里的那张映射。
#[must_use]
pub fn provider_for_endpoint(endpoint: &str) -> Option<&'static str> {
    PROVIDER_ENDPOINTS
        .iter()
        .find(|(key, _)| *key == endpoint.trim())
        .map(|(_, provider)| *provider)
}

/// Endpoint key for a provider id. 对应 `sdkMgmtEndpointForProvider`。
#[must_use]
pub fn endpoint_for_provider(provider: &str) -> Option<&'static str> {
    PROVIDER_ENDPOINTS
        .iter()
        .find(|(_, id)| *id == provider)
        .map(|(key, _)| *key)
}

fn unknown_provider() -> Response {
    err(
        StatusCode::NOT_FOUND,
        ERR_UNKNOWN_PROVIDER,
        "unknown provider",
    )
}

/// Every non-tombstoned credential for one provider, oldest first.
///
/// 对应 `sdkMgmtProviderAuths` —— ordered by creation with the id as tiebreak, so
/// the positional index a `PUT` body relies on is stable.
async fn provider_records(state: &PanelState, provider: &str) -> anyhow::Result<Vec<AuthRecord>> {
    let mut records: Vec<AuthRecord> = state
        .auth_store
        .list()
        .await?
        .into_iter()
        .filter(|record| record.provider == provider && !is_deleted(record))
        .collect();
    records.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(records)
}

fn store_failure(error: &anyhow::Error, code: i32, message: &str) -> Response {
    tracing::error!(%error, "auth store operation failed");
    err(StatusCode::INTERNAL_SERVER_ERROR, code, message)
}

// ---------------------------------------------------------------- GET

/// `GET /{provider}`. 对应 `SDKMgmtProviderGetHandler`。
pub async fn get(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(endpoint): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    if endpoint.ends_with(AUTH_URL_SUFFIX) {
        return super::oauth::auth_url(&state, &headers, &endpoint, None).await;
    }
    let Some(provider) = provider_for_endpoint(&endpoint) else {
        return unknown_provider();
    };
    match provider_records(&state, provider).await {
        Ok(records) => {
            let items: Vec<Value> = records
                .iter()
                .enumerate()
                .map(|(index, record)| serialize_pool_entry(record, index))
                .collect();
            ok(json!({ endpoint.trim(): items }))
        }
        Err(error) => store_failure(
            &error,
            ERR_REGISTER_FAILED,
            "failed to list provider API keys",
        ),
    }
}

// ---------------------------------------------------------------- POST

/// `POST /{provider}`. 对应 `SDKMgmtProviderPostHandler`。
///
/// Items without a *raw* API key are skipped rather than rejected: the console
/// re-submits the whole pool, and the rows it did not touch still carry their
/// masked previews. Only if that leaves nothing to create is this a 400.
pub async fn post(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(endpoint): Path<String>,
    headers: axum::http::HeaderMap,
    body: Option<axum::Json<Value>>,
) -> Response {
    if endpoint.ends_with(AUTH_URL_SUFFIX) {
        let payload = body.as_ref().map(|axum::Json(value)| value);
        return super::oauth::auth_url(&state, &headers, &endpoint, payload).await;
    }
    let Some(provider) = provider_for_endpoint(&endpoint) else {
        return unknown_provider();
    };
    let items = match parse_payload(body) {
        Ok(items) => items,
        Err(message) => return invalid_payload(message),
    };

    let now = Utc::now();
    let mut created: Vec<Value> = Vec::new();
    for item in &items {
        if !has_raw_api_key(item) {
            continue;
        }
        let record = record_from_payload(provider, item, None, now);
        if let Err(error) = state.auth_store.save(&record).await {
            return store_failure(
                &error,
                ERR_REGISTER_FAILED,
                "failed to register provider API key",
            );
        }
        let index = created.len();
        created.push(serialize_pool_entry(&record, index));
    }

    if created.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_API_KEY_REQUIRED,
            "api key is required",
        );
    }
    ok(json!({"items": created, "message": "created"}))
}

// ---------------------------------------------------------------- PUT

/// `PUT /{provider}`. 对应 `SDKMgmtProviderPutHandler`。
///
/// Each submitted item is matched against an existing credential by id, then by
/// name, then by position. An item that matches nothing is skipped; if none
/// match, the whole request is a 404 rather than a silent no-op.
pub async fn put(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(endpoint): Path<String>,
    body: Option<axum::Json<Value>>,
) -> Response {
    let Some(provider) = provider_for_endpoint(&endpoint) else {
        return unknown_provider();
    };
    let items = match parse_payload(body) {
        Ok(items) => items,
        Err(message) => return invalid_payload(message),
    };
    let existing = match provider_records(&state, provider).await {
        Ok(records) => records,
        Err(error) => {
            return store_failure(
                &error,
                ERR_UPDATE_FAILED,
                "failed to update provider API key",
            );
        }
    };

    let now = Utc::now();
    let mut updated: Vec<Value> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let Some(found) = find_record(&existing, item, index) else {
            continue;
        };
        let record = record_from_payload(provider, item, Some(found), now);
        if let Err(error) = state.auth_store.save(&record).await {
            return store_failure(
                &error,
                ERR_UPDATE_FAILED,
                "failed to update provider API key",
            );
        }
        updated.push(serialize_pool_entry(&record, index));
    }

    if updated.is_empty() {
        return err(
            StatusCode::NOT_FOUND,
            ERR_UPDATE_NOT_FOUND,
            "provider API key not found",
        );
    }
    ok(json!({"items": updated, "message": "updated"}))
}

// ---------------------------------------------------------------- DELETE

/// `DELETE /{provider}`. 对应 `SDKMgmtProviderDeleteHandler`。
///
/// # The `tombstoned` field
///
/// 旧实现无法从运行中的 SDK manager 移除凭证——它没有 public remove——所以
/// 禁用内存副本并删除行，
/// reporting both lists plus a `manager_remove_note` explaining the gap. There
/// is no such manager here: the row is simply deleted. The field set is kept
/// verbatim (the console reads `deleted`), with the two lists agreeing because
/// there is no longer any state that could disagree.
pub async fn delete(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(endpoint): Path<String>,
    body: Option<axum::Json<Value>>,
) -> Response {
    let Some(provider) = provider_for_endpoint(&endpoint) else {
        return unknown_provider();
    };
    let Some(axum::Json(raw)) = body else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_INVALID_JSON,
            "provider payload is required",
        );
    };
    let Some((items, desired_state)) = parse_delete_payload(&raw) else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_INVALID_JSON,
            "provider payload is required",
        );
    };
    let existing = match provider_records(&state, provider).await {
        Ok(records) => records,
        Err(error) => {
            return store_failure(
                &error,
                ERR_REGISTER_FAILED,
                "failed to list provider API keys",
            );
        }
    };

    let mut target_ids = targets_to_delete(&existing, &items, desired_state);
    target_ids.sort();

    for id in &target_ids {
        if let Err(error) = state.auth_store.delete(id).await {
            return store_failure(
                &error,
                ERR_UPDATE_FAILED,
                "failed to delete provider API key",
            );
        }
    }

    if target_ids.is_empty() {
        return err(
            StatusCode::NOT_FOUND,
            ERR_DELETE_NOT_FOUND,
            "provider API key not found",
        );
    }
    ok(json!({
        "deleted": target_ids,
        "tombstoned": target_ids,
        "in_memory_filtered": target_ids,
        "direct_remove": false,
        "message": "deleted",
        "manager_remove_note":
            "SDK manager has no public remove method; tombstoned credentials are omitted from GET and usage until reload",
    }))
}

/// Which credentials a delete request targets. 对应 `sdkMgmtDeleteProviderAuths`。
///
/// With `desired_state`, the submitted items are the ones to **keep** and
/// everything else in the pool goes. Without it, they are the ones to remove.
#[must_use]
pub fn targets_to_delete(
    existing: &[AuthRecord],
    items: &[Map<String, Value>],
    desired_state: bool,
) -> Vec<String> {
    let mut matched: Vec<String> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if let Some(record) = find_record(existing, item, index)
            && !matched.contains(&record.id)
        {
            matched.push(record.id.clone());
        }
    }
    if desired_state {
        existing
            .iter()
            .filter(|record| !matched.contains(&record.id))
            .map(|record| record.id.clone())
            .collect()
    } else {
        matched
    }
}

// ---------------------------------------------------------------- usage

/// `GET /api-key-usage`. 对应 `SDKMgmtAPIKeyUsageHandler`。
///
/// Buckets are keyed by `"<base-url>|<masked api key>"` and repeated under both
/// the endpoint key and the provider id, because
/// `providerConfig.ts::matchUsage` looks in whichever it finds first.
///
/// The counters are all zero — see [`super::record`] for why the persisted
/// record has no per-process success/failure tallies.
pub async fn api_key_usage(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    let records = match state.auth_store.list().await {
        Ok(records) => records,
        Err(error) => {
            return store_failure(&error, ERR_REGISTER_FAILED, "failed to list credentials");
        }
    };

    let mut usage = Map::new();
    for record in records.iter().filter(|record| !is_deleted(record)) {
        let Some(endpoint) = endpoint_for_provider(&record.provider) else {
            continue;
        };
        let entry_key = format!(
            "{}|{}",
            attr(record, &["base_url", "base-url"]),
            mask_secret(&api_key(record))
        );
        let entry = json!({"success": 0, "failed": 0, "recent_requests": []});
        for bucket_key in [endpoint, record.provider.as_str()] {
            let bucket = usage
                .entry(bucket_key.to_owned())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(bucket) = bucket.as_object_mut() {
                bucket.insert(entry_key.clone(), entry.clone());
            }
        }
    }
    ok(Value::Object(usage))
}

// ---------------------------------------------------------------- payloads

/// Normalises the three body shapes the console sends into a flat item list.
///
/// 对应 `sdkMgmtParseProviderPayload` + `sdkMgmtPayloadItems`. A bare array, a
/// `{value|keys|items: [...]}` wrapper, and a single bare object are all
/// accepted; anything else is a 400.
fn parse_payload(body: Option<axum::Json<Value>>) -> Result<Vec<Map<String, Value>>, &'static str> {
    // The error is the operator-facing message; both cases are 400 + 4000, so
    // carrying a whole `Response` here would only make the error variant large.
    let Some(axum::Json(raw)) = body else {
        return Err("invalid JSON payload");
    };
    let items = payload_items(&raw);
    if items.is_empty() {
        return Err("provider payload is required");
    }
    Ok(items)
}

/// The 400 both [`parse_payload`] failures produce.
fn invalid_payload(message: &'static str) -> Response {
    err(StatusCode::BAD_REQUEST, ERR_INVALID_JSON, message)
}

/// 对应 `sdkMgmtPayloadItems`。
#[must_use]
pub fn payload_items(raw: &Value) -> Vec<Map<String, Value>> {
    match raw {
        Value::Array(values) => expand_entries(&records_from_array(values)),
        Value::Object(object) => {
            for key in ["value", "keys", "items"] {
                if let Some(Value::Array(values)) = object.get(key) {
                    return expand_entries(&records_from_array(values));
                }
            }
            expand_entries(std::slice::from_ref(object))
        }
        _ => Vec::new(),
    }
}

/// 对应 `sdkMgmtParseProviderDeletePayload`。返回的 bool 即 `desiredStateArray` 语义。
#[must_use]
pub fn parse_delete_payload(raw: &Value) -> Option<(Vec<Map<String, Value>>, bool)> {
    match raw {
        Value::Array(values) => Some((expand_entries(&records_from_array(values)), true)),
        Value::Object(object) => {
            for key in ["value", "keys", "items"] {
                if let Some(Value::Array(values)) = object.get(key) {
                    return Some((expand_entries(&records_from_array(values)), true));
                }
            }
            Some((expand_entries(std::slice::from_ref(object)), false))
        }
        _ => None,
    }
}

/// 对应 `sdkMgmtRecordsFromArray` —— non-object elements are dropped silently.
fn records_from_array(values: &[Value]) -> Vec<Map<String, Value>> {
    values
        .iter()
        .filter_map(|value| value.as_object().cloned())
        .collect()
}

/// Flattens the console's grouped form. 对应 `sdkMgmtExpandProviderRecords`。
///
/// One `base-url` with several `api-key-entries` becomes one item per key, each
/// inheriting the group's fields. The `api-key-entries` key itself is dropped
/// from the merged item so it cannot be re-expanded.
#[must_use]
pub fn expand_entries(items: &[Map<String, Value>]) -> Vec<Map<String, Value>> {
    let mut expanded = Vec::new();
    for item in items {
        let entries = match item.get("api-key-entries") {
            Some(Value::Array(entries)) if !entries.is_empty() => entries,
            _ => {
                expanded.push(item.clone());
                continue;
            }
        };
        for entry in entries {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            let mut merged: Map<String, Value> = item
                .iter()
                .filter(|(key, _)| key.as_str() != "api-key-entries")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            for (key, value) in entry {
                merged.insert(key.clone(), value.clone());
            }
            expanded.push(merged);
        }
    }
    expanded
}

/// First present key rendered as a trimmed string. 对应 `sdkMgmtString`。
#[must_use]
pub fn payload_string(item: &Map<String, Value>, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| item.get(*key).map(value_to_string))
        .unwrap_or_default()
}

/// 对应 `sdkMgmtPayloadString` —— a scalar becomes its text, anything else its
/// JSON encoding, and a present key always yields `Some` even when empty.
fn payload_string_opt(item: &Map<String, Value>, key: &str) -> Option<String> {
    item.get(key).map(value_to_string)
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        other => other.to_string(),
    }
}

/// 对应 `sdkMgmtPayloadBool` —— a string `"true"` counts, any other non-bool is
/// `false`, and an absent key is `None` (leave the flag alone).
fn payload_bool(item: &Map<String, Value>, key: &str) -> Option<bool> {
    match item.get(key)? {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => Some(text.eq_ignore_ascii_case("true")),
        _ => Some(false),
    }
}

/// Whether an item carries a real, unmasked API key. 对应 `sdkMgmtHasRawAPIKey`。
#[must_use]
pub fn has_raw_api_key(item: &Map<String, Value>) -> bool {
    let value = payload_string(item, &["api-key", "api_key", "apiKey"]);
    !value.is_empty() && !looks_masked(&value)
}

/// Builds the credential a create/update writes. 对应 `sdkMgmtAuthFromPayload`。
///
/// With `existing`, the submitted fields are layered onto it, so an update that
/// omits a field keeps it — and, critically, a masked `api-key` never
/// overwrites the stored secret.
#[must_use]
pub fn record_from_payload(
    provider: &str,
    item: &Map<String, Value>,
    existing: Option<&AuthRecord>,
    now: chrono::DateTime<Utc>,
) -> AuthRecord {
    let mut record = match existing {
        Some(existing) => {
            let mut record = existing.clone();
            record.updated_at = now;
            if record.created_at == chrono::DateTime::UNIX_EPOCH {
                record.created_at = now;
            }
            record
        }
        None => {
            let mut record =
                AuthRecord::new(uuid::Uuid::new_v4().to_string(), provider.to_owned(), now);
            // A caller-supplied id is honoured only on create; on update the id
            // is what identified the row in the first place.
            let supplied = payload_string(item, &["id", "auth_id", "_id"]);
            if !supplied.is_empty() {
                record.id = supplied;
            }
            record
        }
    };
    record.provider = provider.to_owned();

    let name = payload_string(item, &["name", "label"]);
    if !name.is_empty() {
        record.label = name;
    }
    // Present-but-empty clears the prefix; absent leaves it.
    if let Some(prefix) = payload_string_opt(item, "prefix") {
        record.prefix = prefix;
    }
    if let Some(proxy) =
        payload_string_opt(item, "proxy-url").or_else(|| payload_string_opt(item, "proxy_url"))
    {
        record.proxy_url = proxy.clone();
        record.set_attribute("proxy_url", proxy);
    }

    for (json_key, attr_key) in [
        ("base-url", "base_url"),
        ("models-url", "models_url"),
        ("priority", "priority"),
        ("websockets", "websockets"),
        ("experimental-cch-signing", "experimental_cch_signing"),
    ] {
        if let Some(value) = payload_string_opt(item, json_key) {
            record.set_attribute(attr_key, value);
        }
    }

    let metadata = ensure_object(&mut record.metadata);
    let raw_key = payload_string(item, &["api-key", "api_key", "apiKey"]);
    if !raw_key.is_empty() && !looks_masked(&raw_key) {
        metadata.insert("api_key".to_owned(), json!(raw_key));
    }
    for (json_key, metadata_key) in [
        ("headers", "headers"),
        ("models", "models"),
        ("excluded-models", "excluded_models"),
    ] {
        if let Some(value) = item.get(json_key) {
            metadata.insert(metadata_key.to_owned(), value.clone());
        }
    }

    if let Some(disabled) = payload_bool(item, "disabled") {
        record.disabled = disabled;
        if disabled {
            record.status = AuthStatus::Disabled;
        } else if record.status == AuthStatus::Disabled {
            record.status = AuthStatus::Active;
        }
    }
    record
}

/// Metadata is `Value`, but every writer treats it as an object; a column that
/// somehow holds a scalar is replaced rather than panicking.
fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("just ensured an object")
}

/// Matches a submitted item to a stored credential.
///
/// 对应 `sdkMgmtFindProviderAuth` —— id, then label/id-as-name, then the
/// item's position in the submitted list against the pool's own order. The
/// positional fallback is why [`provider_records`] must sort deterministically.
#[must_use]
pub fn find_record<'a>(
    existing: &'a [AuthRecord],
    item: &Map<String, Value>,
    index: usize,
) -> Option<&'a AuthRecord> {
    let id = payload_string(item, &["id", "auth_id"]);
    if !id.is_empty()
        && let Some(found) = existing.iter().find(|record| record.id == id)
    {
        return Some(found);
    }
    let name = payload_string(item, &["name", "label"]);
    if !name.is_empty()
        && let Some(found) = existing
            .iter()
            .find(|record| record.label == name || record.id == name)
    {
        return Some(found);
    }
    existing.get(index)
}

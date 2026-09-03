//! `/auth-files**` — the credential inventory: list, upload, enable/disable,
//! delete, plus the quota and per-model views over the same rows.
//!
//! 对应 `SDKMgmtAuthFiles{List,Create,Update,Delete}Handler`、
//! `SDKMgmtAuthFiles{Quota,Models}Handler` 及其辅助函数
//! （`sdkMgmtSortedAuths`、`sdkMgmtAuthFileMatchesQuery`、
//! `sdkMgmtAuthFromUpload`、`sdkMgmtProviderFromAuthJSON`、
//! `sdkMgmtToggleAuthFiles`、`sdkMgmtDeleteAuthFiles`、`sdkMgmtFindAuthFile`）。
//!
//! "Auth file" is the historical name: these were `.json` files on disk before
//! they were rows. The upload endpoint still takes those files, which is why
//! the provider has to be *inferred* from the JSON's shape.

use axum::extract::{Multipart, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::Utc;
use gw_authcore::{AuthRecord, AuthStatus};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::record::{
    is_deleted, serialize_auth_file, serialize_models, serialize_quota, stable_name,
};
use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

const ERR_BAD_REQUEST: i32 = 4001;
const ERR_LIST_FAILED: i32 = 5003;

/// Largest upload accepted, per file（对应 `io.LimitReader(file, 4<<20)`）。
const MAX_UPLOAD_BYTES: usize = 4 << 20;

/// Form fields the upload endpoint reads. 对应 `sdkMgmtUploadedAuthFiles`。
const UPLOAD_FIELDS: [&str; 4] = ["file", "files", "auth_file", "auth_files"];

/// Query string shared by the three read endpoints.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct AuthFileQuery {
    pub provider: Option<String>,
    pub status: Option<String>,
    pub disabled: Option<String>,
    pub q: Option<String>,
    /// Read by the delete endpoint, which accepts targets in the query string
    /// as well as the body.
    pub id: Option<String>,
    pub name: Option<String>,
    pub auth_id: Option<String>,
}

/// Every stored credential, ordered for display.
///
/// 对应 `sdkMgmtSortedAuths` —— grouped by provider, then by the same stable name
/// the mutating endpoints address rows by, so what an operator sees and what
/// they can `PUT` are in the same order.
async fn sorted_records(state: &PanelState) -> anyhow::Result<Vec<AuthRecord>> {
    let mut records = state.auth_store.list().await?;
    records.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| stable_name(left, 0).cmp(&stable_name(right, 0)))
    });
    Ok(records)
}

/// Whether a record passes the query filters. 对应 `sdkMgmtAuthFileMatchesQuery`。
///
/// A malformed `?disabled=` excludes everything rather than being ignored —
/// 解析失败时返回 false，and an operator who typed `disabled=yes`
/// is better served by an empty list than by a silently unfiltered one.
#[must_use]
pub fn matches_query(record: &AuthRecord, query: &AuthFileQuery, index: usize) -> bool {
    if let Some(provider) = trimmed(query.provider.as_ref())
        && !record.provider.eq_ignore_ascii_case(provider)
    {
        return false;
    }
    if let Some(status) = trimmed(query.status.as_ref())
        && !record.status.as_str().eq_ignore_ascii_case(status)
    {
        return false;
    }
    if let Some(raw) = trimmed(query.disabled.as_ref()) {
        match parse_bool(raw) {
            Some(expected) if expected == record.disabled => {}
            _ => return false,
        }
    }
    let Some(needle) = trimmed(query.q.as_ref()) else {
        return true;
    };
    let needle = needle.to_lowercase();
    haystack(record, index).to_lowercase().contains(&needle)
}

/// The text `?q=` searches（这九个字段以换行拼接）。
fn haystack(record: &AuthRecord, index: usize) -> String {
    use super::record::{attr, metadata_string};
    [
        record.id.clone(),
        record.provider.clone(),
        record.label.clone(),
        stable_name(record, index),
        metadata_string(record, "email"),
        metadata_string(record, "account_id"),
        attr(record, &["project_id"]).to_owned(),
        attr(record, &["location"]).to_owned(),
        attr(record, &["base_url", "base-url"]).to_owned(),
    ]
    .join("\n")
}

/// 对标 `strconv.ParseBool`，which is stricter than "not empty".
fn parse_bool(raw: &str) -> Option<bool> {
    match raw {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Some(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Some(false),
        _ => None,
    }
}

fn trimmed(raw: Option<&String>) -> Option<&str> {
    raw.map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn list_failure(error: &anyhow::Error) -> Response {
    tracing::error!(%error, "failed to read credentials");
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        ERR_LIST_FAILED,
        "failed to list auth files",
    )
}

/// The visible records, already filtered and indexed for display.
async fn visible(
    state: &PanelState,
    query: &AuthFileQuery,
) -> anyhow::Result<Vec<(usize, AuthRecord)>> {
    Ok(sorted_records(state)
        .await?
        .into_iter()
        .enumerate()
        .filter(|(index, record)| !is_deleted(record) && matches_query(record, query, *index))
        .collect())
}

// ---------------------------------------------------------------- read

/// `GET /auth-files`. 对应 `SDKMgmtAuthFilesListHandler`。
pub async fn list(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<AuthFileQuery>,
) -> Response {
    match visible(&state, &query).await {
        Ok(records) => {
            let files: Vec<Value> = records
                .iter()
                .map(|(index, record)| serialize_auth_file(record, *index))
                .collect();
            ok(json!({"files": files, "total": files.len()}))
        }
        Err(error) => list_failure(&error),
    }
}

/// `GET /auth-files/quota`. 对应 `SDKMgmtAuthFilesQuotaHandler`。
///
/// The same list is returned under both `quota` and `items`（两个键都发），
/// and the console reads `items`.
pub async fn quota(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<AuthFileQuery>,
) -> Response {
    match visible(&state, &query).await {
        Ok(records) => {
            let items: Vec<Value> = records
                .iter()
                .map(|(index, record)| serialize_quota(record, *index))
                .collect();
            ok(json!({"quota": items, "items": items, "total": items.len()}))
        }
        Err(error) => list_failure(&error),
    }
}

/// `GET /auth-files/models`. 对应 `SDKMgmtAuthFilesModelsHandler`。
pub async fn models(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<AuthFileQuery>,
) -> Response {
    match visible(&state, &query).await {
        Ok(records) => {
            let items: Vec<Value> = records
                .iter()
                .flat_map(|(index, record)| serialize_models(record, *index))
                .collect();
            ok(json!({"models": items, "total": items.len()}))
        }
        Err(error) => list_failure(&error),
    }
}

// ---------------------------------------------------------------- upload

/// `POST /auth-files`. 对应 `SDKMgmtAuthFilesCreateHandler`。
///
/// Multipart upload of one or more `.json` credential files. A file that cannot
/// be understood aborts the whole request with a 400 naming the reason ——
/// 部分导入会让操作者猜不透到底哪一半落库。
pub async fn create(
    State(state): State<PanelState>,
    _admin: AdminUser,
    multipart: Option<Multipart>,
) -> Response {
    let Some(mut multipart) = multipart else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "multipart json auth files are required",
        );
    };

    let now = Utc::now();
    let mut created: Vec<Value> = Vec::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, "malformed multipart upload");
                return err(
                    StatusCode::BAD_REQUEST,
                    ERR_BAD_REQUEST,
                    "multipart json auth files are required",
                );
            }
        };
        if !UPLOAD_FIELDS.contains(&field.name().unwrap_or_default()) {
            continue;
        }
        let filename = field.file_name().unwrap_or_default().to_owned();
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(_) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    ERR_BAD_REQUEST,
                    "failed to read auth file",
                );
            }
        };

        let record = match record_from_upload(&filename, &bytes, now) {
            Ok(record) => record,
            Err(message) => return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, message),
        };
        if let Err(error) = state.auth_store.save(&record).await {
            tracing::error!(%error, "failed to persist uploaded credential");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_LIST_FAILED,
                "failed to register auth file",
            );
        }
        let index = created.len();
        created.push(serialize_auth_file(&record, index));
    }

    if created.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "at least one .json auth file is required",
        );
    }
    ok(json!({"message": "created", "created": created, "count": created.len()}))
}

/// `POST /auth-files/import-local` — read CLI OAuth files already on this host.
///
/// Scans well-known paths under `AGW_LOCAL_OAUTH_HOME` / `$HOME` (Codex,
/// Claude Code, Grok, Kiro). Client-supplied filesystem paths are ignored so
/// an admin request cannot read arbitrary files. A credential whose access or
/// refresh token is already stored is skipped rather than duplicated.
pub async fn import_local(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    let Some(home) = gw_provider::local_oauth::process_home() else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "cannot resolve a home directory to scan for CLI OAuth files",
        );
    };
    let found = gw_provider::local_oauth::discover(&home);
    let mut existing = match sorted_records(&state).await {
        Ok(records) => records,
        Err(error) => return list_failure(&error),
    };
    let now = Utc::now();
    let mut imported: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for cred in found {
        let source = cred.source.display().to_string();
        if already_imported(&existing, &cred) {
            skipped.push(json!({
                "provider": cred.provider,
                "source": source,
                "reason": "already imported",
            }));
            continue;
        }
        let filename = format!("{}-local.json", cred.provider);
        let body = cred.to_upload_json().to_string();
        let mut record = match record_from_upload(&filename, body.as_bytes(), now) {
            Ok(record) => record,
            Err(message) => {
                skipped.push(json!({
                    "provider": cred.provider,
                    "source": source,
                    "reason": message,
                }));
                continue;
            }
        };
        if let Some(name) = cred.source.file_name().and_then(|name| name.to_str()) {
            record.label = name.to_owned();
        }
        if let Err(error) = state.auth_store.save(&record).await {
            tracing::error!(%error, "failed to persist imported credential");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_LIST_FAILED,
                "failed to register imported auth file",
            );
        }
        existing.push(record.clone());
        imported.push(serialize_auth_file(&record, imported.len()));
    }
    ok(json!({
        "imported": imported,
        "skipped": skipped,
        "count": imported.len(),
    }))
}

/// True when this host already stores the same CLI credential.
#[must_use]
pub fn already_imported(
    existing: &[AuthRecord],
    cred: &gw_provider::local_oauth::LocalOauthCred,
) -> bool {
    existing.iter().any(|record| {
        if !record.provider.eq_ignore_ascii_case(cred.provider) {
            return false;
        }
        let refresh_hit = !cred.refresh_token.is_empty()
            && metadata_token(record, "refresh_token") == Some(cred.refresh_token.as_str());
        let access_hit = !cred.access_token.is_empty()
            && metadata_token(record, "access_token") == Some(cred.access_token.as_str());
        refresh_hit || access_hit
    })
}

fn metadata_token<'a>(record: &'a AuthRecord, key: &str) -> Option<&'a str> {
    record
        .metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Parses one uploaded credential file. 对应 `sdkMgmtAuthFromUpload`。
///
/// # Errors
/// The operator-facing message, which passes straight through to the 400.
pub fn record_from_upload(
    filename: &str,
    body: &[u8],
    now: chrono::DateTime<Utc>,
) -> Result<AuthRecord, &'static str> {
    if !filename.to_lowercase().ends_with(".json") {
        return Err("auth file must be .json");
    }
    if body.len() > MAX_UPLOAD_BYTES {
        return Err("failed to read auth file");
    }
    let mut payload: Map<String, Value> = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or("invalid auth JSON")?;
    gw_provider::local_oauth::lift_cli_shape(&mut payload);

    let provider = provider_from_auth_json(&payload)?;
    let mut record = AuthRecord::new(uuid::Uuid::new_v4().to_string(), provider, now);
    if let Some(id) = string_field(&payload, &["id", "auth_id"]) {
        record.id = id;
    }
    record.label = string_field(&payload, &["label", "name", "email"])
        .unwrap_or_else(|| filename.trim_end_matches(".json").to_owned());

    let metadata = record
        .metadata
        .as_object_mut()
        .expect("AuthRecord::new starts with an object");
    if is_google_service_account(&payload) {
        metadata.insert("service_account".to_owned(), Value::Object(payload.clone()));
    }
    for field in ["email", "account_id"] {
        if let Some(value) = string_field(&payload, &[field]) {
            metadata.insert(field.to_owned(), json!(value));
        }
    }
    // Copied verbatim, not stringified: `token_data` and `service_account` are
    // objects, and flattening them would lose the refresh token.
    for field in [
        "api_key",
        "api-key",
        "x-api-key",
        "access_token",
        "refresh_token",
        "id_token",
        "token_data",
        "service_account",
    ] {
        if let Some(value) = payload.get(field) {
            metadata.insert(canonical_key(field), value.clone());
        }
    }
    // An OAuth export nests the tokens one level down; lift them so the
    // `has_access_token` / preview fields see them.
    if let Some(Value::Object(token_data)) = metadata.get("token_data").cloned() {
        for key in [
            "access_token",
            "refresh_token",
            "id_token",
            "email",
            "account_id",
        ] {
            if metadata.contains_key(key) {
                continue;
            }
            if let Some(value) = token_data.get(key)
                && let Some(text) = scalar_text(value)
                && !text.is_empty()
            {
                metadata.insert(key.to_owned(), json!(text));
            }
        }
    }

    for field in [
        "project_id",
        "location",
        "base_url",
        "base-url",
        "proxy_url",
        "proxy-url",
        "prefix",
    ] {
        let Some(value) = string_field(&payload, &[field]) else {
            continue;
        };
        let key = canonical_key(field);
        match key.as_str() {
            "proxy_url" => record.proxy_url = value.clone(),
            "prefix" => record.prefix = value.clone(),
            _ => {}
        }
        record.set_attribute(key, value);
    }
    Ok(record)
}

/// Infers the provider from a credential file. 对应 `sdkMgmtProviderFromAuthJSON`。
///
/// The order is load-bearing: a Google service account is recognised by shape
/// *before* its `type: "service_account"` field can be mistaken for a provider
/// name.
///
/// # Errors
/// When nothing in the file identifies a provider. An OAuth token export gets
/// its own message, because the fix is different — the operator has to say
/// which provider the token belongs to.
pub fn provider_from_auth_json(payload: &Map<String, Value>) -> Result<String, &'static str> {
    let declared = string_field(payload, &["provider", "type"])
        .unwrap_or_default()
        .to_lowercase();

    if is_google_service_account(payload)
        && matches!(
            declared.as_str(),
            "" | "service_account" | "google_service_account"
        )
    {
        return Ok("vertex".to_owned());
    }

    let provider = match declared.as_str() {
        "anthropic" => "claude".to_owned(),
        "openai-compatibility" | "openai_compatibility" | "openai-compatible" => {
            "openai".to_owned()
        }
        other => other.to_owned(),
    };
    if !provider.is_empty() {
        return Ok(provider);
    }
    if let Some(inferred) = gw_provider::local_oauth::infer_provider(payload) {
        return Ok(inferred.to_owned());
    }
    if payload.contains_key("service_account") {
        return Ok("vertex".to_owned());
    }
    if string_field(payload, &["api_key", "api-key", "x-api-key"]).is_some() {
        return Ok("openai".to_owned());
    }
    if payload.contains_key("token_data") || string_field(payload, &["access_token"]).is_some() {
        return Err("provider is required for OAuth token auth JSON");
    }
    Err("provider is required")
}

/// 对应 `sdkMgmtIsGoogleServiceAccount`。
fn is_google_service_account(payload: &Map<String, Value>) -> bool {
    string_field(payload, &["type"])
        .is_some_and(|value| value.eq_ignore_ascii_case("service_account"))
        && string_field(payload, &["private_key"]).is_some()
        && string_field(payload, &["client_email"]).is_some()
}

/// 对应 `sdkMgmtCanonicalAuthKey` —— hyphens become underscores and the
/// `x-api-key` header spelling folds into `api_key`.
fn canonical_key(key: &str) -> String {
    let key = key.trim().to_lowercase().replace('-', "_");
    if key == "x_api_key" {
        "api_key".to_owned()
    } else {
        key
    }
}

/// First non-empty scalar field among `keys`.
fn string_field(payload: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| payload.get(*key))
        .filter_map(scalar_text)
        .find(|value| !value.is_empty())
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_owned()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------- mutate

/// `PUT /auth-files`. 对应 `SDKMgmtAuthFilesUpdateHandler`。
///
/// Only `enable` / `disable`. Names that matched nothing come back in
/// `missing` rather than failing the request, so a batch over a stale list
/// still applies to the rows that do exist.
pub async fn update(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    let Some(axum::Json(raw)) = body else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "invalid JSON body",
        );
    };
    let Some(payload) = raw.as_object() else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "invalid JSON body",
        );
    };

    let action = super::providers::payload_string(payload, &["action"]).to_lowercase();
    let disabled = match action.as_str() {
        "disable" => true,
        "enable" => false,
        _ => {
            return err(
                StatusCode::BAD_REQUEST,
                ERR_BAD_REQUEST,
                "action must be disable or enable",
            );
        }
    };

    let names = payload_string_slice(
        payload,
        &["names", "name", "ids", "id", "auth_ids", "auth_id"],
    );
    if names.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "names are required",
        );
    }

    let records = match sorted_records(&state).await {
        Ok(records) => records,
        Err(error) => return list_failure(&error),
    };

    let mut updated: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for name in &names {
        let Some(found) = find_by_name(&records, name) else {
            missing.push(name.clone());
            continue;
        };
        let mut record = found.clone();
        record.disabled = disabled;
        if disabled {
            record.status = AuthStatus::Disabled;
        } else if record.status == AuthStatus::Disabled {
            record.status = AuthStatus::Active;
        }
        record.updated_at = Utc::now();
        if state.auth_store.save(&record).await.is_ok() {
            let index = updated.len();
            updated.push(stable_name(&record, index));
        }
    }
    ok(json!({"message": "updated", "updated": updated, "missing": missing}))
}

/// `DELETE /auth-files`. 对应 `SDKMgmtAuthFilesDeleteHandler`。
///
/// Targets come from the query string *and* the body, because the console
/// deletes one row by query and a batch by body.
pub async fn remove(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<AuthFileQuery>,
    body: Option<axum::Json<Value>>,
) -> Response {
    let mut targets: Vec<String> = Vec::new();
    let mut push = |value: Option<&str>| {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty())
            && !targets.iter().any(|seen| seen == value)
        {
            targets.push(value.to_owned());
        }
    };
    push(query.id.as_deref());
    push(query.name.as_deref());
    push(query.auth_id.as_deref());
    if let Some(axum::Json(raw)) = &body
        && let Some(payload) = raw.as_object()
    {
        for value in payload_string_slice(
            payload,
            &["ids", "id", "names", "name", "auth_ids", "auth_id"],
        ) {
            push(Some(&value));
        }
    }

    if targets.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "id, name, or auth_id is required",
        );
    }

    let records = match sorted_records(&state).await {
        Ok(records) => records,
        Err(error) => return list_failure(&error),
    };

    let mut deleted: Vec<String> = Vec::new();
    let mut disabled: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for target in &targets {
        let Some(found) = find_by_name(&records, target) else {
            missing.push(target.clone());
            continue;
        };
        if let Err(error) = state.auth_store.delete(&found.id).await {
            tracing::error!(%error, id = %found.id, "failed to delete credential");
            continue;
        }
        deleted.push(found.id.clone());
        let index = disabled.len();
        disabled.push(stable_name(found, index));
    }
    ok(json!({
        "message": "deleted",
        "deleted": deleted,
        "disabled": disabled,
        "missing": missing,
    }))
}

/// Resolves an operator-supplied target to a stored credential.
///
/// 对应 `sdkMgmtFindAuthFile` —— id first, then label, then the display name.
#[must_use]
pub fn find_by_name<'a>(records: &'a [AuthRecord], target: &str) -> Option<&'a AuthRecord> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    records
        .iter()
        .filter(|record| !is_deleted(record))
        .enumerate()
        .find(|(index, record)| {
            record.id == target || record.label == target || stable_name(record, *index) == target
        })
        .map(|(_, record)| record)
}

/// Flattens the several shapes a batch target list arrives in.
///
/// 对应 `sdkMgmtPayloadStringSlice` —— a scalar, a list of scalars, or several
/// keys carrying either; duplicates and blanks are dropped, order preserved.
#[must_use]
pub fn payload_string_slice(payload: &Map<String, Value>, keys: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |value: &Value| {
        let Some(text) = scalar_text(value) else {
            return;
        };
        if !text.is_empty() && !out.iter().any(|seen| seen == &text) {
            out.push(text);
        }
    };
    for key in keys {
        match payload.get(*key) {
            Some(Value::Array(values)) => values.iter().for_each(&mut push),
            Some(value) => push(value),
            None => {}
        }
    }
    out
}

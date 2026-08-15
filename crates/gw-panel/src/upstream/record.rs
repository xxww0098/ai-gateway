//! Reading an [`AuthRecord`] the way the admin panel expects to see it.
//!
//! 对应 SDK 管理面的存取与序列化半层：
//! `sdkMgmtAttr`、`sdkMgmtAttrNumber`、`sdkMgmtAttrBool`、`sdkMgmtMetadata`、
//! `sdkMgmtSafeMetadataString`、`sdkMgmtHasMetadata`、`sdkMgmtAuthAPIKey`、
//! `sdkMgmtMaskSecret`、`sdkMgmtLooksMasked`、`sdkMgmtAuthName`、
//! `sdkMgmtAuthStableName`、`sdkMgmtProxyURL`、`sdkMgmtAuthModels`、
//! `sdkMgmtTimeString`、`sdkMgmtAuthDeleted`，以及三个 `sdkMgmtSerialize*`。
//!
//! # 这里的来源是持久化记录
//!
//! 这些字段过去读自 SDK 的内存态 `Auth`；现在的来源是
//! [`gw_authcore::AuthRecord`]，即*持久化后的*形状——与 `model.AuthRecord`
//! 已经收窄到的一致。两条后果都体现在 JSON 里，且都是有意的：
//!
//! * **`success` / `failed` / `recent_requests` are always `0` / `0` / `[]`.**
//!   Those were per-process counters on the SDK manager; `model.AuthRecord`
//!   explicitly does not persist them, so any gateway serves zeros for every
//!   credential until traffic flows after a restart. The keys stay in the
//!   payload because the console renders them unconditionally.
//! * **`quota` and `model_states` are read out of their `jsonb` columns**
//!   rather than off typed structs. The key casing the SDK wrote is not
//!   knowable from this repository, so every lookup accepts both the snake_case
//!   and the PascalCase spelling.

use chrono::{DateTime, Utc};
use gw_authcore::AuthRecord;
use serde_json::{Map, Value, json};

#[cfg(test)]
mod tests;

/// Attribute marking a credential the panel has tombstoned —— 即删除 handler
/// 写入的 `Attributes["deleted"] = "true"`。
pub const DELETED_ATTRIBUTE: &str = "deleted";

// ---------------------------------------------------------------- attributes

/// 返回 `keys` 中第一个非空（trim 后）的属性。对应 `sdkMgmtAttr`。
///
/// Several attributes are written under two spellings (`base_url` and
/// `base-url`) depending on which handler stored them, which is why this takes
/// a list rather than a single key.
#[must_use]
pub fn attr<'a>(record: &'a AuthRecord, keys: &[&str]) -> &'a str {
    keys.iter()
        .filter_map(|key| record.attributes.get(*key))
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

/// An attribute parsed as a JSON number, falling back to the raw string.
///
/// 对应 `sdkMgmtAttrNumber` —— an absent attribute is `null`, an integral value
/// stays integral (the console shows `priority: 1`, never `1.0`), and anything
/// unparseable is passed through as text rather than being dropped.
#[must_use]
pub fn attr_number(record: &AuthRecord, key: &str) -> Value {
    let raw = attr(record, &[key]);
    if raw.is_empty() {
        return Value::Null;
    }
    if let Ok(int) = raw.parse::<i64>() {
        return json!(int);
    }
    if let Ok(float) = raw.parse::<f64>()
        && float.is_finite()
    {
        return json!(float);
    }
    json!(raw)
}

/// An attribute as a tri-state boolean. 对应 `sdkMgmtAttrBool`。
///
/// Absent is `null`, not `false`: the console distinguishes "websockets are off
/// for this credential" from "this credential does not configure websockets".
#[must_use]
pub fn attr_bool(record: &AuthRecord, key: &str) -> Value {
    let raw = attr(record, &[key]);
    if raw.is_empty() {
        return Value::Null;
    }
    json!(raw == "true")
}

/// Whether this credential has been tombstoned. 对应 `sdkMgmtAuthDeleted`。
///
/// 两个位置都要查：属性与 metadata blob——删除路径会把两处一起写。
#[must_use]
pub fn is_deleted(record: &AuthRecord) -> bool {
    if attr(record, &[DELETED_ATTRIBUTE]).eq_ignore_ascii_case("true") {
        return true;
    }
    match metadata(record, DELETED_ATTRIBUTE) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => text.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// The credential's outbound proxy, from the column or either attribute
/// spelling. 对应 `sdkMgmtProxyURL`。
#[must_use]
pub fn proxy_url(record: &AuthRecord) -> &str {
    if !record.proxy_url.is_empty() {
        return &record.proxy_url;
    }
    attr(record, &["proxy_url", "proxy-url"])
}

// ---------------------------------------------------------------- metadata

/// A raw metadata value. 对应 `sdkMgmtMetadata`。
#[must_use]
pub fn metadata<'a>(record: &'a AuthRecord, key: &str) -> Option<&'a Value> {
    record.metadata.as_object()?.get(key)
}

/// A metadata value as a string, but only when it is a scalar.
///
/// 对应 `sdkMgmtSafeMetadataString` —— the "safe" is that a nested object or
/// array yields `""` rather than being stringified, so a service-account blob
/// can never leak through a field meant to hold an email.
#[must_use]
pub fn metadata_string(record: &AuthRecord, key: &str) -> String {
    match metadata(record, key) {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

/// Whether a metadata key holds something. 对应 `sdkMgmtHasMetadata`。
///
/// A present-but-empty string counts as absent, because the console uses these
/// to decide whether to offer a "rotate token" action.
#[must_use]
pub fn has_metadata(record: &AuthRecord, key: &str) -> bool {
    match metadata(record, key) {
        None | Some(Value::Null) => false,
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    }
}

/// The stored upstream API key, or `""`.
///
/// 对应 `sdkMgmtAuthAPIKey`，即 `fmt.Sprint(auth.Metadata["api_key"])` 那样的行为——
/// therefore renders a **missing** key as the literal `"<nil>"` — which then
/// gets masked into `"<...>"` and shown to the operator as though an OAuth
/// credential had an API key. That artefact is not reproduced: an absent key is
/// `""`, which is what the `if key != ""` guards downstream already expect.
/// Nothing about the field set changes.
#[must_use]
pub fn api_key(record: &AuthRecord) -> String {
    metadata_string(record, "api_key")
}

// ---------------------------------------------------------------- masking

/// Shortens a secret for display. 对应 `sdkMgmtMaskSecret`。
///
/// Short secrets keep one character at each end, longer ones four. 旧实现按字节切片；这里按字符切片，
/// bytes; this slices characters, which is the same thing for the ASCII that
/// credentials actually are and cannot panic on the ones that are not.
#[must_use]
pub fn mask_secret(secret: &str) -> String {
    let secret = secret.trim();
    if secret.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = secret.chars().collect();
    let keep = if chars.len() <= 8 { 1 } else { 4 };
    let head: String = chars.iter().take(keep).collect();
    let tail: String = chars[chars.len() - keep..].iter().collect();
    format!("{head}...{tail}")
}

/// Whether a submitted value is a masked preview rather than a real secret.
///
/// 对应 `sdkMgmtLooksMasked`。This is what stops a console round-trip — load the
/// form, save it unchanged — from overwriting a live credential with
/// `"abcd...wxyz"`.
#[must_use]
pub fn looks_masked(value: &str) -> bool {
    let value = value.trim();
    value.contains("...") || value.contains("••") || value.contains("***")
}

// ---------------------------------------------------------------- naming

/// Display name for a credential in a provider pool. 对应 `sdkMgmtAuthName`。
#[must_use]
pub fn display_name(record: &AuthRecord, index: usize) -> String {
    if record.label.is_empty() {
        format!("Channel-{}", index + 1)
    } else {
        record.label.clone()
    }
}

/// The name auth-file endpoints address a credential by.
///
/// 对应 `sdkMgmtAuthStableName` —— label, then the account email, then the id,
/// and only then the positional `Channel-N`. The ordering matters: `PUT` and
/// `DELETE` on `/auth-files` look credentials up by this string, so it has to
/// stay the same across a restart, which a position-derived name would not.
#[must_use]
pub fn stable_name(record: &AuthRecord, index: usize) -> String {
    if !record.label.is_empty() {
        return record.label.clone();
    }
    let email = metadata_string(record, "email");
    if !email.is_empty() {
        return email;
    }
    if !record.id.is_empty() {
        return record.id.clone();
    }
    display_name(record, index)
}

/// RFC3339 in UTC, or `""` for the zero time. 对应 `sdkMgmtTimeString`。
///
/// 旧实现的零值 `time.Time` 是公元 1 年；Rust 实体把 NULL 列解码成 Unix epoch，
/// 所以两个哨兵值都当作「从未」。
#[must_use]
pub fn time_string(value: DateTime<Utc>) -> String {
    if value == DateTime::UNIX_EPOCH || value.timestamp() <= 0 {
        return String::new();
    }
    value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Same, for a column that is genuinely nullable.
#[must_use]
pub fn opt_time_string(value: Option<DateTime<Utc>>) -> String {
    value.map(time_string).unwrap_or_default()
}

// ---------------------------------------------------------------- models

/// Every model this credential is known to serve, sorted and de-duplicated.
///
/// 对应 `sdkMgmtAuthModels` —— the union of the per-model state keys and the
/// `models` metadata, which may be a list or a comma-separated string.
#[must_use]
pub fn models(record: &AuthRecord) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |value: &str| {
        let value = value.trim();
        if !value.is_empty() && !out.iter().any(|seen| seen == value) {
            out.push(value.to_owned());
        }
    };

    if let Some(states) = record.model_states.as_object() {
        for model in states.keys() {
            push(model);
        }
    }
    match metadata(record, "models") {
        Some(Value::Array(items)) => {
            for item in items {
                push(&scalar_to_string(item));
            }
        }
        Some(Value::String(text)) => {
            for item in text.split(',') {
                push(item);
            }
        }
        _ => {}
    }
    out.sort_unstable();
    out
}

/// 对标 `fmt.Sprint` 对 JSON 标量的表现：字符串不带引号，其余用自然呈现。
fn scalar_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------- quota

/// Reads a quota field, accepting either the snake_case or the PascalCase
/// spelling.
///
/// The SDK owned the struct that produced this `jsonb`, and its tags are not
/// knowable from this repository, so both are tried rather than guessing one
/// and silently reading `null` from every existing row.
fn quota_field<'a>(record: &'a AuthRecord, snake: &str, pascal: &str) -> Option<&'a Value> {
    let object = record.quota.as_object()?;
    object.get(snake).or_else(|| object.get(pascal))
}

/// Whether the provider has rate-limited this credential.
#[must_use]
pub fn quota_exceeded(record: &AuthRecord) -> bool {
    quota_field(record, "exceeded", "Exceeded")
        .and_then(Value::as_bool)
        .unwrap_or_default()
}

/// Provider-supplied explanation for the quota state.
#[must_use]
pub fn quota_reason(record: &AuthRecord) -> String {
    quota_field(record, "reason", "Reason")
        .map(scalar_to_string)
        .unwrap_or_default()
}

/// When the credential is expected to become usable again.
#[must_use]
pub fn quota_next_recover_at(record: &AuthRecord) -> String {
    quota_field(record, "next_recover_at", "NextRecoverAt")
        .map(scalar_to_string)
        .map(|raw| normalize_timestamp(&raw))
        .unwrap_or_default()
}

/// How many times the backoff has doubled.
#[must_use]
pub fn quota_backoff_level(record: &AuthRecord) -> i64 {
    quota_field(record, "backoff_level", "BackoffLevel")
        .and_then(Value::as_i64)
        .unwrap_or_default()
}

/// Renders a stored timestamp the way [`time_string`] renders a typed one.
///
/// The column holds whatever the SDK marshalled — usually RFC3339 already. A
/// zero time（公元 1 年）becomes `""`; anything unparseable is passed through
/// so an operator can still see what is stored.
fn normalize_timestamp(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    match DateTime::parse_from_rfc3339(raw) {
        Ok(parsed) => time_string(parsed.with_timezone(&Utc)),
        Err(_) => raw.to_owned(),
    }
}

// ---------------------------------------------------------------- serialisers

/// One entry of a provider's API-key pool. 对应 `sdkMgmtSerializeAuth`。
///
/// Note the hyphenated keys (`api-key`, `base-url`, `excluded-models`): this
/// payload mirrors the SDK's own config file, and
/// `frontend/src/features/admin-proxy/providerConfig.ts` reads exactly these.
#[must_use]
pub fn serialize_pool_entry(record: &AuthRecord, index: usize) -> Value {
    json!({
        "id": record.id,
        "auth_id": record.id,
        "index": index,
        "name": display_name(record, index),
        "api-key": mask_secret(&api_key(record)),
        "base-url": attr(record, &["base_url", "base-url"]),
        "models-url": attr(record, &["models_url", "models-url"]),
        "proxy-url": proxy_url(record),
        "prefix": record.prefix,
        "priority": attr_number(record, "priority"),
        "disabled": record.disabled || record.status == gw_authcore::AuthStatus::Disabled,
        "headers": metadata(record, "headers"),
        "models": metadata(record, "models"),
        "excluded-models": metadata(record, "excluded_models"),
        "websockets": attr_bool(record, "websockets"),
        "experimental-cch-signing": attr_bool(record, "experimental_cch_signing"),
        "status": record.status.as_str(),
        "unavailable": record.unavailable,
        "success": 0,
        "failed": 0,
        "created_at": record.created_at,
        "updated_at": record.updated_at,
    })
}

/// One row of `/auth-files`. 对应 `sdkMgmtSerializeAuthFile`。
///
/// Unlike the pool entry this payload is snake_case, because it is the panel's
/// own view rather than a mirror of the SDK config file. Secrets appear only as
/// `has_*` booleans plus masked previews.
#[must_use]
pub fn serialize_auth_file(record: &AuthRecord, index: usize) -> Value {
    let models = models(record);
    let mut item = json!({
        "id": record.id,
        "auth_id": record.id,
        "name": stable_name(record, index),
        "label": record.label,
        "provider": record.provider,
        "type": record.provider,
        "status": record.status.as_str(),
        "status_message": record.status_message,
        "disabled": record.disabled,
        "unavailable": record.unavailable,
        "email": metadata_string(record, "email"),
        "runtime_only": attr(record, &["runtime_only"]).eq_ignore_ascii_case("true"),
        "oauth": attr(record, &["oauth"]).eq_ignore_ascii_case("true"),
        "has_api_key": has_metadata(record, "api_key"),
        "has_access_token": has_metadata(record, "access_token"),
        "has_refresh_token": has_metadata(record, "refresh_token"),
        "has_service_account": has_metadata(record, "service_account"),
        "prefix": record.prefix,
        "proxy_url": proxy_url(record),
        "base_url": attr(record, &["base_url", "base-url"]),
        "project_id": attr(record, &["project_id"]),
        "location": attr(record, &["location"]),
        "created_at": time_string(record.created_at),
        "updated_at": time_string(record.updated_at),
        "last_refresh": opt_time_string(record.last_refreshed_at),
        "success": 0,
        "failed": 0,
        "recent_requests": Value::Array(Vec::new()),
        "quota_exceeded": quota_exceeded(record),
        "next_recover_at": quota_next_recover_at(record),
        "models": models,
        "model_count": models.len(),
    });

    // The preview keys are conditional —— 它们的*缺席*正是控制台判断
    // 凭证根本没有携带这种秘密的方式。
    let object = item.as_object_mut().expect("json! built an object");
    let api_key = api_key(record);
    if !api_key.is_empty() {
        object.insert("api_key_preview".to_owned(), json!(mask_secret(&api_key)));
    }
    let access = metadata_string(record, "access_token");
    if !access.is_empty() {
        object.insert(
            "access_token_preview".to_owned(),
            json!(mask_secret(&access)),
        );
    }
    let refresh = metadata_string(record, "refresh_token");
    if !refresh.is_empty() {
        object.insert(
            "refresh_token_preview".to_owned(),
            json!(mask_secret(&refresh)),
        );
    }
    let account_id = metadata_string(record, "account_id");
    if !account_id.is_empty() {
        object.insert("account_id".to_owned(), json!(account_id));
        object.insert("chatgpt_account_id".to_owned(), json!(account_id));
    }
    item
}

/// One row of `/auth-files/quota`. 对应 `sdkMgmtSerializeAuthQuota`。
///
/// Two of the fields are emitted under **both** spellings. That is not a
/// mistake in the port: the handler literally writes `exceeded` and
/// `Exceeded`, `next_recover_at` and `NextRecoverAt`, because the console was
/// written against one shape and the SDK's own admin UI against the other.
#[must_use]
pub fn serialize_quota(record: &AuthRecord, index: usize) -> Value {
    let exceeded = quota_exceeded(record);
    let next_recover_at = quota_next_recover_at(record);
    json!({
        "id": record.id,
        "auth_id": record.id,
        "name": stable_name(record, index),
        "provider": record.provider,
        "exceeded": exceeded,
        "Exceeded": exceeded,
        "reason": quota_reason(record),
        "next_recover_at": next_recover_at,
        "NextRecoverAt": next_recover_at,
        "backoff_level": quota_backoff_level(record),
    })
}

/// Rows of `/auth-files/models`. 对应 `sdkMgmtSerializeAuthModels`。
///
/// One row per model the credential declares, then one more per model that has
/// a *state* but was not declared — those are the ones the proxy discovered by
/// being rejected, and they are exactly what an operator debugging a dead model
/// is looking for.
#[must_use]
pub fn serialize_models(record: &AuthRecord, index: usize) -> Vec<Value> {
    let declared = models(record);
    let name = stable_name(record, index);
    let mut items: Vec<Value> = declared
        .iter()
        .map(|model| {
            json!({
                "id": record.id,
                "auth_id": record.id,
                "name": name,
                "provider": record.provider,
                "model": model,
                "status": record.status.as_str(),
                "disabled": record.disabled,
            })
        })
        .collect();

    let Some(states) = record.model_states.as_object() else {
        return items;
    };
    // Deterministic order: the column is a JSON object, whose iteration order
    // would otherwise leak into the response.
    let mut extra: Vec<(&String, &Value)> = states
        .iter()
        .filter(|(model, state)| !declared.contains(model) && !state.is_null())
        .collect();
    extra.sort_by_key(|(model, _)| model.as_str());

    items.extend(extra.into_iter().map(|(model, state)| {
        let state = state.as_object().cloned().unwrap_or_default();
        json!({
            "id": record.id,
            "auth_id": record.id,
            "name": name,
            "provider": record.provider,
            "model": model,
            "status": state_string(&state, "status", "Status"),
            "status_message": state_string(&state, "status_message", "StatusMessage"),
            "unavailable": state_bool(&state, "unavailable", "Unavailable"),
            "next_retry_after": state_time(&state, "next_retry_after", "NextRetryAfter"),
            "quota_exceeded": state_quota_bool(&state, "exceeded", "Exceeded"),
            "next_recover_at": state_quota_time(&state, "next_recover_at", "NextRecoverAt"),
            "updated_at": state_time(&state, "updated_at", "UpdatedAt"),
        })
    }));
    items
}

fn state_get<'a>(state: &'a Map<String, Value>, snake: &str, pascal: &str) -> Option<&'a Value> {
    state.get(snake).or_else(|| state.get(pascal))
}

fn state_string(state: &Map<String, Value>, snake: &str, pascal: &str) -> String {
    state_get(state, snake, pascal)
        .map(scalar_to_string)
        .unwrap_or_default()
}

fn state_bool(state: &Map<String, Value>, snake: &str, pascal: &str) -> bool {
    state_get(state, snake, pascal)
        .and_then(Value::as_bool)
        .unwrap_or_default()
}

fn state_time(state: &Map<String, Value>, snake: &str, pascal: &str) -> String {
    normalize_timestamp(&state_string(state, snake, pascal))
}

/// The per-model quota block, which nests one level deeper than the rest.
fn state_quota(state: &Map<String, Value>) -> Option<&Map<String, Value>> {
    state_get(state, "quota", "Quota")?.as_object()
}

fn state_quota_bool(state: &Map<String, Value>, snake: &str, pascal: &str) -> bool {
    state_quota(state).is_some_and(|quota| state_bool(quota, snake, pascal))
}

fn state_quota_time(state: &Map<String, Value>, snake: &str, pascal: &str) -> String {
    state_quota(state)
        .map(|quota| state_time(quota, snake, pascal))
        .unwrap_or_default()
}

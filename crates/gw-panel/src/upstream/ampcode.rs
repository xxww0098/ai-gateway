//! `/ampcode**` — the Ampcode upstream's own settings blob.
//!
//! 对应 `SDKMgmtAmpcode*Handler`（共十二个）及
//! `loadAmpcodeConfig` / `saveAmpcodeConfig` / `normalizeAmpcodeInputKeys` /
//! `normalizeAmpcodeResponse`。
//!
//! # Why every key exists twice
//!
//! Five settings are addressed as `upstream-url` by the SDK and as
//! `upstream_url` by the console. 这里存储 **hyphenated** 写法（输入
//! normalisation deletes the snake_case one) and returns **both** (response
//! normalisation adds it back). Anything else and one of the two readers goes
//! blank; see [`KNOWN_KEY_PAIRS`].
//!
//! # Why the singular and plural keys are separate endpoints
//!
//! `upstream-api-key` (one string) and `upstream-api-keys` (a list of objects)
//! are different settings with a one-character difference in name. They are
//! kept as separate handlers rather than merged behind a shared helper that
//! would make the confusion easy to write.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Map, Value, json};

use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

const ERR_BAD_REQUEST: i32 = 4000;
const ERR_INTERNAL: i32 = 5000;

/// The single row the blob lives in（固定取 `id = 1` 这一行）。
const CONFIG_ROW_ID: i64 = 1;

/// Settings that exist under both a hyphenated and a snake_case spelling.
/// 对应 `ampcodeKnownKeyPairs`。
pub const KNOWN_KEY_PAIRS: [(&str, &str); 5] = [
    ("upstream-url", "upstream_url"),
    ("upstream-api-key", "upstream_api_key"),
    ("upstream-api-keys", "upstream_api_keys"),
    ("force-model-mappings", "force_model_mappings"),
    ("model-mappings", "model_mappings"),
];

// ---------------------------------------------------------------- storage

/// 对应 `loadAmpcodeConfig` —— a missing row, a NULL blob and unparseable JSON
/// all read as an empty config rather than an error, because the settings page
/// has to open before it can be filled in.
async fn load(state: &PanelState) -> Result<Map<String, Value>, sqlx::Error> {
    let stored: Option<Option<Value>> =
        sqlx::query_scalar("SELECT config_data FROM ampcode_configs WHERE id = $1")
            .bind(CONFIG_ROW_ID)
            .fetch_optional(&state.pg)
            .await?;
    Ok(stored
        .flatten()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default())
}

/// 对应 `saveAmpcodeConfig`。
async fn save(state: &PanelState, config: &Map<String, Value>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ampcode_configs (id, config_data, created_at, updated_at) \
         VALUES ($1, $2, NOW(), NOW()) \
         ON CONFLICT (id) DO UPDATE SET config_data = EXCLUDED.config_data, updated_at = NOW()",
    )
    .bind(CONFIG_ROW_ID)
    .bind(Value::Object(config.clone()))
    .execute(&state.pg)
    .await
    .map(|_| ())
}

fn load_failure(error: &sqlx::Error) -> Response {
    tracing::error!(%error, "failed to load ampcode config");
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        ERR_INTERNAL,
        "failed to load ampcode config",
    )
}

fn save_failure(error: &sqlx::Error) -> Response {
    tracing::error!(%error, "failed to save ampcode config");
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        ERR_INTERNAL,
        "failed to save ampcode config",
    )
}

// ---------------------------------------------------------------- key shapes

/// Folds a snake_case key into its hyphenated twin. 对应 `normalizeAmpcodeInputKeys`。
///
/// The snake_case key is **removed**, so only one spelling is ever stored — and
/// an existing hyphenated value wins, because a client that sent both meant the
/// canonical one.
pub fn normalize_input(payload: &mut Map<String, Value>) {
    for (hyphen, snake) in KNOWN_KEY_PAIRS {
        let Some(value) = payload.remove(snake) else {
            continue;
        };
        payload.entry(hyphen.to_owned()).or_insert(value);
    }
}

/// Echoes each known setting under both spellings. 对应 `normalizeAmpcodeResponse`。
#[must_use]
pub fn normalize_response(config: &Map<String, Value>) -> Map<String, Value> {
    let mut out = config.clone();
    for (hyphen, snake) in KNOWN_KEY_PAIRS {
        if let Some(value) = out.get(hyphen).cloned() {
            out.entry(snake.to_owned()).or_insert(value);
        } else if let Some(value) = out.get(snake).cloned() {
            out.insert(hyphen.to_owned(), value);
        }
    }
    out
}

// ---------------------------------------------------------------- whole blob

/// `GET /ampcode`. 对应 `SDKMgmtAmpcodeGetHandler`。
pub async fn get(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    match load(&state).await {
        Ok(config) => ok(Value::Object(normalize_response(&config))),
        Err(error) => load_failure(&error),
    }
}

/// `PUT /ampcode`. 对应 `SDKMgmtAmpcodePutHandler`。
///
/// The body may be the settings themselves or `{"ampcode": {...}}`. A present
/// but non-object `ampcode` key is rejected rather than being treated as a
/// setting named "ampcode" — that would silently store the wrong shape.
pub async fn put(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    let Some(axum::Json(raw)) = body else {
        return invalid_body();
    };
    let Some(object) = raw.as_object() else {
        return invalid_body();
    };

    let mut payload = match object.get("ampcode") {
        Some(Value::Object(inner)) => inner.clone(),
        Some(_) => {
            return err(
                StatusCode::BAD_REQUEST,
                ERR_BAD_REQUEST,
                "invalid ampcode wrapper: expected object",
            );
        }
        None => object.clone(),
    };
    normalize_input(&mut payload);

    let mut config = match load(&state).await {
        Ok(config) => config,
        Err(error) => return load_failure(&error),
    };
    for (key, value) in payload {
        config.insert(key, value);
    }
    if let Err(error) = save(&state, &config).await {
        return save_failure(&error);
    }
    ok(Value::Object(normalize_response(&config)))
}

fn invalid_body() -> Response {
    err(
        StatusCode::BAD_REQUEST,
        ERR_BAD_REQUEST,
        "invalid request body",
    )
}

// ---------------------------------------------------------------- list keys

/// Reads a list-valued setting, normalising "absent" and "not a list" to `[]`.
fn list_of(config: &Map<String, Value>, key: &str) -> Vec<Value> {
    match config.get(key) {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// Accepts either a bare array or `{"value": [...]}`——即
/// `json.Unmarshal(raw, &wrapped)` 后的 fall-through 逻辑，两个 list PUT 共用。
///
/// # Errors
/// The operator-facing message for the 400.
fn array_body(raw: &Value, what: &str) -> Result<Vec<Value>, String> {
    if let Some(object) = raw.as_object()
        && let Some(value) = object.get("value")
    {
        return match value {
            Value::Array(items) => Ok(items.clone()),
            _ => Err(format!("invalid {what}: value must be an array")),
        };
    }
    match raw {
        Value::Array(items) => Ok(items.clone()),
        _ => Err(format!("invalid {what}: expected array or {{value:array}}")),
    }
}

/// `GET /ampcode/model-mappings`. 对应 `SDKMgmtAmpcodeModelMappingsGetHandler`。
///
/// Returned under two keys because the console reads `mappings` and the SDK's
/// own config uses `model-mappings`.
pub async fn get_model_mappings(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    match load(&state).await {
        Ok(config) => {
            let mappings = list_of(&config, "model-mappings");
            ok(json!({"model-mappings": mappings, "mappings": mappings}))
        }
        Err(error) => load_failure(&error),
    }
}

/// `PUT /ampcode/model-mappings`. 对应 `SDKMgmtAmpcodeModelMappingsPutHandler`。
pub async fn put_model_mappings(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    put_list(
        state,
        body,
        "model-mappings",
        |mappings| json!({"model-mappings": mappings, "mappings": mappings}),
    )
    .await
}

/// `DELETE /ampcode/model-mappings`. 对应 `SDKMgmtAmpcodeModelMappingsDeleteHandler`。
pub async fn delete_model_mappings(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    match remove_key(&state, "model-mappings").await {
        Ok(()) => ok(json!({"model-mappings": [], "mappings": []})),
        Err(response) => response,
    }
}

/// `GET /ampcode/upstream-api-keys`. 对应 `SDKMgmtAmpcodeUpstreamAPIKeysGetHandler`。
pub async fn get_upstream_api_keys(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    match load(&state).await {
        Ok(config) => ok(json!({"upstream-api-keys": list_of(&config, "upstream-api-keys")})),
        Err(error) => load_failure(&error),
    }
}

/// `PUT /ampcode/upstream-api-keys`. 对应 `SDKMgmtAmpcodeUpstreamAPIKeysPutHandler`。
pub async fn put_upstream_api_keys(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    put_list(
        state,
        body,
        "upstream-api-keys",
        |entries| json!({"upstream-api-keys": entries}),
    )
    .await
}

/// `DELETE /ampcode/upstream-api-keys`。
/// 对应 `SDKMgmtAmpcodeUpstreamAPIKeysDeleteHandler`。
///
/// Unlike the other deletes this one is **selective**: the body lists the keys
/// to remove and everything else survives. An empty list is a 400 rather than a
/// no-op, so a malformed request cannot look like a successful deletion.
pub async fn delete_upstream_api_keys(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    let removals: Vec<String> = body
        .and_then(|axum::Json(raw)| {
            raw.as_object()
                .and_then(|object| object.get("value").cloned())
        })
        .and_then(|value| match value {
            Value::Array(items) => Some(items),
            _ => None,
        })
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if removals.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "invalid request: expected {value:[...upstream-key...]}",
        );
    }

    let mut config = match load(&state).await {
        Ok(config) => config,
        Err(error) => return load_failure(&error),
    };
    // Entries are objects keyed by `upstream-api-key`; anything else in the
    // list is left alone rather than being swept up by a failed match.
    let filtered: Vec<Value> = list_of(&config, "upstream-api-keys")
        .into_iter()
        .filter(|entry| {
            entry
                .as_object()
                .and_then(|entry| entry.get("upstream-api-key"))
                .and_then(Value::as_str)
                .is_none_or(|key| !removals.iter().any(|removed| removed == key))
        })
        .collect();

    config.insert("upstream-api-keys".to_owned(), json!(filtered));
    if let Err(error) = save(&state, &config).await {
        return save_failure(&error);
    }
    ok(json!({"upstream-api-keys": filtered}))
}

// ---------------------------------------------------------------- scalar keys

/// `PUT /ampcode/upstream-url`. 对应 `SDKMgmtAmpcodeUpstreamURLPutHandler`。
pub async fn put_upstream_url(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    put_scalar(state, body, "upstream-url").await
}

/// `DELETE /ampcode/upstream-url`. 对应 `SDKMgmtAmpcodeUpstreamURLDeleteHandler`。
pub async fn delete_upstream_url(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    delete_scalar(state, "upstream-url").await
}

/// `PUT /ampcode/upstream-api-key`. 对应 `SDKMgmtAmpcodeUpstreamAPIKeyPutHandler`。
pub async fn put_upstream_api_key(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    put_scalar(state, body, "upstream-api-key").await
}

/// `DELETE /ampcode/upstream-api-key`。
/// 对应 `SDKMgmtAmpcodeUpstreamAPIKeyDeleteHandler`。
pub async fn delete_upstream_api_key(
    State(state): State<PanelState>,
    _admin: AdminUser,
) -> Response {
    delete_scalar(state, "upstream-api-key").await
}

// ---------------------------------------------------------------- shared

/// Stores a list under `key` and answers with `render`.
async fn put_list(
    state: PanelState,
    body: Option<axum::Json<Value>>,
    key: &str,
    render: impl Fn(&[Value]) -> Value,
) -> Response {
    let Some(axum::Json(raw)) = body else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "cannot read request body",
        );
    };
    let items = match array_body(&raw, key) {
        Ok(items) => items,
        Err(message) => return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, message),
    };

    let mut config = match load(&state).await {
        Ok(config) => config,
        Err(error) => return load_failure(&error),
    };
    config.insert(key.to_owned(), json!(items));
    if let Err(error) = save(&state, &config).await {
        return save_failure(&error);
    }
    ok(render(&items))
}

/// Stores `body.value` as a string under `key`; answers with the whole blob
/// ——两个 scalar PUT 都这么做。
async fn put_scalar(state: PanelState, body: Option<axum::Json<Value>>, key: &str) -> Response {
    let Some(axum::Json(raw)) = body else {
        return invalid_body();
    };
    let Some(value) = raw.as_object().and_then(|object| object.get("value")) else {
        // 对标 `struct{ Value string }` 的绑定：没有 `value` 字段的 body
        // 也能解析并存入空串，而不是报错。
        return store_scalar(state, key, String::new()).await;
    };
    let Some(text) = value.as_str() else {
        return invalid_body();
    };
    store_scalar(state, key, text.to_owned()).await
}

async fn store_scalar(state: PanelState, key: &str, value: String) -> Response {
    let mut config = match load(&state).await {
        Ok(config) => config,
        Err(error) => return load_failure(&error),
    };
    config.insert(key.to_owned(), json!(value));
    if let Err(error) = save(&state, &config).await {
        return save_failure(&error);
    }
    ok(Value::Object(normalize_response(&config)))
}

async fn delete_scalar(state: PanelState, key: &str) -> Response {
    match remove_key(&state, key).await {
        Ok(()) => match load(&state).await {
            Ok(config) => ok(Value::Object(normalize_response(&config))),
            Err(error) => load_failure(&error),
        },
        Err(response) => response,
    }
}

/// Drops `key` from the blob.
async fn remove_key(state: &PanelState, key: &str) -> Result<(), Response> {
    let mut config = load(state).await.map_err(|error| load_failure(&error))?;
    config.remove(key);
    save(state, &config)
        .await
        .map_err(|error| save_failure(&error))
}

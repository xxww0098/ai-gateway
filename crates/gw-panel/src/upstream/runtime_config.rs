//! `/config` and the per-key convenience routes — the gateway's runtime
//! settings blob.
//!
//! 对应 `SDKMgmtConfig{Get,Put}Handler`、`SDKMgmtSDKConfigPatchHandler`、
//! 三个 handler 工厂（`sdkMgmtConfigGetHandlerFn`、
//! `sdkMgmtConfigSetHandlerFn`、`sdkMgmtConfigDeleteHandlerFn`）、三个特殊
//! getter，以及键形状辅助函数（`sdkMgmtCamelToHyphen`、
//! `sdkMgmtHyphenToCamel`、`sdkMgmtNormalizeConfigMap`、
//! `sdkMgmtExpandConfigAliases`）。
//!
//! # Why three spellings of every key
//!
//! The settings blob has been written by an SDK that used `hyphen-case`, by a
//! console that sends `camelCase`, and by hand in `snake_case`. Rather than
//! migrate the stored JSON, 这里 **normalises on write**（everything becomes
//! hyphen-case) and **expands on read** (each hyphenated key is echoed under
//! all three spellings). The console's `configValue()` then finds a value
//! whichever spelling it asks for. Both halves are reproduced: dropping the
//! read-side expansion would blank half the settings page.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Map, Value, json};

use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

/// `provider_configs.provider` the settings blob lives under —— 即
/// `sdkMgmtReadConfig` 里的 `Where("provider = ?", "sdk_config")`。
pub const CONFIG_PROVIDER: &str = "sdk_config";

const ERR_BAD_REQUEST: i32 = 4000;
const ERR_READ_FAILED: i32 = 5004;
const ERR_WRITE_FAILED: i32 = 5005;

/// Fallback when `routing-strategy` was never set —— 即
/// `sdkMgmtConfigGetRoutingStrategyFn` 里的 `"round-robin"` 字面量。
const DEFAULT_ROUTING_STRATEGY: &str = "round-robin";

/// Fallback when `logs-max-total-size-mb` was never set.
const DEFAULT_LOGS_MAX_TOTAL_SIZE_MB: i64 = 100;

// ---------------------------------------------------------------- storage

/// Reads the settings blob, treating "no row" and "unparseable row" alike as
/// an empty config. 对应 `sdkMgmtReadConfig`。
async fn read_config(state: &PanelState) -> Result<Map<String, Value>, sqlx::Error> {
    let stored: Option<Option<Value>> =
        sqlx::query_scalar("SELECT config_data FROM provider_configs WHERE provider = $1")
            .bind(CONFIG_PROVIDER)
            .fetch_optional(&state.pg)
            .await?;
    Ok(stored
        .flatten()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default())
}

/// Writes the settings blob. 对应 `sdkMgmtWriteConfig`。
async fn write_config(state: &PanelState, config: &Map<String, Value>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO provider_configs (provider, config_data, created_at, updated_at) \
         VALUES ($1, $2, NOW(), NOW()) \
         ON CONFLICT (provider) DO UPDATE SET config_data = EXCLUDED.config_data, \
           updated_at = NOW()",
    )
    .bind(CONFIG_PROVIDER)
    .bind(Value::Object(config.clone()))
    .execute(&state.pg)
    .await
    .map(|_| ())
}

fn read_failure(error: &sqlx::Error) -> Response {
    tracing::error!(%error, "failed to read SDK config");
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        ERR_READ_FAILED,
        "failed to read SDK config",
    )
}

fn write_failure(error: &sqlx::Error) -> Response {
    tracing::error!(%error, "failed to save SDK config");
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        ERR_WRITE_FAILED,
        "failed to save SDK config",
    )
}

// ---------------------------------------------------------------- key shapes

/// `fooBar` → `foo-bar`. 对应 `sdkMgmtCamelToHyphen`。
///
/// ASCII-only（与原实现一致）：these are config keys, and treating a
/// non-ASCII uppercase letter as a word boundary would change existing keys.
#[must_use]
pub fn camel_to_hyphen(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for (index, byte) in key.bytes().enumerate() {
        if byte.is_ascii_uppercase() {
            if index > 0 {
                out.push('-');
            }
            out.push(byte.to_ascii_lowercase() as char);
        } else {
            out.push(byte as char);
        }
    }
    out
}

/// `foo-bar` → `fooBar`. 对应 `sdkMgmtHyphenToCamel`。
#[must_use]
pub fn hyphen_to_camel(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper_next = false;
    for byte in key.bytes() {
        if byte == b'-' {
            upper_next = true;
        } else if upper_next {
            out.push(byte.to_ascii_uppercase() as char);
            upper_next = false;
        } else {
            out.push(byte as char);
        }
    }
    out
}

/// The canonical spelling a key is stored under. 对应 `sdkMgmtNormalizeConfigKey`。
///
/// camelCase wins first; only a key that was *not* camelCase gets its
/// underscores turned into hyphens. A key that is already hyphenated passes
/// through both branches unchanged.
#[must_use]
pub fn normalize_key(key: &str) -> String {
    let hyphenated = camel_to_hyphen(key);
    if hyphenated != key {
        hyphenated
    } else {
        key.replace('_', "-")
    }
}

/// Normalises every key, recursively. 对应 `sdkMgmtNormalizeConfigMap`。
#[must_use]
pub fn normalize_config(data: &Map<String, Value>) -> Map<String, Value> {
    data.iter()
        .map(|(key, value)| {
            let value = match value {
                Value::Object(nested) => Value::Object(normalize_config(nested)),
                other => other.clone(),
            };
            (normalize_key(key), value)
        })
        .collect()
}

/// Echoes each hyphenated key under its camelCase and snake_case spellings.
///
/// 对应 `sdkMgmtExpandConfigAliases`. Only the top level is expanded ——
/// nested settings are read whole by the console, not key by key.
#[must_use]
pub fn expand_aliases(config: &Map<String, Value>) -> Map<String, Value> {
    if config.is_empty() {
        return config.clone();
    }
    let mut out = Map::with_capacity(config.len() * 2);
    for (key, value) in config {
        out.insert(key.clone(), value.clone());
        if !key.contains('-') {
            continue;
        }
        let camel = hyphen_to_camel(key);
        if camel != *key {
            out.insert(camel, value.clone());
        }
        let snake = key.replace('-', "_");
        if snake != *key {
            out.insert(snake, value.clone());
        }
    }
    out
}

// ---------------------------------------------------------------- whole blob

/// `GET /config`. 对应 `SDKMgmtConfigGetHandler`。
pub async fn get_config(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    match read_config(&state).await {
        Ok(config) => ok(Value::Object(expand_aliases(&config))),
        Err(error) => read_failure(&error),
    }
}

/// `PUT /config` and `PATCH /admin/sdk-config`.
///
/// 对应 `SDKMgmtConfigPutHandler` 和 `SDKMgmtSDKConfigPatchHandler`，which have
/// identical bodies — the second exists only because it is mounted outside the
/// `/admin/sdk-management` group. Both are merges, not replacements: keys the
/// body omits keep their stored values.
pub async fn put_config(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<Value>>,
) -> Response {
    let Some(axum::Json(incoming)) = body else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "invalid JSON body",
        );
    };
    let Some(incoming) = incoming.as_object() else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "invalid JSON body",
        );
    };

    let mut config = match read_config(&state).await {
        Ok(config) => config,
        Err(error) => return read_failure(&error),
    };
    for (key, value) in normalize_config(incoming) {
        config.insert(key, value);
    }
    if let Err(error) = write_config(&state, &config).await {
        return write_failure(&error);
    }
    ok(json!({"message": "updated"}))
}

// ---------------------------------------------------------------- one key

/// One convenience route's key, plus the extra spellings its getter echoes.
///
/// 原实现用三个 closure 工厂逐个构造；这里用一张表让十八条路由更可读，
/// 并让哪些有默认值一目了然。
#[derive(Debug, Clone, Copy)]
pub struct ConfigKey {
    /// Canonical, hyphen-case key in the stored blob.
    pub key: &'static str,
    /// Extra keys the GET response repeats the value under.
    pub aliases: &'static [&'static str],
    /// Value to serve when the key was never set.
    pub default: Option<ConfigDefault>,
}

/// The two kinds of fallback the special getters carry.
#[derive(Debug, Clone, Copy)]
pub enum ConfigDefault {
    Text(&'static str),
    Number(i64),
}

impl ConfigDefault {
    fn value(self) -> Value {
        match self {
            Self::Text(text) => json!(text),
            Self::Number(number) => json!(number),
        }
    }
}

/// `GET` one config key. 对应 `sdkMgmtConfigGetHandlerFn` 和三个
/// hand-written getters, which differ only in aliases and defaults.
pub async fn get_key(state: &PanelState, spec: ConfigKey) -> Response {
    let config = match read_config(state).await {
        Ok(config) => config,
        Err(error) => return read_failure(&error),
    };
    let value = config.get(spec.key).cloned().unwrap_or(Value::Null);
    // A stored `null` is treated as unset（对标 `!ok || val == nil`）。
    let value = match (value, spec.default) {
        (Value::Null, Some(default)) => default.value(),
        (value, _) => value,
    };

    let mut body = Map::new();
    body.insert(spec.key.to_owned(), value.clone());
    for alias in spec.aliases {
        body.insert((*alias).to_owned(), value.clone());
    }
    ok(Value::Object(body))
}

/// `PUT` one config key. 对应 `sdkMgmtConfigSetHandlerFn`。
///
/// The body is `{"value": …}` and the value is stored verbatim — including
/// `null`, which is how the console clears a setting without deleting the key.
pub async fn set_key(state: &PanelState, key: &str, body: Option<axum::Json<Value>>) -> Response {
    let Some(axum::Json(payload)) = body else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            r#"invalid JSON body: {"value": ...} expected"#,
        );
    };
    let Some(payload) = payload.as_object() else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            r#"invalid JSON body: {"value": ...} expected"#,
        );
    };

    let mut config = match read_config(state).await {
        Ok(config) => config,
        Err(error) => return read_failure(&error),
    };
    config.insert(
        key.to_owned(),
        payload.get("value").cloned().unwrap_or(Value::Null),
    );
    if let Err(error) = write_config(state, &config).await {
        return write_failure(&error);
    }
    ok(json!({"message": "updated"}))
}

/// `DELETE` one config key. 对应 `sdkMgmtConfigDeleteHandlerFn`。
pub async fn delete_key(state: &PanelState, key: &str) -> Response {
    let mut config = match read_config(state).await {
        Ok(config) => config,
        Err(error) => return read_failure(&error),
    };
    config.remove(key);
    if let Err(error) = write_config(state, &config).await {
        return write_failure(&error);
    }
    ok(json!({"message": "deleted"}))
}

// ---------------------------------------------------------------- the table

/// Keys reachable through a plain `GET`/`PUT` pair with no aliases.
///
/// 原实现逐条注册；these key 的形状一致，所以
/// live in one table that the router walks.
pub const PLAIN_KEYS: [&str; 7] = [
    "debug",
    "request-retry",
    "max-retry-interval",
    "request-log",
    "logging-to-file",
    "ws-auth",
    "usage-statistics-enabled",
];

/// The one key with a `PUT`/`DELETE` pair and **no** `GET`.
///
/// 这个 key 只注册了 `PUT`/`DELETE` 两个动词，没有 `GET`。它被排除在
/// [`PLAIN_KEYS`] 之外，而不是为「对称」补一个 `GET`：多出的一条路由本身就是
/// 契约变化，而控制台从不请求这个 key。
pub const PROXY_URL_KEY: &str = "proxy-url";

/// The routing strategy getter. 对应 `sdkMgmtConfigGetRoutingStrategyFn`。
///
/// Note the response key `strategy`, which is neither spelling of the stored
/// key: the console reads that one.
pub const ROUTING_STRATEGY: ConfigKey = ConfigKey {
    key: "routing-strategy",
    aliases: &["strategy", "routingStrategy"],
    default: Some(ConfigDefault::Text(DEFAULT_ROUTING_STRATEGY)),
};

/// 对应 `sdkMgmtConfigGetForceModelPrefixFn`。No default — an unset prefix is
/// `null`, which is different from an empty one.
pub const FORCE_MODEL_PREFIX: ConfigKey = ConfigKey {
    key: "force-model-prefix",
    aliases: &["forceModelPrefix"],
    default: None,
};

/// 对应 `sdkMgmtConfigGetLogsMaxSizeFn`。
pub const LOGS_MAX_TOTAL_SIZE_MB: ConfigKey = ConfigKey {
    key: "logs-max-total-size-mb",
    aliases: &["logsMaxTotalSizeMb"],
    default: Some(ConfigDefault::Number(DEFAULT_LOGS_MAX_TOTAL_SIZE_MB)),
};

/// A plain key with no aliases and no default.
#[must_use]
pub const fn plain(key: &'static str) -> ConfigKey {
    ConfigKey {
        key,
        aliases: &[],
        default: None,
    }
}

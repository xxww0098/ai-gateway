//! `/admin/model-catalog/**` — which models a channel exposes, and where its
//! model list is fetched from.
//!
//! 对应 `AdminModelCatalogModelsURLGetHandler`、
//! `AdminModelCatalogModelsURLPutHandler`、
//! `AdminModelCatalogEnsureOpenAIChannelHandler` 与
//! `AdminModelCatalogOpenAIVisibilityHandler`。
//!
//! # The sentinel row
//!
//! A channel's models URL is not a column on a channel — there is no channel
//! table. 旧实现把它存成一行 `model_catalog_entries`，其 `model_id` 就是
//! sentinel [`MODELS_URL_MODEL_ID`], which is why the URL getter filters on
//! `models_url <> ''` rather than on the model id.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

/// 对应 `apiErrorBadRequest`。
const ERR_BAD_REQUEST: i32 = 4000;
/// 对应 `apiErrorInternal`。
const ERR_INTERNAL: i32 = 5000;

/// `model_id` of the row that carries a channel's models URL instead of a real
/// model. 对应 `ModelID: "__models_url__"` 这一字面量。
pub const MODELS_URL_MODEL_ID: &str = "__models_url__";

/// Query string of `GET /admin/model-catalog/models-url`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ChannelKeyQuery {
    pub channel_key: Option<String>,
}

/// `GET /admin/model-catalog/models-url`. 对应 `AdminModelCatalogModelsURLGetHandler`。
///
/// Every miss — absent parameter, unknown channel, DB error — answers
/// `{"models_url": ""}` with 200. 旧实现同样如此：the console treats "no URL
/// configured" and "could not read it" identically, and an error body here
/// would make the settings page unopenable.
pub async fn get_models_url(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<ChannelKeyQuery>,
) -> Response {
    let channel_key = query
        .channel_key
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if channel_key.is_empty() {
        return ok(json!({"models_url": ""}));
    }

    let url: Option<String> = sqlx::query_scalar(
        "SELECT models_url FROM model_catalog_entries \
         WHERE channel_key = $1 AND models_url <> '' LIMIT 1",
    )
    .bind(channel_key)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten();

    ok(json!({"models_url": url.unwrap_or_default()}))
}

/// Body of `PUT /admin/model-catalog/models-url`.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelsUrlRequest {
    #[serde(default)]
    pub channel_key: String,
    #[serde(default)]
    pub models_url: String,
}

/// `PUT /admin/model-catalog/models-url`. 对应 `AdminModelCatalogModelsURLPutHandler`。
///
/// # 一条既成契约里的「零值即未设置」行为
///
/// 旧实现用 `Where(...).Assign(entry).FirstOrCreate(&entry)` 写入。
/// `Assign` with a *struct* goes through `Updates`, which skips zero-valued
/// fields — so submitting an empty `models_url` against an existing row is a
/// **no-op**, and the response echoes the value already stored rather than the
/// empty one that was sent. On a row that does not exist yet the insert carries
/// every field, so an empty URL is stored.
///
/// That asymmetry means "clear the URL" is not expressible through this
/// endpoint. It is reproduced rather than fixed: the frontend is not changing,
/// and silently starting to honour an empty submission would blank a channel's
/// URL for any operator who saves the form without touching that field.
pub async fn put_models_url(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<ModelsUrlRequest>>,
) -> Response {
    let Some(axum::Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "模型地址格式无效");
    };
    let channel_key = req.channel_key.trim();
    if channel_key.is_empty() {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "模型地址格式无效");
    }
    let models_url = req.models_url.trim();

    let stored: Result<String, _> = sqlx::query_scalar(
        "INSERT INTO model_catalog_entries \
           (channel_key, model_id, visible, models_url, created_at, updated_at) \
         VALUES ($1, $2, false, $3, NOW(), NOW()) \
         ON CONFLICT (channel_key, model_id) DO UPDATE SET \
           models_url = CASE WHEN $3 = '' \
                             THEN model_catalog_entries.models_url ELSE $3 END, \
           updated_at = CASE WHEN $3 = '' \
                             THEN model_catalog_entries.updated_at ELSE NOW() END \
         RETURNING models_url",
    )
    .bind(channel_key)
    .bind(MODELS_URL_MODEL_ID)
    .bind(models_url)
    .fetch_one(&state.pg)
    .await;

    match stored {
        Ok(models_url) => ok(json!({"ok": true, "models_url": models_url})),
        Err(error) => {
            tracing::error!(%error, "failed to save models url");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "保存模型地址失败，请稍后重试",
            )
        }
    }
}

/// Body of `POST /admin/model-catalog/ensure-openai-channel`.
#[derive(Debug, Clone, Deserialize)]
pub struct EnsureChannelRequest {
    #[serde(default)]
    pub channel_key: String,
    #[serde(default)]
    pub model_ids: Vec<String>,
}

/// `POST /admin/model-catalog/ensure-openai-channel`.
/// 对应 `AdminModelCatalogEnsureOpenAIChannelHandler`。
///
/// Idempotent by construction: a model already in the catalog is left exactly
/// as it is — including its visibility, which an operator may have turned off.
/// `created` counts only the rows this call inserted.
pub async fn ensure_channel(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<EnsureChannelRequest>>,
) -> Response {
    let Some(axum::Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "模型目录格式无效");
    };
    let channel_key = req.channel_key.trim();
    if channel_key.is_empty() {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "模型目录格式无效");
    }

    let mut created: i64 = 0;
    for model_id in &req.model_ids {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            continue;
        }
        let result = sqlx::query(
            "INSERT INTO model_catalog_entries \
               (channel_key, model_id, visible, models_url, created_at, updated_at) \
             VALUES ($1, $2, true, '', NOW(), NOW()) \
             ON CONFLICT (channel_key, model_id) DO NOTHING",
        )
        .bind(channel_key)
        .bind(model_id)
        .execute(&state.pg)
        .await;

        match result {
            Ok(result) => created += i64::try_from(result.rows_affected()).unwrap_or_default(),
            Err(error) => {
                tracing::error!(%error, "failed to seed model catalog");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ERR_INTERNAL,
                    "初始化模型目录失败，请稍后重试",
                );
            }
        }
    }

    ok(json!({"ok": true, "created": created}))
}

/// Body of `POST /admin/model-catalog/openai-visibility`.
#[derive(Debug, Clone, Deserialize)]
pub struct VisibilityRequest {
    #[serde(default)]
    pub channel_key: String,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub visible: bool,
}

/// `POST /admin/model-catalog/openai-visibility`.
/// 对应 `AdminModelCatalogOpenAIVisibilityHandler`。
///
/// Unlike [`put_models_url`], 旧实现在这里通过 **map** 赋值，而 map 不会
/// not skip zero values — so `visible: false` really does write `false`. That
/// difference is why hiding a model works and clearing a URL does not.
pub async fn set_visibility(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<VisibilityRequest>>,
) -> Response {
    let Some(axum::Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "可见性格式无效");
    };
    let channel_key = req.channel_key.trim();
    let model_id = req.model_id.trim();
    if channel_key.is_empty() || model_id.is_empty() {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "可见性格式无效");
    }

    let result = sqlx::query(
        "INSERT INTO model_catalog_entries \
           (channel_key, model_id, visible, models_url, created_at, updated_at) \
         VALUES ($1, $2, $3, '', NOW(), NOW()) \
         ON CONFLICT (channel_key, model_id) DO UPDATE SET \
           visible = EXCLUDED.visible, updated_at = NOW()",
    )
    .bind(channel_key)
    .bind(model_id)
    .bind(req.visible)
    .execute(&state.pg)
    .await;

    match result {
        Ok(_) => ok(json!({
            "ok": true,
            "channel_key": channel_key,
            "model_id": model_id,
            "visible": req.visible,
        })),
        Err(error) => {
            tracing::error!(%error, "failed to save model visibility");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "保存可见性失败，请稍后重试",
            )
        }
    }
}

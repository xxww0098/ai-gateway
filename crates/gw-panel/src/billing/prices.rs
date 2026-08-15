//! `/admin/pricing/**` — the four-column price table and the per-group rate
//! multipliers.
//!
//! 对应 `AdminListPricingGroupsHandler`、`AdminUpsertPricingGroupHandler`、
//! `AdminDeletePricingGroupHandler`、`AdminUpsertPricingModelHandler`。
//!
//! # The one thing that must not be got wrong
//!
//! A price upsert has to invalidate the **same** [`ModelPriceCache`] instance
//! the [`Calculator`](gw_pricing::Calculator) reads from. 既有入口的装配处
//! 就写着这一点：build two caches and an operator's
//! price edit silently never reaches billing. Here that instance arrives on
//! [`PanelState::price_cache`](crate::PanelState::price_cache); this module
//! must never construct one of its own.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AdminUser, AuthUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

/// Business code paired with 400 across the panel. 对应 `apiErrorBadRequest`。
const ERR_BAD_REQUEST: i32 = 4000;
/// Business code paired with 404. 对应 `apiErrorNotFound`。
const ERR_NOT_FOUND: i32 = 4004;
/// Business code paired with 500. 对应 `apiErrorInternal`。
const ERR_INTERNAL: i32 = 5000;

/// The group whose multiplier is the system baseline; it cannot be deleted.
/// 对应 `AdminDeletePricingGroupHandler` 里 `name == "default"` 的守卫。
const BASELINE_GROUP: &str = "default";

// ---------------------------------------------------------------- groups

/// One row of `GET /admin/pricing/groups`.
///
/// 旧实现把响应写成 `{"group_name": g.Name, "discount_rate": g.RateMultiplier}` ——
/// note the *response* names differ from the column names, and the frontend
/// (`pages/admin/pricing/AdminPricingPage.tsx`) reads exactly these two.
#[derive(Debug, Clone, Serialize)]
pub struct PricingGroup {
    pub group_name: String,
    pub discount_rate: f64,
}

/// `GET /admin/pricing/groups`. 对应 `AdminListPricingGroupsHandler`.
///
/// The payload is a bare array under `data`, not `{items: […]}`.
pub async fn list_groups(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    let rows: Result<Vec<(String, f64)>, _> =
        sqlx::query_as("SELECT name, rate_multiplier::float8 FROM groups ORDER BY name ASC")
            .fetch_all(&state.pg)
            .await;

    match rows {
        Ok(rows) => ok(rows
            .into_iter()
            .map(|(group_name, discount_rate)| PricingGroup {
                group_name,
                discount_rate,
            })
            .collect::<Vec<_>>()),
        Err(error) => {
            tracing::error!(%error, "failed to list pricing groups");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "获取分组失败，请稍后重试",
            )
        }
    }
}

/// Body of `POST /admin/pricing/groups`.
#[derive(Debug, Clone, Deserialize)]
pub struct UpsertGroupRequest {
    #[serde(default)]
    pub group_name: String,
    #[serde(default)]
    pub discount_rate: f64,
}

/// `POST /admin/pricing/groups`. 对应 `AdminUpsertPricingGroupHandler`.
///
/// Rejects a blank name or a non-positive multiplier, then updates the existing
/// row or inserts one. The response echoes the request, not the stored row.
pub async fn upsert_group(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<UpsertGroupRequest>>,
) -> Response {
    // 旧实现把「body 解析失败」和「body 校验失败」合并成一个 400。
    let Some(axum::Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "请求格式无效");
    };
    let name = req.group_name.trim();
    if name.is_empty() || req.discount_rate <= 0.0 {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "请求格式无效");
    }

    // `UPDATE … WHERE name = $1` then insert on zero rows, mirroring the
    // First/Update/Create branch of the old implementation. A plain `ON CONFLICT`
    // would also work, but this keeps the two failure messages it distinguishes
    // ("更新分组失败" vs "创建分组失败") attached to the operation that actually failed.
    let updated = sqlx::query("UPDATE groups SET rate_multiplier = $1 WHERE name = $2")
        .bind(req.discount_rate)
        .bind(name)
        .execute(&state.pg)
        .await;

    match updated {
        Err(error) => {
            tracing::error!(%error, "failed to update pricing group");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "更新分组失败，请稍后重试",
            );
        }
        Ok(result) if result.rows_affected() == 0 => {
            let created = sqlx::query(
                "INSERT INTO groups (name, rate_multiplier, quota_limit, created_at, updated_at) \
                 VALUES ($1, $2, 0, NOW(), NOW())",
            )
            .bind(name)
            .bind(req.discount_rate)
            .execute(&state.pg)
            .await;
            if let Err(error) = created {
                tracing::error!(%error, "failed to create pricing group");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ERR_INTERNAL,
                    "创建分组失败，请稍后重试",
                );
            }
        }
        Ok(_) => {}
    }

    ok(PricingGroup {
        group_name: name.to_owned(),
        discount_rate: req.discount_rate,
    })
}

/// `DELETE /admin/pricing/groups/:name`. 对应 `AdminDeletePricingGroupHandler`.
///
/// The baseline group is refused with 400 rather than 403: it is treated as an
/// invalid name, not as a permission problem.
pub async fn delete_group(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(name): Path<String>,
) -> Response {
    let name = name.trim();
    if name.is_empty() || name == BASELINE_GROUP {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "无效的分组名称");
    }

    match sqlx::query("DELETE FROM groups WHERE name = $1")
        .bind(name)
        .execute(&state.pg)
        .await
    {
        Err(error) => {
            tracing::error!(%error, "failed to delete pricing group");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "删除分组失败，请稍后重试",
            )
        }
        Ok(result) if result.rows_affected() == 0 => {
            err(StatusCode::NOT_FOUND, ERR_NOT_FOUND, "未找到该分组")
        }
        Ok(_) => ok(json!({"deleted": true})),
    }
}

// ---------------------------------------------------------------- models

/// Body of `POST /admin/pricing/models`.
///
/// The JSON names are the *readable* ones (`input_price_per_1m`), which is what
/// `frontend/src/features/pricing/types.ts` sends. They are deliberately NOT
/// the column names —— the column is `input_price_per1_m`，一个既有 schema 的
/// 命名产物（CONTRACT §3.5）。
#[derive(Debug, Clone, Deserialize)]
pub struct UpsertModelPriceRequest {
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub input_price_per_1m: f64,
    #[serde(default)]
    pub output_price_per_1m: f64,
    #[serde(default)]
    pub cached_input_price_per_1m: f64,
    #[serde(default)]
    pub reasoning_price_per_1m: f64,
}

impl UpsertModelPriceRequest {
    /// Names of the per-1M fields carrying a negative value, in the order the
    /// original implementation checks them.
    ///
    /// Exactly `0` is a valid price; only `< 0` is rejected. The returned names
    /// are the JSON names, which is what the structured warning log carries.
    #[must_use]
    pub fn negative_fields(&self) -> Vec<&'static str> {
        [
            ("input_price_per_1m", self.input_price_per_1m),
            ("output_price_per_1m", self.output_price_per_1m),
            ("cached_input_price_per_1m", self.cached_input_price_per_1m),
            ("reasoning_price_per_1m", self.reasoning_price_per_1m),
        ]
        .into_iter()
        .filter(|(_, value)| *value < 0.0)
        .map(|(name, _)| name)
        .collect()
    }
}

/// The committed row, serialised the way the original `Success(c, price)` helper
/// serialises `model.ModelPrice`.
///
/// `model.ModelPrice` carries **no** json tags, so `encoding/json` falls back
/// to the struct field names — the response keys are PascalCase (`ModelID`,
/// `InputPricePer1M`, …), not the snake_case the *request* uses. That asymmetry
/// looks like a bug and is not: it is the shipped contract.
#[derive(Debug, Clone, Serialize)]
#[expect(
    non_snake_case,
    reason = "field names are the wire format: the original emits its struct field names verbatim"
)]
pub struct ModelPriceResponse {
    pub ID: i64,
    pub ModelID: String,
    pub InputPricePer1M: f64,
    pub OutputPricePer1M: f64,
    pub CachedInputPricePer1M: f64,
    pub ReasoningPricePer1M: f64,
    pub CreatedAt: DateTime<Utc>,
    pub UpdatedAt: DateTime<Utc>,
}

impl From<gw_model::ModelPrice> for ModelPriceResponse {
    fn from(row: gw_model::ModelPrice) -> Self {
        Self {
            ID: row.id,
            ModelID: row.model_id,
            InputPricePer1M: row.input_price_per_1m,
            OutputPricePer1M: row.output_price_per_1m,
            CachedInputPricePer1M: row.cached_input_price_per_1m,
            ReasoningPricePer1M: row.reasoning_price_per_1m,
            CreatedAt: row.created_at,
            UpdatedAt: row.updated_at,
        }
    }
}

/// Column list shared by the upsert's `RETURNING` and the reload.
/// Spelled out so the reverse-intuitive `per1_m` names stay visible.
const PRICE_COLUMNS: &str = "id, model_id, input_price_per1_m, output_price_per1_m, \
     cached_input_price_per1_m, reasoning_price_per1_m, created_at, updated_at";

/// Writes the four columns and returns the committed row.
///
/// Split out of the handler so the property the original tests are really about —
/// "after an upsert the shared cache reports the new prices" — can be exercised
/// against a real database without standing up an HTTP stack and a Redis.
///
/// Every column is written unconditionally, which is the whole point: a price
/// of exactly `0` must be storable. 旧实现需要 `Select(...).Updates(...)` 来绕开
/// 「零值即未设置」的启发式；plain SQL has no such heuristic.
///
/// # Errors
/// Whatever the insert reports. `model_id` is expected pre-trimmed.
pub async fn upsert_price(
    pool: &sqlx::PgPool,
    model_id: &str,
    req: &UpsertModelPriceRequest,
) -> Result<gw_model::ModelPrice, sqlx::Error> {
    sqlx::query_as(&format!(
        "INSERT INTO model_prices (model_id, input_price_per1_m, output_price_per1_m, \
          cached_input_price_per1_m, reasoning_price_per1_m, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, NOW(), NOW()) \
         ON CONFLICT (model_id) DO UPDATE SET \
          input_price_per1_m = EXCLUDED.input_price_per1_m, \
          output_price_per1_m = EXCLUDED.output_price_per1_m, \
          cached_input_price_per1_m = EXCLUDED.cached_input_price_per1_m, \
          reasoning_price_per1_m = EXCLUDED.reasoning_price_per1_m, \
          updated_at = NOW() \
         RETURNING {PRICE_COLUMNS}"
    ))
    .bind(model_id)
    .bind(req.input_price_per_1m)
    .bind(req.output_price_per_1m)
    .bind(req.cached_input_price_per_1m)
    .bind(req.reasoning_price_per_1m)
    .fetch_one(pool)
    .await
}

/// `POST /admin/pricing/models`. 对应 `AdminUpsertPricingModelHandler`.
///
/// Order of operations is load-bearing:
///
/// 1. Validate. A negative price short-circuits **before** any write and before
///    any cache invalidation, so an invalid payload can never mutate runtime
///    pricing (Requirement 6.4 / 6.5).
/// 2. Upsert every one of the four columns unconditionally — a price of exactly
///    `0` must be storable.（旧实现需要 `Select(...).Updates(...)` 来绕开
///    「零值即未设置」的启发式；plain SQL has no such heuristic, so
///    the `ON CONFLICT DO UPDATE` below is already correct.）
/// 3. Invalidate the shared cache so `Calculator::estimate`/`compute` see the
///    new prices without a restart.
///
/// The validation rejection does **not** use the panel envelope: the original
/// calls `AbortWithStatusJSON(400, {"error": "invalid_price"})`, so the body
/// is a bare `{"error":"invalid_price"}`.
pub async fn upsert_model_price(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<UpsertModelPriceRequest>>,
) -> Response {
    let Some(axum::Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "模型价格格式无效");
    };
    let model_id = req.model_id.trim();
    if model_id.is_empty() {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "模型价格格式无效");
    }

    let negative = req.negative_fields();
    if !negative.is_empty() {
        tracing::warn!(
            event = "price_validation_failed",
            model_id = %model_id,
            negative_field = ?negative,
            "price_validation_failed"
        );
        // Bare body, not the panel envelope — see the doc comment.
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": "invalid_price"})),
        )
            .into_response();
    }

    let price = match upsert_price(&state.pg, model_id, &req).await {
        Ok(price) => price,
        Err(error) => {
            tracing::error!(%error, "failed to persist model price");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "保存模型价格失败，请稍后重试",
            );
        }
    };

    // Refresh the in-memory table the Calculator reads. A failure here is
    // logged loudly but still reports success: the row IS committed, and
    // returning 500 would invite the operator to retry a write that already
    // happened. The original implementation makes the same call.
    if let Err(error) = state.price_cache.invalidate(&state.pg).await {
        tracing::warn!(
            event = "pricing_cache_invalidate_failed",
            model_id = %model_id,
            %error,
            "pricing_cache_invalidate_failed"
        );
    }

    ok(ModelPriceResponse::from(price))
}

use axum::response::IntoResponse as _;

// ---------------------------------------------------------------- user view

/// One row of `GET /user/models`. 对应 `listPanelModelCatalog` 里攒的响应对象。
///
/// This is the *readable* naming again (`input_price_per_1m`), matching the
/// admin **request** body and `frontend/src/features/pricing/types.ts` — not
/// the PascalCase of [`ModelPriceResponse`], and not the `per1_m` column names.
/// All three spellings coexist on this one table; copy, never normalise.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub input_price_per_1m: f64,
    pub output_price_per_1m: f64,
    pub cached_input_price_per_1m: f64,
    pub reasoning_price_per_1m: f64,
}

/// 对应 `visibleCatalogModelIDsSorted` — distinct, trimmed, non-empty, sorted.
///
/// Sorted **here** rather than with `ORDER BY`: 旧实现用字节序排序，
/// which is byte-wise, while Postgres orders by the database's collation
/// (`en_US.UTF-8` puts `gpt-4` and `GPT4` in a different order than bytes do).
/// The frontend renders this list in the order it arrives, so the two must not
/// diverge.
///
/// # Errors
/// Whatever the read reports.
async fn visible_catalog_model_ids(pool: &sqlx::PgPool) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT model_id FROM model_catalog_entries WHERE visible = true AND model_id <> $1",
    )
    // The sentinel row is a *setting* (the channel's models URL) parked in the
    // catalog table, not a model. Its id is declared by the module that writes
    // it; re-typing the literal here is how the two would drift (rule 1.9).
    .bind(crate::ops::catalog::MODELS_URL_MODEL_ID)
    .fetch_all(pool)
    .await?;

    let mut ids: Vec<String> = rows
        .into_iter()
        .filter_map(|(id,)| id)
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// 对应 `listPanelModelCatalog` — the visible ids, each joined to its price row.
///
/// A visible model with **no** `model_prices` row still appears, with four
/// zeroes: 旧实现查 map 取不到就得到零值结构体。Dropping it would hide a
/// model the operator marked visible, which is the opposite of what `visible`
/// means.
///
/// # Errors
/// Whatever either read reports.
pub async fn list_catalog(pool: &sqlx::PgPool) -> Result<Vec<CatalogModel>, sqlx::Error> {
    let ids = visible_catalog_model_ids(pool).await?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let prices: Vec<(String, f64, f64, f64, f64)> = sqlx::query_as(
        "SELECT model_id, input_price_per1_m::float8, output_price_per1_m::float8, \
          cached_input_price_per1_m::float8, reasoning_price_per1_m::float8 \
         FROM model_prices WHERE model_id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;

    let by_id: std::collections::HashMap<String, (f64, f64, f64, f64)> = prices
        .into_iter()
        .map(|(id, input, output, cached, reasoning)| (id, (input, output, cached, reasoning)))
        .collect();

    Ok(ids
        .into_iter()
        .map(|id| {
            let (input, output, cached, reasoning) =
                by_id.get(&id).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
            CatalogModel {
                id,
                input_price_per_1m: input,
                output_price_per_1m: output,
                cached_input_price_per_1m: cached,
                reasoning_price_per_1m: reasoning,
            }
        })
        .collect())
}

/// `GET /user/models`. 对应 `ModelsHandler`.
///
/// # Why this lives in `billing`, not `identity`
///
/// Rule 1.6: deleting a feature should mean deleting one folder. This is the
/// **user-facing read of the same four-column price table** the admin route
/// above writes — same columns, same units, same `per1_m`/`per_1m` naming trap.
/// Splitting them by caller role would put one concept in two folders, and the
/// next change to the price schema would necessarily miss one of them.
///
/// # `rate_multiplier` is the caller's, and for a JWT caller it is always 1.0
///
/// 旧实现读 `bc.RateMult`，而鉴权中间件**只在
/// API-key path**; the JWT branch leaves the `BillingCtx` at its `1.0`
/// initialiser and never looks up the user's group. The panel UI authenticates
/// with a JWT, so it always sees `1.0` here. That is the shipped behaviour —
/// "fixing" it would change what every logged-in user is quoted.
///
/// The `?key_id=` the frontend sometimes appends is likewise ignored, exactly
/// as the original implementation ignores it.
pub async fn user_models(State(state): State<PanelState>, user: AuthUser) -> Response {
    match list_catalog(&state.pg).await {
        Ok(models) => ok(json!({
            "models": models,
            "rate_multiplier": user.rate_multiplier,
        })),
        Err(error) => {
            tracing::error!(%error, "failed to load model catalog");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_INTERNAL,
                "加载模型目录失败，请稍后重试",
            )
        }
    }
}

//! `GET /api/service/models` —— 目录 + 现价的只读镜像。
//!
//! 契约 §3.7：`data` 是 `{"models":[…]}`，每个模型至少有 `id` 与 `owned_by`。价格
//! 字段是**可选**的，给了就必须用 `gw-pricing` 现有列名、金额用 decimal 字符串 ——
//! 这里按后者实现。
//!
//! # 字段名为什么是 `input_price_per1_m`
//!
//! 这就是 `model_prices` 的列名（`1` 与 `M` 之间断开，CONTRACT §3.5 的既有事实）:
//! 契约说"字段名以 gw-pricing 现有列名为准"，所以不做 `per_1m` 之类的"修正" ——
//! `frontend` 那套可读命名（`input_price_per_1m`）是面板的线上形状，不是这里的。
//!
//! # `owned_by` 是渠道 key
//!
//! `model_catalog_entries` 没有 `owned_by` 列；AGW 的模型目录是"渠道 × 模型"，
//! `/v1/models` 把 `channel_key` 映射成 OpenAI 的 `owned_by`（gw-proxy 的 catalog
//! 适配器），这里照同一口径。同一模型被多个渠道提供时，与 `/v1/models` 一样取
//! `channel_key` 字典序最小的那个（`DISTINCT ON`）。

use axum::extract::State;
use axum::response::Response;
use serde::Serialize;

use super::{ServiceCaller, amount_str, db_failure};
use crate::PanelState;
use crate::ok;
use crate::ops::catalog::MODELS_URL_MODEL_ID;

/// 契约 §3.7 的 `data`。
#[derive(Debug, Serialize)]
pub struct ModelsView {
    /// 可见模型，按 `model_id` 升序（一条模型一行）。
    pub models: Vec<ServiceModel>,
}

/// 契约 §3.7 的一个模型条目。
#[derive(Debug, Clone, Serialize)]
pub struct ServiceModel {
    /// `model_catalog_entries.model_id`，也是 `/v1/*` 请求里的模型名。
    pub id: String,
    /// 提供该模型的渠道 key（`/v1/models` 里同名字段的来源）。
    pub owned_by: String,
    /// 输入价，每 1M token。
    pub input_price_per1_m: String,
    /// 输出价，每 1M token。
    pub output_price_per1_m: String,
    /// 缓存命中的输入价，每 1M token。
    pub cached_input_price_per1_m: String,
    /// 推理 token 价，每 1M token。
    pub reasoning_price_per1_m: String,
}

/// 目录与价格 join 出来的一行。
#[derive(Debug, sqlx::FromRow)]
struct ModelRow {
    model_id: String,
    channel_key: String,
    input_price_per1_m: f64,
    output_price_per1_m: f64,
    cached_input_price_per1_m: f64,
    reasoning_price_per1_m: f64,
}

/// 目录 + 现价。
///
/// * `visible = TRUE` 是管理员的开关，只在这里过滤一次；
/// * `MODELS_URL_MODEL_ID` 是"渠道的模型列表地址"那行设置，不是模型；
/// * `LEFT JOIN` 而不是 `JOIN`：目录里有、`model_prices` 里还没有的模型要出现，
///   四个价格按 0 展示（与面板 `list_catalog` 的语义一致 —— 隐藏一个管理员标为
///   可见的模型是反的）；
/// * `DISTINCT ON (e.model_id)` 保证一个模型只出现一次，取渠道字典序最小的那个。
const MODEL_COLUMNS: &str = "\
     e.model_id, \
     COALESCE(e.channel_key, '') AS channel_key, \
     COALESCE(p.input_price_per1_m, 0)::float8 AS input_price_per1_m, \
     COALESCE(p.output_price_per1_m, 0)::float8 AS output_price_per1_m, \
     COALESCE(p.cached_input_price_per1_m, 0)::float8 AS cached_input_price_per1_m, \
     COALESCE(p.reasoning_price_per1_m, 0)::float8 AS reasoning_price_per1_m";

/// `GET /api/service/models`。
pub async fn list(State(state): State<PanelState>, _caller: ServiceCaller) -> Response {
    let rows: Result<Vec<ModelRow>, _> = sqlx::query_as(&format!(
        "SELECT DISTINCT ON (e.model_id) {MODEL_COLUMNS} \
         FROM model_catalog_entries e \
         LEFT JOIN model_prices p ON p.model_id = e.model_id \
         WHERE e.visible = TRUE AND e.model_id <> $1 \
         ORDER BY e.model_id, e.channel_key"
    ))
    .bind(MODELS_URL_MODEL_ID)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(ModelsView {
            models: rows
                .iter()
                .map(|row| ServiceModel {
                    id: row.model_id.clone(),
                    owned_by: row.channel_key.clone(),
                    input_price_per1_m: amount_str(row.input_price_per1_m),
                    output_price_per1_m: amount_str(row.output_price_per1_m),
                    cached_input_price_per1_m: amount_str(row.cached_input_price_per1_m),
                    reasoning_price_per1_m: amount_str(row.reasoning_price_per1_m),
                })
                .collect(),
        }),
        Err(error) => db_failure("service_models", &error, "加载模型目录失败，请稍后重试"),
    }
}

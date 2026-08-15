//! 模型价目表与渠道模型目录。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::compat;

/// `model_prices` 的实体，四列定价（USD / 1M tokens）。
///
/// ⚠️ 列名是 `input_price_per1_m` 而不是直觉上的 `input_price_per_1m`：历史建库的
/// 命名规则在 `1` 和 `M` 之间断词，产出 `..._per1_m`。
/// 改成"更好看"的名字就读不到既有价目表了。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModelPrice {
    pub id: Id,
    pub model_id: String,
    #[sqlx(rename = "input_price_per1_m", try_from = "compat::Money")]
    pub input_price_per_1m: f64,
    #[sqlx(rename = "output_price_per1_m", try_from = "compat::Money")]
    pub output_price_per_1m: f64,
    #[sqlx(rename = "cached_input_price_per1_m", try_from = "compat::Money")]
    pub cached_input_price_per_1m: f64,
    #[sqlx(rename = "reasoning_price_per1_m", try_from = "compat::Money")]
    pub reasoning_price_per_1m: f64,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

/// `model_catalog_entries` 的实体。
///
/// `(channel_key, model_id)` 上有复合唯一索引 `idx_model_catalog_channel_model`
/// （名字来自历史 tag 里显式指定的 `uniqueIndex:idx_model_catalog_channel_model`）。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModelCatalogEntry {
    pub id: Id,
    pub channel_key: String,
    pub model_id: String,
    #[sqlx(try_from = "compat::Bool")]
    pub visible: bool,
    #[sqlx(try_from = "compat::Text")]
    pub models_url: String,
    #[sqlx(try_from = "compat::Ts")]
    pub created_at: DateTime<Utc>,
    #[sqlx(try_from = "compat::Ts")]
    pub updated_at: DateTime<Utc>,
}

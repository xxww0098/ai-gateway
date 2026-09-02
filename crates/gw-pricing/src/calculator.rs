//! 模型 id + 分组倍率 → 一次请求的[冻结报价](PricingQuote)。

use std::sync::Arc;

use gw_model::ModelPrice;

use crate::cache::{ModelPriceCache, normalize_model_key};
use crate::money::UnitPrice;
use crate::quote::{PricingQuote, checked_price, checked_rate};

/// 报价的唯一来源。
///
/// 它**只做一件事**：在 Hold 处把价目表读一次、把四列单价与倍率冻成一个
/// [`PricingQuote`]。之后的估算与精算都在那个报价上做，不再回头查缓存 ——
/// 这正是「在途请求不会因为管理员改价而换一个价钱结算」的实现方式。
///
/// 缓存在这里是只读的；刷新归管理流程（[`ModelPriceCache::invalidate`]）。
///
/// 克隆很便宜（一个 `Arc` 加一个 `f64`）—— 按请求克隆它，而不是再包一层 `Arc`。
#[derive(Debug, Clone, Default)]
pub struct Calculator {
    cache: Option<Arc<ModelPriceCache>>,
    /// 构造时就校验过，所以任何一次未命中都不可能把 `NaN` 或负数注入算术。
    default_price_per_1m: UnitPrice,
}

impl Calculator {
    /// 造一个计价器。`default_price_per_1m` 是价目表里没有的模型的每 1M 兜底价，
    /// 单位与 `model_prices` 各列一致。
    ///
    /// 允许 `None` 缓存：那样每次查询都未命中、一律用兜底价。价目表还没加载起来
    /// 的引导路径因此不需要特判。
    ///
    /// [`Calculator::default()`] 是「无缓存 + 零兜底价」，于是每个报价都是零价。
    ///
    /// `NaN`、无穷或负的兜底价在这个边界上被拒绝，变成 [`UnitPrice::ZERO`]：
    /// 配错的兜底价不许进账本，而零是唯一可证明不会多收的值。
    #[must_use]
    pub fn new(cache: Option<Arc<ModelPriceCache>>, default_price_per_1m: f64) -> Self {
        Self {
            cache,
            default_price_per_1m: UnitPrice::new(default_price_per_1m).unwrap_or(UnitPrice::ZERO),
        }
    }

    /// 冻结一次请求的价格。
    ///
    /// * 命中：四列取该行各自的价，任何一列非法（`NaN` / 负）单独退回兜底价。
    /// * 未命中：四列**都**是 `default_price_per_1m`，与历史的未命中口径一致。
    ///
    /// [`PricingQuote::version`] 记下此刻的缓存代次，于是「这笔账按第几版价目表
    /// 算的」是可回答的，而不是靠猜。
    ///
    /// `model_id` 必须是**请求**里的模型名。上游回话里的名字不许走到这里
    /// —— 那等于让上游决定按什么价收租户的钱。
    #[must_use]
    pub fn quote(&self, model_id: &str, rate_mult: f64) -> PricingQuote {
        let version = self.cache.as_ref().map_or(0, |cache| cache.generation());
        let key = normalize_model_key(model_id);
        let rate = checked_rate(rate_mult);
        let Some(row) = self.lookup(model_id) else {
            return PricingQuote::flat(&key, self.default_price_per_1m.get(), rate_mult, version);
        };
        PricingQuote::new(
            key,
            self.price(row.input_price_per_1m),
            self.price(row.output_price_per_1m),
            self.price(row.cached_input_price_per_1m),
            self.price(row.reasoning_price_per_1m),
            rate,
            version,
        )
    }

    /// 一列价格，已校验。
    fn price(&self, column: f64) -> UnitPrice {
        checked_price(column, self.default_price_per_1m)
    }

    /// 经缓存解析模型 id，容忍缓存缺席。
    fn lookup(&self, model_id: &str) -> Option<Arc<ModelPrice>> {
        self.cache.as_ref()?.get(model_id)
    }
}

#[cfg(test)]
mod tests;

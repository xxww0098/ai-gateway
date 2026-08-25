//! [`PricingCalculator`] over `gw_pricing::Calculator`.
//!
//! 一个方法的适配器，因为端口本身只有一个方法：计价在一次请求里只发生一次，
//! 就是 Hold 处那一次报价。估算与精算都在返回的 [`PricingQuote`] 上做，
//! 而那个值拿不到价目表缓存 —— 结算侧因此**没有**二次查价的入口。

use std::sync::Arc;

use gw_pricing::{Calculator, PricingQuote};

use crate::ports::PricingCalculator;

/// The production calculator, sharing one `ModelPriceCache` with the panel's
/// price editor so an admin upsert invalidates the cache this reads.
///
/// 「立即可见」说的是**下一次报价**：已经铸造出去的报价按定义冻住了。
#[derive(Debug, Clone)]
pub struct SharedCalculator(Arc<Calculator>);

impl SharedCalculator {
    pub fn new(calculator: Arc<Calculator>) -> Self {
        Self(calculator)
    }

    /// The underlying calculator, for callers that need more than the port.
    pub fn inner(&self) -> &Arc<Calculator> {
        &self.0
    }
}

impl From<Arc<Calculator>> for SharedCalculator {
    fn from(calculator: Arc<Calculator>) -> Self {
        Self::new(calculator)
    }
}

impl PricingCalculator for SharedCalculator {
    fn quote(&self, model: &str, rate_mult: f64) -> PricingQuote {
        self.0.quote(model, rate_mult)
    }
}

#[cfg(test)]
mod tests;

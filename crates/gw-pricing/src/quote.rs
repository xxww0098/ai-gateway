//! 一次请求的**冻结报价**：四列单价 + 倍率 + 价目表代次。
//!
//! # 为什么结算不许再查一次价目表
//!
//! 收敛前 Hold 查一次价、Settle 再查一次。两次之间隔着整个上游调用 ——
//! 几十毫秒到几分钟。这中间有两件事能改变第二次查到的结果：
//!
//! 1. **管理员改价**。`ModelPriceCache` 是热更新的（面板改价对计价立即可见，
//!    这是特性），于是一个在途请求可能按 A 价准入、按 B 价扣款。
//! 2. **上游把模型名换了**。结算侧此前用 `usage.model`（上游回话里的名字）
//!    当价格键，而上游完全可能回一个别名、一个带日期后缀的具体版本、
//!    或者一个根本不在价目表里的名字 —— 后者直接落到 `default_price_per_1m`。
//!    也就是说**上游能选择网关按什么价收租户的钱**。
//!
//! 两件事都不该发生。所以 Hold 处一次性把四列单价、倍率**和**价格键
//! 冻进这个结构，Settle 只做算术，不碰缓存。[`version`](PricingQuote::version)
//! 记下冻结时的缓存代次，事后能回答「这笔账是按第几版价目表算的」。

use crate::money::{RateMultiplier, TokenCount, UnitPrice};
use crate::usage::BillableUsage;

/// [`PricingQuote::estimate`] 在看到真实回复之前按多少 token 预留。
///
/// 刻意是一个不大、偏松的常量而不是逐模型的启发式：预扣只要**够接近**，
/// 结算会按真实用量对齐。
pub const ESTIMATED_TOKENS: i64 = 1000;

/// 流式估算的放大系数 —— 流式补全的输出长度普遍高于单次调用。
pub const STREAM_MULTIPLIER: f64 = 2.0;

/// `model_prices` 的价格都是「每这么多 token」。
pub const TOKENS_PER_UNIT: f64 = 1_000_000.0;

/// [`PricingQuote::compute`] 的逐列结果。
///
/// 四个分项是可加的，供 `usage_logs.input_cost` / `output_cost` 与运维看的明细。
///
/// [`total_cost`](Self::total_cost) **不是**四个分项之和：它先把
/// 「单价 × token」的原始乘积加起来，再除一次、缩放一次 —— 账本的守恒不变量
/// 就是按这个运算顺序陈述的。改成累加已经除过的分项会漂。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CostBreakdown {
    pub input_cost: f64,
    pub output_cost: f64,
    pub cached_cost: f64,
    pub reasoning_cost: f64,
    pub total_cost: f64,
}

/// 一次请求冻结下来的价格。
///
/// 由 [`Calculator::quote`](crate::Calculator::quote) 在 Hold 处铸造一次，
/// 之后**只读**：字段私有，没有任何 setter。它被放进 `SettleCtx` 一路带到结算，
/// 结算只调 [`compute`](Self::compute)。
#[derive(Debug, Clone, PartialEq)]
pub struct PricingQuote {
    /// 价格键：**请求**里那个模型名的
    /// [`normalize_model_key`](crate::normalize_model_key) 结果。
    /// 上游回话里的模型名永远进不到这里。
    price_key: String,
    input: UnitPrice,
    output: UnitPrice,
    cached: UnitPrice,
    reasoning: UnitPrice,
    multiplier: RateMultiplier,
    /// 冻结时价目表缓存的代次。
    version: u64,
}

impl PricingQuote {
    /// 四列分别定价。
    #[must_use]
    pub fn new(
        price_key: String,
        input: UnitPrice,
        output: UnitPrice,
        cached: UnitPrice,
        reasoning: UnitPrice,
        multiplier: RateMultiplier,
        version: u64,
    ) -> Self {
        Self {
            price_key,
            input,
            output,
            cached,
            reasoning,
            multiplier,
            version,
        }
    }

    /// 四列同价 —— 价目表未命中时的兜底形状。
    ///
    /// 非有限或为负的价格塌成 [`UnitPrice::ZERO`]：那是唯一可证明不会多收的值。
    /// 非有限或为负的倍率塌成 [`RateMultiplier::ONE`]（按未打折的价收是安全方向），
    /// 而**零倍率被保留** —— 零费率分组是一种真实配置。
    #[must_use]
    pub fn flat(price_key: &str, per_1m: f64, rate_mult: f64, version: u64) -> Self {
        let price = UnitPrice::new(per_1m).unwrap_or(UnitPrice::ZERO);
        Self::new(
            price_key.to_owned(),
            price,
            price,
            price,
            price,
            checked_rate(rate_mult),
            version,
        )
    }

    /// 价格键 —— 这笔账是按哪个模型算的。
    #[must_use]
    pub fn price_key(&self) -> &str {
        &self.price_key
    }

    /// 冻结时的价目表代次。
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// 分组倍率，已校验。
    #[must_use]
    pub fn multiplier(&self) -> RateMultiplier {
        self.multiplier
    }

    /// 未命中缓存的输入单价。
    #[must_use]
    pub fn input_price(&self) -> UnitPrice {
        self.input
    }

    /// 可见输出单价。
    #[must_use]
    pub fn output_price(&self) -> UnitPrice {
        self.output
    }

    /// 命中缓存的输入单价。
    #[must_use]
    pub fn cached_price(&self) -> UnitPrice {
        self.cached
    }

    /// 思考 token 单价。
    #[must_use]
    pub fn reasoning_price(&self) -> UnitPrice {
        self.reasoning
    }

    /// 一次已完成请求的准确金额。**结算唯一的算术入口。**
    ///
    /// 四列互斥（见 [`crate::usage`]），所以这里只是四次「单价 × token」。
    ///
    /// # 思考 token 在 `reasoning` 列无价时按**输出价**收，而不是免费
    ///
    /// `model_prices.reasoning_price_per1_m` 的建表默认值是 **0**
    /// （`migrations/0001_init.sql`），绝大多数部署从没填过这一列。若照字面
    /// 按 0 收，后果对两族方言都是灾难性的、而且方向一致 —— 少收：
    ///
    /// * OpenAI / Anthropic：归一化刚把思考从 `output` 里减出去，
    ///   再按 0 收 `reasoning`，等于**整块思考凭空免费**，
    ///   而收敛前它是被算在 `output` 里收过钱的。
    /// * Google：`candidatesTokenCount` 本来就不含思考，按 0 收同样是全免。
    ///   而思考 token 在推理型模型上经常是可见输出的数倍。
    ///
    /// 所以这里的规则与方言无关：**`reasoning` 这一列没有价，就说明这个部署
    /// 没有为思考单独定价，那它就按输出价收**。这既复原了 OpenAI 的历史口径
    /// （思考本来就在 `completion_tokens` 里按输出价收），也正是 Google 自己的
    /// 计费口径（thinking token 按 output 价）。一旦这一列被填上正数，
    /// 思考就按那个价单独计一次，**并且不再折进可见输出** —— 不会重复计价。
    #[must_use]
    pub fn compute(&self, usage: BillableUsage) -> CostBreakdown {
        let (visible, reasoning) = if self.reasoning.is_zero() {
            (
                usage
                    .visible_output
                    .get()
                    .saturating_add(usage.reasoning_output.get()),
                0,
            )
        } else {
            (usage.visible_output.get(), usage.reasoning_output.get())
        };

        let uncached = usage.uncached_input.as_f64();
        let cached = usage.cached_input.as_f64();
        let visible = visible as f64;
        let reasoning = reasoning as f64;
        let rate = self.multiplier.get();

        // 原始乘积先相加，再除一次、缩放一次。
        let raw = self.input.get() * uncached
            + self.cached.get() * cached
            + self.output.get() * visible
            + self.reasoning.get() * reasoning;

        CostBreakdown {
            input_cost: self.input.get() * uncached / TOKENS_PER_UNIT * rate,
            output_cost: self.output.get() * visible / TOKENS_PER_UNIT * rate,
            cached_cost: self.cached.get() * cached / TOKENS_PER_UNIT * rate,
            reasoning_cost: self.reasoning.get() * reasoning / TOKENS_PER_UNIT * rate,
            total_cost: raw / TOKENS_PER_UNIT * rate,
        }
    }

    /// 见到真实回复之前的预扣估算，刻意高估 —— 结算会退掉多余部分。
    ///
    /// 输入与输出各按 [`ESTIMATED_TOKENS`] 计；`stream` 再乘
    /// [`STREAM_MULTIPLIER`]。零单价或零倍率会塌成 0，都是刻意的：
    /// 要不要设下限是调用方的决定。
    #[must_use]
    pub fn estimate(&self, stream: bool) -> f64 {
        self.io_estimate(ESTIMATED_TOKENS, ESTIMATED_TOKENS, stream)
    }

    /// 客户端给了输出上限（`max_tokens` / `max_completion_tokens`）时更紧的上界。
    ///
    /// 非正的上限视作「客户端没给」，退回 [`estimate`](Self::estimate) ——
    /// 缺失或畸形的上限拿不到更紧的界。
    #[must_use]
    pub fn estimate_with_max_tokens(&self, max_output_tokens: i64, stream: bool) -> f64 {
        if max_output_tokens <= 0 {
            return self.estimate(stream);
        }
        self.io_estimate(ESTIMATED_TOKENS, max_output_tokens, stream)
    }

    /// 按**真实**输入 token 数（由 body 长度近似）算的预扣。
    ///
    /// 这是大 prompt 系统性预扣不足的修法：[`estimate`](Self::estimate)
    /// 把输入按固定的 [`ESTIMATED_TOKENS`] 计，于是一个 100k token 的请求
    /// 预留得远远不够，溜过余额闸门，结算时把余额打成负数（产生欠款）。
    ///
    /// 输入按 `max(input_tokens, ESTIMATED_TOKENS)` 计 —— 空 body 也不低于
    /// 历史的名义下限；输出有上限就按上限，否则按 [`ESTIMATED_TOKENS`]。
    #[must_use]
    pub fn estimate_with_tokens(
        &self,
        input_tokens: i64,
        max_output_tokens: i64,
        stream: bool,
    ) -> f64 {
        let out_tok = if max_output_tokens <= 0 {
            ESTIMATED_TOKENS
        } else {
            max_output_tokens
        };
        self.io_estimate(input_tokens.max(ESTIMATED_TOKENS), out_tok, stream)
    }

    /// 三个 estimator 共用的算术：输入价 × 输入量 + 输出价 × 输出量，
    /// 除一次、（可选）放大一次、缩放一次。
    fn io_estimate(&self, input_tokens: i64, output_tokens: i64, stream: bool) -> f64 {
        let input = TokenCount::clamped(input_tokens).as_f64();
        let output = TokenCount::clamped(output_tokens).as_f64();
        let mut base = (self.input.get() * input + self.output.get() * output) / TOKENS_PER_UNIT;
        if stream {
            base *= STREAM_MULTIPLIER;
        }
        base * self.multiplier.get()
    }
}

/// 分组倍率的闸门。
///
/// `NaN` 或负数塌成恒等：按未打折的价计费是安全的失败方向，而 `NaN`
/// 会溜过之后**每一个**比较。零保留 —— 零费率分组是真实配置。
pub(crate) fn checked_rate(rate_mult: f64) -> RateMultiplier {
    RateMultiplier::new(rate_mult).unwrap_or(RateMultiplier::ONE)
}

/// `model_prices` 的一列，已校验。
///
/// 一行带着 `NaN` 或负价的记录退回配置的默认价，而不是毒化下游算术:
/// 那一列是可空的 `numeric`，出现敌意值是数据问题，不是请求问题。
pub(crate) fn checked_price(column: f64, fallback: UnitPrice) -> UnitPrice {
    UnitPrice::new(column).unwrap_or(fallback)
}

#[cfg(test)]
mod tests;

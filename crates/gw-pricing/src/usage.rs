//! 上游原话的 token 计数 → **互斥**的可计价视图。
//!
//! 两个结构，一条归一化函数：
//!
//! * [`ObservedUsage`] —— 上游 usage 信封里的四个数，原样。写进
//!   `usage_logs` 的就是它，审计要能和上游账单对上。
//! * [`BillableUsage`] —— 四个**互不重叠**的列。计价只看它。
//!
//! # 为什么必须归一化：三家上游的「输出」不是一个东西
//!
//! | 上游 | 「输出」字段 | 含不含思考 | 「缓存」字段 |
//! | --- | --- | --- | --- |
//! | OpenAI / Codex | `usage.completion_tokens` | **含**（`completion_tokens_details.reasoning_tokens` 是它的一个明细） | `prompt_tokens_details.cached_tokens` ⊂ `prompt_tokens` |
//! | Anthropic | `usage.output_tokens` | **含**（thinking 块计在里面） | `cache_read_input_tokens` ⊂ 输入 |
//! | Google（gemini / vertex） | `usageMetadata.candidatesTokenCount` | **不含**（思考在 `thoughtsTokenCount`，是并列项） | `cachedContentTokenCount` ⊂ 输入 |
//!
//! 把四个字段当四条独立的价格列直接相乘，对 OpenAI / Anthropic 就是
//! **同一批思考 token 收两遍**：一遍在 `output`（因为它含思考），
//! 一遍在 `reasoning`。归一化把 `reasoning` 从 `output` 里减出去，
//! 于是四列真的互斥，`uncached_input + cached_input` 是全部输入，
//! `visible_output + reasoning_output` 是全部输出。
//!
//! # 归一化只管「谁属于谁」，不管「按什么价收」
//!
//! Google 的思考 token 该不该按输出价收，是**价格**问题，不是语义问题，
//! 所以它在 [`crate::PricingQuote::compute`] 里 —— 那里才看得见
//! `reasoning` 这一列有没有价。见那条注释。

use crate::money::{TokenCount, ValueError};

/// 上游 usage 信封的四个数，**未经任何加工**。
///
/// 字段是 `pub` 且 `i64`（可以是负数）：写日志的那条路必须能拿到上游原话，
/// 哪怕它自相矛盾。校验发生在 [`normalize`] —— 那是钱经过的地方。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObservedUsage {
    pub input: i64,
    pub output: i64,
    pub cached: i64,
    pub reasoning: i64,
}

impl ObservedUsage {
    /// 校验过的构造：任何一列为负都拒绝。
    ///
    /// **不 clamp**。负数信封是无效信封，不是一笔退款 —— 把它当 0 收下，
    /// 等于让上游的错误静默变成网关的少收；把它当负数收下，等于给租户**充值**。
    ///
    /// # Errors
    /// [`ValueError::Negative`] 当任意一列小于零。
    pub fn new(input: i64, output: i64, cached: i64, reasoning: i64) -> Result<Self, ValueError> {
        let observed = Self {
            input,
            output,
            cached,
            reasoning,
        };
        observed.checked()?;
        Ok(observed)
    }

    /// 按 `dialect` 的语义折成互斥的可计价视图。
    ///
    /// # Errors
    /// 见 [`normalize`]。
    pub fn normalize(self, dialect: UsageDialect) -> Result<BillableUsage, ValueError> {
        normalize(dialect, self)
    }

    /// 四列都过一遍 [`TokenCount`] 的非负闸门。
    fn checked(self) -> Result<(TokenCount, TokenCount, TokenCount, TokenCount), ValueError> {
        Ok((
            TokenCount::new(self.input)?,
            TokenCount::new(self.output)?,
            TokenCount::new(self.cached)?,
            TokenCount::new(self.reasoning)?,
        ))
    }
}

impl TryFrom<[i64; 4]> for ObservedUsage {
    type Error = ValueError;

    /// `[input, output, cached, reasoning]`，与 [`ObservedUsage::new`] 同闸门。
    fn try_from([input, output, cached, reasoning]: [i64; 4]) -> Result<Self, ValueError> {
        Self::new(input, output, cached, reasoning)
    }
}

/// 四个**互不重叠**的可计价列。四列之和 = 这次请求的全部 token。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BillableUsage {
    /// 输入里**没有**命中缓存的部分。
    pub uncached_input: TokenCount,
    /// 输入里命中缓存的部分。
    pub cached_input: TokenCount,
    /// 输出里**不含**思考的部分。
    pub visible_output: TokenCount,
    /// 思考 token。
    pub reasoning_output: TokenCount,
}

/// 上游 usage 信封的语义族。
///
/// 分的是**信封的形状**，不是厂商：`gemini` 与 `vertex` 是两套鉴权与端点，
/// 但 wire 协议同为 GenerateContent，`usageMetadata` 字段语义逐字相同。
///
/// 刻意**没有** `Default`：默认方言就是一个会被忘记设置的方言，
/// 而设错方言的代价是每次调用都算错钱。调用方必须说出上游是谁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageDialect {
    /// OpenAI Chat Completions / Responses，以及 Codex。思考 ⊂ 输出。
    OpenAi,
    /// Anthropic Messages。思考 ⊂ 输出。
    Anthropic,
    /// Google GenerateContent（gemini / vertex）。思考与输出**并列**。
    Google,
}

/// 上游原话 → 互斥的可计价视图。
///
/// 三家共同的一条：`cached` 是 `input` 的**子集**（三家的缓存字段都描述
/// 「这次输入里有多少来自缓存」）。所以 `uncached_input = input - cached`。
///
/// 分岔的那条是 `reasoning`：
///
/// * [`UsageDialect::OpenAi`] / [`UsageDialect::Anthropic`]：思考 ⊂ 输出，
///   于是 `visible_output = output - reasoning`。
/// * [`UsageDialect::Google`]：思考与输出并列，`visible_output = output` 原样。
///
/// # Errors
/// [`ValueError::Negative`] 当任意一列为负；
/// [`ValueError::Inconsistent`] 当子集列大于它所属的总量
/// （`cached > input`，或思考 ⊂ 输出的方言下 `reasoning > output`）。
/// 后者是**拒绝**而不是截断：一个自相矛盾的信封说明上游或解析出了问题，
/// 按两列都收就是重复计费，按截断收就是猜。结算把它当「没有信封」处理，
/// 走既有的 fallback / strict 分支。
pub fn normalize(
    dialect: UsageDialect,
    observed: ObservedUsage,
) -> Result<BillableUsage, ValueError> {
    let (input, output, cached, reasoning) = observed.checked()?;

    if cached > input {
        return Err(ValueError::Inconsistent {
            kind: "cached input",
        });
    }
    let uncached_input = TokenCount::new(input.get() - cached.get())?;

    let (visible_output, reasoning_output) = match dialect {
        UsageDialect::OpenAi | UsageDialect::Anthropic => {
            if reasoning > output {
                return Err(ValueError::Inconsistent {
                    kind: "reasoning output",
                });
            }
            (TokenCount::new(output.get() - reasoning.get())?, reasoning)
        }
        UsageDialect::Google => (output, reasoning),
    };

    Ok(BillableUsage {
        uncached_input,
        cached_input: cached,
        visible_output,
        reasoning_output,
    })
}

#[cfg(test)]
mod tests;

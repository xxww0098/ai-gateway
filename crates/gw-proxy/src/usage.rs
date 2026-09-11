//! Terminal settlement: Compute -> Settle | Release -> usage log -> quota.
//!
//! Four outcomes, all of which must be reachable and none of which may panic —
//! this runs after the
//! response has been handed to the client, so a panic here would lose the
//! charge silently:
//!
//! | upstream | usage envelope | strict mode | outcome |
//! | --- | --- | --- | --- |
//! | failed | — | — | `Release`, `usage_logs.failed = true` |
//! | ok | present | — | `Compute` -> `Settle` (precise) |
//! | ok | absent | off | `Settle(max(ActiveHoldAmount, Estimate(stream)))`, tagged `billing_fallback.reason = missing_upstream_usage` |
//! | ok | absent | on | **no** Settle, **no** Release — the hold expires on its TTL — and `usage_logs{failed: true, reason: missing_upstream_usage_strict}` |
//!
//! A fifth case falls out of the fallback path: if `ActiveHoldAmount` itself
//! errors we cannot bound the cost safely, so we behave like strict mode rather
//! than settle at zero (`active_hold_lookup_failed`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use gw_provider::types::UsageRecord;
use serde_json::json;

use crate::ports::{
    BalanceEvent, BillingLedger, PricingCalculator, SettleReceipt, SettlementCommit, TokenUsage,
    UsageLogEntry, UsageStore,
};
use crate::settlectx::SettleCtx;

/// Default low-balance threshold in USD.
pub const DEFAULT_LOW_BALANCE_THRESHOLD: f64 = 1.0;

/// `RawMetadata.billing_fallback.reason` written when upstream omitted usage.
pub const REASON_MISSING_USAGE: &str = "missing_upstream_usage";

/// `RawMetadata.reason` written in strict mode.
pub const REASON_MISSING_USAGE_STRICT: &str = "missing_upstream_usage_strict";

/// `RawMetadata.event` written when the active-hold lookup failed.
pub const EVENT_HOLD_LOOKUP_FAILED: &str = "active_hold_lookup_failed";

/// What the pipeline observed about one finished request.
#[derive(Debug, Clone, Default)]
pub struct UsageOutcome {
    /// Token counts reported by the upstream, if any. `None` means the
    /// upstream published no usage detail.
    pub usage: Option<UsageRecord>,
    /// The upstream call itself failed.
    pub failed: bool,
    /// Upstream credential that served the request, for the audit trail.
    pub auth_id: String,
    pub provider: String,
    pub duration_ms: i64,
}

impl UsageOutcome {
    /// A successful request whose upstream published a usage envelope.
    pub fn precise(usage: UsageRecord) -> Self {
        Self {
            usage: Some(usage),
            ..Self::default()
        }
    }

    /// A failed upstream call: Release, never Settle.
    pub fn failed() -> Self {
        Self {
            failed: true,
            ..Self::default()
        }
    }
}

/// The branch [`Settlement`] took, resolved before any I/O.
///
/// Exposed (and returned by [`plan_settlement`]) so the billing decision can be
/// unit-tested without a database, which is the part that must never regress.
#[derive(Debug, Clone, PartialEq)]
pub enum SettlementPlan {
    /// Upstream failed: release the hold, log the failure, accumulate nothing.
    Release { reason: &'static str },
    /// Charge `cost`; `fallback` is set when the cost came from the estimate
    /// rather than a real usage envelope.
    Settle {
        cost: f64,
        fallback: Option<&'static str>,
    },
    /// Strict mode with no usage envelope: charge nothing, release nothing,
    /// let the hold expire on its TTL, and record the event.
    StrictSkip,
    /// The active-hold lookup failed, so no safe lower bound exists. Same
    /// posture as strict mode.
    HoldLookupFailed,
}

/// Inputs to the settlement decision, all resolved before any write.
#[derive(Debug, Clone, PartialEq)]
pub struct SettlementInputs {
    /// `Compute(tokens)` — meaningful only when the envelope was present.
    pub computed_cost: f64,
    /// Whether the upstream published a usage envelope.
    pub usage_present: bool,
    /// The upstream call failed.
    pub upstream_failed: bool,
    /// `billing.strict_usage_metadata_mode`.
    pub strict_mode: bool,
    /// `ActiveHoldAmount`: `None` means the lookup itself failed.
    pub active_hold: Option<f64>,
    /// `Estimate(model, stream = true, rate_mult)`.
    pub streaming_estimate: f64,
}

/// 上游按 Google GenerateContent 语义报 usage 的 executor。
///
/// `gemini` 与 `vertex` 是两套鉴权与端点前缀，但 wire 协议是**同一个**
/// GenerateContent，`usageMetadata` 的字段语义因此完全一致。
const GOOGLE_SHAPED_PROVIDERS: [&str; 2] = ["gemini", "vertex"];

/// 一个上游的四个**原始**计数列之间是怎么嵌套的。
///
/// 四个计费列必须互斥，而四家上游里有两家报的是**重叠**计数 —— 这是 wire 格式
/// 的事实，不是价格表的策略，所以判定放在中继/计费侧而不是计算器里。
///
/// 依据（厂商一手文档，2026-09 核）：
///
/// * OpenAI `prompt_tokens_details.cached_tokens` 是 `prompt_tokens` 的**明细**
///   （"Breakdown of tokens used in the prompt"）；官方 prompt-caching 指南直接
///   给出减法公式 `ordinaryInputTokens = inputTokens - cachedTokens - cacheWriteTokens`。
///   `completion_tokens_details.reasoning_tokens` 同理是 `completion_tokens` 的明细。
/// * Google `promptTokenCount` 原文写着 "this includes the number of tokens in the
///   cached content"；而 `totalTokenCount = prompt + thoughts + candidates` ——
///   即 thoughts 与 candidates **并列**，与前两家相反。
/// * Anthropic 原文："Total input tokens in a request is the summation of
///   `input_tokens`, `cache_creation_input_tokens`, and `cache_read_input_tokens`" ——
///   三者并列，**不能**再减；`output_tokens` 本身已含 thinking token。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenNesting {
    /// `cached ⊆ input` 且 `reasoning ⊆ output`（OpenAI / Codex）。
    BothNested,
    /// `cached ⊆ input`，但 `reasoning ⊥ output`（Gemini / Vertex）。
    CachedNestedReasoningSibling,
    /// 四列互斥，原样计价（Anthropic，以及所有不认识的上游）。
    Disjoint,
}

fn nesting(provider: &str) -> TokenNesting {
    if GOOGLE_SHAPED_PROVIDERS.contains(&provider) {
        TokenNesting::CachedNestedReasoningSibling
    } else if matches!(provider, "openai" | "codex") {
        TokenNesting::BothNested
    } else {
        TokenNesting::Disjoint
    }
}

/// `total - subset`，两端都夹在合法区间里。
///
/// 上游只报了子集、没报总数时，这里拿到的是已经 `unwrap_or(0)` 的 0，
/// 减出来仍是 0 —— 与中继层「缺失不塌缩成零」的三态约定不同，但这一层
/// 的 `TokenUsage` 本来就是计费视图的整数列，缺失在进入本函数之前就已经归零，
/// 不存在把「没报」误当成「报了 0」的额外损失。
///
/// `saturating_sub` 挡的是上游自相矛盾的数据（子集比总数还大）；`.max(0)` 是
/// 冗余的一层保险，去掉任何一个都不会让负数漏到计费列上。
fn subtract(total: i64, subset: i64) -> i64 {
    total.saturating_sub(subset).max(0)
}

/// 把上游原话的 token 计数归一成**可计价**的视图：四个计费列两两互斥。
///
/// # 为什么必须互斥
///
/// 计算器把四列**相加**计价（`Calculator::compute`）。只要某一列是另一列的
/// 子集，这一块 token 就会被卖两次。两家上游报的正是重叠计数：
///
/// | 上游 | 嵌套 | 不减的后果 |
/// | --- | --- | --- |
/// | OpenAI / Codex | `cached ⊆ input`，`reasoning ⊆ output` | 缓存命中的 token 收「全价 input + 缓存价」，比不命中还贵 |
/// | Google（gemini / vertex） | `cached ⊆ input`，`reasoning ⊥ output` | 同上；思考 token 还必须折进 output |
/// | Anthropic | 四列互斥 | ——（减了反而少收） |
///
/// 判据与原文引用见 [`nesting`]。这是**静默多收费**，不是舍入误差：按 OpenAI 的
/// 缓存折扣（0.1×~0.5× input），命中一块缓存会被收成 1.1×~1.5× 全价。
///
/// # 只作用于计价
///
/// 写进 `usage_logs` 的仍然是上游原话（四列各归各位），否则审计就对不上上游账单了。
/// `Calculator::compute` 拿到的才是互斥视图 —— 这个分工由
/// `crates/gw-proxy/src/usage.rs` 的调用点和 `Settlement::build_entry` 共同守着。
///
/// # 减完之后那一块不是免费的
///
/// 被减出来的 `cached` / `reasoning` 仍然按各自的费率计价；费率留空（0）时回落到
/// 基准费率，见 `gw_pricing::Calculator::compute` 的费率解析。
#[must_use]
pub fn billable_tokens(provider: &str, tokens: TokenUsage) -> TokenUsage {
    match nesting(provider) {
        // OpenAI：两列都是明细，各自把子集从总数里摘出去。
        TokenNesting::BothNested => TokenUsage {
            input: subtract(tokens.input, tokens.cached),
            output: subtract(tokens.output, tokens.reasoning),
            ..tokens
        },
        // Google：输入侧同上；思考 token 是**并列**的，折进 output 按输出价收，
        // 这正是 Google 自己的口径（"Output price (including thinking tokens)"）。
        TokenNesting::CachedNestedReasoningSibling => TokenUsage {
            input: subtract(tokens.input, tokens.cached),
            output: tokens.output.saturating_add(tokens.reasoning),
            reasoning: 0,
            ..tokens
        },
        // Anthropic 与不认识的上游：四列本来就是并列的，原样交给计算器。
        // 不认识就**不减**是刻意的：多减会少收，风险方向比多数更糟。
        TokenNesting::Disjoint => tokens,
    }
}

/// Resolves which of the four settlement branches applies.
///
/// Keeping the branching separate from the I/O is what lets the fallback and
/// strict paths be tested exactly.
pub fn plan_settlement(inputs: &SettlementInputs) -> SettlementPlan {
    if inputs.upstream_failed {
        return SettlementPlan::Release {
            reason: "upstream request failed",
        };
    }
    if inputs.usage_present {
        return SettlementPlan::Settle {
            cost: inputs.computed_cost,
            fallback: None,
        };
    }
    // Strict mode is a default-deny posture: an upstream that stripped the
    // usage envelope suspends billing instead of guessing.
    if inputs.strict_mode {
        return SettlementPlan::StrictSkip;
    }
    // Fallback: bill at least the hold so upstream output is never free.
    match inputs.active_hold {
        None => SettlementPlan::HoldLookupFailed,
        Some(held) => SettlementPlan::Settle {
            cost: held.max(inputs.streaming_estimate),
            fallback: Some(REASON_MISSING_USAGE),
        },
    }
}

/// The settlement engine.
pub struct Settlement {
    ledger: Arc<dyn BillingLedger>,
    calc: Arc<dyn PricingCalculator>,
    store: Arc<dyn UsageStore>,
    low_balance_threshold: f64,
    strict_usage_metadata: AtomicBool,
}

impl Settlement {
    /// Builds the engine over ledger, calculator and store.
    pub fn new(
        ledger: Arc<dyn BillingLedger>,
        calc: Arc<dyn PricingCalculator>,
        store: Arc<dyn UsageStore>,
    ) -> Self {
        Self {
            ledger,
            calc,
            store,
            low_balance_threshold: DEFAULT_LOW_BALANCE_THRESHOLD,
            strict_usage_metadata: AtomicBool::new(false),
        }
    }

    /// Set the low-balance threshold; non-positive keeps the $1 default.
    #[must_use]
    pub fn with_low_balance_threshold(mut self, threshold: f64) -> Self {
        if threshold > 0.0 {
            self.low_balance_threshold = threshold;
        }
        self
    }

    /// Runtime toggle for `billing.strict_usage_metadata_mode`.
    pub fn set_strict_usage_metadata(&self, strict: bool) {
        self.strict_usage_metadata.store(strict, Ordering::SeqCst);
    }

    /// Whether strict mode is currently on.
    pub fn strict_usage_metadata(&self) -> bool {
        self.strict_usage_metadata.load(Ordering::SeqCst)
    }

    /// Settles a successful request that produced no usage envelope.
    ///
    /// This is the hold middleware's safety net (the B2 finalizer path):
    /// streaming responses and handlers that returned without publishing usage
    /// land here and are billed the conservative estimate — never free, never
    /// above what the hold allowed.
    pub async fn settle_missing_usage(&self, ctx: &SettleCtx) {
        self.settle(ctx, UsageOutcome::default()).await;
    }

    /// Terminal accounting for one request. Never panics; every failure is
    /// logged and swallowed.
    pub async fn settle(&self, ctx: &SettleCtx, outcome: UsageOutcome) {
        let usage = outcome.usage.clone().unwrap_or_default();
        let tokens = TokenUsage {
            input: usage.input_tokens.unwrap_or(0),
            output: usage.output_tokens.unwrap_or(0),
            cached: usage.cached_tokens.unwrap_or(0),
            reasoning: usage.reasoning_tokens.unwrap_or(0),
        };
        let model = if usage.model.is_empty() {
            ctx.model.clone()
        } else {
            usage.model.clone()
        };
        // 计价用的是**归一化后**的 token 视图，日志写的是上游原话。
        // 两者不同的唯一一种情况见 [`billable_tokens`]。
        let computed_cost = self.calc.compute(
            &model,
            billable_tokens(&outcome.provider, tokens),
            ctx.rate_mult,
        );

        // The active-hold lookup is only consulted on the fallback path, so it
        // is resolved lazily to keep the precise path at one round-trip.
        let needs_hold_lookup =
            !outcome.failed && outcome.usage.is_none() && !self.strict_usage_metadata();
        let active_hold = if needs_hold_lookup {
            match self
                .ledger
                .active_hold_amount(ctx.user_id, &ctx.request_id)
                .await
            {
                // No hold row is a definite zero, not an unknown.
                Ok(amount) => Some(amount.unwrap_or(0.0)),
                Err(err) => {
                    tracing::warn!(
                        event = EVENT_HOLD_LOOKUP_FAILED,
                        user_id = ctx.user_id,
                        request_id = %ctx.request_id,
                        %err,
                    );
                    None
                }
            }
        } else {
            Some(0.0)
        };

        let plan = plan_settlement(&SettlementInputs {
            computed_cost,
            usage_present: outcome.usage.is_some(),
            upstream_failed: outcome.failed,
            strict_mode: self.strict_usage_metadata(),
            active_hold,
            streaming_estimate: self.calc.estimate(&model, true, ctx.rate_mult),
        });

        match plan {
            SettlementPlan::Release { reason } => {
                if let Err(err) = self.ledger.release(ctx.user_id, &ctx.request_id).await {
                    tracing::warn!(user_id = ctx.user_id, request_id = %ctx.request_id, %err,
                        "ledger release failed");
                }
                let mut entry =
                    self.build_entry(ctx, &outcome, &model, tokens, computed_cost, true);
                entry.raw_metadata = Some(json!({
                    "reason": reason,
                    "timestamp": Utc::now().to_rfc3339(),
                }));
                self.write_log(&entry).await;
            }
            SettlementPlan::StrictSkip => {
                // No Settle, no Release: the Redis hold expires on its natural
                // TTL so out-of-band reconciliation can match this row against
                // the abandoned reservation.
                let mut entry = self.build_entry(ctx, &outcome, &model, tokens, 0.0, true);
                entry.raw_metadata = Some(json!({
                    "reason": REASON_MISSING_USAGE_STRICT,
                    "timestamp": Utc::now().to_rfc3339(),
                }));
                self.write_log(&entry).await;
            }
            SettlementPlan::HoldLookupFailed => {
                // We cannot bound the cost, and a zero-cost Settle would make
                // the request free. Leave the hold to its TTL, like strict mode.
                let mut entry = self.build_entry(ctx, &outcome, &model, tokens, 0.0, true);
                entry.raw_metadata = Some(json!({
                    "event": EVENT_HOLD_LOOKUP_FAILED,
                    "reason": "active hold lookup failed",
                    "timestamp": Utc::now().to_rfc3339(),
                }));
                self.write_log(&entry).await;
            }
            SettlementPlan::Settle { cost, fallback } => {
                self.commit(ctx, &outcome, &model, tokens, cost, fallback)
                    .await;
            }
        }
    }

    /// The atomic settle: debit, usage log and quota accumulation in ONE
    /// transaction, with the Redis reservation cleared only after it commits.
    ///
    /// The ordering matters: clearing the hold before the commit is what used
    /// to leave a user charged with no usage log when the outer write failed.
    async fn commit(
        &self,
        ctx: &SettleCtx,
        outcome: &UsageOutcome,
        model: &str,
        tokens: TokenUsage,
        cost: f64,
        fallback: Option<&'static str>,
    ) {
        let mut entry = self.build_entry(ctx, outcome, model, tokens, cost, false);
        // The fallback tag is known now; `shortfall_usd` is not, because the
        // debit happens inside the transaction. The store merges it in there
        // via [`merge_shortfall`], which is why that helper is public.
        entry.raw_metadata = settle_annotations(fallback, 0.0);
        let commit = SettlementCommit {
            user_id: ctx.user_id,
            request_id: ctx.request_id.clone(),
            actual_cost: cost,
            entry,
            subscription_id: ctx.subscription_id,
        };

        let receipt = match self.store.commit_settlement(&commit).await {
            Ok(receipt) => receipt,
            Err(err) => {
                // The whole transaction — including the debit — rolled back and
                // the hold was NOT cleared, so balance and usage stay consistent
                // and the request is reconcilable. Record the failure outside
                // the dead transaction; accumulate no quota.
                tracing::warn!(
                    user_id = ctx.user_id,
                    request_id = %ctx.request_id,
                    cost,
                    %err,
                    "settle transaction failed",
                );
                let mut failed = commit.entry.clone();
                failed.failed = true;
                // 事务回滚了，所以**一分钱都没动** —— 这条审计行的金额列必须
                // 归零。面板的总花费直接 `SUM(usage_logs.cost)`、不按 `failed`
                // 过滤（`gw-panel/src/ops/dashboard.rs`、`billing/usage/user.rs`、
                // `upstream/usage_stats.rs`），留着全额就会把一个「尝试收 but 失败」
                // 的请求算成真花掉的钱，与 `balance_logs` 对不上账。
                //
                // 尝试收多少不能丢：它是运维判断「这次回滚值不值得追」的唯一线索，
                // 所以挪进 metadata，而不是留在会被求和的列里。
                failed.cost = 0.0;
                failed.raw_metadata = Some(json!({
                    "reason": err.to_string(),
                    "attempted_cost": cost,
                    "timestamp": Utc::now().to_rfc3339(),
                }));
                self.write_log(&failed).await;
                return;
            }
        };

        let SettleReceipt::Committed {
            shortfall,
            balance_before,
            balance_after,
            balance_version,
        } = receipt
        else {
            return; // AlreadySettled: nothing further to do (reconcile path)
        };
        if shortfall > 0.0 {
            tracing::warn!(
                user_id = ctx.user_id,
                request_id = %ctx.request_id,
                shortfall_usd = shortfall,
                "partial debit; shortfall recorded",
            );
        }

        // Post-commit, non-transactional side effects.
        if let Err(err) = self
            .store
            .clear_hold(
                ctx.user_id,
                &ctx.request_id,
                balance_version.map(|version| (balance_after, version)),
            )
            .await
        {
            tracing::warn!(user_id = ctx.user_id, request_id = %ctx.request_id, %err,
                "clear hold failed; reservation will TTL-expire");
        }
        self.check_balance_events(ctx, balance_before, balance_after)
            .await;
    }

    /// Builds the `usage_logs` row.
    fn build_entry(
        &self,
        ctx: &SettleCtx,
        outcome: &UsageOutcome,
        model: &str,
        tokens: TokenUsage,
        cost: f64,
        failed: bool,
    ) -> UsageLogEntry {
        let provider = if outcome.provider.is_empty() {
            outcome
                .usage
                .as_ref()
                .map(|u| u.provider.clone())
                .unwrap_or_default()
        } else {
            outcome.provider.clone()
        };
        UsageLogEntry {
            user_id: ctx.user_id,
            api_key_id: ctx.api_key_id,
            group_id: ctx.group_id,
            request_id: ctx.request_id.clone(),
            idempotency_key: ctx.idempotency_key.clone(),
            model: model.to_owned(),
            provider,
            auth_id: outcome.auth_id.clone(),
            input_tokens: tokens.input,
            output_tokens: tokens.output,
            cached_tokens: tokens.cached,
            reasoning_tokens: tokens.reasoning,
            cost,
            rate_multiplier: ctx.rate_mult,
            stream: ctx.stream,
            duration_ms: outcome.duration_ms,
            ip_address: ctx.ip_address.clone(),
            raw_metadata: None,
            failed,
        }
    }

    /// Inserts a usage log outside any transaction (failure paths).
    async fn write_log(&self, entry: &UsageLogEntry) {
        if let Err(err) = self.store.insert_usage_log(entry).await {
            tracing::warn!(user_id = entry.user_id, request_id = %entry.request_id, %err,
                "insert usage log failed");
        }
    }

    /// Records `low_balance_warning` / `balance_depleted` crossings.
    async fn check_balance_events(&self, ctx: &SettleCtx, before: f64, after: f64) {
        for event in balance_events(before, after, self.low_balance_threshold) {
            let entry = BalanceEvent {
                user_id: ctx.user_id,
                amount: 0.0,
                event_type: event.to_owned(),
                reference: ctx.request_id.clone(),
                metadata: json!({
                    "user_id": ctx.user_id,
                    "current_balance": after,
                    "model": ctx.model,
                    "timestamp": Utc::now().to_rfc3339(),
                }),
            };
            if let Err(err) = self.store.insert_balance_event(&entry).await {
                tracing::warn!(user_id = ctx.user_id, event = %event, %err,
                    "write balance event failed");
            }
        }
    }

    /// 用量存储。`GET /v1/usage` 与 [`crate::reconcile`] 共用。
    pub fn store(&self) -> &Arc<dyn UsageStore> {
        &self.store
    }
}

/// `usage_logs.raw_metadata` annotations for a settled row.
///
/// Returns `None` when neither annotation applies, so a precise, fully-paid
/// settlement keeps the column NULL.
pub fn settle_annotations(
    fallback: Option<&'static str>,
    shortfall: f64,
) -> Option<serde_json::Value> {
    if fallback.is_none() && shortfall <= 0.0 {
        return None;
    }
    let mut out = serde_json::Map::new();
    if let Some(reason) = fallback {
        out.insert("billing_fallback".to_owned(), json!({ "reason": reason }));
    }
    if shortfall > 0.0 {
        // Surfaced through the usage log so reporting can tell a free request
        // from a partially-paid one.
        out.insert("shortfall_usd".to_owned(), json!(shortfall));
    }
    Some(serde_json::Value::Object(out))
}

/// Merges `shortfall_usd` into an existing annotation object.
///
/// A [`UsageStore`] implementation calls this INSIDE its settle transaction,
/// once the partial-debit amount is known, so the annotation rule lives here
/// rather than being re-derived in SQL.
pub fn merge_shortfall(
    metadata: Option<serde_json::Value>,
    shortfall: f64,
) -> Option<serde_json::Value> {
    if shortfall <= 0.0 {
        return metadata;
    }
    let mut map = match metadata {
        Some(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    map.insert("shortfall_usd".to_owned(), json!(shortfall));
    Some(serde_json::Value::Object(map))
}

/// Balance-threshold crossings triggered by one settlement.
pub fn balance_events(before: f64, after: f64, threshold: f64) -> Vec<&'static str> {
    let threshold = if threshold > 0.0 {
        threshold
    } else {
        DEFAULT_LOW_BALANCE_THRESHOLD
    };
    let mut events = Vec::new();
    if before >= threshold && after < threshold && after > 0.0 {
        events.push("low_balance_warning");
    }
    if before > 0.0 && after <= 0.0 {
        events.push("balance_depleted");
    }
    events
}

#[cfg(test)]
mod tests;

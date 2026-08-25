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

use gw_pricing::{ObservedUsage, UsageDialect};

use crate::budget_token::BudgetTokenStore;
use crate::ports::{
    BalanceEvent, BillingLedger, SettleReceipt, SettlementCommit, SubscriptionQuotaStore,
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
    /// 冻结报价上的 `estimate(stream = true)`。
    pub streaming_estimate: f64,
}

/// 上游 provider 名 → usage 信封的语义族。
///
/// 这张表在 `gw-proxy`，不在 `gw-relay`：中继层是**计费盲**的，它连
/// `gw-pricing` 都不依赖。
///
/// `gemini` 与 `vertex` 是两套鉴权与端点前缀，但 wire 协议是**同一个**
/// GenerateContent，`usageMetadata` 的字段语义因此完全一致。
///
/// # 未知上游按 OpenAI 线形读
///
/// 不是随手挑的：`openai` / `codex` / `xai` 以及任何「OpenAI 兼容」上游
/// 都报 `completion_tokens`，而那个字段**含**思考 token。把未知上游按
/// Google 读（思考与输出并列）会把它的思考 token 收两遍。
#[must_use]
pub fn usage_dialect(provider: &str) -> UsageDialect {
    match provider {
        "claude" => UsageDialect::Anthropic,
        "gemini" | "vertex" => UsageDialect::Google,
        _ => UsageDialect::OpenAi,
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
///
/// 注意这里**没有计价器**。价格在 Hold 处就冻进了 [`SettleCtx::quote`]，
/// 结算只在那份报价上做算术 —— 拿不到价目表，也就无从二次查价。
pub struct Settlement {
    ledger: Arc<dyn BillingLedger>,
    store: Arc<dyn UsageStore>,
    /// Release 时要把配额那一格还回去。`None` = 这个部署没开配额。
    quota: Option<Arc<dyn SubscriptionQuotaStore>>,
    budget_tokens: Option<Arc<BudgetTokenStore>>,
    low_balance_threshold: f64,
    strict_usage_metadata: AtomicBool,
}

impl Settlement {
    /// Builds the engine over ledger and store.
    pub fn new(ledger: Arc<dyn BillingLedger>, store: Arc<dyn UsageStore>) -> Self {
        Self {
            ledger,
            store,
            quota: None,
            budget_tokens: None,
            low_balance_threshold: DEFAULT_LOW_BALANCE_THRESHOLD,
            strict_usage_metadata: AtomicBool::new(false),
        }
    }

    /// Attach the process-local budget-token store.
    #[must_use]
    pub fn with_budget_tokens(mut self, store: Arc<BudgetTokenStore>) -> Self {
        self.budget_tokens = Some(store);
        self
    }

    /// 挂上配额存储，让 Release 能把在途预留还回去。
    ///
    /// 结算那一支不需要它：转实际发生在 [`UsageStore::commit_settlement`]
    /// 的**同一个事务**里，否则「删预留」和「加 actual」会分家。
    #[must_use]
    pub fn with_quota_store(mut self, store: Arc<dyn SubscriptionQuotaStore>) -> Self {
        self.quota = Some(store);
        self
    }

    /// 只读账本。流式回写包装用它续租约。
    pub fn ledger(&self) -> &Arc<dyn BillingLedger> {
        &self.ledger
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
        // 上游原话，一个字节不动 —— `usage_logs` 写的是它，审计要能和上游
        // 账单对上。字段可以是负数、可以自相矛盾，那都是上游的事实。
        let observed = ObservedUsage {
            input: usage.input_tokens.unwrap_or(0),
            output: usage.output_tokens.unwrap_or(0),
            cached: usage.cached_tokens.unwrap_or(0),
            reasoning: usage.reasoning_tokens.unwrap_or(0),
        };
        // 上游回话里的模型名**只上日志**。它不许改价格键：那等于让上游
        // 决定按什么价收租户的钱（一个不在价目表里的别名会落到兜底价）。
        let model = if usage.model.is_empty() {
            ctx.model.clone()
        } else {
            usage.model.clone()
        };

        // 计价看的是归一化后的**互斥**四列。负数或自相矛盾的信封在这里被
        // 拒绝，于是它和「上游根本没报 usage」走同一条路（fallback / strict），
        // 而不是变成一笔负数扣款或一次凭空少收。
        let billable = outcome.usage.as_ref().and_then(|_| {
            observed
                .normalize(usage_dialect(&outcome.provider))
                .inspect_err(|err| {
                    tracing::warn!(
                        user_id = ctx.user_id,
                        operation = %ctx.operation,
                        provider = %outcome.provider,
                        %err,
                        "upstream usage envelope refused; settling as if it were absent",
                    );
                })
                .ok()
        });
        let computed_cost = billable.map_or(0.0, |usage| ctx.quote.compute(usage).total_cost);

        // The active-hold lookup is only consulted on the fallback path, so it
        // is resolved lazily to keep the precise path at one round-trip.
        let needs_hold_lookup =
            !outcome.failed && billable.is_none() && !self.strict_usage_metadata();
        let active_hold = if needs_hold_lookup {
            match self
                .ledger
                .active_hold_amount(ctx.user_id, &ctx.operation)
                .await
            {
                // No reservation is a definite zero, not an unknown.
                Ok(amount) => Some(amount.unwrap_or(0.0)),
                Err(err) => {
                    tracing::warn!(
                        event = EVENT_HOLD_LOOKUP_FAILED,
                        user_id = ctx.user_id,
                        operation = %ctx.operation,
                        trace_id = %ctx.client_trace,
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
            usage_present: billable.is_some(),
            upstream_failed: outcome.failed,
            strict_mode: self.strict_usage_metadata(),
            active_hold,
            // 兜底估算也来自**冻结的报价**，不是上游那个模型名的现价。
            streaming_estimate: ctx.quote.estimate(true),
        });

        match plan {
            SettlementPlan::Release { reason } => {
                if let Err(err) = self.ledger.release_once(ctx.user_id, &ctx.operation).await {
                    tracing::warn!(user_id = ctx.user_id, operation = %ctx.operation, %err,
                        "ledger release failed");
                }
                // 配额那一格跟着还：一次失败的上游调用不该吃掉订阅的额度。
                self.release_quota(ctx).await;
                let mut entry =
                    self.build_entry(ctx, &outcome, &model, observed, computed_cost, true);
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
                let mut entry = self.build_entry(ctx, &outcome, &model, observed, 0.0, true);
                entry.raw_metadata = Some(json!({
                    "reason": REASON_MISSING_USAGE_STRICT,
                    "timestamp": Utc::now().to_rfc3339(),
                }));
                self.write_log(&entry).await;
            }
            SettlementPlan::HoldLookupFailed => {
                // We cannot bound the cost, and a zero-cost Settle would make
                // the request free. Leave the hold to its TTL, like strict mode.
                let mut entry = self.build_entry(ctx, &outcome, &model, observed, 0.0, true);
                entry.raw_metadata = Some(json!({
                    "event": EVENT_HOLD_LOOKUP_FAILED,
                    "reason": "active hold lookup failed",
                    "timestamp": Utc::now().to_rfc3339(),
                }));
                self.write_log(&entry).await;
            }
            SettlementPlan::Settle { cost, fallback } => {
                self.commit(ctx, &outcome, &model, observed, cost, fallback)
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
        observed: ObservedUsage,
        cost: f64,
        fallback: Option<&'static str>,
    ) {
        let mut entry = self.build_entry(ctx, outcome, model, observed, cost, false);
        // The fallback tag is known now; `shortfall_usd` is not, because the
        // debit happens inside the transaction. The store merges it in there
        // via [`merge_shortfall`], which is why that helper is public.
        entry.raw_metadata = settle_annotations(fallback, 0.0);
        let commit = SettlementCommit {
            user_id: ctx.user_id,
            operation: ctx.operation.clone(),
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
                    operation = %ctx.operation,
                    cost,
                    %err,
                    "settle transaction failed",
                );
                let mut failed = commit.entry.clone();
                failed.failed = true;
                failed.raw_metadata = Some(json!({
                    "reason": err.to_string(),
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
        } = receipt
        else {
            // AlreadyTerminal: some other caller settled or released this
            // operation. It moved no money here, so there is nothing to clear,
            // deduct or announce.
            return;
        };
        if shortfall > 0.0 {
            tracing::warn!(
                user_id = ctx.user_id,
                operation = %ctx.operation,
                shortfall_usd = shortfall,
                "partial debit; shortfall recorded",
            );
        }

        // Post-commit, non-transactional side effects.
        if let Err(err) = self.store.clear_hold(ctx.user_id, &ctx.operation).await {
            tracing::warn!(user_id = ctx.user_id, operation = %ctx.operation, %err,
                "clear reservation failed; it will TTL-expire");
        }
        if let Some(bts) = &self.budget_tokens {
            bts.deduct_settle(ctx.user_id, cost);
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
        observed: ObservedUsage,
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
            request_id: ctx.client_trace.as_str().to_owned(),
            // The money key on the row. Never the empty string it used to be.
            event_key: ctx.operation.to_string(),
            idempotency_key: ctx.idempotency_key.clone(),
            model: model.to_owned(),
            provider,
            auth_id: outcome.auth_id.clone(),
            // 上游原话的四个数。归一化只作用于**计价**，日志不折不减 ——
            // 否则审计就和上游账单对不上了。
            input_tokens: observed.input,
            output_tokens: observed.output,
            cached_tokens: observed.cached,
            reasoning_tokens: observed.reasoning,
            total_cost: cost,
            actual_cost: cost,
            cost,
            rate_multiplier: ctx.rate_mult(),
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
                reference: ctx.operation.to_string(),
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

    /// 还掉这次操作的配额预留。删一行不存在的预留是成功，所以这里不需要
    /// 记住「刚才到底留没留」。
    async fn release_quota(&self, ctx: &SettleCtx) {
        let Some(store) = &self.quota else {
            return;
        };
        if let Err(err) = store.release_reservation(&ctx.operation).await {
            tracing::warn!(user_id = ctx.user_id, operation = %ctx.operation, %err,
                "quota reservation release failed");
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

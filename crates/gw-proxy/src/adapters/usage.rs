//! [`UsageStore`] over Postgres — the atomic settle.
//!
//! The settle transaction plus hold clearing. One transaction carries three
//! writes that must land together:
//!
//! 1. the balance debit (`Ledger::settle_tx`, which also records any shortfall),
//! 2. the `usage_logs` row,
//! 3. the subscription counter accumulation.
//!
//! The Redis reservation is cleared **only after that transaction commits**.
//! Clearing it earlier — which a nested standalone settle would do — reopens
//! the divergence window: the balance moves, the outer write fails, and the
//! tenant is charged with no usage row to show for it. On
//! rollback the hold is deliberately left in place so the request stays
//! reconcilable.

use async_trait::async_trait;
use gw_infra::Db;
use gw_ledger::Ledger;
use std::sync::Arc;

use crate::ports::{BalanceEvent, Id, SettleReceipt, SettlementCommit, UsageLogEntry, UsageStore};
use crate::usage::merge_shortfall;

/// Transactional settlement writes.
#[derive(Debug, Clone)]
pub struct SqlUsageStore {
    db: Db,
    ledger: Arc<Ledger>,
}

impl SqlUsageStore {
    /// `db` must be the same pool the ledger writes through, or `settle_tx`
    /// would run in a transaction on a different connection and the atomicity
    /// this type exists for would be a fiction.
    pub fn new(db: Db, ledger: Arc<Ledger>) -> Self {
        Self { db, ledger }
    }
}

#[async_trait]
impl UsageStore for SqlUsageStore {
    async fn commit_settlement(&self, commit: &SettlementCommit) -> anyhow::Result<SettleReceipt> {
        let mut tx = self.db.begin().await?;

        // The reconcile path's idempotency guard. Re-checked HERE, inside the
        // transaction, rather than by the caller beforehand: two reconcilers
        // racing on the same orphaned hold would both see "no row" outside it
        // and both charge.
        if commit.skip_if_already_logged {
            // `(i32,)`, not `(i64,)`: the literal `1` is INT4, and sqlx checks
            // the wire type rather than widening.
            let existing: Option<(i32,)> =
                sqlx::query_as("SELECT 1 FROM usage_logs WHERE request_id = $1 LIMIT 1")
                    .bind(&commit.request_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if existing.is_some() {
                tx.rollback().await?;
                return Ok(SettleReceipt::AlreadySettled);
            }
        }

        let outcome = self
            .ledger
            .settle_tx(
                &mut tx,
                commit.user_id,
                &commit.request_id,
                commit.actual_cost,
            )
            .await?;

        // Read the balance back inside the same transaction so it reflects the
        // debit above; the pre-settle figure is what the balance actually lost.
        //
        // `compat::Money`, not `f64`: every money column is `numeric`,
        // and sqlx will not decode that into `f64` — it errors at runtime, not
        // at compile time, so this is only visible against a real database.
        let balance_after: f64 = sqlx::query_as::<_, (gw_model::compat::Money,)>(
            "SELECT balance FROM users WHERE id = $1",
        )
        .bind(commit.user_id)
        .fetch_optional(&mut *tx)
        .await?
        .map_or(0.0, |(balance,)| balance.0);
        let balance_before = balance_after + outcome.debited;

        // The shortfall is only knowable now, so the annotation is folded in
        // here rather than by the caller — and through the crate's own helper,
        // so a partially-paid request stays distinguishable from a free one.
        let mut entry = commit.entry.clone();
        entry.raw_metadata = merge_shortfall(entry.raw_metadata, outcome.shortfall);
        insert_usage_log(&mut tx, &entry).await?;

        // Accumulate quota only for a real charge against a live subscription.
        // A lapsed or cancelled one is filtered in the predicate rather than
        // read first, so the check and the update cannot race.
        if let Some(subscription_id) = commit.subscription_id
            && subscription_id != 0
            && commit.actual_cost > 0.0
        {
            sqlx::query(
                // The cast is explicit so the addition happens in `numeric`.
                // Without it Postgres resolves `numeric + float8` by widening
                // the column to binary float, which is the one arithmetic this
                // codebase must not do to money.
                "UPDATE subscriptions SET \
                    daily_usage_usd = daily_usage_usd + CAST($2 AS numeric), \
                    weekly_usage_usd = weekly_usage_usd + CAST($2 AS numeric), \
                    monthly_usage_usd = monthly_usage_usd + CAST($2 AS numeric), \
                    updated_at = NOW() \
                 WHERE id = $1 AND status = 'active' AND expires_at > NOW()",
            )
            .bind(subscription_id)
            .bind(commit.actual_cost)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        Ok(SettleReceipt::Committed {
            shortfall: outcome.shortfall,
            balance_before,
            balance_after,
        })
    }

    async fn insert_usage_log(&self, entry: &UsageLogEntry) -> anyhow::Result<()> {
        let mut conn = self.db.acquire().await?;
        insert_usage_log(&mut conn, entry).await
    }

    async fn insert_balance_event(&self, event: &BalanceEvent) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, metadata, created_at) \
             VALUES ($1, $2, $3, $4, $5, NOW())",
        )
        .bind(event.user_id)
        .bind(event.amount)
        .bind(&event.event_type)
        .bind(&event.reference)
        .bind(&event.metadata)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    async fn clear_hold(&self, user_id: Id, request_id: &str) -> anyhow::Result<()> {
        Ok(self.ledger.clear_hold(user_id, request_id).await?)
    }
}

/// The `usage_logs` insert, shared by the transactional and standalone paths.
///
/// Every column is listed rather than relying on defaults, because the table's
/// `NOT NULL`s do not all carry one. `input_cost` / `output_cost` stay zero: it
/// was never populated — the itemised split is not what gets debited.
async fn insert_usage_log(
    conn: &mut sqlx::PgConnection,
    entry: &UsageLogEntry,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO usage_logs ( \
            user_id, api_key_id, group_id, request_id, idempotency_key, event_key, \
            model, provider, auth_id, \
            tokens_in, tokens_out, input_tokens, output_tokens, reasoning_tokens, cached_tokens, \
            input_cost, output_cost, total_cost, actual_cost, cost, \
            rate_multiplier, stream, duration_ms, ip_address, raw_metadata, failed, created_at \
         ) VALUES ( \
            $1, $2, $3, $4, $5, '', \
            $6, $7, $8, \
            $9, $10, $9, $10, $11, $12, \
            0, 0, $13, $14, $15, \
            $16, $17, $18, $19, $20, $21, NOW() \
         )",
    )
    .bind(entry.user_id)
    .bind(entry.api_key_id)
    .bind(entry.group_id)
    .bind(&entry.request_id)
    .bind(&entry.idempotency_key)
    .bind(&entry.model)
    .bind(&entry.provider)
    .bind(&entry.auth_id)
    // tokens_in / input_tokens and tokens_out / output_tokens are the same
    // numbers under the legacy and current column names; both are written.
    .bind(entry.input_tokens)
    .bind(entry.output_tokens)
    .bind(entry.reasoning_tokens)
    .bind(entry.cached_tokens)
    .bind(entry.total_cost)
    .bind(entry.actual_cost)
    .bind(entry.cost)
    .bind(entry.rate_multiplier)
    .bind(entry.stream)
    .bind(entry.duration_ms)
    .bind(&entry.ip_address)
    .bind(&entry.raw_metadata)
    .bind(entry.failed)
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;

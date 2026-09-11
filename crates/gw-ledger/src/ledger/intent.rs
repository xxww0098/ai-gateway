//! Durable settlement intents: pending holds, settle CAS, and SQL recovery.

use std::time::Duration;

use sqlx::PgConnection;

use super::Ledger;
use crate::LedgerError;

/// A hold that outlived Redis and is still waiting for settle or release.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingIntent {
    pub user_id: i64,
    pub request_id: String,
    pub hold_amount: f64,
}

/// 崩溃恢复扫描的**生产原文**。
///
/// 0014 的 idx_settlement_intents_pending_created 是它的部分索引：表里终态行
/// 占绝对多数、且从建表起就没有任何删除路径，所以没有索引时这就是一次全表
/// 并行顺序扫描（150 万行实测 58ms，并随部署时长线性增长）。
///
/// 与门禁 SQL 同理，这条语句也被 tests/postgres_ledger/plan.rs 直接 EXPLAIN，
/// 公开只是为了让那个回归测试盯住生产原文，而不是它的抄件。$1 = 宽限秒数。
pub const PENDING_INTENT_SCAN_SQL: &str = "\
SELECT user_id, request_id, COALESCE(hold_amount, 0)::float8 \
FROM settlement_intents \
WHERE status = 'pending' AND created_at < NOW() - ($1 * INTERVAL '1 second')";

impl Ledger {
    /// Records a Redis hold in Postgres so recovery does not depend on key TTL.
    pub async fn record_pending_hold(
        &self,
        user_id: i64,
        request_id: &str,
        hold_amount: f64,
    ) -> Result<(), LedgerError> {
        if request_id.is_empty() {
            return Err(LedgerError::InvalidArgument("requestID is required"));
        }
        let inserted: Option<String> = sqlx::query_scalar(
            "INSERT INTO settlement_intents (request_id, user_id, hold_amount, status) VALUES ($1, $2, $3, 'pending') ON CONFLICT (request_id) DO NOTHING RETURNING request_id",
        )
        .bind(request_id)
        .bind(user_id)
        .bind(hold_amount)
        .fetch_optional(&self.db)
        .await?;
        if inserted.is_some() {
            return Ok(());
        }
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM settlement_intents WHERE request_id = $1")
                .bind(request_id)
                .fetch_optional(&self.db)
                .await?;
        match status.as_deref() {
            Some("pending") => Ok(()),
            _ => Err(LedgerError::AlreadySettled),
        }
    }

    /// Marks a still-pending intent released. Missing or terminal rows are no-ops.
    pub async fn release_pending_hold(&self, request_id: &str) -> Result<(), LedgerError> {
        if request_id.is_empty() {
            return Err(LedgerError::InvalidArgument("requestID is required"));
        }
        sqlx::query(
            "UPDATE settlement_intents SET status = 'released' WHERE request_id = $1 AND status = 'pending'",
        )
        .bind(request_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Pending intents older than `older_than`, the recovery set for reconcile.
    pub async fn list_pending_intents(
        &self,
        older_than: Duration,
    ) -> Result<Vec<PendingIntent>, LedgerError> {
        let rows: Vec<(i64, String, f64)> = sqlx::query_as(PENDING_INTENT_SCAN_SQL)
            .bind(older_than.as_secs() as i64)
            .fetch_all(&self.db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(user_id, request_id, hold_amount)| PendingIntent {
                user_id,
                request_id,
                hold_amount,
            })
            .collect())
    }

    /// Occupies `request_id` as settled. A second caller must not debit.
    pub(super) async fn claim_settlement_intent(
        conn: &mut PgConnection,
        request_id: &str,
        user_id: i64,
        hold_amount: f64,
    ) -> Result<(), LedgerError> {
        let claimed: Option<String> = sqlx::query_scalar(
            "INSERT INTO settlement_intents (request_id, user_id, hold_amount, status, settled_at) VALUES ($1, $2, $3, 'settled', NOW()) ON CONFLICT (request_id) DO UPDATE SET status = 'settled', settled_at = NOW() WHERE settlement_intents.status = 'pending' RETURNING request_id",
        )
        .bind(request_id)
        .bind(user_id)
        .bind(hold_amount)
        .fetch_optional(&mut *conn)
        .await?;
        if claimed.is_none() {
            return Err(LedgerError::AlreadySettled);
        }
        Ok(())
    }
}

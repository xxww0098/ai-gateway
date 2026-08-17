//! Orphan-hold detection.

use std::collections::HashMap;
use std::time::Duration;

use crate::keys::{self, HOLDS_SCAN_PATTERN};
use crate::{Ledger, LedgerError};

/// A Redis reservation that has outlived a normal request.
///
/// Settle and release both clear a hold promptly, so a hold still present well
/// past its TTL means the owning request crashed (or leaked) before settling —
/// the crash window that synchronous deferred settlement cannot itself close.
/// Surfacing these drives the `agw_orphaned_holds` gauge and, later,
/// supervised remediation.
#[derive(Debug, Clone, PartialEq)]
pub struct StaleHold {
    pub user_id: i64,
    pub request_id: String,
    pub amount: f64,
    /// Seconds between the hold's creation timestamp and the scan.
    pub age_seconds: i64,
}

impl Ledger {
    /// Returns holds whose creation timestamp is older than `older_than`.
    ///
    /// Read-only: it never settles or releases, so it is safe to run anywhere
    /// (a startup check, a periodic ops scan). `older_than` should exceed the
    /// longest expected request duration so live long-running requests are not
    /// flagged.
    ///
    /// One deliberate difference from a naive iterator: the `SCAN` cursor is
    /// drained into a key list *before* any per-user reads, because a single
    /// multiplexed connection cannot interleave an open cursor with other
    /// commands. The result is identical; only the memory profile differs,
    /// and it is bounded by the number of users with holds.
    ///
    /// A per-key read failure skips that key rather than aborting the scan, so
    /// one unreadable user cannot hide every other stale hold.
    ///
    /// # Errors
    /// Propagates a failure of the `SCAN` itself. Returns an empty vector
    /// when no Redis is configured.
    pub async fn scan_stale_holds(
        &self,
        older_than: Duration,
    ) -> Result<Vec<StaleHold>, LedgerError> {
        let Some(mut conn) = self.redis_conn() else {
            return Ok(Vec::new());
        };
        let now = Self::now_unix();
        let cutoff = now - older_than.as_secs() as i64;

        let mut hold_keys: Vec<String> = Vec::new();
        let mut cursor: u64 = 0;
        loop {
            let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(HOLDS_SCAN_PATTERN)
                .arg("COUNT")
                .arg(200)
                .query_async(&mut conn)
                .await?;
            hold_keys.extend(batch);
            if next == 0 {
                break;
            }
            cursor = next;
        }

        let mut out = Vec::new();
        for key in hold_keys {
            // Skips the companion timestamp hashes, which share the prefix.
            let Some(user_id) = keys::user_id_from_holds_key(&key) else {
                continue;
            };
            let Ok(members) = redis::cmd("ZRANGE")
                .arg(&key)
                .arg(0)
                .arg(-1)
                .arg("WITHSCORES")
                .query_async::<Vec<(String, f64)>>(&mut conn)
                .await
            else {
                continue;
            };
            let Ok(timestamps) = redis::cmd("HGETALL")
                .arg(keys::holds_ts_key(user_id))
                .query_async::<HashMap<String, String>>(&mut conn)
                .await
            else {
                continue;
            };
            out.extend(collect_stale(user_id, &members, &timestamps, cutoff, now));
        }
        Ok(out)
    }
}

/// Pairs one user's hold members with their creation timestamps and keeps the
/// ones older than `cutoff`.
///
/// A member is skipped when its timestamp is missing, unparseable, or zero —
/// an unknown age is not evidence of staleness, and treating it as stale would
/// have a reconciler chase live requests. The `ts == 0` guard keeps a
/// zeroed timestamp from ever registering as stale.
fn collect_stale(
    user_id: i64,
    members: &[(String, f64)],
    timestamps: &HashMap<String, String>,
    cutoff: i64,
    now: i64,
) -> Vec<StaleHold> {
    members
        .iter()
        .filter_map(|(request_id, amount)| {
            let ts = timestamps.get(request_id)?.parse::<i64>().ok()?;
            if ts == 0 || ts >= cutoff {
                return None;
            }
            Some(StaleHold {
                user_id,
                request_id: request_id.clone(),
                amount: *amount,
                age_seconds: now - ts,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;

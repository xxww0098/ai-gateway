//! The two hot-path billing queries must stay served by their partial
//! indexes.
//!
//! `0014` adds two partial indexes whose *only* consumer is a predicate the
//! planner has to recognise syntactically. If someone rewrites
//! `UNRESOLVED_DEBT_PREDICATE` back into a shape the index predicate cannot
//! imply, every behavioural test stays green — the result set is identical —
//! and the query silently goes back to scanning a tenant's whole journal.
//! Nothing but a plan assertion catches that.
//!
//! `enable_seqscan = off` is the point, not a trick: it asks "is there an
//! index path at all for this query?", which is exactly the property the
//! predicate rewrite would destroy. Plan *shape* on a tiny fixture table is
//! otherwise a planner preference, and asserting on it would be brittle.

use super::common::Fixture;
use sqlx::Executor;

/// `EXPLAIN` for `sql` with its `$n` placeholders bound to `args`, on a
/// session where sequential scans carry the planner's disable penalty.
///
/// Parameters go through `PREPARE` / `EXPLAIN EXECUTE` rather than string
/// substitution, so the statement the planner sees is byte-for-byte the
/// production text.
async fn ordered_plan(fx: &Fixture, sql: &str, args: &[&str]) -> String {
    let mut conn = fx.pool.acquire().await.expect("acquire a connection");
    // A pooled connection may still hold a statement from an earlier test.
    conn.execute("DEALLOCATE ALL")
        .await
        .expect("clear statements left on the pooled connection");
    conn.execute("SET enable_seqscan = off")
        .await
        .expect("penalise sequential scans for this session");
    conn.execute(format!("PREPARE plan_probe AS {sql}").as_str())
        .await
        .expect("prepare the production SQL");
    let lines: Vec<String> = sqlx::query_scalar(&format!(
        "EXPLAIN EXECUTE plan_probe({})",
        args.join(", ")
    ))
    .fetch_all(&mut *conn)
    .await
    .expect("explain the prepared statement");
    conn.execute("DEALLOCATE plan_probe")
        .await
        .expect("release the prepared statement");
    lines.join("\n")
}

/// The shortfall gate on the request path (`has_unresolved_shortfall`, run by
/// the hold middleware and the panel's subscription purchase) must reach
/// `idx_balance_logs_shortfall_open` instead of walking a tenant's journal.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn the_shortfall_gate_is_served_by_its_partial_index() {
    let fx = Fixture::postgres_only().await;
    let plan = ordered_plan(
        &fx,
        &gw_ledger::shortfall_gate_sql(),
        &["'settle'", "'credit'", "0"],
    )
    .await;

    assert!(
        plan.contains("idx_balance_logs_shortfall_open"),
        "欠款门禁没有走到它的部分索引；谓词大概率被改成了无法蕴含索引谓词的形状。\n{plan}",
    );
}

/// The crash-recovery scan must reach
/// `idx_settlement_intents_pending_created`. The table gains a row per
/// billable request and never loses one, so the un-indexed plan degrades with
/// deployment age rather than with load.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn the_recovery_scan_is_served_by_its_partial_index() {
    let fx = Fixture::postgres_only().await;
    let plan = ordered_plan(&fx, gw_ledger::PENDING_INTENT_SCAN_SQL, &["300"]).await;

    assert!(
        plan.contains("idx_settlement_intents_pending_created"),
        "恢复扫描没有走到它的部分索引。\n{plan}",
    );
}

//! 离线的形状检查：不连库，只盯「两处判定共用同一份谓词」和「SQL 拼出来的
//! reference 与 Rust 侧的构造函数同源」这两条会静默漂移的性质。

use super::{UNRESOLVED_DEBT_PREDICATE, WRITE_OFF_SQL, count_sql, exists_sql, page_sql};
use crate::shortfall_resolve_reference;

/// 门禁、总数、分页三条查询必须嵌的是**同一份**债务侧谓词。
/// 谁把它复制一份再改一处，都会造出「列表里没有、门却锁着」的状态。
#[test]
fn every_debt_query_embeds_the_one_predicate() {
    for sql in [exists_sql(), count_sql(), page_sql()] {
        assert!(
            sql.contains(UNRESOLVED_DEBT_PREDICATE),
            "查询没有嵌入共用谓词：{sql}"
        );
    }
}

/// SQL 里拼接的注销 reference 前缀，与 [`shortfall_resolve_reference`] 生成的
/// 前缀必须一致 —— 期望值从构造函数里推出来，不是抄回源码里的字面量。
#[test]
fn the_sql_builds_the_same_resolve_reference_prefix() {
    let built = shortfall_resolve_reference("REQ", 1);
    let prefix = built.split_once("REQ").expect("request id 出现在引用里").0;
    let quoted = format!("'{prefix}'");

    assert!(WRITE_OFF_SQL.contains(&quoted), "注销语句用了别的前缀");
    assert!(
        UNRESOLVED_DEBT_PREDICATE.contains(&quoted),
        "判定谓词用了别的前缀"
    );
}

/// A pool that would fail on first use, so a passing test has provably never
/// reached Postgres — the same property the `gw-proxy` directory tests pin.
fn unreachable_db() -> sqlx::PgPool {
    use std::str::FromStr as _;
    let opts = sqlx::postgres::PgConnectOptions::from_str("postgres://127.0.0.1:1/nonexistent")
        .expect("a syntactically valid DSN");
    sqlx::postgres::PgPoolOptions::new()
        // The error path is part of the assertion; fail it in milliseconds
        // instead of the 30-second acquire default.
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy_with(opts)
}

/// 一条新鲜的「干净」判定直接放行，整个调用不碰 Postgres；
/// 而 settle 侧记下欠款后，同一个用户的下一次判定必须回到真查询
/// （这里表现为对不可达库的报错），缓存不许替它挡下来。
#[tokio::test]
async fn a_fresh_clean_verdict_skips_postgres_until_a_debt_drops_it() {
    let ledger = crate::Ledger::new(unreachable_db(), None);

    ledger.note_shortfall_verdict(7, false);
    assert_eq!(
        ledger.has_unresolved_shortfall(7).await.ok(),
        Some(false),
        "缓存的干净判定必须放行且不触库"
    );

    ledger.note_shortfall_verdict(7, true);
    assert!(
        ledger.has_unresolved_shortfall(7).await.is_err(),
        "欠款掐掉缓存后必须重新触库（此处即对不可达库报错）"
    );
}

/// 别的用户的欠款不许掐掉这个用户的干净判定。
#[tokio::test]
async fn a_debt_only_drops_its_own_users_verdict() {
    let ledger = crate::Ledger::new(unreachable_db(), None);

    ledger.note_shortfall_verdict(7, false);
    ledger.note_shortfall_verdict(8, true);
    assert_eq!(ledger.has_unresolved_shortfall(7).await.ok(), Some(false));
}
